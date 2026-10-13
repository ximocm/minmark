//! Operaciones de archivo del proyecto (ISSUE-023): §14.8, §14.9 y §16.
//!
//! - §14.8: crear Markdown, crear carpeta, renombrar, mover, duplicar;
//!   `copy_relative_path` produce la ruta relativa §17 para la UI.
//! - §14.9: `trash` envía a la **papelera del sistema** (implementación propia
//!   de la especificación XDG trash, std-only §4.1 — decisión §4.4 registrada
//!   en ISSUE-023); nunca borrado permanente.
//! - §16: [`plan_path_change`] analiza todas las referencias conocidas
//!   afectadas (enlaces e imágenes Markdown, wiki links activos, rutas
//!   `.bib`/`.csl`, composiciones y `.mdproject.toml`) **sin tocar nada**;
//!   [`apply_path_change`] aplica todo o revierte todo (rollback). Si alguna
//!   referencia conocida no se puede actualizar con garantías, el plan falla
//!   y no hay efectos: nunca una operación parcial.
//! - §14.6: los archivos ignorados por `.gitignore` de la raíz quedan fuera
//!   del análisis de enlaces.
//! - §48: toda ruta interna pasa por [`resolve_in_project`].
//!
//! Las referencias se localizan con [`minmark_document`] (texto puro); las
//! rutas relativas se reescriben según §17 (desde el archivo que contiene la
//! referencia), preservando forma (`<…>`), título, fragmento `#…` y estilo
//! de cita en TOML/YAML.

use crate::ignore::IgnorePatterns;
use crate::paths::{resolve_in_project, walk_tree};
use crate::resolve::{has_scheme, split_fragment, wiki_name_matches};
use crate::{write_atomic, FsError};
use minmark_document::{
    apply_replacements, find_markdown_references, find_toml_references, DestForm, RefKind,
    Reference, Span,
};
use std::path::{Component, Path, PathBuf};

/// Extensiones que cuentan como «ruta documental» en valores de
/// configuración (§14.4, §25.1): solo ellas y los valores con `/` se
/// reescriben como rutas.
const PATH_EXTENSIONS: &[&str] = &[
    "md", "markdown", "png", "jpg", "jpeg", "svg", "webp", "gif", "bib", "csl", "toml",
];

/// Opciones de un rename/movimiento §16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChangeOptions {
    /// Wiki links activos (§19.3, desactivados por defecto): solo cuando lo
    /// están son «wiki links activos» y se actualizan (§16).
    pub wiki_links: bool,
}

/// Plan validado de rename/movimiento §16: pendiente de aplicar.
#[derive(Debug, Clone)]
pub struct PathChange {
    from: PathBuf,
    to: PathBuf,
    /// Contenidos nuevos por archivo de referencia (ya reescrito).
    updates: Vec<(PathBuf, String)>,
}

impl PathChange {
    pub fn from(&self) -> &Path {
        &self.from
    }

    pub fn to(&self) -> &Path {
        &self.to
    }

    /// Archivos de referencia que serán reescritos (para diagnóstico §3.7).
    pub fn updated_files(&self) -> impl Iterator<Item = &Path> {
        self.updates.iter().map(|(path, _)| path.as_path())
    }
}

// --- Operaciones §14.8 -------------------------------------------------------

/// Crea un Markdown nuevo `parent/name` (§14.8). `name` debe ser un nombre
/// simple; si ya existe, [`FsError::AlreadyExists`] (§3.7: sin sobrescritura).
pub fn create_markdown(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, FsError> {
    create_entry(root, parent, name, true)
}

/// Crea una carpeta nueva `parent/name` (§14.8).
pub fn create_folder(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, FsError> {
    create_entry(root, parent, name, false)
}

fn create_entry(root: &Path, parent: &Path, name: &str, file: bool) -> Result<PathBuf, FsError> {
    let path = checked_target(root, parent, name)?;
    if file {
        std::fs::write(&path, b"").map_err(|source| FsError::Io { path: path.clone(), source })?;
    } else {
        std::fs::create_dir(&path).map_err(|source| FsError::Io { path: path.clone(), source })?;
    }
    Ok(path)
}

/// Duplica `path` (archivo, carpeta o symlink) con nombre numerado
/// `base-2.ext`, `base-3.ext`, … (§18.3): nunca sobrescribe (§3.7).
pub fn duplicate(root: &Path, path: &Path) -> Result<PathBuf, FsError> {
    let path = resolve_in_project(root, path)?;
    if !path.exists() && !path.is_symlink() {
        return Err(FsError::Io {
            path: path.clone(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no existe"),
        });
    }
    let parent = path.parent().unwrap_or(root).to_path_buf();
    let name = file_name_str(&path);
    let (stem, ext) = split_name(&name);
    let target = numbered_path(&parent, &stem, ext.as_deref());
    copy_entry(&path, &target)?;
    Ok(target)
}

/// Renombra/mueve `from` → `to` **sin** actualizar referencias: solo para
/// internos ya validados. La UI debe usar [`plan_path_change`] +
/// [`apply_path_change`] (§16). Sin sobrescritura (§3.7) y todo §48.
pub fn move_entry(root: &Path, from: &Path, to: &Path) -> Result<(), FsError> {
    let from = resolve_in_project(root, from)?;
    let to = resolve_in_project(root, to)?;
    if !from.exists() {
        return Err(FsError::Io {
            path: from.clone(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no existe"),
        });
    }
    if to.exists() {
        return Err(FsError::AlreadyExists { path: to });
    }
    if to.starts_with(&from) {
        return Err(FsError::InvalidOperation {
            reason: "no se puede mover una carpeta dentro de sí misma",
        });
    }
    std::fs::rename(&from, &to).map_err(|source| FsError::Io { path: from, source })
}

/// «Copiar ruta relativa» §14.8/§17: ruta relativa posix de `target`
/// respecto a `from_dir` (normalmente la raíz del proyecto).
pub fn relative_path(from_dir: &Path, target: &Path) -> String {
    let from: Vec<Component> = from_dir.components().collect();
    let to: Vec<Component> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = Vec::new();
    for _ in common..from.len() {
        parts.push("..".to_owned());
    }
    for component in &to[common..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

// --- Papelera §14.9 ----------------------------------------------------------

/// Import Markdown (§15, ISSUE-024): **copia** un `.md` externo dentro del
/// proyecto (el original queda intacto) en `dest_dir` y devuelve la copia.
///
/// - el origen debe ser un Markdown externo a la raíz (§15: lo externo no
///   pertenece al proyecto hasta importarse);
/// - `dest_dir` se valida contra la raíz (§48);
/// - colisión de nombres → renombrado `base-2.ext` (§18.3), nunca
///   sobrescritura (§3.7).
pub fn import_markdown(root: &Path, src: &Path, dest_dir: &Path) -> Result<PathBuf, FsError> {
    let src = std::fs::canonicalize(src).map_err(|source| FsError::Io {
        path: src.to_path_buf(),
        source,
    })?;
    let is_markdown = src
        .extension()
        .is_some_and(|ext| ext == "md" || ext == "markdown");
    if !is_markdown {
        return Err(FsError::InvalidOperation {
            reason: "Import Markdown solo admite archivos .md/.markdown (§15)",
        });
    }
    let root = std::fs::canonicalize(root).map_err(|source| FsError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    if src.starts_with(&root) {
        return Err(FsError::InvalidOperation {
            reason: "el archivo ya pertenece al proyecto (§15: Import es para externos)",
        });
    }
    let dest_dir = resolve_in_project(&root, dest_dir)?;
    let name = file_name_str(&src);
    let (stem, ext) = split_name(&name);
    let target = numbered_path(&dest_dir, &stem, ext.as_deref());
    std::fs::copy(&src, &target).map_err(|source| FsError::Io { path: src, source })?;
    Ok(target)
}

/// Envía `path` (archivo o carpeta) a la papelera del sistema (§14.9).
///
/// Implementación de la especificación XDG trash: `$XDG_DATA_HOME/Trash/`
/// con `files/<nombre>` y `info/<nombre>.trashinfo` (`Path` exacto y
/// `DeletionDate`). `DeletionDate` va en UTC: sin dependencias de fecha (§4.1)
/// no hay conversión de zona local; el valor es informativo y `Path` —lo que
/// permite restaurar— se preserva exacto (decisión registrada en ISSUE-023).
pub fn trash(path: &Path) -> Result<(), FsError> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        .ok_or_else(|| FsError::InvalidOperation {
            reason: "sin HOME/XDG_DATA_HOME no hay papelera disponible",
        })?;
    trash_to(&base.join("Trash"), path)
}

/// [`trash`] sobre una raíz de papelera explícita (tests; la XDG real es
/// `$XDG_DATA_HOME/Trash`).
pub fn trash_to(trash_root: &Path, path: &Path) -> Result<(), FsError> {
    let name = file_name_str(path);
    let (stem, ext) = split_name(&name);
    std::fs::create_dir_all(trash_root.join("files"))
        .and_then(|()| std::fs::create_dir_all(trash_root.join("info")))
        .map_err(|source| FsError::Io { path: trash_root.to_path_buf(), source })?;
    let target = numbered_path(&trash_root.join("files"), &stem, ext.as_deref());
    let trash_name = file_name_str(&target);

    let absolute = std::fs::canonicalize(path).map_err(|source| FsError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let info = trash_root.join("info").join(format!("{trash_name}.trashinfo"));
    let info_body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(&absolute.to_string_lossy()),
        utc_timestamp(),
    );
    write_atomic(&info, info_body.as_bytes()).map_err(|source| FsError::Io {
        path: info.clone(),
        source,
    })?;

    if let Err(source) = move_entry_raw(path, &target) {
        let _ = std::fs::remove_file(&info);
        return Err(FsError::Io { path: path.to_path_buf(), source });
    }
    Ok(())
}

/// `fs::rename` con respaldo copia-borra para cruces de dispositivo.
fn move_entry_raw(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_entry_io(from, to)?;
            remove_entry_io(from)
        }
        Err(source) => Err(source),
    }
}

fn copy_entry_io(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from)?;
        return std::os::unix::fs::symlink(target, to);
    }
    if meta.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_entry_io(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

fn remove_entry_io(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn copy_entry(from: &Path, to: &Path) -> Result<(), FsError> {
    copy_entry_io(from, to).map_err(|source| FsError::Io { path: from.to_path_buf(), source })
}

/// Percent-encoding de la ruta original para `Trash/info` (XDG).
fn percent_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Marca de tiempo UTC `YYYY-MM-DDTHH:MM:SS` para `DeletionDate`.
fn utc_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let time = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}", time / 3600, (time % 3600) / 60, time % 60)
}

/// Días desde 1970-01-01 → fecha civil (algoritmo de Howard Hinnant).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

// --- Rename/movimiento inteligente §16 ---------------------------------------

/// Analiza y valida un rename/movimiento dentro del proyecto (§16): calcula
/// la reescritura de **todas** las referencias conocidas afectadas o falla
/// con [`FsError::UnsafeReference`] sin efectos (all-or-nothing).
pub fn plan_path_change(
    root: &Path,
    from: &Path,
    to: &Path,
    options: ChangeOptions,
) -> Result<PathChange, FsError> {
    let root = std::fs::canonicalize(root).map_err(|source| FsError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let from = resolve_in_project(&root, from)?;
    let to = resolve_in_project(&root, to)?;
    if !from.exists() {
        return Err(FsError::Io {
            path: from.clone(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no existe"),
        });
    }
    if to.exists() {
        return Err(FsError::AlreadyExists { path: to.clone() });
    }
    if to.starts_with(&from) {
        return Err(FsError::InvalidOperation {
            reason: "no se puede mover una carpeta dentro de sí misma",
        });
    }

    let ignore = IgnorePatterns::from_root(&root);
    let scan = scan_reference_files(&root, &ignore)?;
    let moved_files: Vec<PathBuf> = scan
        .iter()
        .filter(|path| *path == &from || path.starts_with(&from))
        .cloned()
        .collect();

    let mut updates = Vec::new();
    for file in &scan {
        let source = std::fs::read_to_string(file).map_err(|source| FsError::Io {
            path: file.clone(),
            source,
        })?;
        let is_toml = file.file_name().is_some_and(|n| n == ".mdproject.toml");
        let refs = if is_toml {
            find_toml_references(&source)
        } else {
            find_markdown_references(&source)
        };
        let file_moved = moved_files.iter().any(|m| m == file);
        let file_dir = file.parent().unwrap_or(&root).to_path_buf();
        // Solo las carpetas movidas cambian de directorio los archivos que
        // contienen; al renombrar un archivo su base §17 no cambia.
        let new_file_dir = if from.is_dir() && file_dir.starts_with(&from) {
            to.join(relative_path(&from, &file_dir))
        } else {
            file_dir.clone()
        };

        let mut edits: Vec<(Span, String)> = Vec::new();
        for reference in &refs {
            if let Some(edit) = plan_reference(
                reference,
                file,
                &file_dir,
                &new_file_dir,
                &root,
                &from,
                &to,
                file_moved,
                options,
                &scan,
            )? {
                if edit.1 != source[edit.0.start as usize..edit.0.end as usize] {
                    if reference.encoded {
                        return Err(FsError::UnsafeReference {
                            path: file.clone(),
                            dest: reference.dest.clone(),
                        });
                    }
                    if !form_is_safe(reference.form, &edit.1, reference.quote) {
                        return Err(FsError::UnsafeReference {
                            path: file.clone(),
                            dest: reference.dest.clone(),
                        });
                    }
                    edits.push(edit);
                }
            }
        }
        if !edits.is_empty() {
            updates.push((file.clone(), apply_replacements(&source, &edits)));
        }
    }
    Ok(PathChange { from, to, updates })
}

/// Aplica un plan §16: primero las reescrituras (atómicas por archivo), luego
/// el movimiento. Ante **cualquier** fallo se restaura todo lo ya hecho y se
/// propaga el error: nunca una operación parcial.
pub fn apply_path_change(change: &PathChange) -> Result<(), FsError> {
    let mut journal: Vec<(PathBuf, Option<String>)> = Vec::new();
    for (path, content) in &change.updates {
        let original = std::fs::read_to_string(path).map_err(|source| FsError::Io {
            path: path.clone(),
            source,
        })?;
        if let Err(source) = write_atomic(path, content.as_bytes()) {
            rollback(&journal);
            return Err(FsError::Io { path: path.clone(), source });
        }
        journal.push((path.clone(), Some(original)));
    }
    if let Err(source) = std::fs::rename(&change.from, &change.to) {
        rollback(&journal);
        return Err(FsError::Io { path: change.from.clone(), source });
    }
    Ok(())
}

fn rollback(journal: &[(PathBuf, Option<String>)]) {
    for (path, original) in journal {
        if let Some(original) = original {
            let _ = write_atomic(path, original.as_bytes());
        }
    }
}

/// Decide la reescritura de una referencia (`None` = sin cambios).
///
/// Reglas: destino con esquema (`https:`, `mailto:`…) o solo ancla `#…` nunca
/// es ruta. Los enlaces resuelven desde su archivo (§17); los valores de
/// configuración YAML desde su archivo y los TOML desde la raíz del proyecto.
/// Se reescriben (a) las referencias cuyo destino resuelve a la entrada
/// movida o a un descendiente y (b) todas las rutas de archivos que se
/// mueven (para preservar su destino §17). Wiki links (§19.3) solo si están
/// activos.
#[allow(clippy::too_many_arguments)]
fn plan_reference(
    reference: &Reference,
    file: &Path,
    file_dir: &Path,
    new_file_dir: &Path,
    root: &Path,
    from: &Path,
    to: &Path,
    file_moved: bool,
    options: ChangeOptions,
    scan: &[PathBuf],
) -> Result<Option<(Span, String)>, FsError> {
    if reference.kind == RefKind::WikiLink {
        return plan_wiki_reference(reference, root, from, to, options, scan);
    }

    let (path_part, suffix) = split_fragment(&reference.dest);
    if path_part.is_empty() || has_scheme(path_part) {
        return Ok(None);
    }
    let base = if reference.kind == RefKind::ConfigPath && file_name_str(file) == ".mdproject.toml" {
        root
    } else {
        file_dir
    };
    let resolved = if Path::new(path_part).is_absolute() {
        normalize(Path::new(path_part))
    } else {
        normalize(&base.join(path_part))
    };
    let affected = resolved == *from || resolved.starts_with(from);
    let recompute = file_moved && is_pathish(path_part);
    if !affected && !recompute {
        return Ok(None);
    }
    let target = if affected {
        to.join(relative_path(from, &resolved))
    } else {
        resolved
    };
    let new_part = relative_path(new_file_dir, &target);
    let mut replacement = render_dest(reference.form, &new_part, suffix, reference.quote);
    // La sustitución cubre solo el span del destino: se recompone la forma.
    replacement = strip_dest_form(reference.form, &replacement, reference.quote);
    Ok(Some((reference.span, replacement)))
}

/// Wiki links §19.3: se resuelven por nombre. Solo se actualizan cuando están
/// activos (§16: «wiki links activos»); la coincidencia ambigua es
/// [`FsError::UnsafeReference`] (§16: sin garantías → abortar).
fn plan_wiki_reference(
    reference: &Reference,
    root: &Path,
    from: &Path,
    to: &Path,
    options: ChangeOptions,
    scan: &[PathBuf],
) -> Result<Option<(Span, String)>, FsError> {
    if !options.wiki_links {
        return Ok(None);
    }
    let (name, suffix) = split_fragment(&reference.dest);
    if name.is_empty() || has_scheme(name) {
        return Ok(None);
    }
    // La regla de nombre vive en [`crate::resolve::wiki_name_matches`] y opera
    // sobre rutas relativas a la raíz (§17): una única respuesta para la
    // resolución (§19.3) y para esta reescritura (§16).
    let mut matches: Vec<&PathBuf> = scan
        .iter()
        .filter(|path| wiki_name_matches(Path::new(&relative_path(root, path)), name))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() > 1 {
        return Err(FsError::UnsafeReference {
            path: PathBuf::from("«wiki link ambiguo»"),
            dest: reference.dest.clone(),
        });
    }
    let matched = matches.pop().expect("len == 1");
    if !(matched == from || matched.starts_with(from)) {
        return Ok(None);
    }
    let new_path = to.join(relative_path(from, matched));
    let new_name = if name.contains('/') {
        strip_md_extension(Path::new(&relative_path(root, &new_path)))
            .to_string_lossy()
            .into_owned()
    } else {
        stem_str(&file_name_str(&new_path))
    };
    let replacement = format!("{new_name}{suffix}");
    if replacement == reference.dest {
        return Ok(None);
    }
    Ok(Some((reference.span, replacement)))
}

/// ¿Están los wiki links activos (§19.3) en este proyecto?
///
/// Detección mínima sobre `.mdproject.toml`: `wiki_links = true` en una
/// línea (§58.1 nivel proyecto). Sin archivo o sin la clave, el default es
/// `false` (§58.2: wiki links desactivados).
pub fn wiki_links_active(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".mdproject.toml"))
        .map(|source| {
            source.lines().any(|line| {
                let line = line.trim();
                let Some((key, value)) = line.split_once('=') else {
                    return false;
                };
                key.trim() == "wiki_links" && value.trim() == "true"
            })
        })
        .unwrap_or(false)
}

/// Archivos que pueden contener referencias conocidas (§16): Markdown y
/// `.mdproject.toml`, sin los ignorados por `.gitignore` (§14.6).
fn scan_reference_files(root: &Path, ignore: &IgnorePatterns) -> Result<Vec<PathBuf>, FsError> {
    let mut files = Vec::new();
    for entry in walk_tree(root)? {
        if entry.is_dir {
            continue;
        }
        let rel = relative_path(root, &entry.path);
        if ignore.is_ignored(Path::new(&rel), false) {
            continue;
        }
        let is_markdown = entry
            .path
            .extension()
            .is_some_and(|ext| ext == "md" || ext == "markdown");
        let is_project_toml = entry.path.file_name().is_some_and(|n| n == ".mdproject.toml");
        if is_markdown || is_project_toml {
            files.push(entry.path);
        }
    }
    Ok(files)
}

// --- Utilidades de nombres y destinos ----------------------------------------

fn checked_target(root: &Path, parent: &Path, name: &str) -> Result<PathBuf, FsError> {
    let mut components = Path::new(name).components();
    let simple = matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !simple || name.is_empty() {
        return Err(FsError::InvalidOperation {
            reason: "el nombre debe ser un nombre simple (sin / ni ..)",
        });
    }
    let parent = resolve_in_project(root, parent)?;
    let path = resolve_in_project(root, &parent.join(name))?;
    if path.exists() || path.is_symlink() {
        return Err(FsError::AlreadyExists { path });
    }
    Ok(path)
}

fn file_name_str(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn stem_str(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => stem.to_owned(),
        _ => name.to_owned(),
    }
}

fn strip_md_extension(path: &Path) -> PathBuf {
    let mut text = path.to_string_lossy().into_owned();
    for ext in [".markdown", ".md"] {
        if text.ends_with(ext) {
            text.truncate(text.len() - ext.len());
            break;
        }
    }
    PathBuf::from(text)
}

fn split_name(name: &str) -> (String, Option<String>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && !ext.contains('.') => {
            (stem.to_owned(), Some(ext.to_owned()))
        }
        _ => (name.to_owned(), None),
    }
}

/// `base.ext` libre o `base-2.ext`, `base-3.ext`, … (§18.3).
fn numbered_path(dir: &Path, stem: &str, ext: Option<&str>) -> PathBuf {
    let render = |n: u32| {
        let name = if n <= 1 {
            stem.to_owned()
        } else {
            format!("{stem}-{n}")
        };
        dir.join(match ext {
            Some(ext) => format!("{name}.{ext}"),
            None => name,
        })
    };
    let mut n = 1;
    while render(n).exists() || render(n).is_symlink() {
        n += 1;
    }
    render(n)
}

/// ¿El valor de configuración parece ruta documental (§14.4)?
fn is_pathish(dest: &str) -> bool {
    let (path_part, _) = split_fragment(dest);
    if path_part.contains('/') {
        return true;
    }
    match path_part.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => {
            PATH_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
        }
        _ => false,
    }
}

/// Normalización léxica (sin tocar el disco): resuelve `.` y `..`.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Compone el destino según su forma original (sin título: el span solo cubre
/// el destino).
fn render_dest(form: DestForm, path_part: &str, suffix: &str, quote: char) -> String {
    match form {
        DestForm::Angle => format!("<{path_part}{suffix}>"),
        DestForm::Quoted => format!("{quote}{path_part}{suffix}{quote}"),
        DestForm::Plain => format!("{path_part}{suffix}"),
    }
}

/// Quita del render la forma que no forma parte del span de sustitución.
fn strip_dest_form(form: DestForm, rendered: &str, quote: char) -> String {
    match form {
        DestForm::Angle => rendered
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_owned(),
        DestForm::Quoted => rendered
            .trim_start_matches(quote)
            .trim_end_matches(quote)
            .to_owned(),
        DestForm::Plain => rendered.to_owned(),
    }
}

/// ¿La forma de destino puede representar `dest` sin ambigüedad (§16)?
fn form_is_safe(form: DestForm, dest: &str, quote: char) -> bool {
    match form {
        DestForm::Plain => !dest
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '(' | ')' | '<' | '>')),
        DestForm::Angle => !dest.chars().any(|c| matches!(c, '<' | '>' | '\n')),
        DestForm::Quoted => {
            !dest.contains(quote) && !dest.chars().any(|c| matches!(c, '\n' | '\\'))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    fn temp_tree(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("minmark-ops-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn crear_y_no_sobrescribir_3_7() {
        let root = temp_tree("crear");
        let md = create_markdown(&root, &root, "nuevo.md").unwrap();
        assert_eq!(md, root.join("nuevo.md"));
        let dir = create_folder(&root, &root, "carpeta").unwrap();
        assert!(dir.is_dir());
        // Ya existe: error explícito, jamás sobrescritura (§3.7).
        assert!(matches!(
            create_markdown(&root, &root, "nuevo.md"),
            Err(FsError::AlreadyExists { .. })
        ));
        // Nombre no simple: fuera del proyecto (§48).
        assert!(matches!(
            create_markdown(&root, &root, "../fuga.md"),
            Err(FsError::InvalidOperation { .. })
        ));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicar_numera_y_no_sobrescribe_18_3() {
        let root = temp_tree("duplicar");
        fs::write(root.join("diagram.png"), "png").unwrap();
        let copy1 = duplicate(&root, &root.join("diagram.png")).unwrap();
        assert_eq!(copy1, root.join("diagram-2.png"));
        let copy2 = duplicate(&root, &root.join("diagram.png")).unwrap();
        assert_eq!(copy2, root.join("diagram-3.png"));
        fs::create_dir(root.join("cap")).unwrap();
        fs::write(root.join("cap/a.md"), "a").unwrap();
        let dir_copy = duplicate(&root, &root.join("cap")).unwrap();
        assert_eq!(dir_copy, root.join("cap-2"));
        assert_eq!(fs::read_to_string(dir_copy.join("a.md")).unwrap(), "a");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ruta_relativa_17() {
        assert_eq!(relative_path(Path::new("/p"), Path::new("/p/a.md")), "a.md");
        assert_eq!(relative_path(Path::new("/p/cap"), Path::new("/p/a.md")), "../a.md");
        assert_eq!(
            relative_path(Path::new("/p/cap"), Path::new("/p/cap/x/y.png")),
            "x/y.png"
        );
        assert_eq!(relative_path(Path::new("/p"), Path::new("/p")), ".");
    }

    #[test]
    fn papelera_xdg_con_info_14_9() {
        let root = temp_tree("trash");
        let trash_root = root.join("Trash");
        fs::write(root.join("borrar.md"), "adios").unwrap();
        fs::create_dir(root.join("carpeta")).unwrap();
        fs::write(root.join("carpeta/x.md"), "x").unwrap();

        trash_to(&trash_root, &root.join("borrar.md")).unwrap();
        trash_to(&trash_root, &root.join("carpeta")).unwrap();
        assert!(!root.join("borrar.md").exists());
        assert!(!root.join("carpeta").exists());
        assert_eq!(
            fs::read_to_string(trash_root.join("files/borrar.md")).unwrap(),
            "adios"
        );
        assert!(trash_root.join("files/carpeta/x.md").exists());
        let info = fs::read_to_string(trash_root.join("info/borrar.md.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\n"), "{info}");
        assert!(info.contains(&format!(
            "Path={}",
            percent_encode(&root.join("borrar.md").to_string_lossy())
        )));
        assert!(info.contains("DeletionDate="));
        // Nombres únicos en la papelera (§3.7: sin colisiones).
        fs::write(root.join("borrar.md"), "otro").unwrap();
        trash_to(&trash_root, &root.join("borrar.md")).unwrap();
        assert!(trash_root.join("files/borrar-2.md").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_entry_valida_48_y_no_sobrescribe() {
        let root = temp_tree("move");
        fs::write(root.join("a.md"), "a").unwrap();
        fs::write(root.join("b.md"), "b").unwrap();
        fs::create_dir(root.join("dir")).unwrap();
        assert!(matches!(
            move_entry(&root, &root.join("a.md"), &root.join("b.md")),
            Err(FsError::AlreadyExists { .. })
        ));
        assert!(matches!(
            move_entry(&root, &root.join("a.md"), &root.join("../fuga.md")),
            Err(FsError::PathOutsideProject { .. })
        ));
        assert!(matches!(
            move_entry(&root, &root.join("dir"), &root.join("dir/sub")),
            Err(FsError::InvalidOperation { .. })
        ));
        move_entry(&root, &root.join("a.md"), &root.join("c.md")).unwrap();
        assert!(root.join("c.md").exists());
        let _ = fs::remove_dir_all(&root);
    }

    /// Proyecto estándar de tests §16.
    fn proyecto_16(tag: &str) -> PathBuf {
        let root = temp_tree(tag);
        fs::create_dir(root.join("chapters")).unwrap();
        fs::create_dir(root.join("assets")).unwrap();
        fs::write(root.join("index.md"), "Ve a [C1](chapters/chapter1.md).\n").unwrap();
        fs::write(
            root.join("chapters/chapter1.md"),
            "# C1\n\n![Imagen](../assets/image.png) y [Índice](../index.md#top).\n",
        )
        .unwrap();
        fs::write(root.join("assets/image.png"), "png").unwrap();
        fs::write(root.join("refs.bib"), "@article{x}").unwrap();
        root
    }

    #[test]
    fn rename_actualiza_enlaces_desde_subcarpetas_17() {
        let root = proyecto_16("rename");
        let plan = plan_path_change(
            &root,
            &root.join("chapters"),
            &root.join("capitulos"),
            ChangeOptions::default(),
        )
        .unwrap();
        apply_path_change(&plan).unwrap();

        assert!(root.join("capitulos/chapter1.md").exists());
        assert_eq!(
            fs::read_to_string(root.join("index.md")).unwrap(),
            "Ve a [C1](capitulos/chapter1.md).\n"
        );
        // Dentro de la carpeta movida, los destinos fuera se reescriben para
        // conservar su resolución §17 (mismo nivel → mismo texto).
        assert_eq!(
            fs::read_to_string(root.join("capitulos/chapter1.md")).unwrap(),
            "# C1\n\n![Imagen](../assets/image.png) y [Índice](../index.md#top).\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn mover_a_mayor_profundidad_reescribe_los_fuera() {
        let root = proyecto_16("profundo");
        let plan = plan_path_change(
            &root,
            &root.join("chapters"),
            &root.join("parte1/chapters"),
            ChangeOptions::default(),
        )
        .unwrap();
        // `parte1/chapters` no existe: plan_path_change resuelve destino
        // inexistente (Save As-like §48).
        assert!(!plan.updates.is_empty());
        // Crear el destino ya es parte del movimiento: mover con destino
        // inexistente exige que su padre exista.
        fs::create_dir(root.join("parte1")).unwrap();
        apply_path_change(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("parte1/chapters/chapter1.md")).unwrap(),
            "# C1\n\n![Imagen](../../assets/image.png) y [Índice](../../index.md#top).\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("index.md")).unwrap(),
            "Ve a [C1](parte1/chapters/chapter1.md).\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn referencia_no_actualizable_aborta_sin_efectos_16() {
        let root = proyecto_16("abort");
        fs::write(
            root.join("raro.md"),
            "Destino con espacio: [x](chapters/chapter 1.md)\n",
        )
        .unwrap();
        // chapter1.md es afectado; el destino con espacio plano no es seguro
        // si su reescritura cambia de forma… primero forzamos el caso seguro:
        // renombrar `chapters/chapter1.md` → `con (paréntesis).md` hace el
        // destino plano inseguro.
        let before = fs::read_to_string(root.join("index.md")).unwrap();
        let result = plan_path_change(
            &root,
            &root.join("chapters/chapter1.md"),
            &root.join("chapters/con (paréntesis).md"),
            ChangeOptions::default(),
        );
        assert!(matches!(result, Err(FsError::UnsafeReference { .. })), "{result:?}");
        // Sin efectos: nada tocado (§16: abort completo).
        assert!(root.join("chapters/chapter1.md").exists());
        assert_eq!(fs::read_to_string(root.join("index.md")).unwrap(), before);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn wiki_links_solo_si_activos_19_3() {
        let root = temp_tree("wiki");
        fs::write(root.join("architecture.md"), "# arch").unwrap();
        fs::write(root.join("doc.md"), "Ver [[architecture#database]].\n").unwrap();

        // Inactivos (default §19.3): `[[...]]` es texto literal y no se toca.
        let plan = plan_path_change(
            &root,
            &root.join("architecture.md"),
            &root.join("arch.md"),
            ChangeOptions::default(),
        )
        .unwrap();
        assert!(plan.updates.is_empty());
        apply_path_change(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("doc.md")).unwrap(),
            "Ver [[architecture#database]].\n"
        );

        // Activos: se actualiza conservando el fragmento (§16).
        fs::write(root.join("doc.md"), "Ver [[arch#database]].\n").unwrap();
        let plan = plan_path_change(
            &root,
            &root.join("arch.md"),
            &root.join("arquitectura.md"),
            ChangeOptions { wiki_links: true },
        )
        .unwrap();
        apply_path_change(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("doc.md")).unwrap(),
            "Ver [[arquitectura#database]].\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn wiki_ambiguo_aborta_16() {
        let root = temp_tree("wiki-ambiguo");
        fs::create_dir(root.join("a")).unwrap();
        fs::create_dir(root.join("b")).unwrap();
        fs::write(root.join("a/doc.md"), "a").unwrap();
        fs::write(root.join("b/doc.md"), "b").unwrap();
        fs::write(root.join("ref.md"), "[[doc]]\n").unwrap();
        let result = plan_path_change(
            &root,
            &root.join("a/doc.md"),
            &root.join("a/otro.md"),
            ChangeOptions { wiki_links: true },
        );
        assert!(matches!(result, Err(FsError::UnsafeReference { .. })), "{result:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn front_matter_bib_y_mdproject_toml() {
        let root = temp_tree("config");
        fs::write(root.join("refs.bib"), "@article{x}").unwrap();
        fs::write(
            root.join("doc.md"),
            "---\nbibliography: refs.bib\ntitle: Demo\n---\n# H\n",
        )
        .unwrap();
        fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nfiles = [\"doc.md\"]\nbib = \"refs.bib\"\n",
        )
        .unwrap();

        let plan = plan_path_change(
            &root,
            &root.join("refs.bib"),
            &root.join("bib/references.bib"),
            ChangeOptions::default(),
        )
        .unwrap();
        fs::create_dir(root.join("bib")).unwrap();
        apply_path_change(&plan).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("doc.md")).unwrap(),
            "---\nbibliography: bib/references.bib\ntitle: Demo\n---\n# H\n"
        );
        let toml = fs::read_to_string(root.join(".mdproject.toml")).unwrap();
        assert!(toml.contains("bib = \"bib/references.bib\""), "{toml}");
        assert!(toml.contains("files = [\"doc.md\"]"), "{toml}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ignorados_por_gitignore_fuera_del_analisis_14_6() {
        let root = temp_tree("ignore");
        fs::write(root.join(".gitignore"), "generado.md\n").unwrap();
        fs::write(root.join("generado.md"), "[](objetivo.md)\n").unwrap();
        fs::write(root.join("objetivo.md"), "o").unwrap();
        fs::write(root.join("doc.md"), "[](objetivo.md)\n").unwrap();

        let plan = plan_path_change(
            &root,
            &root.join("objetivo.md"),
            &root.join("destino.md"),
            ChangeOptions::default(),
        )
        .unwrap();
        apply_path_change(&plan).unwrap();
        // El ignorado NO se analiza ni se reescribe (§14.6).
        assert_eq!(
            fs::read_to_string(root.join("generado.md")).unwrap(),
            "[](objetivo.md)\n"
        );
        assert_eq!(fs::read_to_string(root.join("doc.md")).unwrap(), "[](destino.md)\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fallo_a_mitad_revuelve_todo_16() {
        use std::os::unix::fs::PermissionsExt;

        let root = proyecto_16("rollback");
        fs::write(root.join("otro.md"), "[x](chapters/chapter1.md)\n").unwrap();
        let plan = plan_path_change(
            &root,
            &root.join("chapters/chapter1.md"),
            &root.join("chapters/c1.md"),
            ChangeOptions::default(),
        )
        .unwrap();
        assert_eq!(plan.updated_files().count(), 2);

        // El segundo archivo a reescribir queda sin escritura: el primero ya
        // escrito debe revertirse (all-or-nothing).
        let mut updates = plan.updates.clone();
        updates.sort();
        let bloqueado = updates[1].0.parent().unwrap().to_path_buf();
        fs::set_permissions(&bloqueado, fs::Permissions::from_mode(0o555)).unwrap();

        let result = apply_path_change(&plan);
        fs::set_permissions(&bloqueado, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err(), "{result:?}");
        assert!(root.join("chapters/chapter1.md").exists(), "el movimiento no se hizo");
        for file in [root.join("index.md"), root.join("otro.md")] {
            let content = fs::read_to_string(&file).unwrap();
            assert!(
                content.contains("chapter1.md"),
                "{file:?} no se revirtió: {content}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn import_markdown_copia_sin_tocar_el_original_15() {
        // Raíz de proyecto y un externo fuera de ella (en el mismo tmp).
        let externo = temp_tree("import-externo");
        let root = temp_tree("import-proy");
        fs::create_dir(root.join("docs")).unwrap();
        let src = externo.join("fuera.md");
        fs::write(&src, "# Externo\n").unwrap();

        let copia = import_markdown(&root, &src, &root.join("docs")).unwrap();
        assert_eq!(copia, root.join("docs/fuera.md"));
        // §15: copia, no movimiento; el original queda intacto.
        assert_eq!(fs::read_to_string(&src).unwrap(), "# Externo\n");
        assert_eq!(fs::read_to_string(&copia).unwrap(), "# Externo\n");

        // Colisión §3.7 → renombrado §18.3, sin sobrescritura.
        fs::write(&src, "# Externo 2\n").unwrap();
        let copia2 = import_markdown(&root, &src, &root.join("docs")).unwrap();
        assert_eq!(copia2, root.join("docs/fuera-2.md"));
        assert_eq!(fs::read_to_string(&copia).unwrap(), "# Externo\n");

        // §48: destino fuera de la raíz rechazado.
        assert!(matches!(
            import_markdown(&root, &src, &externo),
            Err(FsError::PathOutsideProject { .. })
        ));
        // §15: solo Markdown.
        fs::write(externo.join("imagen.png"), "png").unwrap();
        assert!(matches!(
            import_markdown(&root, &externo.join("imagen.png"), &root),
            Err(FsError::InvalidOperation { .. })
        ));
        // §15: lo que ya está en el proyecto no se importa.
        assert!(matches!(
            import_markdown(&root, &copia, &root),
            Err(FsError::InvalidOperation { .. })
        ));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&externo);
    }

    #[test]
    fn wiki_links_activos_por_mdproject_toml() {
        let root = temp_tree("wiki-activo");
        assert!(!wiki_links_active(&root), "default §58.2: desactivados");
        fs::write(root.join(".mdproject.toml"), "wiki_links = true\n").unwrap();
        assert!(wiki_links_active(&root));
        fs::write(root.join(".mdproject.toml"), "wiki_links = false\n").unwrap();
        assert!(!wiki_links_active(&root));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn destinos_con_esquema_o_ancla_no_son_rutas() {
        let root = temp_tree("esquemas");
        fs::write(root.join("x.md"), "# x").unwrap();
        fs::write(
            root.join("doc.md"),
            "[w](https://ejemplo.com/x.md) [m](mailto:a@b) [a](#sección) [r](x.md)\n",
        )
        .unwrap();
        let plan = plan_path_change(
            &root,
            &root.join("x.md"),
            &root.join("y.md"),
            ChangeOptions::default(),
        )
        .unwrap();
        apply_path_change(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("doc.md")).unwrap(),
            "[w](https://ejemplo.com/x.md) [m](mailto:a@b) [a](#sección) [r](y.md)\n"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
