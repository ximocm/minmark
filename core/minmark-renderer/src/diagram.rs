//! SVG de diagramas DOT con los glifos resueltos a contornos (ADR-007).
//!
//! Paridad Preview/PDF obligatoria: el texto se convierte en trayectorias
//! con las Liberation Sans empaquetadas (§33), de modo que ambas superficies
//! dibujan exactamente los mismos glifos, independientes de las fuentes del
//! host y del `body_font` del documento.

use std::sync::Arc;

/// SVG del diagrama DOT para Preview y PDF (§7.3.1).
///
/// `Err` = marcador visible `⚠ [diagrama no válido: …]` (patrón ADR-002:
/// nunca silenciar ni deformar).
pub fn diagram_svg(dot: &str) -> Result<String, String> {
    let svg = minmark_diagram::render_dot(dot)?;
    outline_text(&svg)
}

/// Convierte el texto del SVG en contornos con las fuentes empaquetadas:
/// el resultado no contiene elementos `<text>` ni depende de ninguna fuente
/// externa. Las familias genéricas se fijan a Liberation Sans (§33) para que
/// la selección sea determinista; las tipografías de `typst-assets` amplían
/// la cobertura igual que en el world del PDF.
fn outline_text(svg: &str) -> Result<String, String> {
    let mut fonts = fontdb::Database::new();
    for data in crate::world::SANS_FONTS {
        fonts.load_font_data(data.to_vec());
    }
    for data in typst_assets::fonts() {
        fonts.load_font_data(data.to_vec());
    }
    fonts.set_serif_family("Liberation Sans");
    fonts.set_sans_serif_family("Liberation Sans");
    fonts.set_cursive_family("Liberation Sans");
    fonts.set_fantasy_family("Liberation Sans");
    fonts.set_monospace_family("Liberation Sans");
    let options = usvg::Options {
        fontdb: Arc::new(fonts),
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(svg, &options).map_err(|error| marker(&error.to_string()))?;
    // `WriteOptions::preserve_text = false` (por defecto): el writer emite el
    // texto ya convertido en trayectorias.
    Ok(tree.to_string(&usvg::WriteOptions::default()))
}

fn marker(detail: &str) -> String {
    let detail: String = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("⚠ [diagrama no válido: {detail}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_glifos_salen_como_contornos() {
        // Paridad: sin <text> ni familias de fuente, el SVG dibuja igual en
        // Preview que en el PDF.
        let svg = diagram_svg("digraph { a -> \"niño\"; }").expect("render");
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("<text"), "el texto debe salir aplanado: {svg}");
        assert!(
            !svg.contains("font-family"),
            "sin familias de fuente externas: {svg}"
        );
    }

    #[test]
    fn el_render_es_determinista() {
        let a = diagram_svg("digraph { a -> b; }").expect("render");
        let b = diagram_svg("digraph { a -> b; }").expect("render");
        assert_eq!(a, b);
    }

    #[test]
    fn xml_invalido_no_se_convierte_en_svg_roto() {
        // Un carácter XML inválido en el DOT no puede producir un SVG que
        // rompa el PDF: cae al marcador visible.
        let error = diagram_svg("digraph { \"a\u{1}\" -> b; }").expect_err("debe fallar");
        assert!(error.starts_with("⚠ [diagrama no válido:"), "{error}");
    }
}
