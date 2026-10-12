//! Generación de código Typst desde la AST (ISSUE-018, §5.3).
//!
//! Pipeline normativo (§5.3): Markdown → AST → **composición Typst** → PDF.
//! El usuario nunca escribe Typst (§24): esta generación es interna y traduce
//! math LaTeX con [`minmark_math`] (ADR-002).
//!
//! Apariencia (§32.2): fondo blanco, texto oscuro, siempre. Configuración
//! básica (§58.2): A4, márgenes 25 mm, Portrait — aplicada desde `Settings`.

use crate::page_size_mm;
use minmark_config::{
    FontFamily, Orientation, PageNumberPosition, ParagraphStyle, Settings, TextAlignment,
};
use minmark_document::{
    Align, Ast, Block, BlockKind, CitationItem, Inline, ListItem, Substitution,
};
use minmark_math::translate;
use minmark_renderer::Palette;
use std::fmt::Write;

/// Genera el documento Typst completo para la AST y los ajustes dados.
pub fn generate(ast: &Ast, settings: &Settings) -> String {
    let mut out = preamble(settings);
    render_blocks(&ast.blocks, settings, &mut out);
    out
}

fn preamble(settings: &Settings) -> String {
    // §32.3–32.6: tamaño, márgenes en mm (§32.4) y orientación.
    let (mut width, mut height) = page_size_mm(settings.pdf_page_size);
    if settings.pdf_orientation == Orientation::Landscape {
        std::mem::swap(&mut width, &mut height);
    }
    let margins = &settings.pdf_margins_mm;
    let mut out = format!(
        "#set page(width: {width}mm, height: {height}mm, \
         margin: (top: {}mm, right: {}mm, bottom: {}mm, left: {}mm)",
        margins.top_mm, margins.right_mm, margins.bottom_mm, margins.left_mm
    );
    if settings.pagination_enabled {
        // §32.7–32.8: números de página solo con paginación activada.
        let _ = write!(
            out,
            ", numbering: \"1\", number-align: {}",
            number_align(settings.pdf_page_number_position)
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
    // §33.5 alineación.
    match settings.text_alignment {
        TextAlignment::Left => {}
        TextAlignment::Center => out.push_str("#set align(center)\n"),
        TextAlignment::Right => out.push_str("#set align(right)\n"),
    }
    out.push('\n');
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

fn render_blocks(blocks: &[Block], settings: &Settings, out: &mut String) {
    for block in blocks {
        render_block(block, settings, out);
    }
}

fn render_block(block: &Block, settings: &Settings, out: &mut String) {
    match &block.kind {
        BlockKind::Heading { level, content, .. } => {
            for _ in 0..(*level).clamp(1, 6) {
                out.push('=');
            }
            out.push(' ');
            render_inlines(content, settings, out);
            out.push_str("\n\n");
        }
        BlockKind::Paragraph { content } => {
            render_inlines(content, settings, out);
            out.push_str("\n\n");
        }
        BlockKind::Quote { blocks } => {
            out.push_str("#quote(block: true)[\n");
            render_blocks(blocks, settings, out);
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
                render_list_item(item, settings, out);
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
            let columns = header.first().map(Vec::len).unwrap_or(aligns.len()).max(1);
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
                render_inlines(cell, settings, out);
                out.push_str("],\n");
            }
            out.push_str("  ),\n");
            for row in rows {
                for cell in row {
                    out.push_str("    [");
                    render_inlines(cell, settings, out);
                    out.push_str("],\n");
                }
            }
            out.push_str(")\n\n");
        }
        BlockKind::MathBlock { latex, .. } => {
            let translation = translate(latex);
            if translation.is_supported() {
                // §24: math de bloque con el subconjunto traducido (ADR-002).
                out.push_str("$\n");
                out.push_str(&translation.typst);
                out.push_str("\n$\n\n");
            } else {
                // NFR-026: marcador visible, nunca silenciar ni deformar.
                out.push_str(&escape_text(&translation.marker()));
                out.push_str("\n\n");
            }
        }
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
            render_blocks(blocks, settings, out);
            out.push_str("]\n\n");
        }
        BlockKind::Rule => out.push_str("#line(length: 100%)\n\n"),
    }
}

fn render_list_item(item: &ListItem, settings: &Settings, out: &mut String) {
    out.push_str("  [\n");
    render_blocks(&item.blocks, settings, out);
    out.push_str("  ],\n");
}

fn render_inlines(inlines: &[Inline], settings: &Settings, out: &mut String) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(&escape_text(text)),
            Inline::Code(code) => {
                out.push_str("#raw(");
                out.push_str(&typst_string(code));
                out.push(')');
            }
            Inline::Emphasis(content) => wrap(out, "_", "_", content, settings),
            Inline::Strong(content) => wrap(out, "*", "*", content, settings),
            Inline::Strikethrough(content) => {
                out.push_str("#strike[");
                render_inlines(content, settings, out);
                out.push(']');
            }
            Inline::Link {
                destination,
                content,
                ..
            } => {
                let _ = write!(out, "#link({})[", typst_string(destination));
                render_inlines(content, settings, out);
                out.push(']');
            }
            Inline::Image {
                destination,
                alt,
                attrs,
                ..
            } => {
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
            }
            Inline::Math(latex) => {
                let translation = translate(latex);
                if translation.is_supported() {
                    out.push('$');
                    out.push_str(&translation.typst);
                    out.push('$');
                } else {
                    out.push_str(&escape_text(&translation.marker()));
                }
            }
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
                Substitution::Toc => out.push_str("#outline()"),
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
) {
    out.push_str(open);
    render_inlines(content, settings, out);
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

fn plain_text(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(t) | Inline::Code(t) => out.push_str(t),
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
        generate(&parse(md), &Settings::default())
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
        let out = generate(&ast, &settings);
        assert!(out.contains("numbering: \"1\""));
        assert!(out.contains("number-align: bottom + right"));

        let settings = Settings {
            pdf_orientation: Orientation::Landscape,
            pdf_margins_mm: minmark_config::Margins::uniform(10.0),
            ..Settings::default()
        };
        let out = generate(&parse("# T\n"), &settings);
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

        let out = source("::: warning \"Atención\"\nCuidado.\n:::\n");
        assert!(out.contains("#block(stroke: (left: 3pt + rgb(\"#e65100\"))"));
        assert!(out.contains("*Atención*"));
    }

    #[test]
    fn math_traducida_y_marcador() {
        // §24 + ADR-002.
        let out = source("$$\n\\frac{a}{b}\n$$\n");
        assert!(out.contains("$\na/b\n$"));

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
        assert!(out.contains("#outline()"));
        assert!(out.contains("#pagebreak()"));
        assert!(out.contains("{{ref:fig:a}}"));
    }
}
