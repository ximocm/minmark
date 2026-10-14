//! Exportación PDF del documento (§32.1, ISSUE-035) — glue sin GTK.
//!
//! El motor vive en [`minmark_pdf`] (ISSUE-018: AST → Typst → PDF, texto
//! seleccionable NFR-030); aquí solo se encadena el parse del dialecto con la
//! exportación, para poder verificar el pipeline completo sin display.

use minmark_config::Settings;
use minmark_pdf::{PdfError, TypstEngine};
#[cfg(test)]
use minmark_pdf::PdfEngine;

/// Exporta el texto Markdown a bytes PDF con los ajustes `pdf_*` de
/// [`Settings`] (§32.3–§32.6: tamaño, márgenes y orientación).
#[cfg(test)]
pub fn export_document(source: &str, settings: &Settings) -> Result<Vec<u8>, PdfError> {
    let ast = minmark_parser::parse_with(
        source,
        &minmark_parser::ParseOptions {
            wiki_links: settings.wiki_links,
            ..Default::default()
        },
    );
    TypstEngine.export(&crate::academic::prepare_pdf(&ast, settings), settings)
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

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    fn pdf_con_referencias_locales_y_fallbacks_se_exporta() {
        let source = "# Título {#intro}\n\n| A |\n| - |\n| B |\n{#tbl:one}\n\n$$\nx=1\n$$\n{#eq:one}\n\n{{ref:intro}} {{ref:tbl:one}} {{ref:eq:one}} {{ref:missing}}\n\n[roto](#ausente)\n";
        for lang in ["es", "en"] {
            let settings = Settings {
                language: lang.into(),
                ..Default::default()
            };
            let pdf = export_document(source, &settings).expect("PDF con destinos internos");
            assert!(pdf.starts_with(b"%PDF"));
        }
    }
}

/// Incluye las imágenes locales del documento como recursos en memoria.
/// El motor recibe alias, nunca permiso para leer rutas arbitrarias.
pub fn export_document_at(
    source: &str,
    settings: &Settings,
    document: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
) -> Result<Vec<u8>, PdfError> {
    let (ast, assets) = document_assets(source, settings, document, root)?;
    TypstEngine.export_with_assets(&ast, settings, assets)
}

pub fn paginated_preview_at(
    source: &str,
    settings: &Settings,
    document: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
) -> Result<String, PdfError> {
    let (ast, assets) = document_assets(source, settings, document, root)?;
    TypstEngine.preview_with_assets(&ast, settings, assets)
}

fn document_assets(
    source: &str,
    settings: &Settings,
    document: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
) -> Result<
    (
        minmark_document::Ast,
        std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ),
    PdfError,
> {
    use minmark_document::{Block, BlockKind, Inline};
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };
    struct Assets<'a> {
        base: &'a Path,
        root: &'a Path,
        files: BTreeMap<PathBuf, Vec<u8>>,
    }
    impl Assets<'_> {
        fn inlines(&mut self, inlines: &mut [Inline]) -> Result<(), PdfError> {
            for inline in inlines {
                match inline {
                    Inline::Image { destination, .. } => {
                        if minmark_fs::resolve::has_scheme(destination)
                            || destination.starts_with("//")
                        {
                            return Err(PdfError::Compose(
                                "La imagen debe ser un recurso local".into(),
                            ));
                        }
                        let index = minmark_fs::ProjectIndex::new(self.root.to_path_buf());
                        let open = minmark_fs::OpenBuffers::new();
                        let resolver = minmark_fs::Resolver::new(&index, &open);
                        let from =
                            minmark_fs::relative_path(self.root, &self.base.join("source.md"));
                        let target = resolver.resolve(destination, &from);
                        if target.kind != minmark_fs::TargetKind::Local {
                            return Err(PdfError::Compose(format!(
                                "No se puede resolver la imagen: {destination}"
                            )));
                        }
                        let path = self.root.join(target.rel_path.unwrap());
                        let bytes = std::fs::read(&path).map_err(PdfError::Io)?;
                        let alias = format!(
                            "assets/{}.{}",
                            self.files.len(),
                            path.extension().and_then(|e| e.to_str()).unwrap_or("bin")
                        );
                        self.files.insert(PathBuf::from(&alias), bytes);
                        *destination = alias;
                    }
                    Inline::Emphasis(c)
                    | Inline::Strong(c)
                    | Inline::Strikethrough(c)
                    | Inline::Link { content: c, .. } => self.inlines(c)?,
                    _ => {}
                }
            }
            Ok(())
        }
        fn blocks(&mut self, blocks: &mut [Block]) -> Result<(), PdfError> {
            for block in blocks {
                match &mut block.kind {
                    BlockKind::Heading { content, .. } | BlockKind::Paragraph { content } => {
                        self.inlines(content)?
                    }
                    BlockKind::Quote { blocks }
                    | BlockKind::Callout { blocks, .. }
                    | BlockKind::FootnoteDefinition { blocks, .. } => self.blocks(blocks)?,
                    BlockKind::List { items, .. } => {
                        for item in items {
                            self.blocks(&mut item.blocks)?;
                        }
                    }
                    BlockKind::Table { header, rows, .. } => {
                        for cell in header.iter_mut().chain(rows.iter_mut().flatten()) {
                            self.inlines(cell)?;
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        }
    }
    let Some(base) = document.and_then(Path::parent) else {
        return Ok((
            crate::academic::prepare_pdf(&minmark_parser::parse(source), settings),
            BTreeMap::new(),
        ));
    };
    let base = std::fs::canonicalize(if base.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base
    })
    .map_err(PdfError::Io)?;
    let root = std::fs::canonicalize(root.unwrap_or(&base)).map_err(PdfError::Io)?;
    let mut ast = minmark_parser::parse_with(
        source,
        &minmark_parser::ParseOptions {
            wiki_links: settings.wiki_links,
            ..Default::default()
        },
    );
    let mut assets = Assets {
        base: &base,
        root: &root,
        files: BTreeMap::new(),
    };
    assets.blocks(&mut ast.blocks)?;
    Ok((crate::academic::prepare_pdf(&ast, settings), assets.files))
}

#[cfg(test)]
mod figure_tests {
    use super::*;
    #[test]
    fn figura_local_y_referencia_exportadas_sin_acceso_de_typst_al_disco() {
        let root = std::env::temp_dir().join(format!("minmark-pdf-figure-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("figure.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20" fill="red"/></svg>"#).unwrap();
        let source = "![Figura](figure.svg){#fig:one}\n\n{{ref:fig:one}}";
        let pdf = export_document_at(
            source,
            &Settings::default(),
            Some(&root.join("document.md")),
            Some(&root),
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let _ = std::fs::remove_dir_all(root);
    }
}
