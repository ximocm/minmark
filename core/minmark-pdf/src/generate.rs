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
use minmark_document::{
    Align, Ast, Block, BlockKind, CitationItem, Inline, ListItem, Substitution,
};
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
    let mut out = preamble(&ast, settings, meta);
    for block in &ast.blocks {
        if let BlockKind::FootnoteDefinition { label, blocks } = &block.kind {
            let _ = writeln!(out, "#let mm-note-{label} = [");
            render_blocks(blocks, settings, &labels, &mut out);
            out.push_str("]\n");
        }
    }
    render_blocks(&ast.blocks, settings, &labels, &mut out);
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

    // §33: tipografía (sans/serif/mono libres; fallback del World embebido).
    let fonts = match settings.body_font {
        FontFamily::SansSerif => "\"Inter\", \"DejaVu Sans\", \"Libertinus Serif\"",
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
    out: &mut String,
) {
    for block in blocks {
        render_block(block, settings, labels, out);
    }
}

fn render_block(
    block: &Block,
    settings: &Settings,
    labels: &std::collections::BTreeSet<String>,
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
            render_blocks(blocks, settings, labels, out);
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
                render_list_item(item, settings, labels, out);
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
            let columns = header.len().max(aligns.len()).max(1);
            let _ = writeln!(out, "#table(");
            let _ = writeln!(out, "  columns: {columns},");
            if !aligns.is_empty() {
                let aligns: Vec<&str> = aligns
                    .iter()
                    .map(|a| match a {
                        Align::Center => "center",
                        Align::Right => "right",
                        Align::Left | Align::None => "left",
                    })
                    .collect();
                let _ = writeln!(out, "  align: ({},),", aligns.join(", "));
            }
            out.push_str("  table.header(\n");
            for cell in header {
                out.push_str("    [");
                render_inlines(cell, settings, labels, out);
                out.push_str("],\n");
            }
            out.push_str("  ),\n");
            for row in rows {
                for cell in row {
                    out.push_str("    [");
                    render_inlines(cell, settings, labels, out);
                    out.push_str("],\n");
                }
            }
            out.push_str(")\n\n");
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
            render_blocks(blocks, settings, labels, out);
            out.push_str("]\n\n");
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
    out: &mut String,
) {
    out.push_str("  [\n");
    render_blocks(&item.blocks, settings, labels, out);
    out.push_str("  ],\n");
}

fn render_inlines(
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
fn color_hex(name: &str, settings: &Settings) -> String {
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
fn escape_text(text: &str) -> String {
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
fn typst_string(text: &str) -> String {
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
        assert!(out.contains("columns: 2,"));

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
}
