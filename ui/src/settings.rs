//! Lectura local de las capas; la precedencia sigue perteneciendo al core.
use minmark_config::{parse_scalar_layer, resolve, ConfigLayer, Level, Settings, Value};
use std::path::{Path, PathBuf};

/// Ruta de `config.toml` global (§58.1): `minmark/config.toml` bajo el
/// directorio de configuración por SO (`minmark_config::dirs`): XDG en Linux
/// (`$XDG_CONFIG_HOME` o `~/.config`), `%APPDATA%` en Windows y
/// `~/Library/Application Support` en macOS. Sin base utilizable no hay
/// configuración global.
pub fn global_path() -> Option<PathBuf> {
    minmark_config::dirs::config_dir().map(|base| base.join("minmark").join("config.toml"))
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

/// Cadena efectiva de una composición (§58.1, ISSUE-023): global > proyecto >
/// composición (los `overrides` de su tabla `[[composition]]`) > documento
/// (front matter del primer archivo del documento lógico).
///
/// `files` son los archivos incluidos en orden de composición (§20.1) como
/// `(ruta relativa a la raíz, fuente completo)`. La bibliografía (ISSUE-024)
/// no sigue precedencia sino unión: las rutas `bibliography` del front matter
/// de cada archivo incluido (en orden, resueltas con [`resource_paths`]
/// relativas al directorio de SU archivo y con los límites §48 existentes)
/// más las de proyecto y global. `bibliography_style` conserva sus niveles
/// actuales: no tiene nivel composición (§58.2). Los avisos siguen siendo no
/// bloqueantes (§58.1).
pub fn load_composition_at(
    root: &Path,
    composition: &minmark_config::Composition,
    files: &[(PathBuf, String)],
) -> Settings {
    let mut layers = Vec::new();
    if let Some(path) = global_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut layer = parse_scalar_layer(&text, Level::Global);
            resource_paths(&mut layer, path.parent(), path.parent());
            layers.push(layer);
        }
    }
    if let Ok(text) = std::fs::read_to_string(root.join(".mdproject.toml")) {
        let mut layer = parse_scalar_layer(&text, Level::Project);
        resource_paths(&mut layer, Some(root), Some(root));
        layers.push(layer);
    }
    layers.push(composition.overrides.clone());
    if let Some((rel, source)) = files.first() {
        if let Some(fm) = minmark_parser::parse(source).front_matter {
            let mut layer = parse_scalar_layer(&fm, Level::Document);
            let base = root.join(rel);
            resource_paths(&mut layer, base.parent(), Some(root));
            layers.push(layer);
        }
    }

    // ISSUE-024: unión de bibliografías — front matter de cada archivo en
    // orden de composición, luego proyecto y global (bibliography no tiene
    // nivel composición, §58.2; el front matter de los archivos i>0 no es
    // capa de la cadena pero su bibliografía sí se une).
    let mut union: Vec<String> = Vec::new();
    for (rel, source) in files {
        let Some(fm) = minmark_parser::parse(source).front_matter else {
            continue;
        };
        let mut layer = parse_scalar_layer(&fm, Level::Document);
        let base = root.join(rel);
        resource_paths(&mut layer, base.parent(), Some(root));
        if let Some(value) = layer.entries.get("bibliography") {
            push_bibliography(&mut union, value);
        }
    }
    for level in [Level::Project, Level::Global] {
        if let Some(layer) = layers.iter().find(|layer| layer.level == level) {
            if let Some(value) = layer.entries.get("bibliography") {
                push_bibliography(&mut union, value);
            }
        }
    }

    let resolved = resolve(&layers);
    for warning in resolved.warnings {
        eprintln!("minmark: ajuste {}: {:?}", warning.key, warning.reason);
    }
    let mut settings = resolved.settings;
    let mut vistas = std::collections::HashSet::new();
    union.retain(|path| vistas.insert(path.clone()));
    settings.bibliography = union;
    settings
}

/// Acumula los elementos de una declaración `bibliography` ya resuelta por
/// [`resource_paths`] (cadena o lista; el resto no es una ruta válida).
fn push_bibliography(out: &mut Vec<String>, value: &Value) {
    match value {
        Value::Text(text) => {
            if !text.trim().is_empty() {
                out.push(text.clone());
            }
        }
        Value::List(values) => {
            for value in values {
                if let Value::Text(text) = value {
                    if !text.trim().is_empty() {
                        out.push(text.clone());
                    }
                }
            }
        }
        _ => {}
    }
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
            if key == "bibliography_style"
                && matches!(value, Value::Text(text) if text.eq_ignore_ascii_case("IEEE"))
            {
                continue;
            }
            path(value, base, boundary);
        }
    }
}

#[cfg(test)]
mod academic_tests {
    use super::*;
    #[test]
    fn bibliography_lists_csl_precedence_and_resource_boundaries() {
        let root =
            std::env::temp_dir().join(format!("minmark-config-academic-{}", std::process::id()));
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join(".mdproject.toml"), "bibliography_style = \"project.csl\"\ntoc_max_level = 6\ncounter_numbering = \"by_chapter_reset\"").unwrap();
        for file in [
            "project.csl",
            "notes/doc.csl",
            "notes/one.bib",
            "notes/two.bib",
            "notes/IEEE",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let document = root.join("notes/paper.md");
        let source = "---\nbibliography: [one.bib, two.bib]\nbibliography_style: doc.csl\ntoc_max_level: 2\n---\n# Documento";
        let settings = load_at(Some(&root), source, Some(&document));
        assert_eq!(settings.bibliography.len(), 2);
        assert_eq!(
            Path::new(&settings.bibliography[0]),
            root.join("notes/one.bib")
        );
        assert_eq!(
            Path::new(&settings.bibliography_style),
            root.join("notes/doc.csl")
        );
        assert_eq!(settings.toc_max_level, 2);
        assert_eq!(
            settings.counter_numbering,
            minmark_config::NumberingScope::ByChapterReset
        );
        let named_ieee = load_at(
            Some(&root),
            "---\nbibliography: IEEE\n---\n",
            Some(&document),
        );
        assert_eq!(
            Path::new(&named_ieee.bibliography[0]),
            root.join("notes/IEEE")
        );
        let outside = root.with_extension("outside.bib");
        std::fs::write(&outside, "").unwrap();
        let source = format!("---\nbibliography: {}\n---\n# X", outside.display());
        assert!(
            load_at(Some(&root), &source, Some(&document)).bibliography[0]
                .starts_with("blocked://")
        );
        std::fs::remove_file(outside).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod composition_chain_tests {
    use super::*;
    use minmark_config::{parse_compositions, Orientation};

    /// §58.1 + ISSUE-023/024: cadena documento > composición > proyecto >
    /// global/defecto sobre los ajustes, unión de bibliografías de los dos
    /// archivos con sus límites §48, y `bibliography_style` sin nivel
    /// composición (§58.2).
    #[test]
    fn cadena_de_composicion_y_union_de_bibliografias() {
        let root =
            std::env::temp_dir().join(format!("minmark-composition-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("caps")).unwrap();
        std::fs::create_dir_all(root.join("notas")).unwrap();
        let project = "\
pdf_orientation = \"portrait\"
pdf_page_size = \"Letter\"
toc_max_level = 4
bibliography = [\"proj.bib\"]
bibliography_style = \"project.csl\"
[[composition]]
name = \"Libro\"
files = [\"caps/uno.md\", \"notas/dos.md\"]
toc_max_level = 3
pdf_orientation = \"landscape\"
bibliography_style = \"comp.csl\"
";
        std::fs::write(root.join(".mdproject.toml"), project).unwrap();
        let (compositions, warnings) = parse_compositions(project);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(compositions.len(), 1);
        let composition = &compositions[0];

        let outside = root.with_extension("outside.bib");
        std::fs::write(&outside, "").unwrap();
        let files = [
            (
                PathBuf::from("caps/uno.md"),
                "---\ntoc_max_level: 1\nbibliography: [one.bib]\n---\n# Uno".to_string(),
            ),
            (
                PathBuf::from("notas/dos.md"),
                format!(
                    "---\nbibliography: [two.bib, {}]\n---\n# Dos",
                    outside.display()
                ),
            ),
        ];
        let settings = load_composition_at(&root, composition, &files);

        // documento > composición > proyecto > defecto (§58.1).
        assert_eq!(settings.toc_max_level, 1, "documento sobre composición");
        assert_eq!(
            settings.pdf_orientation,
            Orientation::Landscape,
            "composición sobre proyecto"
        );
        assert_eq!(
            settings.pdf_page_size,
            minmark_config::PageSize::Letter,
            "proyecto sobre defecto/global"
        );
        // §58.2: `bibliography_style` no tiene nivel composición, la capa de
        // composición se ignora con aviso y gana la de proyecto.
        assert_eq!(
            Path::new(&settings.bibliography_style),
            root.join("project.csl")
        );
        // ISSUE-024: unión en orden — front matter de cada archivo (resuelto
        // relativo al dir de su archivo), luego proyecto; el recurso fuera
        // del límite §48 queda bloqueado en su posición.
        assert_eq!(
            settings.bibliography.len(),
            4,
            "{:?}",
            settings.bibliography
        );
        assert_eq!(
            Path::new(&settings.bibliography[0]),
            root.join("caps/one.bib")
        );
        assert_eq!(
            Path::new(&settings.bibliography[1]),
            root.join("notas/two.bib")
        );
        assert!(
            settings.bibliography[2].starts_with("blocked://"),
            "{}",
            settings.bibliography[2]
        );
        assert_eq!(Path::new(&settings.bibliography[3]), root.join("proj.bib"));

        // Sin front matter en el primer archivo, la composición prevalece
        // sobre proyecto para el resto de la cadena.
        let sin_documento = [(PathBuf::from("caps/uno.md"), "# Uno".to_string())];
        let settings = load_composition_at(&root, composition, &sin_documento);
        assert_eq!(settings.toc_max_level, 3, "composición sobre proyecto");

        std::fs::remove_file(outside).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
