//! Recursos de GTK incluidos en los paquetes portátiles de Windows.

#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};

#[cfg(any(windows, test))]
fn bundled_schemas(executable: &Path) -> Option<PathBuf> {
    let schemas = executable.parent()?.join("share/glib-2.0/schemas");
    schemas
        .join("gschemas.compiled")
        .is_file()
        .then_some(schemas)
}

/// Antes de inicializar GTK/GIO: las DLL del ZIP/MSI no viven en el prefijo
/// de gvsbuild y GIO no puede descubrir sus schemas por sí solo.
pub fn configure() {
    #[cfg(windows)]
    if let Some(schemas) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(bundled_schemas)
    {
        std::env::set_var("GSETTINGS_SCHEMA_DIR", schemas);
    }
}

/// GSettings aborta el proceso si GTK intenta crear un selector sin schema.
/// Comprobarlo permite explicar un paquete incompleto sin perder el buffer.
pub fn check_file_dialog() -> Result<(), String> {
    let available = gtk4::gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gtk.gtk4.Settings.FileChooser", true))
        .is_some();
    if available {
        Ok(())
    } else {
        Err("Faltan los recursos de configuración de GTK para abrir el selector de archivos. Reinstala el paquete completo de minmark; tu documento sigue abierto y puedes copiar su contenido.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_se_resuelven_junto_al_ejecutable_sin_depender_del_directorio_actual() {
        let root = std::env::temp_dir().join(format!("minmark-runtime-{}", std::process::id()));
        let package = root.join("paquete con espacios y acentos á");
        let schemas = package.join("share/glib-2.0/schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        let executable = package.join("minmark.exe");
        assert_eq!(bundled_schemas(&executable), None);
        std::fs::write(schemas.join("gschemas.compiled"), b"fixture").unwrap();
        assert_eq!(bundled_schemas(&executable), Some(schemas));
        std::fs::remove_dir_all(root).unwrap();
    }
}
