//! Generación de código Typst desde la AST (ISSUE-018, §5.3).
//!
//! Pipeline normativo (§5.3): Markdown → AST → **composición Typst** → PDF.
//! El usuario nunca escribe Typst (§24): esta generación es interna y traduce
//! math LaTeX con [`minmark_math`] (ADR-002).
//!
//! Apariencia (§32.2): fondo blanco, texto oscuro, siempre. Configuración
//! básica (§58.2): A4, márgenes 25 mm, Portrait — aplicada desde `Settings`.

use crate::{first_h1_title, page_size_mm, DocumentMeta};
use minmark_config::{
    FontFamily, Orientation, PageNumberPosition, ParagraphStyle, Settings, TextAlignment,
};
use minmark_document::{Ast, Block, BlockKind, CitationItem, Inline, ListItem, Substitution};
use minmark_renderer::Palette;
use std::fmt::Write;

/// Genera el documento Typst completo para la AST, los ajustes y los
/// metadatos documentales (§32.9) dados.
pub fn generate(ast: &Ast, settings: &Settings, meta: &DocumentMeta) -> String {
    let (ast, mut diagnostics) = minmark_renderer::Bibliography::default().resolve(ast);
    let (ast, ref_diagnostics) = minmark_renderer::resolve_cross_references(&ast, settings);
    diagnostics.extend(ref_diagnostics);
    let (ast, note_diagnostics) = minmark_renderer::resolve_footnotes(&ast);
    diagnostics.extend(note_diagnostics);
    let labels = minmark_renderer::reference_ids(&ast);
    let tables = crate::fit::table_identities(&ast);
    let mut out = preamble(&ast, settings, meta);
    for block in &ast.blocks {
        if let BlockKind::FootnoteDefinition { label, blocks } = &block.kind {
            let _ = writeln!(out, "#let mm-note-{label} = [");
            render_blocks(blocks, settings, &labels, &tables, &mut out);
            out.push_str("]\n");
        }
    }
    render_blocks(&ast.blocks, settings, &labels, &tables, &mut out);
    for diagnostic in diagnostics {
        out.push_str(&escape_text(&diagnostic));
        out.push_str("\n\n");
    }
    out
}

fn preamble(ast: &Ast, settings: &Settings, meta: &DocumentMeta) -> String {
    // §32.3–32.6: tamaño, márgenes en mm (§32.4) y orientación.
    let (mut width, mut height) = page_size_mm(settings.pdf_page_size);
    if settings.pdf_orientation == Orientation::Landscape {
        std::mem::swap(&mut width, &mut height);
    }
    let margins = &settings.pdf_margins_mm;
    // §32.9: estado con el texto plano del H1 más reciente, para {chapter}.
    // Se declara antes de `#set page` porque las plantillas lo leen.
    let mut out = String::from("#let mm-chapter = state(\"minmark-chapter\", \"\")\n");
    // §35.2: helpers del reparto de tablas (mínimas por columna); puros
    // `#let` sin dependencias, se emiten una sola vez.
    out.push_str(crate::fit::TRACK_HELPERS);
    let _ = write!(
        out,
        "#set page(width: {width}mm, height: {height}mm, \
         margin: (top: {}mm, right: {}mm, bottom: {}mm, left: {}mm)",
        margins.top_mm, margins.right_mm, margins.bottom_mm, margins.left_mm
    );

    // §32.9: plantillas de header/footer; vacías o None = sin marginal.
    // §32.7–32.8: Typst descarta su número cuando el marginal de `number-align`
    // lleva contenido explícito, así que si la plantilla ocupa ese mismo lado,
    // el número se emite dentro de la plantilla; si no, sigue al número
    // automático de `numbering`.
    let position = settings.pdf_page_number_position;
    let number_in_header = settings.pagination_enabled
        && matches!(
            position,
            PageNumberPosition::HeaderLeft
                | PageNumberPosition::HeaderCenter
                | PageNumberPosition::HeaderRight
        );
    if let Some(template) = active_template(&settings.pdf_header) {
        let _ = write!(
            out,
            ", header: {}",
            marginal(template, ast, settings, meta, number_in_header, position)
        );
    }
    if let Some(template) = active_template(&settings.pdf_footer) {
        let _ = write!(
            out,
            ", footer: {}",
            marginal(
                template,
                ast,
                settings,
                meta,
                settings.pagination_enabled && !number_in_header,
                position
            )
        );
    }
    if settings.pagination_enabled {
        // §32.7–32.8: números de página solo con paginación activada.
        let _ = write!(
            out,
            ", numbering: \"1\", number-align: {}",
            number_align(position)
        );
    }
    out.push_str(")\n");

    // §33: tipografía (sans/serif/mono libres; todas resuelven en el World
    // embebido). §33.1: el default sans-serif es Liberation Sans —antes la
    // cadena apuntaba a familias inexistentes (Inter/DejaVu Sans) y caía a
    // una serif— con Libertinus Serif como último recurso.
    let fonts = match settings.body_font {
        FontFamily::SansSerif => "\"Liberation Sans\", \"Libertinus Serif\"",
        FontFamily::Serif => "\"Libertinus Serif\", \"New Computer Modern\"",
        FontFamily::Monospace => "\"DejaVu Sans Mono\", \"New Computer Modern\"",
    };
    let _ = write!(
        out,
        "#set text(font: ({fonts}), size: {}pt)\n",
        settings.font_size_pt
    );
    // §33.3 interlineado; §33.4 párrafos.
    let leading = (settings.line_height - 1.0).max(0.0);
    let _ = writeln!(out, "#set par(leading: {leading:.2}em)");
    match settings.paragraph_style {
        ParagraphStyle::VerticalSpacing => out.push_str("#set par(first-line-indent: 0pt)\n"),
        ParagraphStyle::Indented => out.push_str("#set par(first-line-indent: 1.5em)\n"),
    }
    // §33.5 alineación (solo el cuerpo; los márgenes se alinean a mano en
    // [`marginal`] para que las plantillas siempre empiecen por la izquierda).
    match settings.text_alignment {
        TextAlignment::Left => {}
        TextAlignment::Center => out.push_str("#set align(center)\n"),
        TextAlignment::Right => out.push_str("#set align(right)\n"),
    }
    out.push('\n');
    out
}

/// §32.9: plantilla activa de un marginal; `None` si no existe o está vacía
/// (el comportamiento por defecto es sin header/footer).
fn active_template(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|text| !text.trim().is_empty())
}

/// Compone el contenido de un marginal (§32.9): la plantilla alineada a la
/// izquierda y, cuando Typst descarta su número (§32.8), el número de página
/// en la posición `pdf_page_number_position`.
fn marginal(
    template: &str,
    ast: &Ast,
    settings: &Settings,
    meta: &DocumentMeta,
    embed_number: bool,
    position: PageNumberPosition,
) -> String {
    let body = render_template(template, ast, settings, meta);
    if !embed_number {
        // El argumento de `#set page` se evalúa en modo código: sin `#`, y el
        // cuerpo `[...]` vuelve a ser markup.
        return format!("align(left)[{body}]");
    }
    let number = "#(context counter(page).display(\"1\"))";
    match position {
        PageNumberPosition::HeaderLeft | PageNumberPosition::FooterLeft => format!(
            "grid(columns: (auto, 1fr), gutter: .5em, align: (left, left), [{number}], [{body}])"
        ),
        PageNumberPosition::HeaderCenter | PageNumberPosition::FooterCenter => format!(
            "grid(columns: (1fr, auto, 1fr), gutter: .5em, align: (left, center, left), \
             [{body}], [{number}], [])"
        ),
        PageNumberPosition::HeaderRight | PageNumberPosition::FooterRight => format!(
            "grid(columns: (1fr, auto), gutter: .5em, align: (left, right), [{body}], [{number}])"
        ),
    }
}

/// §32.9: expande una plantilla de header/footer a markup Typst. Las variables
/// conocidas se sustituyen: `{title}` (con fallback al primer H1), `{author}`,
/// `{project}` y `{date}` por su valor escapado; `{chapter}` por el estado que
/// actualiza cada H1; `{page}` y `{pages}` por el contador de páginas, vacíos
/// sin paginación (§32.7). Cualquier otra `{...}` queda literal (§7.4.5) y el
/// resto del texto se escapa con [`escape_text`].
fn render_template(template: &str, ast: &Ast, settings: &Settings, meta: &DocumentMeta) -> String {
    let title = meta
        .title
        .clone()
        .unwrap_or_else(|| first_h1_title(ast).unwrap_or_default());
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}').map(|index| open + index) else {
            out.push_str(&escape_text(&rest[open..]));
            return out;
        };
        out.push_str(&escape_text(&rest[..open]));
        let name = &rest[open + 1..close];
        match name {
            "title" => out.push_str(&escape_text(&title)),
            "author" => out.push_str(&escape_text(meta.author.as_deref().unwrap_or_default())),
            "project" => out.push_str(&escape_text(meta.project.as_deref().unwrap_or_default())),
            "date" => out.push_str(&escape_text(meta.date.as_deref().unwrap_or_default())),
            "chapter" => out.push_str("#(context mm-chapter.get())"),
            "page" if settings.pagination_enabled => {
                out.push_str("#(context counter(page).display(\"1\"))")
            }
            "pages" if settings.pagination_enabled => {
                out.push_str("#(context numbering(\"1\", ..counter(page).final()))")
            }
            "page" | "pages" => {} // §32.7: sin paginación, vacíos.
            _ => out.push_str(&escape_text(&format!("{{{name}}}"))),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(&escape_text(rest));
    out
}

fn number_align(position: PageNumberPosition) -> &'static str {
    match position {
        PageNumberPosition::HeaderLeft => "top + left",
        PageNumberPosition::HeaderCenter => "top + center",
        PageNumberPosition::HeaderRight => "top + right",
        PageNumberPosition::FooterLeft => "bottom + left",
        PageNumberPosition::FooterCenter => "bottom + center",
        PageNumberPosition::FooterRight => "bottom + right",
    }
}

fn render_blocks(
    blocks: &[Block],
    settings: &Settings,
    labels: &std::collections::BTreeSet<String>,
    tables: &std::collections::BTreeMap<u32, String>,
    out: &mut String,
) {
    for block in blocks {
        render_block(block, settings, labels, tables, out);
    }
}

fn render_block(
    block: &Block,
    settings: &Settings,
    labels: &std::collections::BTreeSet<String>,
    tables: &std::collections::BTreeMap<u32, String>,
    out: &mut String,
) {
    let attrs = match &block.kind {
        BlockKind::Heading { attrs, .. }
        | BlockKind::Table { attrs, .. }
        | BlockKind::MathBlock { attrs, .. } => attrs.as_ref(),
        _ => None,
    };
    if let Some(id) = attrs.and_then(|a| a.id.as_ref()) {
        let _ = writeln!(out, "#box[]#label({})", typst_string(id));
    }
    match &block.kind {
        BlockKind::Bibliography {
            entries,
            hanging_indent,
            second_field_align,
            line_spacing,
            entry_spacing,
        } => {
            out.push_str("#block[\n");
            let _ = writeln!(
                out,
                "#set par(leading: {}em, hanging-indent: {}em)",
                (line_spacing - 1.0).max(0.0),
                if *hanging_indent && !*second_field_align {
                    1.5
                } else {
                    0.0
                }
            );
            out.push_str(if settings.language.starts_with("es") {
                "*Bibliografía*\n\n"
            } else {
                "*References*\n\n"
            });
            for (label, content) in entries {
                if *second_field_align {
                    out.push_str("#grid(columns: (auto, 1fr), gutter: .5em, [");
                    render_inlines(label, settings, labels, out);
                    out.push_str("], [");
                    render_inlines(content, settings, labels, out);
                    out.push_str("])\n");
                } else {
                    render_inlines(label, settings, labels, out);
                    if !label.is_empty() {
                        out.push(' ');
                    }
                    render_inlines(content, settings, labels, out);
                    out.push_str("\n\n");
                }
                let _ = writeln!(out, "#v({entry_spacing}em)");
            }
            out.push_str("]\n\n");
        }
        BlockKind::Heading { level, content, .. } => {
            if *level == 1 {
                // §32.9: {chapter} lee este estado; el footer lo ve al final
                // de la página y el header al empezarla (orden de frames).
                let _ = writeln!(
                    out,
                    "#mm-chapter.update({})",
                    typst_string(&plain_text(content))
                );
            }
            for _ in 0..(*level).clamp(1, 6) {
                out.push('=');
            }
            out.push(' ');
            render_inlines(content, settings, labels, out);
            out.push_str("\n\n");
        }
        BlockKind::Paragraph { content } => {
            render_inlines(content, settings, labels, out);
            out.push_str("\n\n");
        }
        BlockKind::Quote { blocks } => {
            out.push_str("#quote(block: true)[\n");
            render_blocks(blocks, settings, labels, tables, out);
            out.push_str("]\n\n");
        }
        BlockKind::List {
            ordered,
            start,
            items,
        } => {
            if *ordered {
                let _ = writeln!(out, "#enum(start: {start},");
            } else {
                out.push_str("#list(\n");
            }
            for item in items {
                render_list_item(item, settings, labels, tables, out);
            }
            out.push_str(")\n\n");
        }
        BlockKind::CodeBlock { language, code } => {
            // Raw block: el contenido es literal (§7.4.1 código primero).
            let fence = "`".repeat(3.max(longest_backtick_run(code) + 1));
            out.push_str(&fence);
            if let Some(lang) = language {
                out.push_str(lang);
            }
            out.push('\n');
            out.push_str(code);
            if !code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
            out.push_str("\n\n");
        }
        BlockKind::Table {
            aligns,
            header,
            rows,
            ..
        } => {
            // §35.2: el envoltorio decide el tamaño en runtime; la identidad
            // solo la usa el panic de suelo para nombrar la tabla.
            let identity = tables
                .get(&block.span.start)
                .cloned()
                .unwrap_or_else(|| "tabla sin ID".to_owned());
            crate::fit::render_table(out, settings, labels, aligns, header, rows, &identity);
        }
        BlockKind::MathBlock { latex, .. } => {
            match minmark_renderer::validated_math(latex) {
                Ok(translation) => {
                    // §24: math de bloque con el subconjunto traducido (ADR-002).
                    out.push_str("$\n");
                    out.push_str(&translation);
                    out.push_str("\n$\n\n");
                }
                Err(marker) => {
                    // NFR-026: marcador visible, nunca silenciar ni deformar.
                    eprintln!("minmark: {marker}");
                    out.push_str(&escape_text(&marker));
                    out.push_str("\n\n");
                }
            }
        }
        BlockKind::FootnoteDefinition { .. } => {}
        BlockKind::Callout {
            callout_type,
            title,
            blocks,
        } => {
            // §22: barra de color del tipo (paleta Light: PDF siempre blanco).
            let hex = Palette::LIGHT.color(callout_type).unwrap_or("#d0d7de");
            let _ = writeln!(
                out,
                "#block(stroke: (left: 3pt + rgb(\"{hex}\")), inset: (left: 10pt, y: 6pt))["
            );
            if let Some(title) = title {
                out.push('*');
                out.push_str(&escape_text(title));
                out.push_str("*\n");
            }
            render_blocks(blocks, settings, labels, tables, out);
            out.push_str("]\n");
        }
        BlockKind::Rule => out.push_str("#line(length: 100%)\n\n"),
    }
    if !matches!(block.kind, BlockKind::Heading { .. }) {
        if let Some(caption) = attrs.and_then(|a| a.caption.as_ref()) {
            let _ = writeln!(out, "#align(center)[{}]\n", escape_text(caption));
        }
    }
}

fn render_list_item(
    item: &ListItem,
    settings: &Settings,
    labels: &std::collections::BTreeSet<String>,
    tables: &std::collections::BTreeMap<u32, String>,
    out: &mut String,
) {
    out.push_str("  [\n");
    render_blocks(&item.blocks, settings, labels, tables, out);
    out.push_str("  ],\n");
}

pub(crate) fn render_inlines(
    inlines: &[Inline],
    settings: &Settings,
    labels: &std::collections::BTreeSet<String>,
    out: &mut String,
) {
    for inline in inlines {
        match inline {
            Inline::Styled { content, style } => {
                let mut body = String::new();
                render_inlines(content, settings, labels, &mut body);
                if style.italic {
                    body = format!("#text(style: \"italic\")[{body}]");
                }
                if style.bold {
                    body = format!("#text(weight: \"bold\")[{body}]");
                }
                if style.light {
                    body = format!("#text(weight: \"light\")[{body}]");
                }
                if style.small_caps {
                    body = format!("#smallcaps[{body}]");
                }
                if style.underline {
                    body = format!("#underline[{body}]");
                }
                if style.vertical == 1 {
                    body = format!("#super[{body}]");
                }
                if style.vertical == -1 {
                    body = format!("#sub[{body}]");
                }
                out.push_str(&body);
            }
            Inline::FootnoteReference(label) => {
                let (number, occurrence) = label.split_once('-').unwrap_or((label, "1"));
                if occurrence == "1" {
                    let _ = write!(out, "#footnote(mm-note-{number})<mm-note-{number}>");
                } else {
                    let _ = write!(out, "#footnote(<mm-note-{number}>)");
                }
            }
            Inline::Text(text) => out.push_str(&escape_text(text)),
            Inline::Code(code) => {
                out.push_str("#raw(");
                out.push_str(&typst_string(code));
                out.push(')');
            }
            Inline::Emphasis(content) => wrap(out, "_", "_", content, settings, labels),
            Inline::Strong(content) => wrap(out, "*", "*", content, settings, labels),
            Inline::Strikethrough(content) => {
                out.push_str("#strike[");
                render_inlines(content, settings, labels, out);
                out.push(']');
            }
            Inline::Link {
                destination,
                content,
                ..
            } => {
                if let Some(id) = destination
                    .strip_prefix('#')
                    .filter(|id| labels.contains(*id))
                {
                    let _ = write!(out, "#link(label({}))[", typst_string(id));
                } else {
                    let _ = write!(out, "#link({})[", typst_string(destination));
                }
                render_inlines(content, settings, labels, out);
                out.push(']');
            }
            Inline::Image {
                destination,
                alt,
                attrs,
                ..
            } => {
                let caption = attrs.as_ref().and_then(|a| a.caption.as_deref());
                if caption.is_some() {
                    out.push_str("#block(breakable: false)[");
                }
                if let Some(id) = attrs.as_ref().and_then(|a| a.id.as_ref()) {
                    let _ = write!(out, "#box[]#label({})", typst_string(id));
                }
                let _ = write!(
                    out,
                    "#image({}, alt: {}",
                    typst_string(destination),
                    typst_string(&plain_text(alt))
                );
                if let Some(width) = attrs.as_ref().and_then(|a| a.width_percent) {
                    let _ = write!(out, ", width: {width}%");
                }
                out.push(')');
                if let Some(caption) = caption {
                    out.push_str("#parbreak()#align(center)[#text(size: 0.9em)[");
                    out.push_str(&escape_text(caption));
                    out.push_str("]]]");
                }
            }
            Inline::Math(latex) => match minmark_renderer::validated_math(latex) {
                Ok(translation) => {
                    out.push('$');
                    out.push_str(&translation);
                    out.push('$');
                }
                Err(marker) => {
                    eprintln!("minmark: {marker}");
                    out.push_str(&escape_text(&marker));
                }
            },
            Inline::Color {
                foreground,
                background,
                text,
            } => {
                // §23. PDF siempre blanco/texto oscuro (§32.2): colores de la
                // paleta Light; fondo solo como resaltado.
                let content = escape_text(text);
                let mut wrapped = content;
                if let Some(bg) = background {
                    wrapped = format!(
                        "#highlight(fill: rgb(\"{}\"))[{wrapped}]",
                        color_hex(bg, settings)
                    );
                }
                if let Some(fg) = foreground {
                    wrapped = format!(
                        "#text(fill: rgb(\"{}\"))[{wrapped}]",
                        color_hex(fg, settings)
                    );
                }
                out.push_str(&wrapped);
            }
            Inline::Citation { items } => {
                // Texto de reserva hasta la composición bibliográfica real
                // (ADR-003, Fase 5).
                out.push_str(&escape_text(&citation_text(items)));
            }
            Inline::WikiLink { target } => {
                out.push_str(&escape_text(&format!("[[{target}]]")));
            }
            Inline::Substitution(sub) => match sub {
                // §29: TOC con composición nativa. §31: salto de página.
                Substitution::Toc => {
                    let _ = write!(
                        out,
                        "#outline(title: none, depth: {})",
                        settings.toc_max_level
                    );
                }
                Substitution::PageBreak => out.push_str("#pagebreak()"),
                // §26.5: numeración real en Fase 5 (§27).
                Substitution::CrossRef { id } => {
                    out.push_str(&escape_text(&format!("{{{{ref:{id}}}}}")))
                }
            },
            Inline::TaskListMarker(checked) => out.push_str(if *checked { "[x] " } else { "[ ] " }),
            Inline::SoftBreak => out.push('\n'),
            Inline::HardBreak => out.push_str("#linebreak()\n"),
        }
    }
}

fn wrap(
    out: &mut String,
    open: &str,
    close: &str,
    content: &[Inline],
    settings: &Settings,
    labels: &std::collections::BTreeSet<String>,
) {
    out.push_str(open);
    render_inlines(content, settings, labels, out);
    out.push_str(close);
}

pub(crate) fn citation_text(items: &[CitationItem]) -> String {
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

/// Texto plano de una secuencia de inlines: `alt` de imágenes (§7.4.4), valor
/// de `{chapter}` y título de respaldo de `{title}` (§32.9).
pub(crate) fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
            Inline::Emphasis(content)
            | Inline::Strong(content)
            | Inline::Strikethrough(content)
            | Inline::Link { content, .. }
            | Inline::Styled { content, .. } => out.push_str(&plain_text(content)),
            Inline::Color { text, .. } => out.push_str(text),
            Inline::WikiLink { target } => out.push_str(target),
            Inline::SoftBreak | Inline::HardBreak => out.push(' '),
            _ => {}
        }
    }
    out
}

/// Valor hex de un color §23: paleta del proyecto (`custom_colors`) y, si no,
/// la paleta Light (§32.2: PDF siempre blanco con texto oscuro).
pub(crate) fn color_hex(name: &str, settings: &Settings) -> String {
    if let Some(value) = settings.custom_colors.get(name) {
        if value.starts_with('#') {
            return value.clone();
        }
        if let Some(hex) = Palette::LIGHT.color(value) {
            return hex.to_string();
        }
        return value.clone();
    }
    Palette::LIGHT.color(name).unwrap_or("#000000").to_string()
}

/// Escapa el texto para markup Typst (los caracteres especiales son literales).
pub(crate) fn escape_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '#' | '[' | ']' | '*' | '_' | '`' | '\\' | '$' | '@' | '~' | '<' | '>' | '-' | '+'
            | '=' | '/' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Escapa un valor para cadena Typst `"..."`.
pub(crate) fn typst_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn longest_backtick_run(code: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for c in code.chars() {
        if c == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_parser::parse;

    fn source(md: &str) -> String {
        generate(&parse(md), &Settings::default(), &DocumentMeta::default())
    }

    #[test]
    fn preambulo_con_config_basica_58_2() {
        // §32.3/32.5/32.6: A4, 25 mm, Portrait por defecto.
        let out = source("# T\n");
        assert!(out.contains("#set page(width: 210mm, height: 297mm"));
        assert!(out.contains("margin: (top: 25mm, right: 25mm, bottom: 25mm, left: 25mm)"));
        // §32.7: sin números de página sin paginación activada.
        assert!(!out.contains("numbering"));
    }

    #[test]
    fn familias_tipograficas_del_world_33() {
        // §33.1: el default sans-serif emite Liberation Sans (asset sans del
        // World embebido), no una serif; serif y mono emiten familias que
        // también están en el World.
        let out = source("# T\n");
        assert!(
            out.contains("#set text(font: (\"Liberation Sans\", \"Libertinus Serif\")"),
            "default sans-serif sin Liberation Sans:\n{out}"
        );
        for (body_font, esperado) in [
            (
                FontFamily::Serif,
                "#set text(font: (\"Libertinus Serif\", \"New Computer Modern\")",
            ),
            (
                FontFamily::Monospace,
                "#set text(font: (\"DejaVu Sans Mono\", \"New Computer Modern\")",
            ),
        ] {
            let settings = Settings {
                body_font,
                ..Settings::default()
            };
            let out = generate(&parse("# T\n"), &settings, &DocumentMeta::default());
            assert!(out.contains(esperado), "{esperado} ausente:\n{out}");
        }
    }

    #[test]
    fn configuracion_pdf_por_ajustes() {
        let settings = Settings {
            pagination_enabled: true,
            preview_max_width_px: 800,
            ..Settings::default()
        };
        let ast = parse("# T\n");
        let out = generate(&ast, &settings, &DocumentMeta::default());
        assert!(out.contains("numbering: \"1\""));
        assert!(out.contains("number-align: bottom + right"));

        let settings = Settings {
            pdf_orientation: Orientation::Landscape,
            pdf_margins_mm: minmark_config::Margins::uniform(10.0),
            ..Settings::default()
        };
        let out = generate(&parse("# T\n"), &settings, &DocumentMeta::default());
        assert!(out.contains("#set page(width: 297mm, height: 210mm"));
        assert!(out.contains("top: 10mm"));
    }

    #[test]
    fn bloques_basicos() {
        let out = source("# Título\n\nPárrafo.\n\n- uno\n\n> cita\n\n---\n");
        assert!(out.contains("= Título"));
        assert!(out.contains("Párrafo."));
        assert!(out.contains("#list("));
        assert!(out.contains("#quote(block: true)["));
        assert!(out.contains("#line(length: 100%)"));

        let out = source("3. tres\n");
        assert!(out.contains("#enum(start: 3,"));
    }

    #[test]
    fn codigo_tabla_y_callout() {
        let out = source("```rs\nlet a = 1;\n```\n");
        assert!(out.contains("```rs\nlet a = 1;\n```"));

        let out = source("| A | B |\n|:--|--:|\n| 1 | 2 |\n");
        assert!(out.contains("#table("));
        assert!(out.contains("table.header("));
        assert!(out.contains("align: (left, right,),"));
        assert!(out.contains("columns: mm-fit-cols,"));

        let out = source("::: warning \"Atención\"\nCuidado.\n:::\n");
        assert!(out.contains("#block(stroke: (left: 3pt + rgb(\"#e65100\"))"));
        assert!(out.contains("*Atención*"));
    }

    #[test]
    fn math_traducida_y_marcador() {
        // §24 + ADR-002.
        let out = source("$$\n\\frac{a}{b}\n$$\n");
        assert!(out.contains("$\nfrac(a, b)\n$"));

        // El marcador viaja escapado como texto literal.
        let out = source("$$\n\\foo{x}\n$$\n");
        assert!(out.contains("math no soportado"));
        assert!(out.contains("\\\\foo"));
        assert!(!out.contains('$'));
    }

    #[test]
    fn texto_escapado_y_colores() {
        // El texto del documento nunca inicia sintaxis Typst.
        let out = source("# heading [no link]\n\n1 = 1 - 2\n");
        assert!(out.contains("\\[no link\\]"));
        assert!(out.contains("1 \\= 1 \\- 2"));

        let out = source("{danger: rojo}\n");
        assert!(out.contains("#text(fill: rgb(\"#c62828\"))[rojo]"));

        let out = source("{+highlight: fondo}\n");
        assert!(out.contains("#highlight(fill: rgb(\"#fff59d\"))[fondo]"));
    }

    #[test]
    fn sustituciones() {
        let out = source("{{toc}}\n\n{{pagebreak}}\n\n{{ref:fig:a}}\n");
        assert!(out.contains("#outline(title: none, depth: 3)"));
        assert!(out.contains("#pagebreak()"));
        assert!(out.contains("Referencia inexistente: fig:a"));
    }

    #[test]
    fn pie_de_figura_numerado_debajo_de_imagen_en_ambos_idiomas() {
        let source = "![Esquema](image.svg){#fig:one}";
        for (language, name) in [("es", "Figura"), ("en", "Figure")] {
            let settings = Settings {
                language: language.into(),
                ..Default::default()
            };
            let out = generate(
                &minmark_parser::parse(source),
                &settings,
                &DocumentMeta::default(),
            );
            let image = out.find("#image(").unwrap();
            let caption = out.find(&format!("{name} 1. Esquema")).unwrap();
            assert!(image < caption);
            assert!(out[image..caption].contains("#parbreak()#align(center)"));
            assert!(out.contains("#block(breakable: false)["));
        }
    }

    /// §28.2/§28.3 (ISSUE-027): el caption de tabla y el opcional de
    /// ecuación salen debajo de su bloque, con la etiqueta numerada ya
    /// expandida («Tabla 1. …», «Ecuación 1. …»), igual que en el Preview.
    #[test]
    fn captions_de_tabla_y_ecuacion_debajo_con_su_etiqueta() {
        let out = source(
            "| A |\n| - |\n| B |\n{#tbl:results \"Resultados\"}\n\n$$\nE = mc^2\n$$\n{#eq:energy \"Equivalencia\"}\n",
        );
        let tabla = out.find("#table(").expect("tabla");
        // El caption renderizado va tras el envoltorio §35.2 de la tabla; la
        // misma identidad en el panic de suelo no debe confundirse con él.
        let cap_tabla = out
            .find("#align(center)[Tabla 1. Resultados]")
            .expect("caption de tabla");
        assert!(
            tabla < cap_tabla,
            "el caption va debajo de la tabla:\n{out}"
        );
        let math = out.find("$\n").expect("bloque de ecuación");
        let cap_eq = out
            .find("Ecuación 1. Equivalencia")
            .expect("caption de ecuación");
        assert!(math < cap_eq, "el caption va debajo de la ecuación:\n{out}");
        assert!(
            cap_tabla < math,
            "el orden de los bloques se conserva:\n{out}"
        );
    }

    /// §28.3/§28.1 (ISSUE-027): la ecuación con ID y sin caption conserva
    /// solo su etiqueta debajo; la imagen ordinaria sin `fig:` no gana
    /// caption ni numeración.
    #[test]
    fn ecuacion_sin_caption_solo_etiqueta_e_imagen_ordinaria_intacta() {
        let out = source("$$\nx\n$$\n{#eq:only}\n\n![Solo alt](icon.svg)\n");
        assert!(
            out.contains("Ecuación 1"),
            "la ecuación con ID lleva su etiqueta: {out}"
        );
        assert!(out.contains("#image("), "la imagen se renderiza: {out}");
        assert!(
            !out.contains("Figura"),
            "sin ID de figura no hay etiqueta ni caption: {out}"
        );
    }

    /// §26.5/§26.6/§28 + ISSUE-027 (criterio de paridad): las etiquetas de
    /// caption y su numeración coinciden entre el Preview (HTML) y el PDF
    /// (fuente Typst) porque ambos pasan por la misma expansión.
    #[test]
    fn etiquetas_de_caption_coinciden_entre_preview_y_pdf() {
        let md = "# Cap\n\n![Esquema](a.svg){#fig:one}\n\n| A |\n| - |\n| B |\n{#tbl:one \"Resultados\"}\n\n$$\nx\n$$\n{#eq:one}\n";
        let ast = minmark_parser::parse(md);
        let html = minmark_renderer::render_preview(&ast, md, &Settings::default());
        let pdf = source(md);
        for etiqueta in ["Figura 1. Esquema", "Tabla 1. Resultados", "Ecuación 1"] {
            assert!(html.contains(etiqueta), "Preview sin «{etiqueta}»:\n{html}");
            assert!(pdf.contains(etiqueta), "PDF sin «{etiqueta}»:\n{pdf}");
        }
    }

    #[test]
    fn plantillas_de_header_footer_en_preamble_32_9() {
        // §32.9: el state de {chapter} se declara antes de `#set page` y las
        // plantillas se expanden con los metadatos; {foo} queda literal.
        let settings = Settings {
            pagination_enabled: true,
            pdf_header: Some("Cab {title}".into()),
            pdf_footer: Some("Pie {author} {foo}".into()),
            ..Settings::default()
        };
        let meta = DocumentMeta {
            title: Some("Mi Doc".into()),
            author: Some("Autora".into()),
            project: None,
            date: None,
        };
        let out = generate(&parse("# T\n"), &settings, &meta);
        let state = out.find("#let mm-chapter").unwrap();
        let page = out.find("#set page").unwrap();
        assert!(state < page, "el state debe declararse antes de {page:?}");
        assert!(out.contains(", header: align(left)[Cab Mi Doc]"));
        assert!(out.contains("[Pie Autora {foo}]"), "{out}");
        // §32.8: con la plantilla en el lado del número (FooterRight por
        // defecto) el número se emite dentro del footer.
        assert!(out.contains("grid(columns: (1fr, auto)"));
        assert!(out.contains("numbering: \"1\""));
        assert!(out.contains("number-align: bottom + right"));
    }

    #[test]
    fn numero_embebido_sigue_pdf_page_number_position_32_8() {
        for (position, expected) in [
            (PageNumberPosition::FooterLeft, "grid(columns: (auto, 1fr)"),
            (
                PageNumberPosition::FooterCenter,
                "grid(columns: (1fr, auto, 1fr)",
            ),
            (PageNumberPosition::FooterRight, "grid(columns: (1fr, auto)"),
            // El número va en el header y el footer solo lleva plantilla.
            (
                PageNumberPosition::HeaderCenter,
                ", footer: align(left)[Pie]",
            ),
        ] {
            let settings = Settings {
                pagination_enabled: true,
                pdf_page_number_position: position,
                pdf_footer: Some("Pie".into()),
                ..Settings::default()
            };
            let out = generate(&parse("# T\n"), &settings, &DocumentMeta::default());
            assert!(
                out.contains(expected),
                "posición {position:?}: falta {expected:?} en:\n{out}"
            );
        }
    }

    #[test]
    fn plantillas_vacias_o_none_sin_marginales_32_9() {
        for header in [None, Some(String::new()), Some("   \n".to_owned())] {
            let settings = Settings {
                pdf_header: header,
                pdf_footer: Some(String::new()),
                ..Settings::default()
            };
            let out = generate(&parse("# T\n"), &settings, &DocumentMeta::default());
            assert!(!out.contains("header:"), "sin header: {out}");
            assert!(!out.contains("footer:"), "sin footer: {out}");
        }
    }

    #[test]
    fn sin_paginacion_page_y_pages_vacios_32_7() {
        // §32.7: sin paginación no hay numbering ni contadores; {page} y
        // {pages} se expanden vacíos (queda solo el texto literal).
        let settings = Settings {
            pdf_footer: Some("P[{page}]S[{pages}]".into()),
            ..Settings::default()
        };
        let out = generate(&parse("# T\n"), &settings, &DocumentMeta::default());
        assert!(
            out.contains(", footer: align(left)[P\\[\\]S\\[\\]]"),
            "{out}"
        );
        assert!(!out.contains("counter(page)"), "{out}");
        assert!(!out.contains("numbering"), "{out}");
    }

    /// §35.2 (ISSUE-027 criterio 5): toda tabla se emite dentro de
    /// `#layout(size => ...)` con la decisión de tamaño en runtime, con
    /// inset/stroke explícitos a tamaño base (5 pt / 1 pt, los valores por
    /// defecto de Typst) y el caption renderizado fuera del envoltorio.
    #[test]
    fn tabla_emite_envoltorio_de_ajuste_35_2() {
        let out = source("| A |\n| - |\n| B |\n{#tbl:one \"Resultados\"}\n");
        assert!(out.contains("#layout(size => {"), "{out}");
        assert!(
            out.contains("let mm-fit-u = if mm-fit-P + mm-fit-B <= size.width"),
            "la decisión es en runtime:\n{out}"
        );
        assert!(
            out.contains("set text(size: mm-fit-base * mm-fit-u)"),
            "el cuerpo solo se reduce (nunca amplía):\n{out}"
        );
        assert!(out.contains("inset: 5pt * mm-fit-u"), "{out}");
        assert!(out.contains("stroke: 1pt * mm-fit-u"), "{out}");
        assert!(out.contains("measure(["), "los átomos se miden:\n{out}");
        // La tabla sigue siendo nativa (reparto multipágina) con sus celdas y
        // el reparto de columnas respetando las mínimas (§35.2).
        assert!(out.contains("#table("), "{out}");
        assert!(out.contains("table.header("), "{out}");
        assert!(out.contains("columns: mm-fit-cols,"), "{out}");
        assert!(out.contains("mm-fit-track(("), "suelos por columna:\n{out}");
        // El caption renderizado queda fuera del envoltorio, tras la tabla.
        let open = out.find("#layout(").expect("layout");
        let close = open + out[open..].find("})").expect("cierre del layout");
        let caption = out
            .find("#align(center)[Tabla 1. Resultados]")
            .expect("caption renderizado");
        assert!(close < caption, "el caption va fuera del layout:\n{out}");
    }

    /// §35.2: el `panic` de suelo lleva el mensaje con la identidad de la
    /// tabla (caption expandido, etiqueta del ID o nº de orden) y el valor
    /// vigente de `table_min_font_pt`.
    #[test]
    fn panic_de_suelo_identifica_la_tabla_35_2() {
        let suelo = "no cabe en el ancho disponible con table_min_font_pt=8pt (§35.2)";
        // Con caption: manda el caption ya expandido (§28).
        let out = source("| A |\n| - |\n| B |\n{#tbl:one \"Resultados\"}\n");
        assert!(
            out.contains(&format!("panic(\"Tabla 1. Resultados: {suelo}\")")),
            "{out}"
        );
        // Con ID y sin caption: la identidad es la etiqueta numerada.
        let out = source("| A |\n| - |\n| B |\n{#tbl:one}\n");
        assert!(
            out.contains(&format!("panic(\"Tabla 1: {suelo}\")")),
            "{out}"
        );
        // Sin identidad: nº de orden documental (1-based).
        let out = source("| A |\n| - |\n| B |\n\n| C |\n| - |\n| D |\n");
        assert!(
            out.contains(&format!("panic(\"tabla sin ID nº 1: {suelo}\")")),
            "{out}"
        );
        assert!(
            out.contains(&format!("panic(\"tabla sin ID nº 2: {suelo}\")")),
            "{out}"
        );
    }

    /// §35.2: `table_min_font_pt` alimenta el suelo emitido y el mensaje de
    /// error; `font_size_pt` es la base que nunca se supera.
    #[test]
    fn table_min_font_pt_alimenta_el_suelo_35_2() {
        let settings = Settings {
            table_min_font_pt: 30.0,
            ..Settings::default()
        };
        let out = generate(
            &parse("| A |\n| - |\n| B |\n"),
            &settings,
            &DocumentMeta::default(),
        );
        assert!(out.contains("let mm-fit-base = 12pt"), "{out}");
        assert!(out.contains("let mm-fit-floor = 30pt"), "{out}");
        assert!(out.contains("table_min_font_pt=30pt"), "{out}");
        let out = source("| A |\n| - |\n| B |\n");
        assert!(out.contains("let mm-fit-floor = 8pt"), "{out}");
        assert!(out.contains("table_min_font_pt=8pt"), "{out}");
    }
}
