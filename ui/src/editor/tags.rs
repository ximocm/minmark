//! Plan de tags del Markdown Mode (ISSUE-015) — lógica pura, sin GTK.
//!
//! Traduce los tokens de [`minmark_parser::highlight`] (spans en BYTES del
//! fuente) a rangos en CARÁCTERES unicode sobre el texto del `TextBuffer`,
//! junto con el estilo de cada `gtk::TextTag` a partir de una
//! [`minmark_renderer::Palette`]. Los tests de este módulo son puros: no
//! inicializan GTK ni crean widgets.

use minmark_parser::{Highlight, HighlightKind};
use minmark_renderer::Palette;

/// Rango de un token en CARÁCTERES unicode sobre el texto del buffer
/// (índices 0-based de `char`, como `gtk::TextIter::offset()`).
///
/// `end_char` es exclusivo: `[start_char, end_char)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRange {
    pub start_char: usize,
    pub end_char: usize,
    pub kind: HighlightKind,
}

/// Convierte highlights (spans en BYTES del fuente) a rangos en CARÁCTERES
/// sobre `text` (índices de char unicode), recortando a límites de char
/// válidos y descartando rangos vacíos; salida en orden de aparición.
///
/// Un span que empieza o termina a mitad de un char multibyte se recorta al
/// char contiguo completo (inicio hacia arriba, final hacia abajo); si tras el
/// recorte no queda ningún char entero, el rango se descarta sin más (nunca
/// panic ni slicing fuera de fronteras). A igual posición de inicio se
/// conserva el orden del parser (orden estable).
pub fn tag_plan(text: &str, highlights: &[Highlight]) -> Vec<TagRange> {
    let mut plan: Vec<TagRange> = Vec::with_capacity(highlights.len());
    for highlight in highlights {
        let Some((start_char, end_char)) =
            char_range(text, highlight.span.start as usize, highlight.span.end as usize)
        else {
            continue;
        };
        plan.push(TagRange {
            start_char,
            end_char,
            kind: highlight.kind,
        });
    }
    // Orden de aparición con empates estables (conservan el orden del parser).
    plan.sort_by_key(|range| range.start_char);
    plan
}

/// Convierte un rango `[start_byte, end_byte)` del fuente a `[start_char,
/// end_char)` sobre `text` (índices de char unicode), con las mismas reglas
/// de recorte que [`tag_plan`]; `None` si no queda ningún char entero.
pub(crate) fn char_range(
    text: &str,
    start_byte: usize,
    end_byte: usize,
) -> Option<(usize, usize)> {
    let start_byte = ceil_char_boundary(text, start_byte);
    let end_byte = floor_char_boundary(text, end_byte);
    if start_byte >= end_byte {
        return None;
    }
    let start_char = text[..start_byte].chars().count();
    let end_char = start_char + text[start_byte..end_byte].chars().count();
    Some((start_char, end_char))
}

/// Frontera de char ≥ `index` (para inicios de span).
fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Frontera de char ≤ `index` (para finales de span, exclusivos).
fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Estilo de un `gtk::TextTag` para un tipo de token bajo una paleta (§23).
///
/// Los valores de color salen de la [`Palette`] (fuente única de color §23):
/// campos estructurales (`fg`, `muted`, `quote`, `rule`, `code_bg`) y la
/// paleta semántica/física vía [`Palette::color`]. Los colores de tema
/// completo de los widgets los fija el integrador.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagStyle {
    pub fg: Option<&'static str>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub monospace: bool,
    pub bg: Option<&'static str>,
    /// Escala tipográfica en tanto por ciento (`100` = normal); `None` deja
    /// la escala del widget (headings de Visual, ISSUE-030).
    pub scale: Option<u32>,
    /// Sangrado izquierdo en px (`None` = sin sangrado; citas de Visual,
    /// ISSUE-031).
    pub left_margin: Option<u32>,
}

/// Estilo por tipo de token (§23): Heading/Strong en negrita, Emphasis en
/// cursiva, Code/CodeBlock monoespaciados con fondo de código.
pub fn tag_style(kind: HighlightKind, palette: &Palette) -> TagStyle {
    let mut style = TagStyle {
        fg: Some(palette.fg),
        bold: false,
        italic: false,
        underline: false,
        monospace: false,
        bg: None,
        scale: None,
        left_margin: None,
    };
    match kind {
        HighlightKind::FrontMatter => style.fg = Some(palette.muted),
        HighlightKind::Heading => style.bold = true,
        HighlightKind::Quote => {
            style.fg = Some(palette.quote);
            style.italic = true;
        }
        HighlightKind::List => style.fg = palette.color("blue"),
        HighlightKind::CodeBlock | HighlightKind::Code => {
            style.monospace = true;
            style.bg = Some(palette.code_bg);
        }
        HighlightKind::Table => style.fg = palette.color("green"),
        HighlightKind::Rule => style.fg = Some(palette.rule),
        HighlightKind::Emphasis => style.italic = true,
        HighlightKind::Strong => style.bold = true,
        HighlightKind::Strikethrough => style.fg = Some(palette.muted),
        HighlightKind::Link | HighlightKind::WikiLink => style.fg = palette.color("info"),
        HighlightKind::Image => style.fg = palette.color("purple"),
        HighlightKind::Math | HighlightKind::MathBlock => style.fg = palette.color("purple"),
        HighlightKind::Color | HighlightKind::Substitution => style.fg = palette.color("orange"),
        HighlightKind::Citation => style.fg = palette.color("green"),
        HighlightKind::Callout => style.fg = palette.color("note"),
    }
    style
}

/// Nombre estable del `gtk::TextTag` por tipo de token.
pub fn tag_name(kind: HighlightKind) -> &'static str {
    match kind {
        HighlightKind::FrontMatter => "mm-front-matter",
        HighlightKind::Heading => "mm-heading",
        HighlightKind::Quote => "mm-quote",
        HighlightKind::List => "mm-list",
        HighlightKind::CodeBlock => "mm-code-block",
        HighlightKind::Code => "mm-code",
        HighlightKind::Table => "mm-table",
        HighlightKind::Rule => "mm-rule",
        HighlightKind::Emphasis => "mm-emphasis",
        HighlightKind::Strong => "mm-strong",
        HighlightKind::Strikethrough => "mm-strikethrough",
        HighlightKind::Link => "mm-link",
        HighlightKind::Image => "mm-image",
        HighlightKind::Math => "mm-math",
        HighlightKind::MathBlock => "mm-math-block",
        HighlightKind::Color => "mm-color",
        HighlightKind::Citation => "mm-citation",
        HighlightKind::WikiLink => "mm-wiki-link",
        HighlightKind::Substitution => "mm-substitution",
        HighlightKind::Callout => "mm-callout",
    }
}

/// Todos los tipos, para crear la TextTagTable completa.
pub fn all_kinds() -> &'static [HighlightKind] {
    &[
        HighlightKind::FrontMatter,
        HighlightKind::Heading,
        HighlightKind::Quote,
        HighlightKind::List,
        HighlightKind::CodeBlock,
        HighlightKind::Code,
        HighlightKind::Table,
        HighlightKind::Rule,
        HighlightKind::Emphasis,
        HighlightKind::Strong,
        HighlightKind::Strikethrough,
        HighlightKind::Link,
        HighlightKind::Image,
        HighlightKind::Math,
        HighlightKind::MathBlock,
        HighlightKind::Color,
        HighlightKind::Citation,
        HighlightKind::WikiLink,
        HighlightKind::Substitution,
        HighlightKind::Callout,
    ]
}

#[cfg(test)]
mod tests {
    use minmark_document::Span;
    use minmark_parser::{Highlight, HighlightKind};
    use minmark_renderer::Palette;

    use super::{all_kinds, tag_name, tag_plan, tag_style, TagRange};

    fn token(start: u32, end: u32, kind: HighlightKind) -> Highlight {
        Highlight {
            span: Span { start, end },
            kind,
        }
    }

    /// 1. Bytes → chars con `ñ` (2 bytes), `👍` (4 bytes) y `日本` (3+3).
    /// Texto: "ñ👍日本" = 12 bytes, 4 chars.
    #[test]
    fn tag_plan_bytes_a_chars_unicode() {
        let text = "ñ👍日本";
        assert_eq!(text.len(), 12);
        assert_eq!(text.chars().count(), 4);

        let plan = tag_plan(
            text,
            &[
                token(0, 2, HighlightKind::Heading), // "ñ"        → chars [0,1)
                token(2, 6, HighlightKind::Strong),  // "👍"       → chars [1,2)
                token(6, 12, HighlightKind::Code),   // "日本"     → chars [2,4)
                token(2, 9, HighlightKind::Link),    // "👍日"     → chars [1,3)
            ],
        );

        assert_eq!(
            plan,
            vec![
                TagRange {
                    start_char: 0,
                    end_char: 1,
                    kind: HighlightKind::Heading
                },
                TagRange {
                    start_char: 1,
                    end_char: 2,
                    kind: HighlightKind::Strong
                },
                TagRange {
                    start_char: 1,
                    end_char: 3,
                    kind: HighlightKind::Link
                },
                TagRange {
                    start_char: 2,
                    end_char: 4,
                    kind: HighlightKind::Code
                },
            ]
        );
    }

    /// 2. Span a mitad de char multibyte o vacío → descartado sin panic
    /// (recorte a límites de char).
    #[test]
    fn tag_plan_recorta_a_limites_de_char() {
        let text = "a日本"; // 1 + 3 + 3 = 7 bytes, 3 chars.

        // Vacíos e invertidos: descartados.
        assert!(tag_plan(text, &[token(3, 3, HighlightKind::Code)]).is_empty());
        assert!(tag_plan(text, &[token(5, 2, HighlightKind::Code)]).is_empty());
        // Entero dentro de un char multibyte: tras recortar no queda nada.
        assert!(tag_plan(text, &[token(1, 2, HighlightKind::Code)]).is_empty());
        assert!(tag_plan(text, &[token(4, 5, HighlightKind::Code)]).is_empty());
        // Fuera del texto: recortado a vacío, sin panic.
        assert!(tag_plan(text, &[token(7, 9, HighlightKind::Code)]).is_empty());
        assert!(tag_plan("", &[token(0, 1, HighlightKind::Code)]).is_empty());

        // Final a mitad de char (byte 3 dentro de `日`): se recorta al char
        // completo previo → [0,1).
        assert_eq!(
            tag_plan(text, &[token(0, 3, HighlightKind::Code)]),
            vec![TagRange {
                start_char: 0,
                end_char: 1,
                kind: HighlightKind::Code
            }]
        );
        // Inicio a mitad de char (byte 2 dentro de `日`): se recorta al char
        // completo siguiente → [2,3).
        assert_eq!(
            tag_plan(text, &[token(2, 7, HighlightKind::Code)]),
            vec![TagRange {
                start_char: 2,
                end_char: 3,
                kind: HighlightKind::Code
            }]
        );
    }

    /// 3. Orden estable: salida en orden de aparición; empates conservan el
    /// orden del parser.
    #[test]
    fn tag_plan_orden_estable() {
        let text = "ñ👍日本"; // 4 chars.

        // Fuera de orden: la salida sigue el orden de aparición.
        let plan = tag_plan(
            text,
            &[
                token(6, 12, HighlightKind::Code), // chars [2,4)
                token(0, 2, HighlightKind::Heading), // chars [0,1)
            ],
        );
        assert_eq!(
            plan,
            vec![
                TagRange {
                    start_char: 0,
                    end_char: 1,
                    kind: HighlightKind::Heading
                },
                TagRange {
                    start_char: 2,
                    end_char: 4,
                    kind: HighlightKind::Code
                },
            ]
        );

        // Empate de inicio (contenedor + token interno): estable = orden del
        // parser, contenedor primero.
        let plan = tag_plan(
            text,
            &[
                token(0, 12, HighlightKind::Quote),
                token(0, 6, HighlightKind::Strong),
                token(2, 6, HighlightKind::Emphasis),
            ],
        );
        assert_eq!(
            plan.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![
                HighlightKind::Quote,
                HighlightKind::Strong,
                HighlightKind::Emphasis
            ]
        );
        assert_eq!(
            plan.iter().map(|r| (r.start_char, r.end_char)).collect::<Vec<_>>(),
            vec![(0, 4), (0, 2), (1, 2)]
        );
    }

    /// 4. `tag_style`: Heading/Strong en negrita, Emphasis en cursiva,
    /// Code/CodeBlock monoespaciados con `bg`; colores distintos entre
    /// `Palette::LIGHT` y `Palette::DARK`.
    #[test]
    fn tag_style_tipos_y_paletas() {
        for palette in [&Palette::LIGHT, &Palette::DARK] {
            assert!(tag_style(HighlightKind::Heading, palette).bold);
            let strong = tag_style(HighlightKind::Strong, palette);
            assert!(strong.bold && !strong.italic);

            let emphasis = tag_style(HighlightKind::Emphasis, palette);
            assert!(emphasis.italic && !emphasis.bold);

            for kind in [HighlightKind::Code, HighlightKind::CodeBlock] {
                let code = tag_style(kind, palette);
                assert!(code.monospace);
                assert_eq!(code.bg, Some(palette.code_bg));
            }
        }

        for &kind in all_kinds() {
            assert_ne!(
                tag_style(kind, &Palette::LIGHT),
                tag_style(kind, &Palette::DARK),
                "el estilo de {kind:?} debe depender de la paleta"
            );
        }
    }

    #[test]
    fn nombres_de_tag_estables_y_unicos() {
        assert_eq!(all_kinds().len(), 20);
        let mut names: Vec<&str> = all_kinds().iter().map(|&kind| tag_name(kind)).collect();
        names.sort_unstable();
        let unique = {
            let before = names.len();
            names.dedup();
            names.len() == before
        };
        assert!(unique, "los nombres de tag deben ser únicos por tipo");
    }
}
