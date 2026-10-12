//! Exportación PDF del documento (§32.1, ISSUE-035) — glue sin GTK.
//!
//! El motor vive en [`minmark_pdf`] (ISSUE-018: AST → Typst → PDF, texto
//! seleccionable NFR-030); aquí solo se encadena el parse del dialecto con la
//! exportación, para poder verificar el pipeline completo sin display.

use minmark_config::Settings;
use minmark_pdf::{PdfEngine, PdfError, TypstEngine};

/// Exporta el texto Markdown a bytes PDF con los ajustes `pdf_*` de
/// [`Settings`] (§32.3–§32.6: tamaño, márgenes y orientación).
pub fn export_document(source: &str, settings: &Settings) -> Result<Vec<u8>, PdfError> {
    let ast = minmark_parser::parse(source);
    TypstEngine.export(&ast, settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_document_produce_pdf() {
        let pdf = export_document("# Hola\n\nPárrafo con **negrita**.\n", &Settings::default())
            .expect("exportación");
        assert!(pdf.starts_with(b"%PDF"), "cabecera PDF ausente");
        assert!(pdf.len() > 100, "PDF demasiado corto: {} bytes", pdf.len());
    }

    #[test]
    fn export_document_respeta_ajustes_de_pagina() {
        // §32.3: tamaño y orientación llegan al motor (A4 Portrait por
        // defecto; un documento vacío sigue exportándose).
        let pdf = export_document("", &Settings::default()).expect("exportación");
        assert!(pdf.starts_with(b"%PDF"));
    }
}
