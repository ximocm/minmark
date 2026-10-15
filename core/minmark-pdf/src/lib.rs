//! Exportación PDF de minmark sobre Typst (§5.3, §32).
//!
//! Typst es el **único** motor PDF (§4.3 dependencia estratégica, ADR-003):
//! `TypstEngine` implementa [`PdfEngine`] con el pipeline normativo
//! Markdown → AST → código Typst → Typst → PDF.
//!
//! - Apariencia (§32.2): fondo blanco, texto oscuro, siempre.
//! - Configuración básica (§58.2): A4, márgenes 25 mm, Portrait.
//! - Headers/footers (§32.9): plantillas de `pdf_header`/`pdf_footer` con
//!   [`DocumentMeta`] y el estado de `{chapter}`, compartidas por el PDF y el
//!   Preview paginado.
//! - NFR-030: PDF con texto seleccionable (texto real, no imagen).
//! - Math: LaTeX → Typst con el subconjunto de ADR-002 ([`minmark_math`]);
//!   fuera del subconjunto, marcador visible (NFR-026).

mod generate;
mod world;

pub use generate::generate;

use minmark_config::{PageSize, Settings};
use minmark_document::{Ast, BlockKind};
use std::fmt;

/// Metadatos documentales de las plantillas de header/footer (§32.9).
///
/// Los valores estáticos (`{title}`, `{author}`, `{project}` y `{date}`)
/// se inyectan en `Settings::pdf_header`/`Settings::pdf_footer` al generar el
/// código Typst; `{chapter}`, `{page}` y `{pages}` se resuelven en el motor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocumentMeta {
    /// `{title}`: front matter `title` o, en su defecto, el primer H1.
    pub title: Option<String>,
    /// `{author}`: front matter `author`.
    pub author: Option<String>,
    /// `{project}`: lo aporta el llamador (nivel documento §58.1).
    pub project: Option<String>,
    /// `{date}`: lo aporta el llamador; el núcleo no depende del reloj
    /// (`MiniWorld::today` devuelve `None` a propósito para resultados
    /// reproducibles).
    pub date: Option<String>,
}

impl DocumentMeta {
    /// Deriva título/autor del front matter (scan mínimo de líneas clave:
    /// valor, como `minmark_renderer::resolve_cross_references`); título con
    /// fallback al texto plano del primer H1 de la AST y después a vacío.
    /// `project` y `date` quedan en `None`.
    pub fn from_ast(ast: &Ast) -> Self {
        let mut title = None;
        let mut author = None;
        if let Some(front_matter) = &ast.front_matter {
            for line in front_matter.lines() {
                if let Some((key, value)) = line.split_once(':').map(|(k, v)| (k.trim(), v.trim()))
                {
                    let value = value.trim_matches(['\'', '"']);
                    match key {
                        "title" if !value.is_empty() => title = Some(value.to_owned()),
                        "author" if !value.is_empty() => author = Some(value.to_owned()),
                        _ => {}
                    }
                }
            }
        }
        let title = Some(title.unwrap_or_else(|| first_h1_title(ast).unwrap_or_default()));
        Self {
            title,
            author,
            project: None,
            date: None,
        }
    }
}

/// Texto plano del primer H1 de la AST (§27.1), si existe. Es el fallback de
/// `{title}` (§32.9) cuando el llamador no aporta título.
pub(crate) fn first_h1_title(ast: &Ast) -> Option<String> {
    ast.blocks.iter().find_map(|block| match &block.kind {
        BlockKind::Heading {
            level: 1, content, ..
        } => Some(generate::plain_text(content)),
        _ => None,
    })
}

/// Frontera del motor PDF (§5.1 «comunicación con el motor PDF»).
///
/// Implementación única: [`TypstEngine`] (ISSUE-018). El trait aísla la
/// dependencia estratégica del resto del core (§4.3).
pub trait PdfEngine {
    /// Exporta la AST a bytes PDF, aplicando los ajustes `pdf_*` de
    /// [`Settings`] y las plantillas de header/footer (§32.9) con los
    /// metadatos documentales `meta`.
    fn export(
        &self,
        ast: &Ast,
        settings: &Settings,
        meta: &DocumentMeta,
    ) -> Result<Vec<u8>, PdfError>;
}

/// Errores del pipeline de exportación PDF.
#[derive(Debug)]
pub enum PdfError {
    /// Error de E/S al escribir la salida.
    Io(std::io::Error),
    /// Error de composición del motor (compilación Typst o exportación).
    Compose(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(source) => write!(f, "error de E/S al exportar PDF: {source}"),
            Self::Compose(message) => write!(f, "error de composición PDF: {message}"),
        }
    }
}

impl std::error::Error for PdfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            Self::Compose(_) => None,
        }
    }
}

/// Motor PDF basado en Typst (ISSUE-018).
#[derive(Debug, Default, Clone, Copy)]
pub struct TypstEngine;

impl PdfEngine for TypstEngine {
    fn export(
        &self,
        ast: &Ast,
        settings: &Settings,
        meta: &DocumentMeta,
    ) -> Result<Vec<u8>, PdfError> {
        self.export_with_assets(ast, settings, std::collections::BTreeMap::new(), meta)
    }
}

impl TypstEngine {
    /// Recursos autorizados por el host, con rutas virtuales relativas.
    /// Typst sigue sin acceso libre al filesystem ni a la red.
    pub fn export_with_assets(
        &self,
        ast: &Ast,
        settings: &Settings,
        assets: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        meta: &DocumentMeta,
    ) -> Result<Vec<u8>, PdfError> {
        let document = compile_document(ast, settings, assets, meta)?;
        typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default()).map_err(|errors| {
            PdfError::Compose(
                errors
                    .iter()
                    .map(|error| error.message.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })
    }

    /// Preview paginado con la misma composición que el PDF (§29–30, §32.9).
    pub fn preview_with_assets(
        &self,
        ast: &Ast,
        settings: &Settings,
        assets: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        meta: &DocumentMeta,
    ) -> Result<String, PdfError> {
        let document = compile_document(ast, settings, assets, meta)?;
        let mut html = String::from("<!doctype html><html><head><meta charset=\"utf-8\"><style>body{margin:0;background:#e5e5e5}section{margin:24px auto;background:white;max-width:95%;width:max-content;box-shadow:0 1px 8px #999}svg{max-width:100%;height:auto;display:block}</style></head><body>");
        for (index, page) in document.pages.iter().enumerate() {
            html.push_str(&format!(
                "<section class=\"mm-paged-page\" aria-label=\"Página {}\">",
                index + 1
            ));
            html.push_str(&minmark_renderer::namespace_svg(
                &typst_svg::svg(page),
                &format!("mm-page-{index}-"),
            ));
            html.push_str("</section>");
        }
        html.push_str("</body></html>");
        Ok(html)
    }
}

fn compile_document(
    ast: &Ast,
    settings: &Settings,
    assets: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    meta: &DocumentMeta,
) -> Result<typst::layout::PagedDocument, PdfError> {
    let source = generate(ast, settings, meta);
    let world = world::MiniWorld::new(source, assets);
    let warned = typst::compile(&world);
    let document = warned.output.map_err(|errors| {
        PdfError::Compose(
            errors
                .iter()
                .map(|error| error.message.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;
    Ok(document)
}

/// Dimensiones nominales de página en milímetros (ancho × alto, Portrait).
///
/// Tamaños nominales según §32.3; `Custom` usa las dimensiones declaradas.
/// La orientación Landscape (§32.6) se aplica intercambiando ambos valores en
/// la composición.
pub fn page_size_mm(size: PageSize) -> (f32, f32) {
    match size {
        PageSize::A3 => (297.0, 420.0),
        PageSize::A4 => (210.0, 297.0),
        PageSize::A5 => (148.0, 210.0),
        PageSize::A6 => (105.0, 148.0),
        PageSize::Letter => (215.9, 279.4),
        PageSize::Legal => (215.9, 355.6),
        PageSize::Executive => (184.2, 266.7),
        PageSize::Custom {
            width_mm,
            height_mm,
        } => (width_mm, height_mm),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_parser::parse;

    #[test]
    fn tamaños_nominales_en_milimetros() {
        assert_eq!(page_size_mm(PageSize::A4), (210.0, 297.0));
        assert_eq!(page_size_mm(PageSize::A3), (297.0, 420.0));
        assert_eq!(page_size_mm(PageSize::A5), (148.0, 210.0));
        assert_eq!(page_size_mm(PageSize::A6), (105.0, 148.0));
        assert_eq!(page_size_mm(PageSize::Letter), (215.9, 279.4));
        assert_eq!(page_size_mm(PageSize::Legal), (215.9, 355.6));
        assert_eq!(page_size_mm(PageSize::Executive), (184.2, 266.7));
    }

    #[test]
    fn custom_usa_sus_dimensiones() {
        let size = PageSize::Custom {
            width_mm: 120.0,
            height_mm: 240.0,
        };
        assert_eq!(page_size_mm(size), (120.0, 240.0));
    }

    #[test]
    fn el_defecto_es_a4() {
        // §32.3 / §58.2.
        assert_eq!(Settings::default().pdf_page_size, PageSize::A4);
    }

    #[test]
    fn exporta_documento_simple_a_pdf() {
        // §5.3 pipeline completo + NFR-030 (texto seleccionable).
        let ast = parse("# Hola\n\nEsto es un párrafo de prueba.\n");
        let bytes = TypstEngine
            .export(&ast, &Settings::default(), &DocumentMeta::default())
            .unwrap();

        assert!(bytes.starts_with(b"%PDF-"), "no es un PDF");
        let pdf = String::from_utf8_lossy(&bytes);
        // A4: 210 × 297 mm ≈ 595.28 × 841.89 pt.
        assert!(pdf.contains("MediaBox"), "sin MediaBox");
        assert!(pdf.contains("595"), "MediaBox no es A4 (ancho)");
        assert!(pdf.contains("841"), "MediaBox no es A4 (alto)");
        // Texto seleccionable: fuentes con mapa de caracteres, no imagen.
        assert!(pdf.contains("/Font"), "sin fuentes incrustadas");
        assert!(
            pdf.contains("/ToUnicode"),
            "sin mapa de texto seleccionable"
        );
    }

    #[test]
    fn exporta_math_y_marcadores() {
        let ast = parse("$$\n\\frac{a}{b}\n$$\n");
        let bytes = TypstEngine
            .export(&ast, &Settings::default(), &DocumentMeta::default())
            .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));

        // NFR-026: la math no soportada exporta con marcador, no falla.
        let ast = parse("$$\n\\foo{x}\n$$\n");
        let bytes = TypstEngine
            .export(&ast, &Settings::default(), &DocumentMeta::default())
            .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn exporta_con_paginacion_y_tamano_custom() {
        let settings = Settings {
            pagination_enabled: true,
            ..Settings::default()
        };
        let ast = parse("# T\n\nCuerpo.\n");
        let bytes = TypstEngine
            .export(&ast, &settings, &DocumentMeta::default())
            .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));

        let settings = Settings {
            pdf_page_size: PageSize::Custom {
                width_mm: 120.0,
                height_mm: 240.0,
            },
            ..Settings::default()
        };
        let bytes = TypstEngine
            .export(&ast, &settings, &DocumentMeta::default())
            .unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        // 120 mm ≈ 340.16 pt, 240 mm ≈ 680.31 pt.
        assert!(pdf.contains("340"), "ancho custom incorrecto");
        assert!(pdf.contains("680"), "alto custom incorrecto");
    }
}

#[cfg(test)]
mod footnote_layout_tests {
    use super::*;
    #[test]
    fn long_notes_repeat_reorder_and_paginate() {
        let long = "Nota larga verificable. ".repeat(220);
        let source = format!("# Uno\n\nB[^b] y A[^a] y B[^b].\n\n{{{{pagebreak}}}}\n\n# Dos\n\nOtra referencia a A[^a].\n\n[^a]: Nota corta A.\n\n[^b]: {long}\n");
        let ast = minmark_parser::parse(&source);
        let settings = Settings {
            pagination_enabled: true,
            ..Default::default()
        };
        let document = compile_document(
            &ast,
            &settings,
            Default::default(),
            &DocumentMeta::default(),
        )
        .unwrap();
        assert!(document.pages.len() >= 2);
        let pdf = TypstEngine
            .export(&ast, &settings, &DocumentMeta::default())
            .unwrap();
        if let Some(directory) = std::env::var_os("MINMARK_QA_OUTPUT_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::PathBuf::from(directory).join("long-notes.pdf"),
                pdf,
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod headers_footers_tests {
    //! §32.7–§32.9 sobre la composición real: se compila con el motor y se lee
    //! el texto efectivo de cada página (cuerpo + márgenes) desde los frames
    //! de Typst, que es lo que verán el PDF y el Preview paginado.

    use super::*;
    use minmark_parser::parse;

    struct Page {
        width: f64,
        height: f64,
        items: Vec<(f64, f64, String)>,
    }

    impl Page {
        /// Texto de la página en orden de composición (el footer va al final).
        fn text(&self) -> String {
            self.items
                .iter()
                .map(|(_, _, text)| text.as_str())
                .collect()
        }
    }

    fn compile_pages(ast: &Ast, settings: &Settings, meta: &DocumentMeta) -> Vec<Page> {
        let document = compile_document(ast, settings, Default::default(), meta).unwrap();
        document
            .pages
            .iter()
            .map(|page| {
                let mut items = Vec::new();
                collect(&page.frame, 0.0, 0.0, &mut items);
                Page {
                    width: page.frame.size().x.to_pt(),
                    height: page.frame.size().y.to_pt(),
                    items,
                }
            })
            .collect()
    }

    fn collect(frame: &typst::layout::Frame, ox: f64, oy: f64, out: &mut Vec<(f64, f64, String)>) {
        use typst::layout::FrameItem;
        for (pos, item) in frame.items() {
            match item {
                FrameItem::Text(text) => out.push((
                    ox + pos.x.to_pt(),
                    oy + pos.y.to_pt(),
                    text.text.to_string(),
                )),
                FrameItem::Group(group) => {
                    collect(&group.frame, ox + pos.x.to_pt(), oy + pos.y.to_pt(), out)
                }
                _ => {}
            }
        }
    }

    #[test]
    fn plantilla_con_todas_las_variables_32_9() {
        // §32.9: todas las variables en el footer con paginación activada;
        // {chapter} cambia al entrar en el segundo H1 y {page}/{pages} son
        // coherentes (1 de 2, 2 de 2).
        let source = "# Capitulo Uno\n\nCuerpo de relleno sin dígitos.\n\n{{pagebreak}}\n\n# Capitulo Dos\n\nMás cuerpo sin dígitos.\n";
        let settings = Settings {
            pagination_enabled: true,
            pdf_footer: Some("{title}|{author}|{project}|{date}|{chapter}|{page}/{pages}".into()),
            ..Settings::default()
        };
        let meta = DocumentMeta {
            title: Some("Mi Título".into()),
            author: Some("Autora X".into()),
            project: Some("Proyecto Y".into()),
            date: Some("2026-10-15".into()),
        };
        let pages = compile_pages(&parse(source), &settings, &meta);
        assert_eq!(pages.len(), 2, "el {{pagebreak}} parte en dos páginas");
        // El footer se compone el último; la plantilla va entera y {chapter}
        // cambia al entrar en el segundo H1: la otra página no lo contiene.
        let page_one = pages[0].text();
        assert!(
            page_one.contains("Mi Título|Autora X|Proyecto Y|2026-10-15|Capitulo Uno|1/2"),
            "{page_one}"
        );
        assert!(!page_one.contains("Capitulo Dos"), "{page_one}");
        let page_two = pages[1].text();
        assert!(
            page_two.contains("Mi Título|Autora X|Proyecto Y|2026-10-15|Capitulo Dos|2/2"),
            "{page_two}"
        );
        assert!(!page_two.contains("Capitulo Uno"), "{page_two}");
    }

    #[test]
    fn sin_paginacion_page_y_pages_vacios_32_7() {
        // §32.7: sin paginación no se muestra número en ninguna posición y
        // {page}/{pages} se expanden vacíos.
        let settings = Settings {
            pdf_footer: Some("Pie[{page}]y[{pages}]".into()),
            ..Settings::default()
        };
        let pages = compile_pages(
            &parse("# Título\n\nCuerpo sin dígitos.\n"),
            &settings,
            &DocumentMeta::default(),
        );
        let text = pages[0].text();
        assert!(text.contains("Pie[]y[]"), "{text}");
        assert!(!text.chars().any(|c| c.is_ascii_digit()), "{text}");
    }

    #[test]
    fn paginacion_footer_con_plantilla_y_numero_32_8() {
        // §32.7–32.8: con paginación, plantilla en el footer y posición
        // FooterRight (por defecto), el número de página Y el texto de la
        // plantilla aparecen: Typst descarta su numbering marginal cuando el
        // footer lleva contenido, así que el número lo emite la plantilla.
        let source =
            "# Título\n\nCuerpo sin dígitos.\n\n{{pagebreak}}\n\nMás cuerpo sin dígitos.\n";
        let settings = Settings {
            pagination_enabled: true,
            pdf_footer: Some("PIE-FIJO".into()),
            ..Settings::default()
        };
        let pages = compile_pages(&parse(source), &settings, &DocumentMeta::default());
        assert_eq!(pages.len(), 2);
        for (index, page) in pages.iter().enumerate() {
            let text = page.text();
            assert!(text.contains("PIE-FIJO"), "{text}");
            let digits: Vec<char> = text.chars().filter(char::is_ascii_digit).collect();
            let expected = char::from(b'1' + index as u8);
            assert_eq!(
                digits,
                [expected],
                "solo un número, el de la página: {text}"
            );
            let pie = page
                .items
                .iter()
                .find(|(_, _, item)| item == "PIE-FIJO")
                .expect("plantilla en el footer");
            assert!(pie.1 > page.height * 0.7, "la plantilla va al pie: {pie:?}");
            assert!(
                pie.0 < page.width * 0.3,
                "la plantilla va a la izquierda: {pie:?}"
            );
            let number = page
                .items
                .iter()
                .find(|(_, _, item)| item.contains(expected))
                .expect("número de página en el footer");
            assert!(
                number.1 > page.height * 0.7,
                "el número va al pie: {number:?}"
            );
            assert!(number.0 > page.width * 0.6, "FooterRight: {number:?}");
        }
    }

    #[test]
    fn document_meta_desde_front_matter_y_primer_h1_32_9() {
        // §21 + §32.9: título y autor del front matter (con comillas).
        let ast =
            parse("---\ntitle: \"Mi Título Doc\"\nauthor: Autora X\n---\n# Otro H1\n\ntexto\n");
        let meta = DocumentMeta::from_ast(&ast);
        assert_eq!(meta.title.as_deref(), Some("Mi Título Doc"));
        assert_eq!(meta.author.as_deref(), Some("Autora X"));
        assert_eq!(meta.project, None);
        assert_eq!(meta.date, None);

        // Fallback: texto plano del primer H1, formato incluido.
        let meta = DocumentMeta::from_ast(&parse("# **Primer** H1\n\ntexto\n"));
        assert_eq!(meta.title.as_deref(), Some("Primer H1"));
        assert_eq!(meta.author, None);

        // Sin front matter ni H1: vacío.
        let meta = DocumentMeta::from_ast(&parse("solo texto\n"));
        assert_eq!(meta.title.as_deref(), Some(""));
    }

    #[test]
    fn variables_desconocidas_literales_y_plantillas_vacias_32_9() {
        // §7.4.5: fuera de las variables de §32.9 la sintaxis {…} es literal,
        // también dentro de una plantilla.
        let settings = Settings {
            pdf_header: Some("A {foo} B {title}".into()),
            ..Settings::default()
        };
        let meta = DocumentMeta {
            title: Some("TituloX".into()),
            ..Default::default()
        };
        let pages = compile_pages(&parse("# Título\n"), &settings, &meta);
        let text = pages[0].text();
        assert!(text.contains("A {foo} B TituloX"), "{text}");

        // Plantilla vacía o None = sin header/footer.
        let settings = Settings {
            pdf_header: Some(String::new()),
            pdf_footer: Some("  ".into()),
            ..Settings::default()
        };
        let out = generate(&parse("# T\n"), &settings, &DocumentMeta::default());
        assert!(!out.contains("header:"), "{out}");
        assert!(!out.contains("footer:"), "{out}");
        let out = generate(
            &parse("# T\n"),
            &Settings::default(),
            &DocumentMeta::default(),
        );
        assert!(!out.contains("header:"), "{out}");
        assert!(!out.contains("footer:"), "{out}");
    }

    #[test]
    fn exportacion_con_pagina_custom_y_header_32_3() {
        // §32.3 + §32.9: una página custom más la cabecera compone sin error,
        // en el PDF y en el Preview paginado (§29–30).
        let settings = Settings {
            pdf_page_size: PageSize::Custom {
                width_mm: 120.0,
                height_mm: 240.0,
            },
            pagination_enabled: true,
            pdf_header: Some("Cab {title} de {author}".into()),
            ..Settings::default()
        };
        let ast = parse("# Documento\n\ncuerpo\n");
        let meta = DocumentMeta::from_ast(&ast);
        let bytes = TypstEngine.export(&ast, &settings, &meta).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let html = TypstEngine
            .preview_with_assets(&ast, &settings, Default::default(), &meta)
            .unwrap();
        assert!(html.contains("mm-paged-page"));
    }
}
