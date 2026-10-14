//! Lectura local de las capas; la precedencia sigue perteneciendo al core.
use minmark_config::{parse_scalar_layer, resolve, ConfigLayer, Level, Settings, Value};
use std::path::{Path, PathBuf};

pub fn global_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("minmark/config.toml"))
}
pub fn load_at(root: Option<&Path>, source: &str, document: Option<&Path>) -> Settings {
    let mut layers = Vec::new();
    if let Some(path) = global_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut layer = parse_scalar_layer(&text, Level::Global);
            resource_paths(&mut layer, path.parent(), path.parent());
            layers.push(layer);
        }
    }
    if let Some(text) = root.and_then(|p| std::fs::read_to_string(p.join(".mdproject.toml")).ok()) {
        let mut layer = parse_scalar_layer(&text, Level::Project);
        resource_paths(&mut layer, root, root);
        layers.push(layer);
    }
    if let Some(fm) = minmark_parser::parse(source).front_matter {
        let mut layer = parse_scalar_layer(&fm, Level::Document);
        let base = document.and_then(Path::parent).or(root);
        resource_paths(&mut layer, base, root.or(base));
        layers.push(layer);
    }
    let resolved = resolve(&layers);
    for warning in resolved.warnings {
        eprintln!("minmark: ajuste {}: {:?}", warning.key, warning.reason);
    }
    resolved.settings
}

fn resource_paths(layer: &mut ConfigLayer, base: Option<&Path>, boundary: Option<&Path>) {
    fn path(value: &mut Value, base: Option<&Path>, boundary: Option<&Path>) {
        match value {
            Value::Text(text) => {
                let Some(base) = base else {
                    return;
                };
                let joined = base.join(&*text);
                let resolved = std::fs::canonicalize(&joined).unwrap_or(joined);
                if let (Ok(root), Ok(file)) = (
                    std::fs::canonicalize(boundary.unwrap_or(base)),
                    std::fs::canonicalize(&resolved),
                ) {
                    if !file.starts_with(root) {
                        *text = format!("blocked://recurso fuera del ámbito: {text}");
                        return;
                    }
                }
                *text = resolved.to_string_lossy().into_owned();
            }
            Value::List(values) => {
                for value in values {
                    path(value, base, boundary);
                }
            }
            _ => {}
        }
    }
    for key in ["bibliography", "bibliography_style"] {
        if let Some(value) = layer.entries.get_mut(key) {
            if key == "bibliography_style" && matches!(value, Value::Text(text) if text.eq_ignore_ascii_case("IEEE")) { continue; }
            path(value, base, boundary);
        }
    }
}

#[cfg(test)]
mod academic_tests {
    use super::*;
    #[test]
    fn bibliography_lists_csl_precedence_and_resource_boundaries() {
        let root = std::env::temp_dir().join(format!("minmark-config-academic-{}", std::process::id()));
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join(".mdproject.toml"), "bibliography_style = \"project.csl\"\ntoc_max_level = 6\ncounter_numbering = \"by_chapter_reset\"").unwrap();
        for file in ["project.csl", "notes/doc.csl", "notes/one.bib", "notes/two.bib", "notes/IEEE"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let document = root.join("notes/paper.md");
        let source = "---\nbibliography: [one.bib, two.bib]\nbibliography_style: doc.csl\ntoc_max_level: 2\n---\n# Documento";
        let settings = load_at(Some(&root), source, Some(&document));
        assert_eq!(settings.bibliography.len(), 2);
        assert_eq!(Path::new(&settings.bibliography[0]), root.join("notes/one.bib"));
        assert_eq!(Path::new(&settings.bibliography_style), root.join("notes/doc.csl"));
        assert_eq!(settings.toc_max_level, 2);
        assert_eq!(settings.counter_numbering, minmark_config::NumberingScope::ByChapterReset);
        let named_ieee = load_at(Some(&root), "---\nbibliography: IEEE\n---\n", Some(&document));
        assert_eq!(Path::new(&named_ieee.bibliography[0]), root.join("notes/IEEE"));
        let outside = root.with_extension("outside.bib");
        std::fs::write(&outside, "").unwrap();
        let source = format!("---\nbibliography: {}\n---\n# X", outside.display());
        assert!(load_at(Some(&root), &source, Some(&document)).bibliography[0].starts_with("blocked://"));
        std::fs::remove_file(outside).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
