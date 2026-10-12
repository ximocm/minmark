//! Configuración de minmark (§58).
//!
//! Modelo único de ajustes ([`Settings`], tabla de defaults de §58.2) y cadena
//! única de precedencia de 5 niveles (§58.1): documento → composición →
//! proyecto → global → defecto. Un solo sistema de serialización/resolución
//! para toda la aplicación (§4.2).
//!
//! Crate sin dependencias: solo `std` (§4.1).

mod settings;
mod theme;

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

    Resolved {
        settings,
        warnings,
    }
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
        let resolved = resolve(&layers(&[(
            Level::Project,
            "no_existe",
            Value::Bool(true),
        )]));
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
