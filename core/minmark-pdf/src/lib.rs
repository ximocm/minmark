//! Exportación PDF de minmark sobre Typst (§5.3, §32).
//!
//! Typst es el **único** motor PDF (§4.3 dependencia estratégica, ADR-003):
//! `TypstEngine` implementa [`PdfEngine`] con el pipeline normativo
//! Markdown → AST → código Typst → Typst → PDF.
//!
//! - Apariencia (§32.2): fondo blanco, texto oscuro, siempre.
//! - Configuración básica (§58.2): A4, márgenes 25 mm, Portrait.
//! - NFR-030: PDF con texto seleccionable (texto real, no imagen).
//! - Math: LaTeX → Typst con el subconjunto de ADR-002 ([`minmark_math`]);
//!   fuera del subconjunto, marcador visible (NFR-026).

mod generate;
mod world;

pub use generate::generate;

use minmark_config::{PageSize, Settings};
use minmark_document::Ast;
use std::fmt;

/// Frontera del motor PDF (§5.1 «comunicación con el motor PDF»).
///
/// Implementación única: [`TypstEngine`] (ISSUE-018). El trait aísla la
/// dependencia estratégica del resto del core (§4.3).
pub trait PdfEngine {
    /// Exporta la AST a bytes PDF, aplicando los ajustes `pdf_*` de [`Settings`].
    fn export(&self, ast: &Ast, settings: &Settings) -> Result<Vec<u8>, PdfError>;
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
    fn export(&self, ast: &Ast, settings: &Settings) -> Result<Vec<u8>, PdfError> {
        self.export_with_assets(ast, settings, std::collections::BTreeMap::new())
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
    ) -> Result<Vec<u8>, PdfError> {
        let source = generate(ast, settings);
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
        let bytes = TypstEngine.export(&ast, &Settings::default()).unwrap();

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
        let bytes = TypstEngine.export(&ast, &Settings::default()).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));

        // NFR-026: la math no soportada exporta con marcador, no falla.
        let ast = parse("$$\n\\foo{x}\n$$\n");
        let bytes = TypstEngine.export(&ast, &Settings::default()).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn exporta_con_paginacion_y_tamano_custom() {
        let settings = Settings {
            pagination_enabled: true,
            ..Settings::default()
        };
        let ast = parse("# T\n\nCuerpo.\n");
        let bytes = TypstEngine.export(&ast, &settings).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));

        let settings = Settings {
            pdf_page_size: PageSize::Custom {
                width_mm: 120.0,
                height_mm: 240.0,
            },
            ..Settings::default()
        };
        let bytes = TypstEngine.export(&ast, &settings).unwrap();
        let pdf = String::from_utf8_lossy(&bytes);
        // 120 mm ≈ 340.16 pt, 240 mm ≈ 680.31 pt.
        assert!(pdf.contains("340"), "ancho custom incorrecto");
        assert!(pdf.contains("680"), "alto custom incorrecto");
    }
}
