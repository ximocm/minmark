//! Referencias cruzadas derivadas: el mismo AST de salida para HTML y PDF.
use minmark_config::{NumberingScope, Settings};
use minmark_document::{Ast, Attributes, Block, BlockKind, Inline, Substitution};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Context {
    labels: BTreeMap<String, String>,
    duplicates: BTreeSet<String>,
    headings: [u32; 6],
    counters: [u32; 3],
    numbers: BTreeMap<u32, String>,
}
impl Context {
    fn register(&mut self, attrs: &Option<Attributes>, label: String) {
        if let Some(id) = attrs.as_ref().and_then(|a| a.id.as_ref()) {
            if self.labels.insert(id.clone(), label).is_some() {
                self.duplicates.insert(id.clone());
            }
        }
    }
    fn caption(&mut self, kind: usize, settings: &Settings) -> String {
        self.counters[kind] += 1;
        let names = if settings.language.starts_with("es") {
            ["Figura", "Tabla", "Ecuación"]
        } else {
            ["Figure", "Table", "Equation"]
        };
        let number = if settings.counter_numbering == NumberingScope::ByChapter {
            format!("{}.{}", self.headings[0], self.counters[kind])
        } else {
            self.counters[kind].to_string()
        };
        format!("{} {number}", names[kind])
    }
    fn inlines(&mut self, inlines: &[Inline], settings: &Settings) {
        for inline in inlines {
            match inline {
                Inline::Image { attrs, .. } => {
                    if attrs.as_ref().and_then(|a| a.id.as_ref()).is_some() {
                        let label = self.caption(0, settings);
                        self.register(attrs, label);
                    }
                }
                Inline::Emphasis(c)
                | Inline::Strong(c)
                | Inline::Strikethrough(c)
                | Inline::Link { content: c, .. } => self.inlines(c, settings),
                _ => {}
            }
        }
    }
    fn blocks(&mut self, blocks: &[Block], settings: &Settings) {
        for block in blocks {
            match &block.kind {
                BlockKind::Heading {
                    level,
                    content,
                    attrs,
                } => {
                    let level = (*level).clamp(1, 6) as usize - 1;
                    self.headings[level] += 1;
                    self.headings[level + 1..].fill(0);
                    if level == 0 && settings.counter_numbering != NumberingScope::Continuous {
                        self.counters.fill(0);
                    }
                    let number = self.headings[..=level]
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(".");
                    let label = if settings.heading_numbering {
                        format!("{number} {}", super::plain_text(content))
                    } else {
                        super::plain_text(content)
                    };
                    self.numbers.insert(block.span.start, number);
                    self.register(attrs, label);
                    self.inlines(content, settings);
                }
                BlockKind::Paragraph { content } => self.inlines(content, settings),
                BlockKind::Quote { blocks } | BlockKind::Callout { blocks, .. } => {
                    self.blocks(blocks, settings)
                }
                BlockKind::List { items, .. } => {
                    for item in items {
                        self.blocks(&item.blocks, settings);
                    }
                }
                BlockKind::Table {
                    attrs,
                    header,
                    rows,
                    ..
                } => {
                    if attrs.as_ref().and_then(|a| a.id.as_ref()).is_some() {
                        let label = self.caption(1, settings);
                        self.register(attrs, label);
                    }
                    for cell in header.iter().chain(rows.iter().flatten()) {
                        self.inlines(cell, settings);
                    }
                }
                BlockKind::MathBlock { attrs, .. } => {
                    if attrs.as_ref().and_then(|a| a.id.as_ref()).is_some() {
                        let label = self.caption(2, settings);
                        self.register(attrs, label);
                    }
                }
                _ => {}
            }
        }
    }
    fn expand_inlines(&self, inlines: &mut [Inline]) {
        for inline in inlines {
            match inline {
                Inline::Substitution(Substitution::CrossRef { id }) => {
                    *inline = if self.duplicates.contains(id) {
                        Inline::Text(format!("[ID duplicado: {id}]"))
                    } else if let Some(label) = self.labels.get(id) {
                        Inline::Link {
                            destination: format!("#{id}"),
                            title: String::new(),
                            content: vec![Inline::Text(label.clone())],
                        }
                    } else {
                        Inline::Text(format!("[Referencia inexistente: {id}]"))
                    };
                }
                Inline::Emphasis(c)
                | Inline::Strong(c)
                | Inline::Strikethrough(c)
                | Inline::Link { content: c, .. } => self.expand_inlines(c),
                Inline::Image { alt, attrs, .. } => {
                    self.expand_inlines(alt);
                    if let Some(attrs) = attrs {
                        if let Some(id) = attrs.id.as_ref().filter(|id| id.starts_with("fig:")) {
                            let caption = super::plain_text(alt);
                            attrs.caption = Some(match self.labels.get(id) {
                                Some(label) if !self.duplicates.contains(id) => {
                                    if caption.is_empty() {
                                        label.clone()
                                    } else {
                                        format!("{label}. {caption}")
                                    }
                                }
                                _ => caption,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn expand_blocks(&self, blocks: &mut [Block], settings: &Settings) {
        for block in blocks {
            match &mut block.kind {
                BlockKind::Heading { content, .. } => {
                    self.expand_inlines(content);
                    if settings.heading_numbering {
                        if let Some(n) = self.numbers.get(&block.span.start) {
                            content.insert(0, Inline::Text(format!("{n} ")));
                        }
                    }
                }
                BlockKind::Paragraph { content } => self.expand_inlines(content),
                BlockKind::Quote { blocks } | BlockKind::Callout { blocks, .. } => {
                    self.expand_blocks(blocks, settings)
                }
                BlockKind::List { items, .. } => {
                    for item in items {
                        self.expand_blocks(&mut item.blocks, settings);
                    }
                }
                BlockKind::Table {
                    header,
                    rows,
                    attrs,
                    ..
                } => {
                    self.expand_caption(attrs);
                    for cell in header.iter_mut().chain(rows.iter_mut().flatten()) {
                        self.expand_inlines(cell);
                    }
                }
                BlockKind::MathBlock { attrs, .. } => self.expand_caption(attrs),
                BlockKind::FootnoteDefinition { blocks, .. } => {
                    self.expand_blocks(blocks, settings)
                }
                _ => {}
            }
        }
    }
    fn expand_caption(&self, attrs: &mut Option<Attributes>) {
        if let Some(attrs) = attrs {
            if let Some(label) = attrs.id.as_ref().and_then(|id| self.labels.get(id)) {
                attrs.caption = Some(match &attrs.caption {
                    Some(caption) => format!("{label}. {caption}"),
                    None => label.clone(),
                });
            }
        }
    }
}

/// Se transforma únicamente una copia derivada del AST, nunca el Markdown.
/// Los IDs duplicados se diagnostican aunque no haya un token que los use.
pub fn resolve_cross_references(ast: &Ast, settings: &Settings) -> (Ast, Vec<String>) {
    let mut settings = settings.clone();
    if let Some(fm) = &ast.front_matter {
        for line in fm.lines() {
            if let Some(("language", value)) =
                line.split_once(':').map(|(k, v)| (k.trim(), v.trim()))
            {
                settings.language = value.trim_matches(['\'', '"']).to_owned();
            }
        }
    }
    let mut context = Context::default();
    context.blocks(&ast.blocks, &settings);
    let mut derived = ast.clone();
    context.expand_blocks(&mut derived.blocks, &settings);
    let diagnostics = context
        .duplicates
        .iter()
        .map(|id| format!("ID duplicado: {id}"))
        .collect();
    (derived, diagnostics)
}

/// IDs declarados, para convertir únicamente destinos internos existentes
/// en enlaces nativos del PDF (un enlace roto no impide exportar).
pub fn reference_ids(ast: &Ast) -> BTreeSet<String> {
    let mut context = Context::default();
    context.blocks(&ast.blocks, &Settings::default());
    context.labels.into_keys().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "# Inicio {#intro}\n\n![á](image.png){#fig:one}\n\n| A |\n| - |\n| B |\n{#tbl:one}\n\n$$\nx=1\n$$\n{#eq:one}\n\n{{ref:intro}} {{ref:fig:one}} {{ref:tbl:one}} {{ref:eq:one}}\n";
    #[test]
    fn pie_de_figura_debajo_con_alt_escapado_e_idioma_sin_mutar_ast() {
        let source = "![**A** & B](image.svg){#fig:one}\n\n![Sin pie](icon.svg)";
        let ast = minmark_parser::parse(source);
        let before = ast.clone();
        for (language, name) in [("es", "Figura"), ("en", "Figure")] {
            let settings = Settings {
                language: language.into(),
                ..Default::default()
            };
            let html = crate::render_preview(&ast, source, &settings);
            assert!(
                html.contains(&format!(
                    "/><span class=\"mm-caption\">{name} 1. A &amp; B</span>"
                )),
                "{html}"
            );
            assert_eq!(html.matches("class=\"mm-caption\"").count(), 1);
        }
        assert_eq!(ast, before);
    }

    #[test]
    fn etiquetas_destinos_e_idioma_compartidos() {
        let ast = minmark_parser::parse(SOURCE);
        let before = ast.clone();
        for (lang, labels) in [
            ("es", ["Figura 1", "Tabla 1", "Ecuación 1"]),
            ("en", ["Figure 1", "Table 1", "Equation 1"]),
        ] {
            let settings = Settings {
                language: lang.into(),
                ..Default::default()
            };
            let html = crate::render_preview(&ast, SOURCE, &settings);
            for label in labels {
                assert!(html.contains(label), "{html}");
            }
            for id in ["intro", "fig:one", "tbl:one", "eq:one"] {
                assert!(html.contains(&format!("href=\"#{id}\"")), "{id}: {html}");
                assert!(html.contains(&format!("id=\"{id}\"")), "{id}: {html}");
            }
        }
        assert_eq!(ast, before);
    }
    #[test]
    fn front_matter_prevalece_y_duplicados_y_faltantes_son_visibles() {
        let source = "---\nlanguage: en\n---\n\n![a](x){#fig:a}\n\n{{ref:fig:a}} {{ref:no}}\n";
        let html =
            crate::render_preview(&minmark_parser::parse(source), source, &Settings::default());
        assert!(html.contains("Figure 1"));
        assert!(html.contains("Referencia inexistente: no"));
        let source = "# A {#same}\n\n# B {#same}\n\n{{ref:same}}";
        let (ast, errors) =
            resolve_cross_references(&minmark_parser::parse(source), &Settings::default());
        assert_eq!(errors, ["ID duplicado: same"]);
        assert!(format!("{ast:?}").contains("ID duplicado: same"));
    }
    #[test]
    fn renumerar_por_capitulo_y_headings_sin_tocar_fuente() {
        let source = "# Uno\n\n![a](x){#fig:a}\n\n# Dos\n\n## Parte {#parte}\n\n![b](x){#fig:b}\n\n{{ref:fig:b}} {{ref:parte}}";
        let settings = Settings {
            counter_numbering: NumberingScope::ByChapter,
            heading_numbering: true,
            ..Default::default()
        };
        let html = crate::render_preview(&minmark_parser::parse(source), source, &settings);
        assert!(html.contains("Figura 2.1"));
        assert!(html.contains("class=\"mm-caption\">Figura 2.1. b</span>"));
        assert!(html.contains("2.1 Parte"));
        let html =
            crate::render_preview(&minmark_parser::parse(source), source, &Settings::default());
        assert!(html.contains("Figura 2"));
        assert!(html.contains("class=\"mm-caption\">Figura 2. b</span>"));
        assert!(!html.contains("2.1 Parte"));
    }
}
