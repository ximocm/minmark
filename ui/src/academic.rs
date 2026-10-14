//! Recursos locales del host; el motor CSL solo recibe texto autorizado.
use minmark_config::Settings;
use minmark_document::{Ast, Block, BlockKind, Inline, Span};
use minmark_renderer::Bibliography;

pub fn load(settings: &Settings) -> Bibliography {
    let mut errors = Vec::new();
    let mut sources = Vec::new();
    for path in &settings.bibliography {
        match std::fs::read_to_string(path) {
            Ok(text) => sources.push((path.clone(), text)),
            Err(error) => errors.push(format!(
                "No se puede leer la bibliografía ({path}): {error}"
            )),
        }
    }
    let style = if settings.bibliography_style.eq_ignore_ascii_case("IEEE") {
        None
    } else {
        match std::fs::read_to_string(&settings.bibliography_style) {
            Ok(text) => Some(text),
            Err(error) => {
                errors.push(format!(
                    "No se puede leer el CSL ({}): {error}; se usa IEEE",
                    settings.bibliography_style
                ));
                None
            }
        }
    };
    let mut bibliography = Bibliography::from_sources(&sources, style.as_deref());
    for error in errors {
        bibliography.add_diagnostic(error);
    }
    bibliography
}

pub fn prepare_pdf(ast: &Ast, settings: &Settings) -> Ast {
    let (mut ast, diagnostics) = load(settings).resolve(ast);
    for message in diagnostics {
        eprintln!("minmark: {message}");
        ast.blocks.push(Block {
            span: Span::default(),
            kind: BlockKind::Paragraph {
                content: vec![Inline::Text(message)],
            },
        });
    }
    ast
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_resources_are_visible_and_recover_after_creation() {
        let root =
            std::env::temp_dir().join(format!("minmark-qa-resources-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let bibliography = root.join("references.bib");
        let style = root.join("style.csl");
        let settings = Settings {
            bibliography: vec![bibliography.to_string_lossy().into_owned()],
            bibliography_style: style.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let source = "[@smith2025]";
        let html = crate::preview::render_document(source, &settings);
        assert!(html.contains("No se puede leer la bibliografía"));
        assert!(html.contains("No se puede leer el CSL"));
        std::fs::write(
            &bibliography,
            include_str!("../../docs/qa/fixtures/references.bib"),
        )
        .unwrap();
        std::fs::write(
            &style,
            include_str!("../../docs/qa/fixtures/author-year.csl"),
        )
        .unwrap();
        let html = crate::preview::render_document(source, &settings);
        assert!(html.contains("Smith, 2025"));
        assert!(!html.contains("mm-diagnostic"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
