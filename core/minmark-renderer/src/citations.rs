//! Un único motor CSL para citas y bibliografía, sin acceso a disco ni red.
use hayagriva::citationberg::{taxonomy::Locator, IndependentStyle, LocaleFile};
use hayagriva::{
    BibliographyDriver, BibliographyRequest, CitationRequest, Library, LocatorPayload,
    SpecificLocator,
};
use minmark_document::{Ast, Block, BlockKind, CitationItem, Inline, Span};
use std::collections::BTreeSet;

pub struct Bibliography {
    library: Library,
    style: IndependentStyle,
    diagnostics: Vec<String>,
    conflicts: BTreeSet<String>,
}
impl Default for Bibliography {
    fn default() -> Self {
        Self::from_sources(&[], None)
    }
}
impl Bibliography {
    /// El host proporciona el contenido de los recursos autorizados.
    pub fn from_sources(sources: &[(String, String)], style: Option<&str>) -> Self {
        let mut diagnostics = Vec::new();
        let mut library = Library::new();
        let mut conflicts = BTreeSet::new();
        for (name, source) in sources {
            match hayagriva::io::from_biblatex_str(source) {
                Ok(parsed) => {
                    for entry in parsed.iter() {
                        if library.get(entry.key()).is_some() {
                            conflicts.insert(entry.key().to_owned());
                            diagnostics.push(format!(
                                "Clave bibliográfica duplicada: {} ({name})",
                                entry.key()
                            ));
                        } else {
                            library.push(entry);
                        }
                    }
                }
                Err(error) => {
                    diagnostics.push(format!("Bibliografía inválida ({name}): {error:?}"))
                }
            }
        }
        let default_style = || {
            IndependentStyle::from_xml(include_str!("../assets/ieee.csl"))
                .expect("embedded IEEE CSL")
        };
        let style = match style.map(IndependentStyle::from_xml) {
            Some(Ok(style)) => style,
            Some(Err(error)) => {
                diagnostics.push(format!("CSL inválido: {error}; se usa IEEE"));
                default_style()
            }
            None => default_style(),
        };
        Self {
            library,
            style,
            diagnostics,
            conflicts,
        }
    }
    pub fn add_diagnostic(&mut self, error: String) {
        self.diagnostics.push(error);
    }
    pub fn suggestions(&self, query: &str) -> Vec<(String, String)> {
        let query = query.to_lowercase();
        self.library
            .iter()
            .filter(|entry| !self.conflicts.contains(entry.key()))
            .filter_map(|entry| {
                let title = entry.title().map(ToString::to_string).unwrap_or_default();
                let authors = entry
                    .authors()
                    .map(|authors| {
                        authors
                            .iter()
                            .map(|a| {
                                format!("{} {}", a.given_name.as_deref().unwrap_or(""), a.name)
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let year = entry
                    .date_any()
                    .map(|d| d.year.to_string())
                    .unwrap_or_default();
                let label = format!("{} · {authors} · {title} · {year}", entry.key());
                label
                    .to_lowercase()
                    .contains(&query)
                    .then(|| (entry.key().to_owned(), label))
            })
            .take(50)
            .collect()
    }
    pub fn resolve(&self, ast: &Ast) -> (Ast, Vec<String>) {
        let mut derived = ast.clone();
        let mut used_notes = BTreeSet::new();
        crate::academic::visit_body_inlines(&mut derived.blocks, &mut |inline| {
            if let Inline::FootnoteReference(label) = inline {
                used_notes.insert(label.clone());
            }
        });
        retain_used_notes(&mut derived.blocks, &used_notes);
        let mut groups = Vec::<Vec<CitationItem>>::new();
        crate::academic::visit_inlines(&mut derived.blocks, &mut |inline| {
            if let Inline::Citation { items } = inline {
                groups.push(items.clone());
            }
        });
        let locales = [
            LocaleFile::from_xml(include_str!("../assets/locales-en-US.xml"))
                .expect("embedded CSL locale")
                .into(),
        ];
        let mut driver = BibliographyDriver::new();
        let mut diagnostics = self.diagnostics.clone();
        let mut markers = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            let mut items = Vec::new();
            let mut missing = String::new();
            for item in group {
                if let Some(entry) = self
                    .library
                    .get(&item.key)
                    .filter(|_| !self.conflicts.contains(&item.key))
                {
                    items.push(hayagriva::CitationItem::with_locator(
                        entry,
                        item.locator.as_deref().map(locator),
                    ));
                } else {
                    let error = format!("Cita inexistente o ambigua: {}", item.key);
                    diagnostics.push(error.clone());
                    missing.push_str(&format!(" [{error}]"));
                }
            }
            driver.citation(CitationRequest::new(
                items,
                &self.style,
                None,
                &locales,
                Some(index + 1),
            ));
            markers.push(missing);
        }
        let result = driver.finish(BibliographyRequest {
            style: &self.style,
            locale: None,
            locale_files: &locales,
        });
        let mut texts = result
            .citations
            .into_iter()
            .zip(markers)
            .map(|(citation, marker)| {
                let mut content = csl_inlines(&citation.citation);
                if !marker.is_empty() {
                    content.push(Inline::Text(marker));
                }
                (citation.note_number, content)
            });
        let mut citation_notes = Vec::new();
        let mut index = 0;
        crate::academic::visit_inlines(&mut derived.blocks, &mut |inline| {
            if matches!(inline, Inline::Citation { .. }) {
                let (note, text) = texts.next().unwrap_or_default();
                index += 1;
                if note.is_some() {
                    let mut label = format!("mm-citation-{index}");
                    while used_notes.contains(&label) {
                        label.push_str("-csl");
                    }
                    *inline = Inline::FootnoteReference(label.clone());
                    citation_notes.push(Block {
                        span: Span::default(),
                        kind: BlockKind::FootnoteDefinition {
                            label,
                            blocks: vec![Block {
                                span: Span::default(),
                                kind: BlockKind::Paragraph { content: text },
                            }],
                        },
                    });
                } else {
                    *inline = Inline::Styled {
                        content: text,
                        style: Default::default(),
                    };
                }
            }
        });
        // Una cita de estilo note dentro de una nota ya existente no crea
        // una nota anidada: su contenido se compone en la nota del usuario.
        for block in &mut derived.blocks {
            if let BlockKind::FootnoteDefinition { blocks, .. } = &mut block.kind {
                crate::academic::visit_inlines(blocks, &mut |inline| {
                    if let Inline::FootnoteReference(label) = inline {
                        if let Some(content) =
                            citation_notes.iter().find_map(|note| match &note.kind {
                                BlockKind::FootnoteDefinition { label: key, blocks }
                                    if key == label =>
                                {
                                    match &blocks[0].kind {
                                        BlockKind::Paragraph { content } => Some(content.clone()),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            })
                        {
                            *inline = Inline::Styled {
                                content,
                                style: Default::default(),
                            };
                        }
                    }
                });
            }
        }
        derived.blocks.extend(citation_notes);
        if let Some(bibliography) = result.bibliography.filter(|b| !b.items.is_empty()) {
            derived.blocks.push(Block {
                span: Span::default(),
                kind: BlockKind::Bibliography {
                    entries: bibliography
                        .items
                        .iter()
                        .map(|item| {
                            (
                                item.first_field.as_ref().map(csl_child).unwrap_or_default(),
                                csl_inlines(&item.content),
                            )
                        })
                        .collect(),
                    hanging_indent: bibliography.hanging_indent,
                    second_field_align: bibliography.second_field_align.is_some(),
                    line_spacing: bibliography.line_spacing.get() as f32,
                    entry_spacing: bibliography.entry_spacing as f32,
                },
            });
        }
        (derived, diagnostics)
    }
}
fn retain_used_notes(blocks: &mut Vec<Block>, used: &BTreeSet<String>) {
    blocks.retain(|block| !matches!(&block.kind, BlockKind::FootnoteDefinition { label, .. } if !used.contains(label)));
    for block in blocks {
        match &mut block.kind {
            BlockKind::Quote { blocks } | BlockKind::Callout { blocks, .. } => {
                retain_used_notes(blocks, used)
            }
            BlockKind::List { items, .. } => {
                for item in items {
                    retain_used_notes(&mut item.blocks, used);
                }
            }
            _ => {}
        }
    }
}
fn csl_inlines(children: &hayagriva::ElemChildren) -> Vec<Inline> {
    children.0.iter().flat_map(csl_child).collect()
}
fn csl_child(child: &hayagriva::ElemChild) -> Vec<Inline> {
    use hayagriva::ElemChild;
    match child {
        ElemChild::Text(text) | ElemChild::Link { text, .. } => vec![csl_text(text)],
        ElemChild::Elem(element) => csl_inlines(&element.children),
        ElemChild::Markup(text) => vec![Inline::Text(text.clone())],
        ElemChild::Transparent { .. } => vec![Inline::Text("[Elemento CSL no disponible]".into())],
    }
}
fn csl_text(text: &hayagriva::Formatted) -> Inline {
    use hayagriva::citationberg::{
        FontStyle, FontVariant, FontWeight, TextDecoration, VerticalAlign,
    };
    let format = &text.formatting;
    let style = minmark_document::InlineStyle {
        italic: format.font_style != FontStyle::Normal,
        bold: format.font_weight == FontWeight::Bold,
        light: format.font_weight == FontWeight::Light,
        small_caps: format.font_variant == FontVariant::SmallCaps,
        underline: format.text_decoration == TextDecoration::Underline,
        vertical: match format.vertical_align {
            VerticalAlign::Sup => 1,
            VerticalAlign::Sub => -1,
            _ => 0,
        },
    };
    if style == Default::default() {
        Inline::Text(text.text.clone())
    } else {
        Inline::Styled {
            content: vec![Inline::Text(text.text.clone())],
            style,
        }
    }
}
fn locator(text: &str) -> SpecificLocator<'_> {
    let (prefix, value) = text.split_once(' ').unwrap_or(("p.", text));
    let kind = match prefix.trim_end_matches('.') {
        "p" | "pp" => Locator::Page,
        "ch" => Locator::Chapter,
        "sec" => Locator::Section,
        "fig" => Locator::Figure,
        "tbl" => Locator::Table,
        "eq" => Locator::Equation,
        _ => return SpecificLocator(Locator::Page, LocatorPayload::Str(text)),
    };
    SpecificLocator(kind, LocatorPayload::Str(value.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sources() -> Vec<(String, String)> {
        vec![
            (
                "one.bib".into(),
                include_str!("../../../docs/qa/fixtures/references.bib").into(),
            ),
            (
                "two.bib".into(),
                include_str!("../../../docs/qa/fixtures/extra.bib").into(),
            ),
        ]
    }
    #[test]
    fn ieee_groups_locators_and_bibliography_share_a_driver() {
        let bibliography = Bibliography::from_sources(&sources(), None);
        let source = "[@smith2025] [@smith2025; @jones2024] [@smith2025, p. 42]";
        let ast = minmark_parser::parse(source);
        let (derived, diagnostics) = bibliography.resolve(&ast);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(minmark_parser::parse(source), ast);
        let text = format!("{derived:?}");
        assert!(text.contains("[1]"), "{text}");
        assert!(text.contains("42"));
        assert!(text.contains("Energy Research"));
        assert!(text.contains("Local Scientific Editing"));
    }
    #[test]
    fn author_year_csl_and_all_four_search_fields() {
        let bibliography = Bibliography::from_sources(
            &sources(),
            Some(include_str!("../../../docs/qa/fixtures/author-year.csl")),
        );
        for query in ["smith2025", "Smith", "Alice", "Energy", "2025"] {
            assert_eq!(bibliography.suggestions(query)[0].0, "smith2025");
        }
        let (ast, errors) = bibliography.resolve(&minmark_parser::parse("[@smith2025]"));
        assert!(errors.is_empty());
        let text = ast
            .blocks
            .iter()
            .filter_map(|block| match &block.kind {
                BlockKind::Paragraph { content } => Some(crate::plain_text(content)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Smith, 2025"), "{text}");
    }
    #[test]
    fn malformed_files_unknown_keys_and_conflicts_are_visible() {
        let mut sources = sources();
        sources.push(sources[0].clone());
        sources.push(("bad.bib".into(), "@book{".into()));
        let bibliography = Bibliography::from_sources(&sources, Some("invalid CSL"));
        let (ast, diagnostics) =
            bibliography.resolve(&minmark_parser::parse("[@missing; @smith2025]"));
        assert!(diagnostics.iter().any(|d| d.contains("duplicada")));
        assert!(diagnostics
            .iter()
            .any(|d| d.contains("Bibliografía inválida")));
        assert!(diagnostics.iter().any(|d| d.contains("CSL inválido")));
        assert!(format!("{ast:?}").contains("Cita inexistente o ambigua"));
        assert!(bibliography.suggestions("smith").is_empty());
    }
}

#[cfg(test)]
mod note_style_tests {
    use super::*;
    #[test]
    fn note_class_csl_uses_linked_notes() {
        let style = include_str!("../../../docs/qa/fixtures/author-year.csl")
            .replace("class=\"in-text\"", "class=\"note\"");
        let bibliography = Bibliography::from_sources(
            &[(
                "refs.bib".into(),
                include_str!("../../../docs/qa/fixtures/references.bib").into(),
            )],
            Some(&style),
        );
        let source = "Cita [@smith2025].";
        let html = crate::render_preview_with_bibliography(
            &minmark_parser::parse(source),
            source,
            &minmark_config::Settings::default(),
            &bibliography,
        );
        assert!(html.contains("role=\"doc-noteref\""));
        assert!(html.contains("Smith, 2025"));
        assert!(!html.contains("Nota inexistente"));
    }
}

#[cfg(test)]
mod formatting_tests {
    use super::*;
    #[test]
    fn ieee_keeps_italics_and_unused_notes_do_not_add_citations() {
        let bibliography = Bibliography::from_sources(
            &[(
                "refs.bib".into(),
                include_str!("../../../docs/qa/fixtures/references.bib").into(),
            )],
            None,
        );
        let source = "[@smith2025]\n\n[^unused]: [@missing]";
        let html = crate::render_preview_with_bibliography(
            &minmark_parser::parse(source),
            source,
            &minmark_config::Settings::default(),
            &bibliography,
        );
        assert!(html.contains("font-style:italic;\">Energy Research</span>"));
        assert!(!html.contains("Cita inexistente"));
        assert!(!html.contains("mm-footnotes"));
    }
}
