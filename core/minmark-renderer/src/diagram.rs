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
    let mut svg = tree.to_string(&usvg::WriteOptions::default());
    // El writer de usvg emite solo `width`/`height` en la raíz. Sin `viewBox`
    // el CSS de Preview (`.mm-diagram svg { max-width: 100% }`) reduce la caja
    // pero no el dibujo y el diagrama se sale del área visible (§7.3.1,
    // ADR-007: paridad Preview/PDF — el PDF sí escala por width/height). El
    // viewBox va a origen 0 con el tamaño del árbol: usvg ya hornea en los
    // nodos la transformación del viewBox original. Si la raíz ya lo trae, no
    // se inyecta: un atributo duplicado daría XML mal formado para el PDF.
    let root_end = svg.find('>').map_or(svg.len(), |end| end + 1);
    if !svg[..root_end].contains("viewBox=") {
        let size = tree.size();
        let view_box = format!("viewBox=\"0 0 {} {}\" ", size.width(), size.height());
        svg = svg.replacen("<svg ", &format!("<svg {view_box}"), 1);
    }
    Ok(svg)
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
        assert!(
            !svg.contains("<text"),
            "el texto debe salir aplanado: {svg}"
        );
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
    fn el_svg_conserva_el_viewbox_para_poder_escalar() {
        // Sin viewBox en la raíz, el CSS de Preview (`.mm-diagram svg {
        // max-width: 100%; height: auto }`) reduce la caja del SVG pero no el
        // dibujo y el diagrama se sale del área visible, mientras que el PDF
        // sí lo escala: se rompe la paridad de ADR-007 (§7.3.1).
        let svg = diagram_svg("digraph { rankdir=LR; a [shape=box]; a -> b; }").expect("render");
        let root = svg.split('>').next().expect("raíz del svg");
        let attr = |name: &str| -> Option<f64> {
            root.split(&format!("{name}=\""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .and_then(|value| value.parse().ok())
        };
        let (Some(w), Some(h)) = (attr("width"), attr("height")) else {
            panic!("raíz sin width/height: {root}");
        };
        let nums: Vec<f64> = root
            .split("viewBox=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .map(|vb| {
                vb.split_whitespace()
                    .filter_map(|n| n.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(nums.len(), 4, "sin viewBox válido en la raíz: {root}");
        assert_eq!(
            root.matches("viewBox=\"").count(),
            1,
            "viewBox duplicado en la raíz: {root}"
        );
        assert_eq!(
            (nums[0], nums[1]),
            (0.0, 0.0),
            "viewBox sin origen 0: {root}"
        );
        assert!(
            (nums[2] - w).abs() < 0.01 && (nums[3] - h).abs() < 0.01,
            "viewBox desalineado con el tamaño del SVG: {root}"
        );
    }

    #[test]
    fn xml_invalido_no_se_convierte_en_svg_roto() {
        // Un carácter XML inválido en el DOT no puede producir un SVG que
        // rompa el PDF: cae al marcador visible.
        let error = diagram_svg("digraph { \"a\u{1}\" -> b; }").expect_err("debe fallar");
        assert!(error.starts_with("⚠ [diagrama no válido:"), "{error}");
    }
}
