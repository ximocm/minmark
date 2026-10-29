//! Diagramas DOT (Graphviz) → SVG para Preview y PDF (ADR-007, §7.3.1).
//!
//! Envuelve `layout-rs` (MIT, Rust puro: parser DOT + layout + backend SVG)
//! para que el resto del workspace no conozca el motor. Sin red, sin binarios
//! externos y sin motor de JavaScript: el SVG resultante se embebe igual en
//! el HTML de Preview que como recurso del PDF (paridad obligatoria).
//!
//! Patrón de errores ADR-002/NFR-026: nunca silenciar ni deformar — un DOT
//! inválido (o un fallo del motor) devuelve el marcador visible
//! `⚠ [diagrama no válido: …]` para que las superficies lo muestren y el
//! fuente siga visible y editable (§8.4).

use layout::backends::svg::SVGWriter;
use layout::gv::{DotParser, GraphBuilder};

/// Renderiza un bloque DOT a SVG.
///
/// `Ok(svg)` = diagrama renderizado. `Err(marcador)` = mensaje visible ya
/// formateado (`⚠ [diagrama no válido: …]`) con el detalle del error.
pub fn render_dot(source: &str) -> Result<String, String> {
    let result = std::panic::catch_unwind(|| layout(source));
    match result {
        Ok(rendered) => rendered,
        // Un fallo del motor nunca cierra ni bloquea: se informa igual que un
        // error de sintaxis (ADR-002).
        Err(_) => Err(marker("el motor de layout falló con este contenido")),
    }
}

fn layout(source: &str) -> Result<String, String> {
    let mut parser = DotParser::new(source);
    let graph = parser.process().map_err(|error| marker(&error))?;
    validate(&graph).map_err(|error| marker(&error))?;
    let mut builder = GraphBuilder::new();
    builder.visit_graph(&graph);
    let mut visual = builder.get();
    let mut backend = SVGWriter::new();
    visual.do_it(false, false, false, &mut backend);
    Ok(backend.finalize())
}

/// Valida el subconjunto de DOT que el motor aplica sin deformar (ADR-007):
/// un atributo o valor fuera de este conjunto produce el marcador visible en
/// lugar de un diagrama alterado en silencio (p. ej. `shape=diamond` o
/// `rankdir=BT`, que el motor sustituiría sin avisar).
fn validate(graph: &layout::gv::parser::ast::Graph) -> Result<(), String> {
    use layout::gv::parser::ast::{AttributeList, Stmt, StmtList};

    fn attrs(list: &AttributeList) -> Result<(), String> {
        for (key, value) in list.iter() {
            match key.as_str() {
                "label" | "color" | "fillcolor" | "fontsize" | "penwidth" | "width" => {}
                "shape" => match value.as_str() {
                    "box" | "doublecircle" | "record" | "Mrecord" | "ellipse" | "circle" => {}
                    other => return Err(format!("shape «{other}» no soportado (§7.3.1)")),
                },
                "rankdir" => match value.as_str() {
                    "LR" | "TB" => {}
                    other => return Err(format!("rankdir «{other}» no soportado (§7.3.1)")),
                },
                "style" => match value.as_str() {
                    "solid" | "dashed" | "filled" => {}
                    other => return Err(format!("style «{other}» no soportado (§7.3.1)")),
                },
                other => return Err(format!("atributo «{other}» no soportado (§7.3.1)")),
            }
        }
        Ok(())
    }

    fn stmts(list: &StmtList) -> Result<(), String> {
        for stmt in &list.list {
            match stmt {
                Stmt::Edge(edge) => attrs(&edge.list)?,
                Stmt::Node(node) => attrs(&node.list)?,
                Stmt::Attribute(attribute) => attrs(&attribute.list)?,
                Stmt::SubGraph(sub) => {
                    // Los clusters se dibujarían como si no existieran (el
                    // motor no pinta su caja ni sus rangos): deformación.
                    if sub.name.starts_with("cluster") {
                        return Err(format!(
                            "subgrafo «{}» (cluster) no soportado (§7.3.1)",
                            sub.name
                        ));
                    }
                    stmts(&sub.list)?;
                }
            }
        }
        Ok(())
    }

    stmts(&graph.list)
}

/// Marcador visible de diagrama no válido (§7.3.1, ADR-002).
fn marker(error: &str) -> String {
    let detail: String = error.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("⚠ [diagrama no válido: {detail}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digraph_sencilla_renderiza_svg() {
        let svg = render_dot("digraph { a -> b -> c; }").expect("render");
        assert!(svg.contains("<svg"), "sin elemento svg: {}", &svg[..60.min(svg.len())]);
        assert!(svg.contains("</svg>"), "svg sin cerrar");
    }

    #[test]
    fn grafo_no_dirigido_y_atributos() {
        let svg = render_dot("graph { rankdir=LR; a -- b [label=\"x\"]; }").expect("render");
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn etiquetas_unicode() {
        let svg = render_dot("digraph { \"niño\" -> \"señal\"; }").expect("render");
        assert!(svg.contains("niño"));
    }

    #[test]
    fn dot_invalido_da_marcador_visible() {
        let error = render_dot("digraph { a = ; }").expect_err("debe fallar");
        assert!(error.starts_with("⚠ [diagrama no válido:"), "{error}");
    }

    #[test]
    fn atributos_no_soportados_no_se_deforman_en_silencio() {
        // ADR-007: el motor sustituiría estos valores sin avisar (diamond →
        // elipse, rankdir=BT → TB); el marcador es el contrato.
        for dot in [
            "digraph { a [shape=diamond]; }",
            "digraph { rankdir=BT; a -> b; }",
            "digraph { a [fontcolor=red]; }",
            "digraph { a [style=rounded]; }",
            "digraph { a [style=\"filled,dashed\"]; }",
        ] {
            let error = render_dot(dot).expect_err(&format!("debe fallar: {dot}"));
            assert!(error.starts_with("⚠ [diagrama no válido:"), "{error}");
        }
    }

    #[test]
    fn clusters_no_soportados_producen_marcador() {
        let error = render_dot("digraph { subgraph cluster_0 { a; b; } a -> b; }")
            .expect_err("debe fallar");
        assert!(error.contains("cluster"), "{error}");
        // Un subgrafo sin prefijo cluster es agrupación plana: sí vale.
        assert!(render_dot("digraph { subgraph { a; b; } a -> b; }").is_ok());
    }

    #[test]
    fn atributos_soportados_siguen_renderizando() {
        let svg = render_dot(
            "digraph { rankdir=LR; a [shape=box, style=filled, fillcolor=lightblue, label=\"A\"]; a -> b [style=dashed, color=gray, penwidth=2]; }",
        )
        .expect("render");
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn contenido_hostil_no_entra_en_panico() {
        // Ni la entrada vacía ni la basura cierran el proceso: siempre SVG o
        // marcador visible, nunca un pánico.
        assert!(render_dot("").is_err());
        assert!(render_dot("esto no es dot").is_err());
        assert!(render_dot("digraph { a -> -> b; }").is_err());
    }

    #[test]
    fn diagrama_mediano_es_rapido() {
        // §56-Q6: el layout corre en cada render; con diagramas de tamaño
        // normal debe ser imperceptible.
        let mut source = String::from("digraph {");
        for i in 0..100 {
            source.push_str(&format!("n{i} -> n{};", i + 1));
        }
        source.push('}');
        let start = std::time::Instant::now();
        let svg = render_dot(&source).expect("render");
        assert!(svg.contains("<svg"));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
