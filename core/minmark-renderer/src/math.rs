//! Composición local con el mismo Typst y las mismas fuentes del PDF.
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
type MathCache = BTreeMap<String, Result<String, String>>;
static CACHE: OnceLock<Mutex<MathCache>> = OnceLock::new();

pub fn math_svg(latex: &str) -> Result<String, String> {
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(value) = cache.lock().expect("math cache").get(latex) {
        return value.clone();
    }
    let translation = minmark_math::translate(latex);
    let value = if translation.is_supported() {
        let source = format!("#set page(width: auto, height: auto, margin: 0pt, fill: none)\n#set text(font: \"New Computer Modern\", size: 12pt)\n$\n{}\n$", translation.typst);
        let world = crate::world::MiniWorld::new(source, BTreeMap::new());
        typst::compile::<typst::layout::PagedDocument>(&world)
            .output
            .map(|document| typst_svg::svg(&document.pages[0]))
            .map_err(|errors| {
                format!(
                    "⚠ [math inválido: {}]",
                    errors
                        .iter()
                        .map(|e| e.message.to_string())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })
    } else {
        Err(translation.marker())
    };
    let mut cache = cache.lock().expect("math cache");
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(latex.to_owned(), value.clone());
    value
}

pub fn validated_math(latex: &str) -> Result<String, String> {
    math_svg(latex).map(|_| minmark_math::translate(latex).typst)
}

/// Cada SVG de Typst tiene un ámbito propio de glyphs y máscaras. Al insertarlo
/// inline en HTML necesita IDs distintos de los demás SVG del documento.
pub fn namespace_svg(svg: &str, prefix: &str) -> String {
    svg.replace("id=\"", &format!("id=\"{prefix}"))
        .replace("href=\"#", &format!("href=\"#{prefix}"))
        .replace("url(#", &format!("url(#{prefix}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_subset_composes_and_invalid_math_stays_visible() {
        for latex in [
            "E=mc^2",
            r"\frac{1}{2}",
            r"\frac {a} {b}",
            r"x^ {a+b}",
            r"x^{2}2",
            r"2\frac{1}{2}",
            r"\frac{a}{b}^2",
            r"\sqrt[3]{x}",
            r"x_i^2",
            r"\alpha+\beta",
            r"\sum_{i=1}^{n}i",
            r"\int_0^\infty e^{-x}dx",
            r"\begin{pmatrix}1&2\\3&4\end{pmatrix}",
            r"\sin x+\cos x",
            r"\hat{x}+\bar{x}+\vec{x}",
        ] {
            let svg = math_svg(latex).unwrap_or_else(|error| panic!("{latex}: {error}"));
            assert!(svg.contains("<svg"));
        }
        for latex in [r"\foo{x}", r"\frac{1}", r"x^{", "a}"] {
            assert!(math_svg(latex).is_err(), "{latex}");
        }
    }
}
