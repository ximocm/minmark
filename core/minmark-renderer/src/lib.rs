//! Renderer de Preview de minmark: AST → página HTML (ISSUE-016).
//!
//! Preview **continuo** (§10.1–10.2): sin límites de página, ancho máximo
//! configurable (`preview_max_width_px`, defecto 800 px). La paginación solo
//! existe en Page Mode y PDF (§10.3).
//!
//! - Temas Light/Dark/System (§11) vía `data-theme` + CSS con variables;
//!   la detección del tema del sistema y las paletas de UI son ISSUE-017.
//! - Cada bloque lleva `data-mm-span="inicio,fin"` con su rango en el fuente:
//!   es el puente para el scroll sync y el doble clic (§9.1) con
//!   [`ScrollMap`] y `Editor::position_of`.
//! - Regla normativa §7.2: **nunca** se emite HTML crudo del documento; todo
//!   el contenido se escapa.
//!
//! La WebView (WebKitGTK 6.0, ADR-001) carga esta página en ISSUE-020.

mod scroll;

pub use palette::{theme_css, Palette};
pub use scroll::{Anchor, ScrollMap};

mod palette;

use minmark_config::{Settings, Theme};
use minmark_document::{
    Align, Ast, Attributes, Block, BlockKind, CitationItem, Inline, ListItem, Span, Substitution,
};

/// Renderiza la AST a la página completa del Preview.
pub fn render_preview(ast: &Ast, settings: &Settings) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html data-theme=\"");
    out.push_str(match settings.theme {
        Theme::Light => "light",
        Theme::Dark => "dark",
        Theme::System => "system",
    });
    out.push_str("\">\n<head>\n<meta charset=\"utf-8\">\n<title>minmark</title>\n<style>\n");
    out.push_str(&theme_css());
    out.push_str(BASE_CSS);
    out.push_str("</style>\n</head>\n<body>\n<div class=\"mm-page\" style=\"max-width: ");
    out.push_str(&settings.preview_max_width_px.to_string());
    out.push_str("px\">\n<div class=\"minmark-preview\">");
    for block in &ast.blocks {
        render_block(block, settings, &mut out);
    }
    out.push_str("</div>\n</div>\n</body>\n</html>\n");
    out
}

/// CSS base del Preview (§11): reglas sobre las variables que genera
/// [`theme_css`] desde las paletas [`Palette`].
const BASE_CSS: &str = r#"body { margin: 0; background: var(--mm-bg); color: var(--mm-fg);
  font-family: system-ui, sans-serif; line-height: 1.6; }
.mm-page { margin: 0 auto; padding: 2rem 1.25rem; }
.minmark-preview { overflow-wrap: break-word; }
.minmark-preview code { background: var(--mm-code-bg); padding: 0.1em 0.3em; border-radius: 4px; }
.minmark-preview pre { background: var(--mm-code-bg); padding: 0.75em 1em; border-radius: 6px;
  overflow-x: auto; }
.minmark-preview pre code { background: none; padding: 0; }
.minmark-preview blockquote { border-left: 4px solid var(--mm-rule); margin-left: 0;
  padding-left: 1em; color: var(--mm-quote); }
.minmark-preview table { border-collapse: collapse; }
.minmark-preview th, .minmark-preview td { border: 1px solid var(--mm-rule); padding: 0.35em 0.75em; }
.minmark-preview hr { border: none; border-top: 1px solid var(--mm-rule); }
.minmark-preview img { max-width: 100%; height: auto; }
.mm-callout { border-left: 4px solid var(--mm-rule); padding: 0.5em 1em; border-radius: 4px;
  background: var(--mm-code-bg); }
.mm-callout-title { font-weight: 600; margin: 0 0 0.25em; }
.mm-callout-note { border-left-color: var(--mm-color-note); }
.mm-callout-info { border-left-color: var(--mm-color-info); }
.mm-callout-tip { border-left-color: var(--mm-color-success); }
.mm-callout-warning { border-left-color: var(--mm-color-warning); }
.mm-callout-danger { border-left-color: var(--mm-color-danger); }
.mm-callout-success { border-left-color: var(--mm-color-success); }
.mm-caption { color: var(--mm-muted); font-size: 0.9em; text-align: center; }
.mm-math { font-family: serif; }
.mm-math-block { text-align: center; margin: 1em 0; font-family: serif; }
.mm-ref, .mm-citation { color: var(--mm-color-accent); }
.mm-pagebreak { border-top: 1px dashed var(--mm-rule); margin: 2em 0; }
.mm-toc { border: 1px dashed var(--mm-rule); padding: 0.5em 1em; border-radius: 4px; }
"#;

fn render_block(block: &Block, settings: &Settings, out: &mut String) {
    match &block.kind {
        BlockKind::Heading {
            level,
            content,
            attrs,
        } => {
            let level = (*level).clamp(1, 6);
            out.push_str("<h");
            out.push(char::from(b'0' + level));
            push_span(out, block.span);
            push_id(out, attrs);
            out.push('>');
            render_inlines(content, settings, out);
            out.push_str("</h");
            out.push(char::from(b'0' + level));
            out.push_str(">\n");
        }
        BlockKind::Paragraph { content } => {
            out.push_str("<p");
            push_span(out, block.span);
            out.push('>');
            render_inlines(content, settings, out);
            out.push_str("</p>\n");
        }
        BlockKind::Quote { blocks } => {
            out.push_str("<blockquote");
            push_span(out, block.span);
            out.push_str(">\n");
            render_blocks(blocks, settings, out);
            out.push_str("</blockquote>\n");
        }
        BlockKind::List {
            ordered,
            start,
            items,
        } => {
            out.push_str(if *ordered { "<ol" } else { "<ul" });
            push_span(out, block.span);
            if *ordered && *start != 1 {
                out.push_str(" start=\"");
                push_escaped(out, &start.to_string());
                out.push('"');
            }
            out.push_str(">\n");
            for item in items {
                render_list_item(item, settings, out);
            }
            out.push_str(if *ordered { "</ol>\n" } else { "</ul>\n" });
        }
        BlockKind::CodeBlock { language, code } => {
            out.push_str("<pre");
            push_span(out, block.span);
            out.push('>');
            match language {
                Some(lang) => {
                    out.push_str("<code class=\"language-");
                    push_escaped(out, lang);
                    out.push_str("\">");
                }
                None => out.push_str("<code>"),
            }
            push_escaped(out, code);
            out.push_str("</code></pre>\n");
        }
        BlockKind::Table {
            aligns,
            header,
            rows,
            attrs,
        } => {
            out.push_str("<table");
            push_span(out, block.span);
            push_id(out, attrs);
            out.push_str(">\n");
            push_caption(out, attrs);
            out.push_str("<thead><tr>");
            for (i, cell) in header.iter().enumerate() {
                push_cell(out, "th", cell, aligns.get(i), settings);
            }
            out.push_str("</tr></thead>\n<tbody>\n");
            for row in rows {
                out.push_str("<tr>");
                for (i, cell) in row.iter().enumerate() {
                    push_cell(out, "td", cell, aligns.get(i), settings);
                }
                out.push_str("</tr>\n");
            }
            out.push_str("</tbody>\n</table>\n");
        }
        BlockKind::MathBlock { latex, attrs } => {
            // §24: LaTeX de entrada; el motor la compone (ISSUE-018 para PDF).
            out.push_str("<div class=\"mm-math-block\"");
            push_span(out, block.span);
            push_id(out, attrs);
            out.push('>');
            push_math(out, latex);
            out.push_str("</div>\n");
            push_caption(out, attrs);
        }
        BlockKind::Callout {
            callout_type,
            title,
            blocks,
        } => {
            // §22.
            out.push_str("<aside class=\"mm-callout mm-callout-");
            push_escaped(out, callout_type);
            out.push('"');
            push_span(out, block.span);
            out.push_str(">\n");
            if let Some(title) = title {
                out.push_str("<p class=\"mm-callout-title\">");
                push_escaped(out, title);
                out.push_str("</p>\n");
            }
            render_blocks(blocks, settings, out);
            out.push_str("</aside>\n");
        }
        BlockKind::Rule => {
            out.push_str("<hr");
            push_span(out, block.span);
            out.push_str(" />\n");
        }
    }
}

fn render_blocks(blocks: &[Block], settings: &Settings, out: &mut String) {
    for block in blocks {
        render_block(block, settings, out);
    }
}

fn render_list_item(item: &ListItem, settings: &Settings, out: &mut String) {
    out.push_str("<li");
    push_span(out, item.span);
    out.push('>');
    render_blocks(&item.blocks, settings, out);
    out.push_str("</li>\n");
}

fn push_cell(
    out: &mut String,
    tag: &str,
    content: &[Inline],
    align: Option<&Align>,
    settings: &Settings,
) {
    out.push('<');
    out.push_str(tag);
    match align {
        Some(Align::Left) => out.push_str(" style=\"text-align: left\""),
        Some(Align::Center) => out.push_str(" style=\"text-align: center\""),
        Some(Align::Right) => out.push_str(" style=\"text-align: right\""),
        _ => {}
    }
    out.push('>');
    render_inlines(content, settings, out);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// Marca el elemento con su rango en el fuente (§9.1: sync y doble clic).
fn push_span(out: &mut String, span: Span) {
    out.push_str(" data-mm-span=\"");
    out.push_str(&span.start.to_string());
    out.push(',');
    out.push_str(&span.end.to_string());
    out.push('"');
}

fn push_id(out: &mut String, attrs: &Option<Attributes>) {
    if let Some(id) = attrs.as_ref().and_then(|a| a.id.as_deref()) {
        out.push_str(" id=\"");
        push_escaped(out, id);
        out.push('"');
    }
}

fn push_caption(out: &mut String, attrs: &Option<Attributes>) {
    if let Some(caption) = attrs.as_ref().and_then(|a| a.caption.as_deref()) {
        out.push_str("<div class=\"mm-caption\">");
        push_escaped(out, caption);
        out.push_str("</div>\n");
    }
}

/// Valor CSS de un color por nombre (§23): variable de tema con el valor del
/// proyecto (§58.2 `custom_colors`) como fallback.
fn color_value(name: &str, settings: &Settings) -> String {
    match settings.custom_colors.get(name) {
        Some(value) => format!("var(--mm-color-{name}, {value})"),
        None => format!("var(--mm-color-{name})"),
    }
}

fn render_inlines(inlines: &[Inline], settings: &Settings, out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => push_escaped(out, text),
            Inline::Code(code) => {
                out.push_str("<code>");
                push_escaped(out, code);
                out.push_str("</code>");
            }
            Inline::Emphasis(content) => wrap(out, "em", content, settings),
            Inline::Strong(content) => wrap(out, "strong", content, settings),
            Inline::Strikethrough(content) => wrap(out, "del", content, settings),
            Inline::Link {
                destination,
                title,
                content,
            } => {
                out.push_str("<a href=\"");
                push_escaped(out, destination);
                out.push('"');
                if !title.is_empty() {
                    out.push_str(" title=\"");
                    push_escaped(out, title);
                    out.push('"');
                }
                out.push('>');
                render_inlines(content, settings, out);
                out.push_str("</a>");
            }
            Inline::Image {
                destination,
                title,
                alt,
                attrs,
            } => {
                out.push_str("<img src=\"");
                push_escaped(out, destination);
                out.push_str("\" alt=\"");
                push_escaped(out, &plain_text(alt));
                out.push('"');
                if !title.is_empty() {
                    out.push_str(" title=\"");
                    push_escaped(out, title);
                    out.push('"');
                }
                push_id(out, attrs);
                // §18.5: tamaño solo en porcentaje; el aspecto lo preserva CSS.
                if let Some(width) = attrs.as_ref().and_then(|a| a.width_percent) {
                    out.push_str(" style=\"width: ");
                    out.push_str(&width.to_string());
                    out.push_str("%\"");
                }
                out.push_str(" />");
            }
            Inline::Math(latex) => {
                out.push_str("<span class=\"mm-math\">");
                push_math(out, latex);
                out.push_str("</span>");
            }
            Inline::Color {
                foreground,
                background,
                text,
            } => {
                // §23: `{fg: t}` color de texto, `{+bg: t}` fondo,
                // `{fg+bg: t}` ambos.
                out.push_str("<span class=\"mm-color\"");
                if foreground.is_some() || background.is_some() {
                    out.push_str(" style=\"");
                    if let Some(fg) = foreground {
                        out.push_str("color: ");
                        out.push_str(&color_value(fg, settings));
                        if background.is_some() {
                            out.push(';');
                        }
                    }
                    if let Some(bg) = background {
                        out.push_str("background-color: ");
                        out.push_str(&color_value(bg, settings));
                    }
                    out.push('"');
                }
                out.push('>');
                push_escaped(out, text);
                out.push_str("</span>");
            }
            Inline::Citation { items } => {
                out.push_str("<span class=\"mm-citation\">");
                push_escaped(out, &citation_text(items));
                out.push_str("</span>");
            }
            Inline::WikiLink { target } => {
                out.push_str("<a class=\"mm-wiki-link\" data-target=\"");
                push_escaped(out, target);
                out.push_str("\">");
                push_escaped(out, target);
                out.push_str("</a>");
            }
            Inline::Substitution(sub) => match sub {
                // §29, §31, §26.5: expansión con contexto numérico pendiente.
                Substitution::Toc => out.push_str("<nav class=\"mm-toc\"></nav>"),
                Substitution::PageBreak => out.push_str("<div class=\"mm-pagebreak\"></div>"),
                Substitution::CrossRef { id } => {
                    out.push_str("<span class=\"mm-ref\" data-ref=\"");
                    push_escaped(out, id);
                    out.push_str("\">{{ref:");
                    push_escaped(out, id);
                    out.push_str("}}</span>");
                }
            },
            Inline::TaskListMarker(checked) => {
                out.push_str("<input type=\"checkbox\" disabled");
                if *checked {
                    out.push_str(" checked");
                }
                out.push_str(" />");
            }
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("<br />\n"),
        }
    }
}

/// Texto de reserva de una cita (§25.2), hasta la composición real.
fn citation_text(items: &[CitationItem]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str("; ");
        }
        out.push('@');
        out.push_str(&item.key);
        if let Some(locator) = &item.locator {
            out.push_str(", ");
            out.push_str(locator);
        }
    }
    out.push(']');
    out
}

fn wrap(out: &mut String, tag: &str, content: &[Inline], settings: &Settings) {
    out.push('<');
    out.push_str(tag);
    out.push('>');
    render_inlines(content, settings, out);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// Texto plano de una secuencia de inlines (para `alt` de imágenes).
fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    fn walk(inlines: &[Inline], out: &mut String) {
        for inline in inlines {
            match inline {
                Inline::Text(t) | Inline::Code(t) | Inline::Math(t) => out.push_str(t),
                Inline::Emphasis(c) | Inline::Strong(c) | Inline::Strikethrough(c) => walk(c, out),
                Inline::Link { content, .. } | Inline::Image { alt: content, .. } => {
                    walk(content, out)
                }
                Inline::Color { text, .. } => out.push_str(text),
                Inline::Citation { items } => out.push_str(&citation_text(items)),
                Inline::WikiLink { target } => out.push_str(target),
                Inline::Substitution(Substitution::CrossRef { id }) => {
                    out.push_str("{{ref:");
                    out.push_str(id);
                    out.push_str("}}");
                }
                Inline::Substitution(_) => {}
                Inline::TaskListMarker(_) => {}
                Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            }
        }
    }
    walk(inlines, &mut out);
    out
}

/// Muestra la math o el marcador de sintaxis no soportada (ADR-002, NFR-026:
/// nunca silenciar, omitir ni deformar).
fn push_math(out: &mut String, latex: &str) {
    let translation = minmark_math::translate(latex);
    if translation.is_supported() {
        push_escaped(out, latex);
    } else {
        out.push_str("<span class=\"mm-math-error\">");
        push_escaped(out, &translation.marker());
        out.push_str("</span>");
    }
}

/// Escapa texto o atributos HTML (§7.2: nada del documento sale crudo).
fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_parser::parse;

    fn html(source: &str) -> String {
        render_preview(&parse(source), &Settings::default())
    }

    #[test]
    fn pagina_completa_con_tema_y_ancho() {
        // §10.2: ancho máximo por defecto 800 px, configurable. §11: tema.
        let out = html("# T\n");
        assert!(out.starts_with("<!DOCTYPE html>"));
        assert!(out.contains("<html data-theme=\"system\">"));
        assert!(out.contains("style=\"max-width: 800px\""));
        // El CSS sale de las paletas (§11): ambos temas presentes.
        assert!(out.contains("--mm-bg: #ffffff"));
        assert!(out.contains("--mm-bg: #15181c"));

        for (theme, expected) in [
            (Theme::Light, "<html data-theme=\"light\">"),
            (Theme::Dark, "<html data-theme=\"dark\">"),
            (Theme::System, "<html data-theme=\"system\">"),
        ] {
            let settings = Settings {
                theme,
                preview_max_width_px: 640,
                ..Settings::default()
            };
            let out = render_preview(&parse("# T\n"), &settings);
            assert!(out.contains(expected));
            assert!(out.contains("style=\"max-width: 640px\""));
        }
    }

    #[test]
    fn escapa_todo_el_contenido_del_documento() {
        // §7.2: nunca se ejecuta contenido activo.
        let out = html("<script>alert(1)</script>\n");
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn headings_parrafos_y_listas() {
        let out = html("# Título\n\nCuerpo con *énfasis* y **fuerte**.\n\n3. tres\n");
        assert!(out.contains(">Título</h1>"));
        assert!(out.contains(">Cuerpo con <em>énfasis</em> y <strong>fuerte</strong>.</p>"));
        assert!(out.contains("<ol data-mm-span="));
    }

    #[test]
    fn codigo_con_lenguaje_y_escape() {
        let out = html("```rs\nlet a = 1 < 2;\n```\n");
        assert!(out.contains("<pre data-mm-span="));
        assert!(out.contains("<code class=\"language-rs\">"));
        assert!(out.contains("1 &lt; 2;"));
        assert!(!out.contains("1 < 2"));
    }

    #[test]
    fn tabla_gfm() {
        let out = html("| A | B |\n|---|:-:|\n| 1 | 2 |\n");
        assert!(out.contains("<table data-mm-span="));
        assert!(out.contains("<th>A</th>"));
        assert!(out.contains("<th style=\"text-align: center\">B</th>"));
    }

    #[test]
    fn callouts_math_colores_y_sustituciones() {
        let out = html(
            "::: warning \"Atención\"\nCuidado {danger: aquí}.\n:::\n\n$$\nE = mc^2\n$$\n{#eq:energy}\n",
        );
        assert!(out.contains("<aside class=\"mm-callout mm-callout-warning\""));
        assert!(out.contains("<p class=\"mm-callout-title\">Atención</p>"));
        // §23: color de texto con variable de tema.
        assert!(out.contains("color: var(--mm-color-danger)"));
        assert!(out.contains("<div class=\"mm-math-block\" data-mm-span="));

        let out = html("Ver {{ref:fig:arq}} y {{toc}}.\n");
        assert!(out.contains("data-ref=\"fig:arq\""));
        assert!(out.contains("<nav class=\"mm-toc\"></nav>"));
    }

    #[test]
    fn colores_personalizados_del_proyecto() {
        // §23/§58.2: la paleta se redefine por proyecto — el nombre debe
        // existir al parsear (ParseOptions) y su valor se aplica al renderizar.
        let custom = ["brand".to_string()];
        let options = minmark_parser::ParseOptions {
            custom_colors: &custom,
            ..Default::default()
        };
        let ast = minmark_parser::parse_with("{brand: x}\n", &options);
        let mut settings = Settings::default();
        settings
            .custom_colors
            .insert("brand".to_string(), "#336699".to_string());
        let out = render_preview(&ast, &settings);
        assert!(out.contains("color: var(--mm-color-brand, #336699)"));
    }

    #[test]
    fn data_mm_span_cubre_el_fuente() {
        // §9.1: cada elemento marcado con su rango en el fuente.
        let source = "# H\n\ntexto\n\n> cita\n";
        let out = html(source);
        let ast = parse(source);
        for block in &ast.blocks {
            let marker = format!("data-mm-span=\"{},{}\"", block.span.start, block.span.end);
            assert!(out.contains(&marker), "falta {marker} en el HTML");
        }
    }

    #[test]
    fn math_no_soportada_muestra_marcador() {
        // ADR-002 / NFR-026: nunca silenciar ni deformar.
        let out = html("$$\n\\foo{x}\n$$\n");
        assert!(out.contains("mm-math-error"));
        assert!(out.contains("[math no soportado: \\foo]"));

        // Math soportada se muestra como fuente LaTeX.
        let out = html("$$\nE = mc^2\n$$\n");
        assert!(out.contains("mm-math-block"));
        assert!(!out.contains("no soportado"));
    }

    #[test]
    fn sync_bidireccional_con_scroll_map() {
        // §9.1: posición semántica, no porcentaje bruto.
        let source = "# H\n\nprimer párrafo\n\nsegundo párrafo\n";
        let ast = parse(source);
        let map = ScrollMap::for_ast(&ast);

        let offset = source.find("segundo").unwrap() as u32;
        let anchor = map.anchor_for_offset(offset).unwrap();
        assert_eq!(map.offset_for_anchor(anchor), Some(ast.blocks[2].span.start));
        // Doble clic (§9.1): rango del elemento → posición del editor.
        let mut editor =
            minmark_document::Editor::new(minmark_document::Document::from_text(
                None,
                source.to_string(),
            ));
        let position = editor.position_of(map.offset_for_anchor(anchor).unwrap() as usize);
        editor
            .set_selection(minmark_document::Selection::collapsed(position))
            .unwrap();
        assert_eq!(position, minmark_document::Position::new(4, 0));
    }
}
