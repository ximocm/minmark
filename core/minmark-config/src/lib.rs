//! Configuración de minmark (§58).
//!
//! Modelo único de ajustes ([`Settings`], tabla de defaults de §58.2) y cadena
//! única de precedencia de 5 niveles (§58.1): documento → composición →
//! proyecto → global → defecto. Un solo sistema de serialización/resolución
//! para toda la aplicación (§4.2).
//!
//! Crate sin dependencias: solo `std` (§4.1).

mod compositions;
mod settings;
mod theme;

pub use compositions::{parse_compositions, Composition, CompositionWarning};
pub use theme::{detect_system_theme, resolve_theme, ResolvedTheme, SystemTheme};

pub use settings::{
    FontFamily, HeadingNumberFormat, Margins, NumberingScope, Orientation, PageNumberPosition,
    PageSize, ParagraphStyle, Settings, TextAlignment, Theme, VisualSyntax, KEYS,
    SNAPSHOT_LIMIT_DEFAULT_BYTES,
};

use settings::levels_for;
use std::collections::{BTreeMap, BTreeSet};

/// Nivel de configuración (§58.1). El orden de la enumeración es la cadena de
/// precedencia: `Document` es el nivel más alto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// YAML front matter del documento (§21).
    Document,
    /// `[[composition]]` en `.mdproject.toml` (§20).
    Composition,
    /// `.mdproject.toml` (§14).
    Project,
    /// Configuración global de la aplicación.
    Global,
}

impl Level {
    /// Prioridad en la cadena de precedencia: menor = mayor prioridad.
    pub fn precedence(self) -> u8 {
        match self {
            Self::Document => 0,
            Self::Composition => 1,
            Self::Project => 2,
            Self::Global => 3,
        }
    }
}

/// Valor de configuración sin interpretar, tal como lo declara cada nivel.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

/// Ajustes declarados por un nivel de la cadena (§58.1).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLayer {
    pub level: Level,
    pub entries: BTreeMap<String, Value>,
}

impl ConfigLayer {
    pub fn new(level: Level) -> Self {
        Self {
            level,
            entries: BTreeMap::new(),
        }
    }

    /// Declara `key = value` en este nivel.
    pub fn set(&mut self, key: &str, value: Value) {
        self.entries.insert(key.to_string(), value);
    }
}

/// Motivo por el que una clave de un nivel se ignora (§58.1: se ignora con
/// aviso no bloqueante).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningReason {
    /// La clave no existe en el vocabulario de ajustes.
    UnknownKey,
    /// La clave existe pero este nivel no aplica a ella (§58.2).
    NotApplicable,
    /// El valor no es válido para el ajuste (se ignora y prevalece el nivel
    /// siguiente de la cadena).
    InvalidValue,
}

/// Aviso no bloqueante emitido al resolver la configuración.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub level: Level,
    pub key: String,
    pub reason: WarningReason,
}

/// Resultado de resolver la cadena de precedencia.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub settings: Settings,
    pub warnings: Vec<Warning>,
}

/// Resuelve la cadena única de precedencia (§58.1).
///
/// Para cada ajuste prevalece el valor del nivel aplicable más alto que lo
/// declara. Claves inexistentes o fuera del ámbito del nivel se ignoran con un
/// aviso no bloqueante por clave y nivel. Un valor inválido emite aviso y deja
/// paso al siguiente nivel de la cadena.
pub fn resolve(layers: &[ConfigLayer]) -> Resolved {
    let mut settings = Settings::default();
    let mut warnings = Vec::new();
    let mut applied: BTreeSet<&str> = BTreeSet::new();

    // De mayor a menor prioridad; dentro de cada nivel, orden determinista
    // (BTreeMap) para avisos reproducibles.
    let mut ordered: Vec<&ConfigLayer> = layers.iter().collect();
    ordered.sort_by_key(|layer| layer.level.precedence());

    for layer in ordered {
        for (key, value) in &layer.entries {
            let Some(levels) = levels_for(key) else {
                warnings.push(Warning {
                    level: layer.level,
                    key: key.clone(),
                    reason: WarningReason::UnknownKey,
                });
                continue;
            };
            if !levels.contains(&layer.level) {
                warnings.push(Warning {
                    level: layer.level,
                    key: key.clone(),
                    reason: WarningReason::NotApplicable,
                });
                continue;
            }
            if applied.contains(key.as_str()) {
                // Precedencia: ya fijado por un nivel superior. No es un error.
                continue;
            }
            match settings.apply(key, value) {
                Ok(()) => {
                    applied.insert(key.as_str());
                }
                Err(reason) => warnings.push(Warning {
                    level: layer.level,
                    key: key.clone(),
                    reason,
                }),
            }
        }
    }

    Resolved { settings, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layers(specs: &[(Level, &str, Value)]) -> Vec<ConfigLayer> {
        let mut by_level: BTreeMap<u8, ConfigLayer> = BTreeMap::new();
        for (level, key, value) in specs {
            let layer = by_level
                .entry(level.precedence())
                .or_insert_with(|| ConfigLayer::new(*level));
            layer.set(key, value.clone());
        }
        by_level.into_values().collect()
    }

    #[test]
    fn precedencia_doc_sobre_proj_sobre_global_sobre_defecto() {
        let resolved = resolve(&layers(&[
            (Level::Global, "preview_max_width_px", Value::Integer(500)),
            (Level::Project, "preview_max_width_px", Value::Integer(600)),
            (Level::Document, "preview_max_width_px", Value::Integer(700)),
        ]));
        assert_eq!(resolved.settings.preview_max_width_px, 700);
        assert!(resolved.warnings.is_empty());

        let resolved = resolve(&layers(&[
            (Level::Global, "preview_max_width_px", Value::Integer(500)),
            (Level::Project, "preview_max_width_px", Value::Integer(600)),
        ]));
        assert_eq!(resolved.settings.preview_max_width_px, 600);

        let resolved = resolve(&layers(&[(
            Level::Global,
            "preview_max_width_px",
            Value::Integer(500),
        )]));
        assert_eq!(resolved.settings.preview_max_width_px, 500);

        let resolved = resolve(&[]);
        assert_eq!(
            resolved.settings.preview_max_width_px,
            Settings::default().preview_max_width_px
        );
    }

    #[test]
    fn nivel_no_aplicable_se_ignora_con_aviso() {
        // §58.2: tema no tiene nivel composición.
        let resolved = resolve(&layers(&[
            (Level::Composition, "theme", Value::Text("dark".to_string())),
            (Level::Global, "theme", Value::Text("light".to_string())),
        ]));
        assert_eq!(resolved.settings.theme, Theme::Light);
        assert_eq!(resolved.warnings.len(), 1);
        assert_eq!(resolved.warnings[0].reason, WarningReason::NotApplicable);
        assert_eq!(resolved.warnings[0].level, Level::Composition);

        // snapshot_limit solo es global (§58.2).
        let resolved = resolve(&layers(&[(
            Level::Project,
            "snapshot_limit",
            Value::Integer(1),
        )]));
        assert_eq!(
            resolved.settings.snapshot_limit_bytes,
            SNAPSHOT_LIMIT_DEFAULT_BYTES
        );
        assert_eq!(resolved.warnings[0].reason, WarningReason::NotApplicable);
    }

    #[test]
    fn clave_desconocida_se_ignora_con_aviso() {
        let resolved = resolve(&layers(&[(Level::Project, "no_existe", Value::Bool(true))]));
        assert_eq!(resolved.settings, Settings::default());
        assert_eq!(resolved.warnings.len(), 1);
        assert_eq!(resolved.warnings[0].reason, WarningReason::UnknownKey);
    }

    #[test]
    fn valor_invalido_avisa_y_pasa_al_siguiente_nivel() {
        let resolved = resolve(&layers(&[
            (Level::Document, "theme", Value::Integer(3)),
            (Level::Global, "theme", Value::Text("dark".to_string())),
        ]));
        assert_eq!(resolved.settings.theme, Theme::Dark);
        assert_eq!(resolved.warnings.len(), 1);
        assert_eq!(resolved.warnings[0].reason, WarningReason::InvalidValue);
    }

    #[test]
    fn todas_las_claves_declaradas_son_aplicables() {
        // Guarda de coherencia entre el vocabulario de claves y la tabla de
        // niveles de §58.2: ninguna clave declarada puede caer como UnknownKey.
        let mut settings = Settings::default();
        for (key, levels) in KEYS {
            assert!(!levels.is_empty(), "{key} sin niveles");
            assert!(
                levels_for(key).is_some(),
                "{key} declarada pero no registrada en levels_for"
            );
            let result = settings.apply(key, &Value::Bool(true));
            assert_ne!(
                result,
                Err(WarningReason::UnknownKey),
                "{key} no reconocida por apply"
            );
        }
    }
}

/// Capa de ajustes escalares en el nivel raíz de TOML o front matter YAML.
/// Las tablas (cabecera que empieza por `[`) se saltan con su cuerpo entero:
/// sus claves no forman parte de la capa. Una tabla inline
/// (`pdf_page_size = { width_mm = 120, height_mm = 240 }` en TOML, la misma
/// con `:` en front matter YAML) sí se ingiere como [`Value::Map`] (§32.3:
/// tamaño Custom). Las listas salvo `bibliography` no se reinterpretan y las
/// claves ajenas a Settings no se ingeren.
pub fn parse_scalar_layer(source: &str, level: Level) -> ConfigLayer {
    let mut layer = ConfigLayer::new(level);
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        if level != Level::Document && line.trim_start().starts_with('[') {
            // Cabecera de tabla: se salta su cuerpo completo (hasta la
            // siguiente cabecera o el fin del archivo) y se sigue leyendo en
            // busca de escalares raíz; en TOML válido solo existen antes de la
            // primera cabecera, y las claves de la tabla no se ingeren.
            while let Some(rest) = lines.peek() {
                if rest.trim_start().starts_with('[') {
                    break;
                }
                lines.next();
            }
            continue;
        }
        if line.starts_with([' ', '\t']) {
            continue;
        }
        let separator = if level == Level::Document { ':' } else { '=' };
        let Some((key, raw)) = line.split_once(separator) else {
            continue;
        };
        let key = key.trim();
        if !KEYS.iter().any(|(known, _)| *known == key) {
            continue;
        }
        let raw = raw.trim();
        let value = if key == "bibliography" && raw.is_empty() && level == Level::Document {
            let mut items = Vec::new();
            while let Some(line) = lines
                .peek()
                .filter(|line| line.starts_with([' ', '\t']) && line.trim_start().starts_with("- "))
            {
                items.push(Value::Text(
                    line.trim_start()[2..]
                        .trim()
                        .trim_matches(['\'', '"'])
                        .to_owned(),
                ));
                lines.next();
            }
            Value::List(items)
        } else if key == "bibliography" && raw.starts_with('[') {
            let Some(end) = raw.rfind(']') else {
                continue;
            };
            let items = raw[1..end]
                .split(',')
                .filter(|item| !item.trim().is_empty())
                .map(|item| Value::Text(item.trim().trim_matches(['\'', '"']).to_owned()))
                .collect();
            Value::List(items)
        } else if raw.starts_with('{') {
            // Tabla inline (§32.3): `pdf_page_size = { width_mm = 120,
            // height_mm = 240 }` en TOML y `pdf_page_size: { width_mm: 120,
            // height_mm: 240 }` en front matter YAML. Una tabla mal formada
            // no se ingiere: la clave se ignora por completo (mismo criterio
            // de robustez que el resto del parser).
            let Some(value) = parse_inline_table(raw, level) else {
                continue;
            };
            value
        } else if let Some(quote) = raw.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let Some(end) = raw[1..].find(quote).map(|i| i + 1) else {
                continue;
            };
            let suffix = raw[end + 1..].trim();
            if !suffix.is_empty() && !suffix.starts_with('#') {
                continue;
            }
            Value::Text(raw[1..end].to_owned())
        } else {
            let raw = raw.split('#').next().unwrap_or("").trim();
            match raw {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => {
                    if let Ok(value) = raw.parse::<i64>() {
                        Value::Integer(value)
                    } else if let Ok(value) = raw.parse::<f64>() {
                        Value::Number(value)
                    } else if level == Level::Document {
                        Value::Text(raw.to_owned())
                    } else {
                        continue;
                    }
                }
            }
        };
        layer.set(key, value);
    }
    layer
}

/// Tabla inline `{ width_mm = 120, height_mm = 240 }` → [`Value::Map`] para
/// [`parse_scalar_layer`] (§32.3: `pdf_page_size` Custom en TOML y en front
/// matter YAML). Acepta separador de par `=` (TOML) o `:` (YAML), coma final
/// y comentario `#` (dentro o detrás de la tabla). Devuelve `None` —la
/// clave no se ingiere— si la tabla no cierra, hay cadenas sin cerrar, pares
/// sin separador, valores vacíos o colecciones anidadas (ningún ajuste las
/// admite).
fn parse_inline_table(raw: &str, level: Level) -> Option<Value> {
    // `raw` empieza por `{` (comprobado en el llamador).
    let body = &raw[1..];
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut inner_end = None;
    for (index, c) in body.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if open == '"' && c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '}' => {
                inner_end = Some(index);
                break;
            }
            _ => {}
        }
    }
    // Sin `}` de cierre o con una cadena sin cerrar antes de él.
    let inner_end = inner_end?;
    let suffix = body[inner_end + 1..].trim();
    if !suffix.is_empty() && !suffix.starts_with('#') {
        return None;
    }
    let mut map = BTreeMap::new();
    for pair in split_inline_pairs(&body[..inner_end])? {
        if pair.trim().is_empty() {
            continue; // coma final o par vacío
        }
        let (key, value) = split_inline_pair(pair)?;
        map.insert(
            inline_key(key)?.to_owned(),
            parse_inline_scalar(value, level)?,
        );
    }
    Some(Value::Map(map))
}

/// Divide el contenido de una tabla inline en pares por comas fuera de
/// cadenas; un `#` fuera de cadenas termina el contenido (comentario), como
/// en el resto del parser. `None` si queda una cadena sin cerrar.
fn split_inline_pairs(inner: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, c) in inner.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if open == '"' && c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '#' => {
                parts.push(&inner[start..index]);
                return Some(parts);
            }
            ',' => {
                parts.push(&inner[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    quote.is_none().then(|| {
        parts.push(&inner[start..]);
        parts
    })
}

/// Clave y valor de un par `clave = valor` (TOML) o `clave: valor` (YAML),
/// con el separador fuera de cadenas; `None` si no hay separador.
fn split_inline_pair(pair: &str) -> Option<(&str, &str)> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, c) in pair.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if open == '"' && c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '=' | ':' => return Some((pair[..index].trim(), pair[index + 1..].trim())),
            _ => {}
        }
    }
    None
}

/// Clave de un par: desnuda no vacía o entrecomillada completa.
fn inline_key(key: &str) -> Option<&str> {
    if let Some(quote) = key.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let rest = &key[1..];
        let end = rest.find(quote)?;
        return rest[end + 1..].is_empty().then(|| &rest[..end]);
    }
    (!key.is_empty()).then_some(key)
}

/// Escalar dentro de una tabla inline con las mismas reglas que el valor
/// raíz de su nivel: cadena entrecomillada, bool, entero, decimal o —solo en
/// YAML de documento— palabra desnuda (en TOML no existen las cadenas sin
/// comillas). Vacío o colección (`{`/`[`): tabla mal formada.
fn parse_inline_scalar(raw: &str, level: Level) -> Option<Value> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with(['{', '[']) {
        return None;
    }
    if let Some(quote) = raw.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let rest = &raw[1..];
        let end = rest.find(quote)?;
        return rest[end + 1..]
            .is_empty()
            .then(|| Value::Text(rest[..end].to_owned()));
    }
    match raw {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        _ => {}
    }
    if let Ok(value) = raw.parse::<i64>() {
        return Some(Value::Integer(value));
    }
    if let Ok(value) = raw.parse::<f64>() {
        return Some(Value::Number(value));
    }
    (level == Level::Document).then(|| Value::Text(raw.to_owned()))
}

#[cfg(test)]
mod scalar_layer_tests {
    use super::*;
    #[test]
    fn idioma_y_wiki_respetan_ambitos_y_precedencia() {
        let global = parse_scalar_layer("wiki_links = true\nlanguage = \"en\"", Level::Global);
        let project = parse_scalar_layer(
            "wiki_links = false # explícito\nlanguage = 'es'\n[[composition]]\nlanguage = 'fr'",
            Level::Project,
        );
        let document = parse_scalar_layer("language: en\nwiki_links: true", Level::Document);
        let resolved = resolve(&[document, project, global]);
        assert!(!resolved.settings.wiki_links);
        assert_eq!(resolved.settings.language, "en");
        assert_eq!(resolved.warnings[0].reason, WarningReason::NotApplicable);
        assert!(!Settings::default().wiki_links);
    }

    #[test]
    fn tabla_inline_resuelve_page_size_custom_en_los_cuatro_niveles_32_3() {
        // §32.3/§58.2: `pdf_page_size` es de los cuatro niveles y una tabla
        // inline (TOML con `=`, front matter YAML con `:`) produce
        // `Value::Map`, que Settings resuelve a Custom con sus dimensiones.
        for (source, level) in [
            (
                "pdf_page_size = { width_mm = 120, height_mm = 240 }",
                Level::Global,
            ),
            (
                "pdf_page_size = { width_mm = 120, height_mm = 240 }",
                Level::Project,
            ),
            (
                "pdf_page_size = { width_mm = 120, height_mm = 240 }",
                Level::Composition,
            ),
            (
                "pdf_page_size: { width_mm: 120, height_mm: 240 }",
                Level::Document,
            ),
        ] {
            let resolved = resolve(&[parse_scalar_layer(source, level)]);
            assert_eq!(
                resolved.settings.pdf_page_size,
                PageSize::Custom {
                    width_mm: 120.0,
                    height_mm: 240.0,
                },
                "{level:?}"
            );
            assert!(
                resolved.warnings.is_empty(),
                "{level:?}: {:?}",
                resolved.warnings
            );
        }
        // El default sigue siendo A4 (§32.3/§58.2) sin declaración.
        assert_eq!(Settings::default().pdf_page_size, PageSize::A4);
    }

    #[test]
    fn tabla_inline_con_tipos_comentario_y_coma_final_32_3() {
        // §32.3: la tabla conserva el tipo de cada valor (bool, entero,
        // decimal, cadena) con comentario `#` y coma final, como el resto
        // del parser.
        let layer = parse_scalar_layer(
            "custom_colors = { brand = \"#336699\", flag = true, n = 2, x = 1.5, } # colores\n",
            Level::Project,
        );
        assert_eq!(
            layer.entries.get("custom_colors"),
            Some(&Value::Map(BTreeMap::from([
                ("brand".to_string(), Value::Text("#336699".to_string())),
                ("flag".to_string(), Value::Bool(true)),
                ("n".to_string(), Value::Integer(2)),
                ("x".to_string(), Value::Number(1.5)),
            ])))
        );
        // §23/§58.2: Settings recibe el mapa y aplica el color (el mapa solo
        // admite cadenas, como `parse_string_map` exige).
        let resolved = resolve(&[parse_scalar_layer(
            "custom_colors = { brand = \"#336699\" }",
            Level::Project,
        )]);
        assert_eq!(
            resolved
                .settings
                .custom_colors
                .get("brand")
                .map(String::as_str),
            Some("#336699")
        );
        assert!(resolved.warnings.is_empty(), "{:?}", resolved.warnings);
    }

    #[test]
    fn tabla_inline_mal_formada_no_se_ingiere() {
        // §32.3: sintaxis no reconocida → la clave entera se ignora y el
        // resto de la capa se sigue leyendo (mismo criterio de robustez que
        // el resto del parser).
        for (source, level) in [
            (
                "pdf_page_size = { width_mm = 120, height_mm = 240",
                Level::Global,
            ),
            ("pdf_page_size = { width_mm 120 }", Level::Global),
            (
                "pdf_page_size = { width_mm = , height_mm = 240 }",
                Level::Global,
            ),
            ("pdf_page_size = { width_mm = { a = 1 } }", Level::Global),
            ("pdf_page_size = { width_mm = 120 } basura", Level::Global),
            ("pdf_page_size: { width_mm: \"120", Level::Document),
        ] {
            let layer = parse_scalar_layer(source, level);
            assert!(
                layer.entries.is_empty(),
                "ingerida en {level:?}: {source:?} → {:?}",
                layer.entries
            );
        }
        // Una línea mal formada no aborta la capa: la siguiente se ingerie.
        let layer = parse_scalar_layer(
            "pdf_page_size = { width_mm 120 }\nfont_size_pt = 12\n",
            Level::Global,
        );
        assert!(layer.entries.get("pdf_page_size").is_none());
        assert_eq!(layer.entries.get("font_size_pt"), Some(&Value::Integer(12)));
    }
}

#[cfg(test)]
mod bibliography_tests {
    use super::*;
    #[test]
    fn bibliography_accepts_scalar_arrays_and_yaml_lists_with_scope_precedence() {
        for source in [
            "bibliography: [one.bib, two.bib]",
            "bibliography:\n  - one.bib\n  - 'two.bib'",
        ] {
            let document = parse_scalar_layer(source, Level::Document);
            let resolved = resolve(&[document]);
            assert_eq!(resolved.settings.bibliography, ["one.bib", "two.bib"]);
        }
        let resolved = resolve(&[parse_scalar_layer("bibliography: []", Level::Document)]);
        assert!(resolved.settings.bibliography.is_empty());
        let mut composition = ConfigLayer::new(Level::Composition);
        composition.set("bibliography_style", Value::Text("ignored.csl".into()));
        let project = parse_scalar_layer("bibliography_style = \"project.csl\"", Level::Project);
        let document = parse_scalar_layer("bibliography_style: document.csl", Level::Document);
        let resolved = resolve(&[project, composition, document]);
        assert_eq!(resolved.settings.bibliography_style, "document.csl");
        assert!(resolved
            .warnings
            .iter()
            .any(|w| w.reason == WarningReason::NotApplicable));
    }
}
