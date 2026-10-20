//! Exportación PDF del documento (§32.1, ISSUE-035) — glue sin GTK.
//!
//! El motor vive en [`minmark_pdf`] (ISSUE-018: AST → Typst → PDF, texto
//! seleccionable NFR-030); aquí solo se encadena el parse del dialecto con la
//! exportación, para poder verificar el pipeline completo sin display.

use minmark_config::Settings;
#[cfg(test)]
use minmark_pdf::PdfEngine;
use minmark_pdf::{DocumentMeta, PdfError, TypstEngine};

/// Exporta el texto Markdown a bytes PDF con los ajustes `pdf_*` de
/// [`Settings`] (§32.3–§32.6: tamaño, márgenes y orientación) y los
/// metadatos de las plantillas header/footer (§32.9).
#[cfg(test)]
pub fn export_document(source: &str, settings: &Settings) -> Result<Vec<u8>, PdfError> {
    let ast = minmark_parser::parse_with(
        source,
        &minmark_parser::ParseOptions {
            wiki_links: settings.wiki_links,
            ..Default::default()
        },
    );
    let meta = DocumentMeta::from_ast(&ast);
    TypstEngine.export(
        &crate::academic::prepare_pdf(&ast, settings),
        settings,
        &meta,
    )
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
/// `meta` (§32.9) alimenta las variables de `pdf_header`/`pdf_footer`.
pub fn export_document_at(
    source: &str,
    settings: &Settings,
    document: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
    meta: &DocumentMeta,
) -> Result<Vec<u8>, PdfError> {
    let (ast, assets) = document_assets(source, settings, document, root)?;
    TypstEngine.export_with_assets(&ast, settings, assets, meta)
}

pub fn paginated_preview_at(
    source: &str,
    settings: &Settings,
    document: Option<&std::path::Path>,
    root: Option<&std::path::Path>,
    meta: &DocumentMeta,
) -> Result<String, PdfError> {
    let (ast, assets) = document_assets(source, settings, document, root)?;
    TypstEngine.preview_with_assets(&ast, settings, assets, meta)
}

/// Exporta la AST de una composición (§20.4, ISSUE-023): un único PDF con los
/// mismos ajustes efectivos (cadena §58) que la exportación individual. Los
/// destinos de imagen ya están rebasados a la raíz por
/// [`minmark_document::compose`], así que los recursos se resuelven respecto
/// a `root` (Typst sigue sin acceso libre al disco).
pub fn export_composed(
    mut ast: minmark_document::Ast,
    settings: &Settings,
    root: &std::path::Path,
    meta: &DocumentMeta,
) -> Result<Vec<u8>, PdfError> {
    let root = minmark_fs::canonicalize(root).map_err(PdfError::Io)?;
    let assets = ast_assets(&mut ast, &root, &root)?;
    let ast = crate::academic::prepare_pdf(&ast, settings);
    TypstEngine.export_with_assets(&ast, settings, assets, meta)
}

/// Preview paginado de una composición (§29–30, ISSUE-023): misma AST
/// compuesta, mismos ajustes y mismos recursos que [`export_composed`].
/// Toma la AST prestada porque su llamante (`composition::render`) cae al
/// Preview continuo con la misma AST si el render paginado falla.
pub fn paginated_preview_composed(
    ast: &minmark_document::Ast,
    settings: &Settings,
    root: &std::path::Path,
    meta: &DocumentMeta,
) -> Result<String, PdfError> {
    let root = minmark_fs::canonicalize(root).map_err(PdfError::Io)?;
    let mut copia = ast.clone();
    let assets = ast_assets(&mut copia, &root, &root)?;
    let copia = crate::academic::prepare_pdf(&copia, settings);
    TypstEngine.preview_with_assets(&copia, settings, assets, meta)
}

/// Recursos en memoria de una AST con los destinos ya rebasados a la raíz
/// (composición): `base` y `root` son la misma raíz del proyecto. La AST se
/// reescribe en el sitio a los alias de recursos.
fn ast_assets(
    ast: &mut minmark_document::Ast,
    base: &std::path::Path,
    root: &std::path::Path,
) -> Result<std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>, PdfError> {
    let mut assets = Assets::new(base, root);
    assets.walk(ast)?;
    Ok(assets.files)
}

/// Recorrido de imágenes de una AST: cada destino local se lee una vez y se
/// reescribe a un alias `assets/N.ext` en memoria. El motor recibe alias,
/// nunca permiso para leer rutas arbitrarias (§48).
struct Assets<'a> {
    base: &'a std::path::Path,
    root: &'a std::path::Path,
    index: minmark_fs::ProjectIndex,
    files: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
}

impl<'a> Assets<'a> {
    fn new(base: &'a std::path::Path, root: &'a std::path::Path) -> Self {
        Self {
            base,
            root,
            index: minmark_fs::ProjectIndex::new(root.to_path_buf()),
            files: std::collections::BTreeMap::new(),
        }
    }

    fn walk(&mut self, ast: &mut minmark_document::Ast) -> Result<(), PdfError> {
        self.blocks(&mut ast.blocks)
    }

    fn inlines(&mut self, inlines: &mut [minmark_document::Inline]) -> Result<(), PdfError> {
        use minmark_document::Inline;
        for inline in inlines {
            match inline {
                Inline::Image { destination, .. } => {
                    if minmark_fs::resolve::has_scheme(destination) || destination.starts_with("//")
                    {
                        return Err(PdfError::Compose(
                            "La imagen debe ser un recurso local".into(),
                        ));
                    }
                    let open = minmark_fs::OpenBuffers::new();
                    let target = {
                        let resolver = minmark_fs::Resolver::new(&self.index, &open);
                        let from =
                            minmark_fs::relative_path(self.root, &self.base.join("source.md"));
                        resolver.resolve(destination, &from)
                    };
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
                    self.files.insert(std::path::PathBuf::from(&alias), bytes);
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

    fn blocks(&mut self, blocks: &mut [minmark_document::Block]) -> Result<(), PdfError> {
        use minmark_document::BlockKind;
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
    let Some(base) = document.and_then(std::path::Path::parent) else {
        return Ok((
            crate::academic::prepare_pdf(&minmark_parser::parse(source), settings),
            std::collections::BTreeMap::new(),
        ));
    };
    let base = minmark_fs::canonicalize(if base.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        base
    })
    .map_err(PdfError::Io)?;
    let root = minmark_fs::canonicalize(root.unwrap_or(&base)).map_err(PdfError::Io)?;
    let mut ast = minmark_parser::parse_with(
        source,
        &minmark_parser::ParseOptions {
            wiki_links: settings.wiki_links,
            ..Default::default()
        },
    );
    let files = {
        let mut assets = Assets::new(&base, &root);
        assets.walk(&mut ast)?;
        assets.files
    };
    Ok((crate::academic::prepare_pdf(&ast, settings), files))
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
            &DocumentMeta::from_ast(&minmark_parser::parse(source)),
        )
        .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let _ = std::fs::remove_dir_all(root);
    }
}
