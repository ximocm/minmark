//! Resolución y detección del tema del sistema (§11).
//!
//! Temas disponibles: Light, Dark, System (default §58.2). `System` se
//! resuelve contra la preferencia del entorno (XDG/GTK/KDE), con fallback a
//! Light si no se puede determinar.

use crate::Theme;
use std::env;
use std::fs;
use std::process::Command;

/// Preferencia de tema del entorno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTheme {
    Light,
    Dark,
    /// No determinable: se usa el fallback de [`resolve_theme`].
    Unknown,
}

/// Tema efectivamente aplicado (§11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

/// Resuelve el tema configurado contra la preferencia del sistema.
/// `System` con preferencia desconocida cae en Light (comportamiento
/// determinista; la UI puede sobreescribirlo con la preferencia real).
pub fn resolve_theme(theme: Theme, system: SystemTheme) -> ResolvedTheme {
    match theme {
        Theme::Light => ResolvedTheme::Light,
        Theme::Dark => ResolvedTheme::Dark,
        Theme::System => match system {
            SystemTheme::Dark => ResolvedTheme::Dark,
            SystemTheme::Light | SystemTheme::Unknown => ResolvedTheme::Light,
        },
    }
}

/// Detecta la preferencia de tema del entorno (§11: XDG/GTK settings).
///
/// Orden: variables de GTK (la UI es GTK4, ADR-001), GNOME/gsettings
/// (`color-scheme`, `gtk-theme`) y KDE (`kdeglobals`). Sin dependencias:
/// `std::process::Command` y lectura de ficheros de configuración.
pub fn detect_system_theme() -> SystemTheme {
    if let Ok(value) = env::var("GTK_APPLICATION_PREFER_DARK_THEME") {
        if matches!(value.as_str(), "1" | "true" | "yes") {
            return SystemTheme::Dark;
        }
    }
    if let Ok(value) = env::var("GTK_THEME") {
        if let Some(theme) = parse_gtk_theme(&value) {
            return theme;
        }
    }
    if let Some(theme) = gsettings_theme() {
        return theme;
    }
    if let Some(theme) = kde_theme() {
        return theme;
    }
    SystemTheme::Unknown
}

fn gsettings_theme() -> Option<SystemTheme> {
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
        .ok()?;
    if output.status.success() {
        if let Some(theme) = parse_gsettings(&String::from_utf8_lossy(&output.stdout)) {
            return Some(theme);
        }
    }
    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
        .output()
        .ok()?;
    if output.status.success() {
        return parse_gsettings(&String::from_utf8_lossy(&output.stdout));
    }
    None
}

fn kde_theme() -> Option<SystemTheme> {
    // Directorio de configuración centralizado por SO (§45.1): XDG en Linux,
    // %APPDATA% en Windows, ~/Library/Application Support en macOS.
    let config_dir = crate::dirs::config_dir()?;
    let content = fs::read_to_string(config_dir.join("kdeglobals")).ok()?;
    parse_kdeglobals(&content)
}

/// `gsettings get` devuelve la preferencia entre comillas simples:
/// `'prefer-dark'`, `'prefer-light'`, `'default'` (color-scheme) o el nombre
/// del tema GTK (`'Adwaita-dark'`).
fn parse_gsettings(raw: &str) -> Option<SystemTheme> {
    let value = raw.trim().trim_matches('\'').to_ascii_lowercase();
    match value.as_str() {
        "prefer-dark" => Some(SystemTheme::Dark),
        "prefer-light" | "default" => Some(SystemTheme::Light),
        _ if value.ends_with("-dark") => Some(SystemTheme::Dark),
        _ if value.contains("dark") => Some(SystemTheme::Dark),
        _ if !value.is_empty() && value != "none" => Some(SystemTheme::Light),
        _ => None,
    }
}

/// `GTK_THEME=Adwaita:dark` o `GTK_THEME=Adwaita-dark`.
fn parse_gtk_theme(value: &str) -> Option<SystemTheme> {
    let value = value.to_ascii_lowercase();
    if let Some((name, variant)) = value.split_once(':') {
        return match variant.trim() {
            "dark" => Some(SystemTheme::Dark),
            "light" => Some(SystemTheme::Light),
            _ => {
                if name.ends_with("-dark") {
                    Some(SystemTheme::Dark)
                } else {
                    Some(SystemTheme::Light)
                }
            }
        };
    }
    if value.ends_with("-dark") {
        Some(SystemTheme::Dark)
    } else if value.is_empty() {
        None
    } else {
        Some(SystemTheme::Light)
    }
}

/// KDE: `ColorScheme=BreezeDark` en `[General]` de `kdeglobals`.
fn parse_kdeglobals(content: &str) -> Option<SystemTheme> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("ColorScheme=") {
            let value = value.trim().to_ascii_lowercase();
            return Some(if value.contains("dark") {
                SystemTheme::Dark
            } else {
                SystemTheme::Light
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolucion_light_dark_y_system() {
        assert_eq!(
            resolve_theme(Theme::Light, SystemTheme::Dark),
            ResolvedTheme::Light
        );
        assert_eq!(
            resolve_theme(Theme::Dark, SystemTheme::Light),
            ResolvedTheme::Dark
        );
        assert_eq!(
            resolve_theme(Theme::System, SystemTheme::Dark),
            ResolvedTheme::Dark
        );
        assert_eq!(
            resolve_theme(Theme::System, SystemTheme::Light),
            ResolvedTheme::Light
        );
        // Sin preferencia determinable: fallback Light.
        assert_eq!(
            resolve_theme(Theme::System, SystemTheme::Unknown),
            ResolvedTheme::Light
        );
    }

    #[test]
    fn deteccion_gsettings() {
        assert_eq!(parse_gsettings("'prefer-dark'"), Some(SystemTheme::Dark));
        assert_eq!(parse_gsettings("'prefer-light'"), Some(SystemTheme::Light));
        assert_eq!(parse_gsettings("'default'"), Some(SystemTheme::Light));
        assert_eq!(parse_gsettings("'Adwaita-dark'"), Some(SystemTheme::Dark));
        assert_eq!(parse_gsettings("'Adwaita'"), Some(SystemTheme::Light));
        assert_eq!(parse_gsettings("''"), None);
    }

    #[test]
    fn deteccion_gtk_theme() {
        assert_eq!(parse_gtk_theme("Adwaita:dark"), Some(SystemTheme::Dark));
        assert_eq!(parse_gtk_theme("Adwaita:light"), Some(SystemTheme::Light));
        assert_eq!(parse_gtk_theme("Adwaita-dark"), Some(SystemTheme::Dark));
        assert_eq!(parse_gtk_theme("Adwaita"), Some(SystemTheme::Light));
        assert_eq!(parse_gtk_theme(""), None);
    }

    #[test]
    fn deteccion_kde() {
        let content = "[General]\nColorScheme=BreezeDark\n";
        assert_eq!(parse_kdeglobals(content), Some(SystemTheme::Dark));
        let content = "[General]\nColorScheme=Breeze\n";
        assert_eq!(parse_kdeglobals(content), Some(SystemTheme::Light));
        assert_eq!(parse_kdeglobals("[General]\n"), None);
    }

    #[test]
    fn tema_configurable_en_los_tres_niveles() {
        // §58.1/§58.2: tema aplica a doc, proj y global.
        use crate::{resolve, ConfigLayer, Level, Value};

        let mut doc = ConfigLayer::new(Level::Document);
        doc.set("theme", Value::Text("dark".to_string()));
        let resolved = resolve(&[doc]);
        assert_eq!(resolved.settings.theme, Theme::Dark);

        let mut proj = ConfigLayer::new(Level::Project);
        proj.set("theme", Value::Text("dark".to_string()));
        let resolved = resolve(&[proj]);
        assert_eq!(resolved.settings.theme, Theme::Dark);

        let mut global = ConfigLayer::new(Level::Global);
        global.set("theme", Value::Text("light".to_string()));
        let resolved = resolve(&[global]);
        assert_eq!(resolved.settings.theme, Theme::Light);
    }
}
