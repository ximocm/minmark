//! Transformaciones académicas sobre una copia derivada, compartida por HTML/PDF.
use minmark_document::{Ast, Block, BlockKind, Inline};
use std::collections::BTreeMap;

pub(crate) fn visit_inlines(blocks: &mut [Block], visitor: &mut impl FnMut(&mut Inline)) {
    visit_scoped_inlines(blocks, true, visitor);
}
pub(crate) fn visit_body_inlines(blocks: &mut [Block], visitor: &mut impl FnMut(&mut Inline)) {
    visit_scoped_inlines(blocks, false, visitor);
}
fn visit_scoped_inlines(
    blocks: &mut [Block],
    include_notes: bool,
    visitor: &mut impl FnMut(&mut Inline),
) {
    fn inlines(values: &mut [Inline], f: &mut impl FnMut(&mut Inline)) {
        for value in values {
            match value {
                Inline::Styled { content: c, .. }
                | Inline::Emphasis(c)
                | Inline::Strong(c)
                | Inline::Strikethrough(c)
                | Inline::Link { content: c, .. }
                | Inline::Image { alt: c, .. } => inlines(c, f),
                _ => {}
            }
            f(value);
        }
    }
    for block in blocks {
        match &mut block.kind {
            BlockKind::Heading { content, .. } | BlockKind::Paragraph { content } => {
                inlines(content, visitor)
            }
            BlockKind::Quote { blocks } | BlockKind::Callout { blocks, .. } => {
                visit_scoped_inlines(blocks, include_notes, visitor)
            }
            BlockKind::FootnoteDefinition { blocks, .. } if include_notes => {
                visit_scoped_inlines(blocks, include_notes, visitor)
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    visit_scoped_inlines(&mut item.blocks, include_notes, visitor);
                }
            }
            BlockKind::Table { header, rows, .. } => {
                for cell in header.iter_mut().chain(rows.iter_mut().flatten()) {
                    inlines(cell, visitor);
                }
            }
            _ => {}
        }
    }
}

pub fn resolve_footnotes(ast: &Ast) -> (Ast, Vec<String>) {
    fn take_notes(
        blocks: &mut Vec<Block>,
        definitions: &mut BTreeMap<String, Block>,
        errors: &mut Vec<String>,
    ) {
        let mut i = 0;
        while i < blocks.len() {
            if let BlockKind::FootnoteDefinition { label, .. } = &blocks[i].kind {
                let label = label.clone();
                let block = blocks.remove(i);
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    definitions.entry(label.clone())
                {
                    entry.insert(block);
                } else {
                    errors.push(format!("Nota duplicada: {label}"));
                }
                continue;
            }
            match &mut blocks[i].kind {
                BlockKind::Quote { blocks } | BlockKind::Callout { blocks, .. } => {
                    take_notes(blocks, definitions, errors)
                }
                BlockKind::List { items, .. } => {
                    for item in items {
                        take_notes(&mut item.blocks, definitions, errors);
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let mut derived = ast.clone();
    let mut definitions = BTreeMap::new();
    let mut diagnostics = Vec::new();
    take_notes(&mut derived.blocks, &mut definitions, &mut diagnostics);
    let mut numbers = BTreeMap::<String, (usize, usize)>::new();
    visit_inlines(&mut derived.blocks, &mut |inline| {
        if let Inline::FootnoteReference(label) = inline {
            if !definitions.contains_key(label) {
                diagnostics.push(format!("Nota inexistente: {label}"));
                *inline = Inline::Text(format!("[Nota inexistente: {label}]"));
                return;
            }
            let next = numbers.len() + 1;
            let (number, occurrence) = numbers.entry(label.clone()).or_insert((next, 0));
            *occurrence += 1;
            *label = format!("{number}-{occurrence}");
        }
    });
    let mut notes = numbers.into_iter().collect::<Vec<_>>();
    notes.sort_by_key(|(_, (number, _))| *number);
    for (label, (number, _)) in notes {
        let mut note = definitions.remove(&label).expect("known note");
        if let BlockKind::FootnoteDefinition { label, blocks } = &mut note.kind {
            *label = number.to_string();
            // Typst no admite notas anidadas; conservar un diagnóstico visible.
            visit_inlines(blocks, &mut |inline| {
                if let Inline::FootnoteReference(label) = inline {
                    *inline = Inline::Text(format!("[Nota anidada: {label}]"));
                }
            });
        }
        derived.blocks.push(note);
    }
    (derived, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_use_order_repeated_missing_duplicate_and_literal_notes() {
        let source = "B[^b], A[^a], B[^b], falta[^missing]. `[^literal]` \\[^escape]\n\n[^a]: Nota A.\n\n[^b]: Nota B.\n\n[^b]: Duplicada.\n";
        let ast = minmark_parser::parse(source);
        let (derived, diagnostics) = resolve_footnotes(&ast);
        let text = format!("{derived:?}");
        assert!(
            text.contains("1-1") && text.contains("1-2") && text.contains("2-1"),
            "{text}"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("inexistente")),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("duplicada")),
            "{diagnostics:?}"
        );
        assert_eq!(minmark_parser::parse(source), ast);
        let html = crate::render_preview(&ast, source, &minmark_config::Settings::default());
        assert!(html.contains("id=\"mm-note-1\""));
        assert!(html.contains("href=\"#mm-note-ref-1-1\""));
        assert_eq!(html.matches("id=\"mm-note-ref-1-1\"").count(), 1);
    }
}

#[cfg(test)]
mod toc_numbering_tests {
    use minmark_config::{NumberingScope, Settings};
    #[test]
    fn toc_depth_defaults_to_three_accepts_six_and_respects_literals() {
        let source =
            "{{toc}}\n\n# One\n\n## Two\n\n### Three\n\n#### Four\n\n##### Five\n\n###### Six\n";
        for (depth, count) in [(3, 3), (6, 6)] {
            let settings = Settings {
                toc_max_level: depth,
                ..Default::default()
            };
            let html = crate::render_preview(&minmark_parser::parse(source), source, &settings);
            let toc = html
                .split("<nav class=\"mm-toc\"")
                .nth(1)
                .unwrap()
                .split("</nav>")
                .next()
                .unwrap();
            assert_eq!(toc.matches("<a href=").count(), count);
        }
        for source in ["`{{toc}}`", "\\{{toc}}", "```md\n{{toc}}\n```", "# Heading"] {
            assert!(!crate::render_preview(
                &minmark_parser::parse(source),
                source,
                &Settings::default()
            )
            .contains("<nav"));
        }
    }
    #[test]
    fn chapter_reset_and_continuous_share_caption_and_reference_numbers() {
        let source = "# One\n\n$$\nx\n$$\n{#eq:a}\n\n# Two\n\n$$\ny\n$$\n{#eq:b}\n\n{{ref:eq:b}}";
        for (scope, expected) in [
            (NumberingScope::Continuous, "Ecuación 2"),
            (NumberingScope::ByChapter, "Ecuación 2.1"),
            (NumberingScope::ByChapterReset, "Ecuación 1"),
        ] {
            let settings = Settings {
                counter_numbering: scope,
                ..Default::default()
            };
            let html = crate::render_preview(&minmark_parser::parse(source), source, &settings);
            assert!(
                html.contains(&format!("class=\"mm-caption\">{expected}</div>")),
                "{html}"
            );
            assert!(html.contains(&format!("href=\"#eq:b\">{expected}</a>")));
        }
    }
}

#[cfg(test)]
mod normalized_note_tests {
    #[test]
    fn note_labels_share_standard_reference_normalization() {
        let source = "Texto[^NOTA] y otra[^nota].\n\n[^Nota]: Nota con Unicode áβ.\n";
        let ast = minmark_parser::parse(source);
        let (derived, diagnostics) = super::resolve_footnotes(&ast);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let html = crate::render_preview(&ast, source, &minmark_config::Settings::default());
        assert_eq!(html.matches("role=\"doc-noteref\"").count(), 2);
        assert!(format!("{derived:?}").contains("1-2"));
    }
}
