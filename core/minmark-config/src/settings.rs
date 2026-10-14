//! Modelo único de ajustes y valores por defecto (§58.2).
//!
//! Cada ajuste declara los niveles donde aplica (§58.1). Los valores fijos no
//! configurables (§58.2) no son ajustes: UTF-8 solo (§12.5), LF en archivos
//! nuevos (§12.6) y milímetros como unidad PDF (§32.4).

use crate::{Level, Value, WarningReason};
use std::collections::BTreeMap;

/// Límite por defecto de almacenamiento de snapshots: 1 GB (§13.5).
pub const SNAPSHOT_LIMIT_DEFAULT_BYTES: u64 = 1_000_000_000;

/// Niveles que aplican a la mayoría de ajustes (§58.2).
const ALL_LEVELS: &[Level] = &[
    Level::Document,
    Level::Composition,
    Level::Project,
    Level::Global,
];

/// Vocabulario de claves de configuración con los niveles donde aplican
/// (§58.2). Fuente única para la resolución y para validación de capas.
pub const KEYS: &[(&str, &[Level])] = &[
    ("preview_max_width_px", ALL_LEVELS),
    ("pagination", ALL_LEVELS),
    ("theme", &[Level::Document, Level::Project, Level::Global]),
    ("snapshot_limit", &[Level::Global]),
    ("visual_syntax", &[Level::Global]),
    ("wiki_links", &[Level::Project, Level::Global]),
    ("page_break_between_files", &[Level::Composition]),
    ("custom_colors", &[Level::Project]),
    ("custom_callout_types", &[Level::Project]),
    (
        "bibliography",
        &[Level::Document, Level::Project, Level::Global],
    ),
    (
        "bibliography_style",
        &[Level::Document, Level::Project, Level::Global],
    ),
    ("heading_numbering", ALL_LEVELS),
    (
        "language",
        &[Level::Document, Level::Project, Level::Global],
    ),
    ("heading_number_format", ALL_LEVELS),
    ("counter_numbering", ALL_LEVELS),
    ("toc_max_level", ALL_LEVELS),
    ("pdf_page_size", ALL_LEVELS),
    ("pdf_margins_mm", ALL_LEVELS),
    ("pdf_orientation", ALL_LEVELS),
    ("pdf_page_number_position", ALL_LEVELS),
    ("pdf_header", ALL_LEVELS),
    ("pdf_footer", ALL_LEVELS),
    ("body_font", ALL_LEVELS),
    ("font_size_pt", ALL_LEVELS),
    ("line_height", ALL_LEVELS),
    ("paragraph_style", ALL_LEVELS),
    ("text_alignment", ALL_LEVELS),
    ("code_line_numbers", ALL_LEVELS),
    ("code_word_wrap", ALL_LEVELS),
    ("table_min_font_pt", ALL_LEVELS),
];

/// Niveles aplicables a una clave; `None` si la clave no existe.
pub(crate) fn levels_for(key: &str) -> Option<&'static [Level]> {
    KEYS.iter()
        .find(|(name, _)| *name == key)
        .map(|(_, levels)| *levels)
}

/// Tema de la aplicación (§11). Default: System (§58.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

/// Visibilidad de la sintaxis en el modo Visual (§8.3, ISSUE-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisualSyntax {
    /// Ocultar siempre la sintaxis.
    Hidden,
    /// Revelar temporalmente la sintaxis del elemento actualmente editado
    /// (defecto, §58.2).
    #[default]
    RevealCurrent,
}

/// Formato de numeración de headings (§27.1: único formato `1`, `1.1`, `1.1.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeadingNumberFormat {
    #[default]
    Hierarchical,
}

/// Alcance de la numeración de figuras, tablas y ecuaciones (§27.2).
/// Default: continua (§58.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumberingScope {
    #[default]
    Continuous,
    ByChapter,
    ByChapterReset,
}

/// Tamaño de página PDF (§32.3). Unidades: solo milímetros (§32.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSize {
    A3,
    A4,
    A5,
    A6,
    Letter,
    Legal,
    Executive,
    Custom { width_mm: f32, height_mm: f32 },
}

impl Default for PageSize {
    fn default() -> Self {
        Self::A4
    }
}

/// Márgenes PDF en milímetros (§32.5). Default: 25 mm en los cuatro lados.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub top_mm: f32,
    pub right_mm: f32,
    pub bottom_mm: f32,
    pub left_mm: f32,
}

impl Margins {
    pub fn uniform(mm: f32) -> Self {
        Self {
            top_mm: mm,
            right_mm: mm,
            bottom_mm: mm,
            left_mm: mm,
        }
    }
}

impl Default for Margins {
    fn default() -> Self {
        Self::uniform(25.0)
    }
}

/// Orientación de página PDF (§32.6). Default: Portrait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

/// Posición del número de página PDF (§32.8). Default: Footer right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageNumberPosition {
    HeaderLeft,
    HeaderCenter,
    HeaderRight,
    FooterLeft,
    FooterCenter,
    #[default]
    FooterRight,
}

/// Familia tipográfica del cuerpo (§33). Default: sans-serif (§33.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
}

/// Estilo de párrafos (§33.4). Default: espaciado vertical, sin sangría.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParagraphStyle {
    #[default]
    VerticalSpacing,
    Indented,
}

/// Alineación del texto (§33.5). Default: left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Ajustes de la aplicación con sus valores por defecto (§58.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Idioma de las etiquetas derivadas (§26.6).
    pub language: String,
    /// §10.2: ancho máximo del Preview continuo.
    pub preview_max_width_px: u32,
    /// §10.3: paginación activada.
    pub pagination_enabled: bool,
    /// §11.
    pub theme: Theme,
    pub visual_syntax: VisualSyntax,
    /// §13.5: límite de almacenamiento de snapshots, en bytes.
    pub snapshot_limit_bytes: u64,
    /// §19.3.
    pub wiki_links: bool,
    /// §20.3 (solo nivel composición).
    pub page_break_between_files: bool,
    /// §23: paleta de colores redefinida por proyecto, nombre → color.
    pub custom_colors: BTreeMap<String, String>,
    /// §22: tipos `:::` personalizados (nombres).
    pub custom_callout_types: Vec<String>,
    /// §25.4.
    pub bibliography_style: String,
    /// §25.1: recursos bibliográficos locales.
    pub bibliography: Vec<String>,
    /// §27.1.
    pub heading_numbering: bool,
    /// §27.1.
    pub heading_number_format: HeadingNumberFormat,
    /// §27.2.
    pub counter_numbering: NumberingScope,
    /// §29: profundidad de la TOC (H1–H6).
    pub toc_max_level: u8,
    /// §32.3.
    pub pdf_page_size: PageSize,
    /// §32.5.
    pub pdf_margins_mm: Margins,
    /// §32.6.
    pub pdf_orientation: Orientation,
    /// §32.8.
    pub pdf_page_number_position: PageNumberPosition,
    /// §32.9: plantilla de header; `None` = sin header.
    pub pdf_header: Option<String>,
    /// §32.9: plantilla de footer; `None` = sin footer.
    pub pdf_footer: Option<String>,
    /// §33.1.
    pub body_font: FontFamily,
    /// §33.2.
    pub font_size_pt: f32,
    /// §33.3.
    pub line_height: f32,
    /// §33.4.
    pub paragraph_style: ParagraphStyle,
    /// §33.5.
    pub text_alignment: TextAlignment,
    /// §34.1.
    pub code_line_numbers: bool,
    /// §34.2: `false` = scroll horizontal.
    pub code_word_wrap: bool,
    /// §35.2: tipografía mínima al reducir tablas en PDF.
    pub table_min_font_pt: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preview_max_width_px: 800,
            pagination_enabled: false,
            theme: Theme::System,
            visual_syntax: VisualSyntax::default(),
            snapshot_limit_bytes: SNAPSHOT_LIMIT_DEFAULT_BYTES,
            wiki_links: false,
            page_break_between_files: false,
            custom_colors: BTreeMap::new(),
            custom_callout_types: Vec::new(),
            bibliography_style: "IEEE".to_string(),
            bibliography: Vec::new(),
            heading_numbering: false,
            language: "es".into(),
            heading_number_format: HeadingNumberFormat::Hierarchical,
            counter_numbering: NumberingScope::Continuous,
            toc_max_level: 3,
            pdf_page_size: PageSize::A4,
            pdf_margins_mm: Margins::default(),
            pdf_orientation: Orientation::Portrait,
            pdf_page_number_position: PageNumberPosition::FooterRight,
            pdf_header: None,
            pdf_footer: None,
            body_font: FontFamily::SansSerif,
            font_size_pt: 12.0,
            line_height: 1.3,
            paragraph_style: ParagraphStyle::VerticalSpacing,
            text_alignment: TextAlignment::Left,
            code_line_numbers: false,
            code_word_wrap: false,
            table_min_font_pt: 8.0,
        }
    }
}

impl Settings {
    /// Aplica `key = value` sobre estos ajustes.
    ///
    /// `Err(WarningReason::UnknownKey)` = clave fuera del vocabulario;
    /// `Err(WarningReason::InvalidValue)` = valor inaplicable al ajuste.
    pub(crate) fn apply(&mut self, key: &str, value: &Value) -> Result<(), WarningReason> {
        match key {
            "preview_max_width_px" => {
                self.preview_max_width_px = as_u32_in(value, 1..=u32::MAX as u64)? as u32
            }
            "pagination" => self.pagination_enabled = as_bool(value)?,
            "theme" => self.theme = Theme::parse(value)?,
            "visual_syntax" => self.visual_syntax = parse_visual_syntax(value)?,
            "snapshot_limit" => self.snapshot_limit_bytes = as_u64(value)?,
            "wiki_links" => self.wiki_links = as_bool(value)?,
            "page_break_between_files" => self.page_break_between_files = as_bool(value)?,
            "custom_colors" => self.custom_colors = parse_string_map(value)?,
            "custom_callout_types" => self.custom_callout_types = parse_string_list(value)?,
            "bibliography_style" => {
                let style = as_text(value)?;
                if style.trim().is_empty() {
                    return Err(WarningReason::InvalidValue);
                }
                self.bibliography_style = style;
            }
            "bibliography" => {
                self.bibliography = match value {
                    Value::Text(path) if !path.trim().is_empty() => vec![path.clone()],
                    _ => parse_string_list(value)?,
                }
            }
            "heading_numbering" => self.heading_numbering = as_bool(value)?,
            "language" => {
                self.language = match value {
                    Value::Text(language) if !language.trim().is_empty() => language.clone(),
                    _ => return Err(WarningReason::InvalidValue),
                }
            }
            "heading_number_format" => self.heading_number_format = parse_heading_format(value)?,
            "counter_numbering" => self.counter_numbering = parse_counter_numbering(value)?,
            "toc_max_level" => self.toc_max_level = as_u32_in(value, 1..=6)? as u8,
            "pdf_page_size" => self.pdf_page_size = PageSize::parse(value)?,
            "pdf_margins_mm" => self.pdf_margins_mm = parse_margins(value)?,
            "pdf_orientation" => self.pdf_orientation = parse_orientation(value)?,
            "pdf_page_number_position" => {
                self.pdf_page_number_position = parse_page_number_position(value)?
            }
            "pdf_header" => self.pdf_header = parse_optional_text(value)?,
            "pdf_footer" => self.pdf_footer = parse_optional_text(value)?,
            "body_font" => self.body_font = parse_font_family(value)?,
            "font_size_pt" => self.font_size_pt = as_positive_f32(value)?,
            "line_height" => self.line_height = as_positive_f32(value)?,
            "paragraph_style" => self.paragraph_style = parse_paragraph_style(value)?,
            "text_alignment" => self.text_alignment = parse_text_alignment(value)?,
            "code_line_numbers" => self.code_line_numbers = as_bool(value)?,
            "code_word_wrap" => self.code_word_wrap = as_bool(value)?,
            "table_min_font_pt" => self.table_min_font_pt = as_positive_f32(value)?,
            _ => return Err(WarningReason::UnknownKey),
        }
        Ok(())
    }
}

// --- Conversión de valores ---------------------------------------------------

const INVALID: WarningReason = WarningReason::InvalidValue;

fn as_bool(value: &Value) -> Result<bool, WarningReason> {
    match value {
        Value::Bool(b) => Ok(*b),
        _ => Err(INVALID),
    }
}

fn as_u64(value: &Value) -> Result<u64, WarningReason> {
    match value {
        Value::Integer(i) if *i >= 0 => Ok(*i as u64),
        _ => Err(INVALID),
    }
}

fn as_u32_in(value: &Value, range: std::ops::RangeInclusive<u64>) -> Result<u64, WarningReason> {
    let n = as_u64(value)?;
    if range.contains(&n) {
        Ok(n)
    } else {
        Err(INVALID)
    }
}

fn as_f32(value: &Value) -> Result<f32, WarningReason> {
    let n = match value {
        Value::Integer(i) => *i as f32,
        Value::Number(n) if n.is_finite() => *n as f32,
        _ => return Err(INVALID),
    };
    Ok(n)
}

fn as_positive_f32(value: &Value) -> Result<f32, WarningReason> {
    let n = as_f32(value)?;
    if n > 0.0 {
        Ok(n)
    } else {
        Err(INVALID)
    }
}

fn as_text(value: &Value) -> Result<String, WarningReason> {
    match value {
        Value::Text(t) => Ok(t.clone()),
        _ => Err(INVALID),
    }
}

fn parse_optional_text(value: &Value) -> Result<Option<String>, WarningReason> {
    let text = as_text(value)?;
    if text.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

fn parse_string_map(value: &Value) -> Result<BTreeMap<String, String>, WarningReason> {
    match value {
        Value::Map(map) => {
            let mut out = BTreeMap::new();
            for (k, v) in map {
                out.insert(k.clone(), as_text(v)?);
            }
            Ok(out)
        }
        _ => Err(INVALID),
    }
}

fn parse_string_list(value: &Value) -> Result<Vec<String>, WarningReason> {
    match value {
        Value::List(items) => items.iter().map(as_text).collect(),
        _ => Err(INVALID),
    }
}

fn parse_margins(value: &Value) -> Result<Margins, WarningReason> {
    match value {
        // Un solo número: los cuatro lados (§32.5).
        Value::Integer(i) if *i >= 0 => Ok(Margins::uniform(*i as f32)),
        Value::Number(n) if n.is_finite() && *n >= 0.0 => Ok(Margins::uniform(*n as f32)),
        // Cuatro números: top, right, bottom, left (orden CSS).
        Value::List(items) if items.len() == 4 => Ok(Margins {
            top_mm: as_non_negative_f32(&items[0])?,
            right_mm: as_non_negative_f32(&items[1])?,
            bottom_mm: as_non_negative_f32(&items[2])?,
            left_mm: as_non_negative_f32(&items[3])?,
        }),
        _ => Err(INVALID),
    }
}

fn as_non_negative_f32(value: &Value) -> Result<f32, WarningReason> {
    let n = as_f32(value)?;
    if n >= 0.0 {
        Ok(n)
    } else {
        Err(INVALID)
    }
}

/// Normaliza un nombre de valor textual: minúsculas y `_` en lugar de `-`.
fn as_name(value: &Value) -> Result<String, WarningReason> {
    match value {
        Value::Text(t) => Ok(t.trim().to_ascii_lowercase().replace('-', "_")),
        _ => Err(INVALID),
    }
}

impl Theme {
    fn parse(value: &Value) -> Result<Self, WarningReason> {
        match as_name(value)?.as_str() {
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            "system" => Ok(Self::System),
            _ => Err(INVALID),
        }
    }
}

fn parse_visual_syntax(value: &Value) -> Result<VisualSyntax, WarningReason> {
    match as_name(value)?.as_str() {
        "hidden" => Ok(VisualSyntax::Hidden),
        "reveal_current" | "revealcurrent" => Ok(VisualSyntax::RevealCurrent),
        _ => Err(INVALID),
    }
}

fn parse_heading_format(value: &Value) -> Result<HeadingNumberFormat, WarningReason> {
    match as_name(value)?.as_str() {
        "hierarchical" | "1.1.1" => Ok(HeadingNumberFormat::Hierarchical),
        _ => Err(INVALID),
    }
}

fn parse_counter_numbering(value: &Value) -> Result<NumberingScope, WarningReason> {
    match as_name(value)?.as_str() {
        "continuous" | "continua" => Ok(NumberingScope::Continuous),
        "by_chapter" | "chapter" | "por_capitulo" => Ok(NumberingScope::ByChapter),
        "by_chapter_reset" => Ok(NumberingScope::ByChapterReset),
        _ => Err(INVALID),
    }
}

impl PageSize {
    fn parse(value: &Value) -> Result<Self, WarningReason> {
        match value {
            // Con `name = custom` hacen falta `width_mm` y `height_mm` (§32.3).
            Value::Map(map) => {
                let width = map.get("width_mm").ok_or(INVALID)?;
                let height = map.get("height_mm").ok_or(INVALID)?;
                Ok(Self::Custom {
                    width_mm: as_positive_f32(width)?,
                    height_mm: as_positive_f32(height)?,
                })
            }
            _ => match as_name(value)?.as_str() {
                "a3" => Ok(Self::A3),
                "a4" => Ok(Self::A4),
                "a5" => Ok(Self::A5),
                "a6" => Ok(Self::A6),
                "letter" => Ok(Self::Letter),
                "legal" => Ok(Self::Legal),
                "executive" => Ok(Self::Executive),
                _ => Err(INVALID),
            },
        }
    }
}

fn parse_orientation(value: &Value) -> Result<Orientation, WarningReason> {
    match as_name(value)?.as_str() {
        "portrait" => Ok(Orientation::Portrait),
        "landscape" => Ok(Orientation::Landscape),
        _ => Err(INVALID),
    }
}

fn parse_page_number_position(value: &Value) -> Result<PageNumberPosition, WarningReason> {
    match as_name(value)?.as_str() {
        "header_left" => Ok(PageNumberPosition::HeaderLeft),
        "header_center" => Ok(PageNumberPosition::HeaderCenter),
        "header_right" => Ok(PageNumberPosition::HeaderRight),
        "footer_left" => Ok(PageNumberPosition::FooterLeft),
        "footer_center" => Ok(PageNumberPosition::FooterCenter),
        "footer_right" => Ok(PageNumberPosition::FooterRight),
        _ => Err(INVALID),
    }
}

fn parse_font_family(value: &Value) -> Result<FontFamily, WarningReason> {
    match as_name(value)?.as_str() {
        "sans_serif" | "sansserif" => Ok(FontFamily::SansSerif),
        "serif" => Ok(FontFamily::Serif),
        "monospace" => Ok(FontFamily::Monospace),
        _ => Err(INVALID),
    }
}

fn parse_paragraph_style(value: &Value) -> Result<ParagraphStyle, WarningReason> {
    match as_name(value)?.as_str() {
        "vertical_spacing" | "vertical" => Ok(ParagraphStyle::VerticalSpacing),
        "indented" => Ok(ParagraphStyle::Indented),
        _ => Err(INVALID),
    }
}

fn parse_text_alignment(value: &Value) -> Result<TextAlignment, WarningReason> {
    match as_name(value)?.as_str() {
        "left" => Ok(TextAlignment::Left),
        "center" => Ok(TextAlignment::Center),
        "right" => Ok(TextAlignment::Right),
        _ => Err(INVALID),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Value {
        Value::Text(s.to_string())
    }

    #[test]
    fn defaults_son_los_de_58_2() {
        let s = Settings::default();
        assert_eq!(s.preview_max_width_px, 800);
        assert!(!s.pagination_enabled);
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.visual_syntax, VisualSyntax::RevealCurrent);
        assert_eq!(s.snapshot_limit_bytes, SNAPSHOT_LIMIT_DEFAULT_BYTES);
        assert!(!s.wiki_links);
        assert!(!s.page_break_between_files);
        assert!(s.custom_colors.is_empty());
        assert!(s.custom_callout_types.is_empty());
        assert_eq!(s.bibliography_style, "IEEE");
        assert!(!s.heading_numbering);
        assert_eq!(s.heading_number_format, HeadingNumberFormat::Hierarchical);
        assert_eq!(s.counter_numbering, NumberingScope::Continuous);
        assert_eq!(s.toc_max_level, 3);
        assert_eq!(s.pdf_page_size, PageSize::A4);
        assert_eq!(s.pdf_margins_mm, Margins::uniform(25.0));
        assert_eq!(s.pdf_orientation, Orientation::Portrait);
        assert_eq!(s.pdf_page_number_position, PageNumberPosition::FooterRight);
        assert_eq!(s.pdf_header, None);
        assert_eq!(s.pdf_footer, None);
        assert_eq!(s.body_font, FontFamily::SansSerif);
        assert_eq!(s.font_size_pt, 12.0);
        assert_eq!(s.line_height, 1.3);
        assert_eq!(s.paragraph_style, ParagraphStyle::VerticalSpacing);
        assert_eq!(s.text_alignment, TextAlignment::Left);
        assert!(!s.code_line_numbers);
        assert!(!s.code_word_wrap);
        assert_eq!(s.table_min_font_pt, 8.0);
    }

    #[test]
    fn apply_reconoce_valores_textuales_insensible_a_mayusculas() {
        let mut s = Settings::default();
        s.apply("theme", &text("DaRk")).unwrap();
        assert_eq!(s.theme, Theme::Dark);

        s.apply("visual_syntax", &text("Hidden")).unwrap();
        assert_eq!(s.visual_syntax, VisualSyntax::Hidden);
        s.apply("visual_syntax", &text("reveal-current")).unwrap();
        assert_eq!(s.visual_syntax, VisualSyntax::RevealCurrent);

        s.apply("body_font", &text("Sans-Serif")).unwrap();
        assert_eq!(s.body_font, FontFamily::SansSerif);
        s.apply("body_font", &text("monospace")).unwrap();
        assert_eq!(s.body_font, FontFamily::Monospace);

        s.apply("pdf_page_number_position", &text("header-center"))
            .unwrap();
        assert_eq!(s.pdf_page_number_position, PageNumberPosition::HeaderCenter);
    }

    #[test]
    fn apply_rechaza_valores_invalidos() {
        let mut s = Settings::default();
        assert_eq!(s.apply("theme", &Value::Integer(3)), Err(INVALID));
        assert_eq!(s.apply("visual_syntax", &text("a-medias")), Err(INVALID));
        assert_eq!(s.apply("toc_max_level", &Value::Integer(0)), Err(INVALID));
        assert_eq!(s.apply("toc_max_level", &Value::Integer(7)), Err(INVALID));
        assert_eq!(s.apply("font_size_pt", &Value::Number(0.0)), Err(INVALID));
        assert_eq!(
            s.apply("preview_max_width_px", &Value::Integer(-1)),
            Err(INVALID)
        );
        // Nada cambia tras un valor inválido.
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn margenes_por_defecto_uniformes_y_configurables() {
        let mut s = Settings::default();
        s.apply("pdf_margins_mm", &Value::Number(30.5)).unwrap();
        assert_eq!(s.pdf_margins_mm, Margins::uniform(30.5));

        s.apply(
            "pdf_margins_mm",
            &Value::List(vec![
                Value::Integer(10),
                Value::Integer(20),
                Value::Integer(30),
                Value::Integer(40),
            ]),
        )
        .unwrap();
        assert_eq!(
            s.pdf_margins_mm,
            Margins {
                top_mm: 10.0,
                right_mm: 20.0,
                bottom_mm: 30.0,
                left_mm: 40.0,
            }
        );

        assert_eq!(
            s.apply("pdf_margins_mm", &Value::List(vec![Value::Integer(1)])),
            Err(INVALID)
        );
    }

    #[test]
    fn page_size_nominal_y_custom() {
        let mut s = Settings::default();
        s.apply("pdf_page_size", &text("letter")).unwrap();
        assert_eq!(s.pdf_page_size, PageSize::Letter);

        let mut dims = BTreeMap::new();
        dims.insert("width_mm".to_string(), Value::Number(120.0));
        dims.insert("height_mm".to_string(), Value::Number(240.0));
        s.apply("pdf_page_size", &Value::Map(dims)).unwrap();
        assert_eq!(
            s.pdf_page_size,
            PageSize::Custom {
                width_mm: 120.0,
                height_mm: 240.0,
            }
        );

        // "custom" sin dimensiones no es válido (§32.3).
        assert_eq!(s.apply("pdf_page_size", &text("custom")), Err(INVALID));
    }

    #[test]
    fn colecciones_de_colores_y_callouts() {
        let mut s = Settings::default();
        let mut colors = BTreeMap::new();
        colors.insert("brand".to_string(), text("#336699"));
        s.apply("custom_colors", &Value::Map(colors)).unwrap();
        assert_eq!(
            s.custom_colors.get("brand").map(String::as_str),
            Some("#336699")
        );

        s.apply(
            "custom_callout_types",
            &Value::List(vec![text("quiz"), text("exam")]),
        )
        .unwrap();
        assert_eq!(s.custom_callout_types, vec!["quiz", "exam"]);

        assert_eq!(s.apply("custom_colors", &text("brand")), Err(INVALID));
    }

    #[test]
    fn cabeceras_y_pies_vacios_son_none() {
        let mut s = Settings::default();
        s.apply("pdf_header", &text("{title} — {author}")).unwrap();
        s.apply("pdf_footer", &text("   ")).unwrap();
        assert_eq!(s.pdf_header.as_deref(), Some("{title} — {author}"));
        assert_eq!(s.pdf_footer, None);
    }
}
