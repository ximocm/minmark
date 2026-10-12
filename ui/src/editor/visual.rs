//! Plan de tags del modo Visual (ISSUE-030) — lógica pura, sin GTK.
//!
//! Traduce los tramos de [`minmark_parser::visual`] (spans en BYTES del
//! fuente) a rangos en CARÁCTERES unicode sobre el texto del `TextBuffer`
//! único (§6.1), con el estilo de cada `gtk::TextTag` a partir de una
//! [`minmark_renderer::Palette`]. Los tramos de sintaxis (`style: None`) se
//! ocultan con el tag `mm-vis-syntax` (§8.3: oculta siempre o revelada en el
//! elemento editado). Tests puros: sin GTK ni widgets.

use super::tags::{char_range, TagStyle};
use minmark_parser::{CalloutTone, VisualPlan, VisualStyle};
use minmark_renderer::Palette;

/// Tag de sintaxis ocultable (§8.3): marcadores invisibles de ancho cero.
pub const SYNTAX_TAG: &str = "mm-vis-syntax";

/// Rango visual en CARÁCTERES unicode sobre el texto del buffer
/// (`end_char` exclusivo, como `gtk::TextIter::offset()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualTagRange {
    pub start_char: usize,
    pub end_char: usize,
    /// `Some(estilo)` = contenido semántico; `None` = sintaxis ocultable.
    pub style: Option<VisualStyle>,
}

/// Convierte los tramos del plan visual a rangos en CARÁCTERES sobre `text`,
/// en orden de aparición (con las mismas reglas de recorte que
/// [`super::tags::tag_plan`]: sin panic ni slicing fuera de fronteras). Los
/// slots de color personalizados se normalizan al bucket común (§58.2).
pub fn visual_tag_plan(text: &str, plan: &VisualPlan) -> Vec<VisualTagRange> {
    let mut out = Vec::with_capacity(plan.spans.len());
    for span in &plan.spans {
        let start = span.span.start as usize;
        let end = span.span.end as usize;
        if let Some((start_char, end_char)) = char_range(text, start, end) {
            out.push(VisualTagRange {
                start_char,
                end_char,
                style: span.style.map(normalize),
            });
        }
    }
    out.sort_by_key(|range| range.start_char);
    out
}

/// Normaliza un estilo: los slots de color fuera de la paleta §23 (colores
/// personalizados, §58.2) comparten el bucket `custom` mientras el parser no
/// reciba sus valores desde la config.
fn normalize(style: VisualStyle) -> VisualStyle {
    match style {
        VisualStyle::ColorFg(slot) if slot > 17 => VisualStyle::ColorFg(CUSTOM_SLOT),
        VisualStyle::ColorBg(slot) if slot > 17 => VisualStyle::ColorBg(CUSTOM_SLOT),
        other => other,
    }
}

/// Slot del bucket de colores personalizados.
const CUSTOM_SLOT: u8 = 17;

/// Elementos (bloques) en CARÁCTERES sobre `text`, para §8.3 «reveal-current»:
/// se revela la sintaxis del elemento más interno bajo el cursor.
pub fn element_ranges(text: &str, plan: &VisualPlan) -> Vec<(usize, usize)> {
    plan.elements
        .iter()
        .filter_map(|span| char_range(text, span.start as usize, span.end as usize))
        .collect()
}

/// Todos los estilos con tag propio (para crear la `TextTagTable` completa).
pub fn all_styles() -> Vec<VisualStyle> {
    let mut out = Vec::new();
    for level in 1..=6 {
        out.push(VisualStyle::Heading(level));
    }
    out.push(VisualStyle::Strong);
    out.push(VisualStyle::Emphasis);
    out.push(VisualStyle::Link);
    out.push(VisualStyle::Code);
    out.push(VisualStyle::CodeBlock);
    out.push(VisualStyle::Quote);
    for level in 1..=6 {
        out.push(VisualStyle::ListItem(level));
    }
    out.push(VisualStyle::Task);
    out.push(VisualStyle::TableHeader);
    out.push(VisualStyle::TableRule);
    out.push(VisualStyle::ImageAlt);
    for tone in [
        CalloutTone::Note,
        CalloutTone::Info,
        CalloutTone::Tip,
        CalloutTone::Warning,
        CalloutTone::Danger,
        CalloutTone::Success,
        CalloutTone::Neutral,
    ] {
        out.push(VisualStyle::CalloutHeader(tone));
    }
    out.push(VisualStyle::CalloutBody);
    for slot in 0..=CUSTOM_SLOT {
        out.push(VisualStyle::ColorFg(slot));
        out.push(VisualStyle::ColorBg(slot));
    }
    out
}

/// Tags de color por slot (paleta §23 en el orden de `Palette::colors`, más
/// el bucket de personalizados).
const COLOR_FG_TAGS: [&str; 18] = [
    "mm-vis-fg-red",
    "mm-vis-fg-orange",
    "mm-vis-fg-yellow",
    "mm-vis-fg-green",
    "mm-vis-fg-blue",
    "mm-vis-fg-purple",
    "mm-vis-fg-gray",
    "mm-vis-fg-black",
    "mm-vis-fg-white",
    "mm-vis-fg-danger",
    "mm-vis-fg-warning",
    "mm-vis-fg-success",
    "mm-vis-fg-info",
    "mm-vis-fg-note",
    "mm-vis-fg-muted",
    "mm-vis-fg-highlight",
    "mm-vis-fg-accent",
    "mm-vis-fg-custom",
];

const COLOR_BG_TAGS: [&str; 18] = [
    "mm-vis-bg-red",
    "mm-vis-bg-orange",
    "mm-vis-bg-yellow",
    "mm-vis-bg-green",
    "mm-vis-bg-blue",
    "mm-vis-bg-purple",
    "mm-vis-bg-gray",
    "mm-vis-bg-black",
    "mm-vis-bg-white",
    "mm-vis-bg-danger",
    "mm-vis-bg-warning",
    "mm-vis-bg-success",
    "mm-vis-bg-info",
    "mm-vis-bg-note",
    "mm-vis-bg-muted",
    "mm-vis-bg-highlight",
    "mm-vis-bg-accent",
    "mm-vis-bg-custom",
];

/// Nombre estable del `gtk::TextTag` por estilo semántico.
pub fn tag_name(style: VisualStyle) -> &'static str {
    match style {
        VisualStyle::Heading(1) => "mm-vis-h1",
        VisualStyle::Heading(2) => "mm-vis-h2",
        VisualStyle::Heading(3) => "mm-vis-h3",
        VisualStyle::Heading(4) => "mm-vis-h4",
        VisualStyle::Heading(5) => "mm-vis-h5",
        VisualStyle::Heading(6) => "mm-vis-h6",
        VisualStyle::Strong => "mm-vis-strong",
        VisualStyle::Emphasis => "mm-vis-em",
        VisualStyle::Link => "mm-vis-link",
        VisualStyle::Code => "mm-vis-code",
        VisualStyle::CodeBlock => "mm-vis-codeblock",
        VisualStyle::Quote => "mm-vis-quote",
        VisualStyle::ListItem(1) => "mm-vis-li1",
        VisualStyle::ListItem(2) => "mm-vis-li2",
        VisualStyle::ListItem(3) => "mm-vis-li3",
        VisualStyle::ListItem(4) => "mm-vis-li4",
        VisualStyle::ListItem(5) => "mm-vis-li5",
        VisualStyle::ListItem(6) => "mm-vis-li6",
        VisualStyle::Task => "mm-vis-task",
        VisualStyle::TableHeader => "mm-vis-thead",
        VisualStyle::TableRule => "mm-vis-trule",
        VisualStyle::ImageAlt => "mm-vis-imgalt",
        VisualStyle::CalloutHeader(tone) => match tone {
            CalloutTone::Note => "mm-vis-co-note",
            CalloutTone::Info => "mm-vis-co-info",
            CalloutTone::Tip => "mm-vis-co-tip",
            CalloutTone::Warning => "mm-vis-co-warning",
            CalloutTone::Danger => "mm-vis-co-danger",
            CalloutTone::Success => "mm-vis-co-success",
            CalloutTone::Neutral => "mm-vis-co-neutral",
        },
        VisualStyle::CalloutBody => "mm-vis-co-body",
        VisualStyle::ColorFg(slot) => COLOR_FG_TAGS[slot.min(CUSTOM_SLOT) as usize],
        VisualStyle::ColorBg(slot) => COLOR_BG_TAGS[slot.min(CUSTOM_SLOT) as usize],
        // Niveles fuera de 1..=6 no existen en la gramática (§27.1).
        VisualStyle::Heading(_) => "mm-vis-h6",
        VisualStyle::ListItem(_) => "mm-vis-li6",
    }
}

/// Estilo del tag por estilo semántico bajo una paleta (§23): headings con
/// peso y escala por nivel (§27.1), `Strong` en negrita, `Emphasis` en
/// cursiva, `Link` subrayado con color de acento (§19.1), código
/// monoespaciado con fondo de código y cita con sangrado y color de cita
/// (§8.2). Los valores de color salen de la [`Palette`] (§23).
pub fn tag_style(style: VisualStyle, palette: &Palette) -> TagStyle {
    let mut out = TagStyle {
        fg: Some(palette.fg),
        bold: false,
        italic: false,
        underline: false,
        monospace: false,
        bg: None,
        scale: None,
        left_margin: None,
    };
    match style {
        VisualStyle::Heading(level) => {
            out.bold = true;
            out.scale = Some(match level {
                1 => 160,
                2 => 140,
                3 => 125,
                4 => 110,
                5 => 100,
                _ => 90,
            });
        }
        VisualStyle::Strong => out.bold = true,
        VisualStyle::Emphasis => out.italic = true,
        VisualStyle::Link => {
            out.underline = true;
            out.fg = palette.color("accent");
        }
        VisualStyle::Code | VisualStyle::CodeBlock => {
            out.monospace = true;
            out.bg = Some(palette.code_bg);
        }
        VisualStyle::Quote => {
            out.fg = Some(palette.quote);
            out.italic = true;
            out.left_margin = Some(QUOTE_LEFT_MARGIN);
        }
        // Ítems de lista: sangría visual coherente con el nivel (§8.2); la
        // sangría de anidamiento del texto se conserva visible (ISSUE-032).
        VisualStyle::ListItem(level) => {
            out.left_margin = Some(LIST_MARGIN_STEP * level as u32);
        }
        // Tarea GFM: marcador `[ ]`/`[x]` visible, monoespaciado y destacado
        // (decisión ISSUE-032).
        VisualStyle::Task => {
            out.monospace = true;
            out.fg = palette.color("accent");
        }
        // Tabla (§35.1): cabecera en negrita, separador con estilo de regla.
        VisualStyle::TableHeader => out.bold = true,
        VisualStyle::TableRule => out.fg = Some(palette.rule),
        // Imagen (§18): `alt` como chip/etiqueta semántica.
        VisualStyle::ImageAlt => {
            out.fg = palette.color("purple");
            out.bg = Some(palette.code_bg);
        }
        // Callout (§22): línea semántica con el color del tono del tipo y
        // contenido con sangrado y fondo; tipos desconocidos en neutro.
        VisualStyle::CalloutHeader(tone) => {
            out.bold = true;
            out.fg = palette.color(callout_color(tone));
        }
        VisualStyle::CalloutBody => {
            out.bg = Some(palette.code_bg);
            out.left_margin = Some(QUOTE_LEFT_MARGIN);
        }
        // Colores inline (§7.4.4/§23): texto, fondo o combinación.
        VisualStyle::ColorFg(slot) => out.fg = Some(color_value(slot, palette)),
        VisualStyle::ColorBg(slot) => out.bg = Some(color_value(slot, palette)),
    }
    out
}

/// Color de la paleta §23 por tono de callout (mismo mapeo que el Preview).
fn callout_color(tone: CalloutTone) -> &'static str {
    match tone {
        CalloutTone::Note => "note",
        CalloutTone::Info => "info",
        CalloutTone::Tip => "success",
        CalloutTone::Warning => "warning",
        CalloutTone::Danger => "danger",
        CalloutTone::Success => "success",
        CalloutTone::Neutral => "muted",
    }
}

/// Valor de color por slot: paleta §23 (`0..=16`); el bucket de personalizados
/// cae en acento hasta que la config llegue al parser.
fn color_value(slot: u8, palette: &Palette) -> &'static str {
    palette
        .colors
        .get(slot.min(CUSTOM_SLOT) as usize)
        .map(|(_, value)| *value)
        .or_else(|| palette.color("accent"))
        .unwrap_or("#000000")
}

/// Sangrado izquierdo (px) de las líneas de cita (§8.2).
const QUOTE_LEFT_MARGIN: u32 = 12;

/// Sangrado adicional (px) por nivel de anidamiento de lista (§8.2).
const LIST_MARGIN_STEP: u32 = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_document::Span;
    use minmark_parser::VisualSpan;

    fn plan_of(source: &str) -> VisualPlan {
        minmark_parser::visual(source)
    }

    #[test]
    fn plan_en_chars_multibyte_y_ordenado() {
        // chars: "## " 0..3, "ñáé " 3..7, "**" 7..9, "í" 9..10, "**" 10..12, "\n" 12.
        let source = "## ñáé **í**\n";
        let plan = plan_of(source);
        let ranges = visual_tag_plan(source, &plan);
        let syntax: Vec<_> = ranges
            .iter()
            .filter(|r| r.style.is_none())
            .map(|r| (r.start_char, r.end_char))
            .collect();
        assert_eq!(syntax, vec![(0, 3), (7, 9), (10, 12)]);
        let contenido: Vec<_> = ranges
            .iter()
            .filter_map(|r| r.style.map(|s| (r.start_char, r.end_char, s)))
            .collect();
        assert_eq!(
            contenido,
            vec![
                (3, 12, VisualStyle::Heading(2)),
                (9, 10, VisualStyle::Strong),
            ]
        );
        // Orden estable por posición.
        assert!(ranges.windows(2).all(|w| w[0].start_char <= w[1].start_char));
    }

    #[test]
    fn rangos_de_elementos_en_chars() {
        let source = "# T\n\npárrafo\n";
        let plan = plan_of(source);
        let elements = element_ranges(source, &plan);
        assert_eq!(elements.first(), Some(&(0, 4)));
        assert_eq!(elements.last(), Some(&(5, 13)));
    }

    #[test]
    fn nombres_de_tags_unicos() {
        let mut vistos = std::collections::HashSet::new();
        for style in all_styles() {
            assert!(vistos.insert(tag_name(style)), "{style:?}");
        }
        assert_eq!(vistos.len(), all_styles().len());
    }

    #[test]
    fn estilos_visuales() {
        let palette = Palette::LIGHT;
        let h1 = tag_style(VisualStyle::Heading(1), &palette);
        assert!(h1.bold && h1.scale == Some(160));
        let h6 = tag_style(VisualStyle::Heading(6), &palette);
        assert!(h6.bold && h6.scale == Some(90));
        let strong = tag_style(VisualStyle::Strong, &palette);
        assert!(strong.bold && !strong.italic && strong.scale.is_none());
        let em = tag_style(VisualStyle::Emphasis, &palette);
        assert!(em.italic && !em.bold);
        // §19.1: enlace subrayado con color de la paleta.
        let link = tag_style(VisualStyle::Link, &palette);
        assert!(link.underline && link.fg == palette.color("accent"));
        // §7.4.1: código monoespaciado con fondo de código.
        for style in [VisualStyle::Code, VisualStyle::CodeBlock] {
            let code = tag_style(style, &palette);
            assert!(code.monospace && code.bg == Some(palette.code_bg));
        }
        // §8.2: cita con sangrado y color de cita.
        let quote = tag_style(VisualStyle::Quote, &palette);
        assert!(quote.italic && quote.fg == Some(palette.quote));
        assert!(quote.left_margin.is_some());
        // §8.2: ítems con sangría coherente con el nivel; tarea destacada.
        let li1 = tag_style(VisualStyle::ListItem(1), &palette);
        let li2 = tag_style(VisualStyle::ListItem(2), &palette);
        assert!(li1.left_margin.is_some() && li2.left_margin > li1.left_margin);
        let task = tag_style(VisualStyle::Task, &palette);
        assert!(task.monospace && task.fg == palette.color("accent"));
        // §35.1/§18: cabecera en negrita, regla con color de regla, chip de
        // imagen con fondo de código.
        let head = tag_style(VisualStyle::TableHeader, &palette);
        assert!(head.bold);
        let rule = tag_style(VisualStyle::TableRule, &palette);
        assert!(rule.fg == Some(palette.rule));
        let alt = tag_style(VisualStyle::ImageAlt, &palette);
        assert!(alt.fg == palette.color("purple") && alt.bg == Some(palette.code_bg));
        // §22: callout con color del tono; desconocidos en neutro.
        let warning = tag_style(VisualStyle::CalloutHeader(CalloutTone::Warning), &palette);
        assert!(warning.bold && warning.fg == palette.color("warning"));
        let neutral = tag_style(VisualStyle::CalloutHeader(CalloutTone::Neutral), &palette);
        assert!(neutral.fg == palette.color("muted"));
        let body = tag_style(VisualStyle::CalloutBody, &palette);
        assert!(body.bg == Some(palette.code_bg) && body.left_margin.is_some());
        // §7.4.4/§23: colores inline por slot; personalizados en acento.
        let fg = tag_style(VisualStyle::ColorFg(9), &palette);
        assert!(fg.fg == palette.color("danger"));
        let bg = tag_style(VisualStyle::ColorBg(15), &palette);
        assert!(bg.bg == palette.color("highlight"));
        let custom = tag_style(VisualStyle::ColorFg(99), &palette);
        assert!(custom.fg == palette.color("accent"));
    }

    #[test]
    fn plan_callout_y_color_en_chars() {
        // chars: "::: " 0..4, "note" 4..8, "x" 9..10, ":::" 11..14.
        let source = "::: note\nx\n:::\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (0, 4, None),
                (4, 8, Some(VisualStyle::CalloutHeader(CalloutTone::Note))),
                (9, 10, Some(VisualStyle::CalloutBody)),
                (11, 14, None),
            ]
        );

        // chars: "{danger: " 0..9, "x" 9..10, "}" 10..11.
        let source = "{danger: x}\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(
            styles,
            vec![(0, 9, None), (9, 10, Some(VisualStyle::ColorFg(9))), (10, 11, None)]
        );
    }

    #[test]
    fn slots_de_color_personalizados_se_normalizan() {
        let source = "x\n";
        let mut plan = plan_of(source);
        plan.spans.push(minmark_parser::VisualSpan {
            span: minmark_document::Span { start: 0, end: 1 },
            style: Some(VisualStyle::ColorFg(42)),
        });
        let ranges = visual_tag_plan(source, &plan);
        assert_eq!(
            ranges.last().map(|r| r.style),
            Some(Some(VisualStyle::ColorFg(17)))
        );
    }

    #[test]
    fn plan_tabla_e_imagen_en_chars() {
        // chars: "| " 0..2, "A" 2..3, " |" 3..5, "\n|---|" …
        let source = "| A |\n|---|\n| 1 |\n";
        let plan = plan_of(source);
        let styled: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .filter_map(|r| r.style.map(|s| (r.start_char, r.end_char, s)))
            .collect();
        assert_eq!(
            styled,
            vec![
                (2, 3, VisualStyle::TableHeader),
                (7, 10, VisualStyle::TableRule),
            ]
        );

        // chars: "![" 0..2, "alt" 2..5, "](img.png)" 5..15.
        let source = "![alt](img.png)\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (0, 2, None),
                (2, 5, Some(VisualStyle::ImageAlt)),
                (5, 15, None),
            ]
        );
    }

    #[test]
    fn plan_link_en_chars() {
        // chars: "[" 0..1, "a" 1..2, "](b)" 2..6.
        let source = "[a](b)\n";
        let plan = plan_of(source);
        let ranges = visual_tag_plan(source, &plan);
        assert_eq!(
            ranges,
            vec![
                VisualTagRange {
                    start_char: 0,
                    end_char: 1,
                    style: None
                },
                VisualTagRange {
                    start_char: 1,
                    end_char: 2,
                    style: Some(VisualStyle::Link)
                },
                VisualTagRange {
                    start_char: 2,
                    end_char: 6,
                    style: None
                },
            ]
        );
    }

    #[test]
    fn plan_code_y_cita_en_chars() {
        let source = "`x` y\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (0, 1, None),
                (1, 2, Some(VisualStyle::Code)),
                (2, 3, None),
            ]
        );

        let source = "> hola\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(styles, vec![(0, 2, None), (2, 6, Some(VisualStyle::Quote))]);
    }

    #[test]
    fn plan_listas_y_tareas_en_chars() {
        // chars: "- " 0..2, "a" 2..3.
        let source = "- a\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(styles, vec![(0, 2, None), (2, 3, Some(VisualStyle::ListItem(1)))]);

        // chars: "- " 0..2, "[x]" 2..5, " " 5..6, "hecho" 6..11.
        let source = "- [x] hecho\n";
        let plan = plan_of(source);
        let styles: Vec<_> = visual_tag_plan(source, &plan)
            .into_iter()
            .map(|r| (r.start_char, r.end_char, r.style))
            .collect();
        assert_eq!(
            styles,
            vec![
                (0, 2, None),
                (2, 5, Some(VisualStyle::Task)),
                (6, 11, Some(VisualStyle::ListItem(1))),
            ]
        );
    }

    #[test]
    fn recorte_seguro_con_spans_invalidos() {
        let source = "ñé\n";
        let mut plan = plan_of(source);
        // Vacío, invertido y fuera de rango: se descartan sin panic.
        plan.spans.push(VisualSpan {
            span: Span { start: 0, end: 0 },
            style: None,
        });
        plan.spans.push(VisualSpan {
            span: Span { start: 2, end: 1 },
            style: None,
        });
        plan.spans.push(VisualSpan {
            span: Span {
                start: 900,
                end: 999,
            },
            style: None,
        });
        let ranges = visual_tag_plan(source, &plan);
        assert!(ranges.iter().all(|r| r.start_char < r.end_char));
    }
}
