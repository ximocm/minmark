//! Lectura local de las capas; la precedencia sigue perteneciendo al core.
use minmark_config::{parse_scalar_layer, resolve, Level, Settings};
use std::path::{Path, PathBuf};

pub fn global_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("minmark/config.toml"))
}
pub fn load(root: Option<&Path>, source: &str) -> Settings {
    let mut layers = Vec::new();
    if let Some(text) = global_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        layers.push(parse_scalar_layer(&text, Level::Global));
    }
    if let Some(text) = root.and_then(|p| std::fs::read_to_string(p.join(".mdproject.toml")).ok()) {
        layers.push(parse_scalar_layer(&text, Level::Project));
    }
    if let Some(fm) = minmark_parser::parse(source).front_matter {
        layers.push(parse_scalar_layer(&fm, Level::Document));
    }
    let resolved = resolve(&layers);
    for warning in resolved.warnings {
        eprintln!("minmark: ajuste {}: {:?}", warning.key, warning.reason);
    }
    resolved.settings
}
