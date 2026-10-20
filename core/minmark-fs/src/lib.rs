//! Filesystem de minmark (§12, §13, §48).
//!
//! Frontera de almacenamiento del core: file I/O sobre el buffer
//! ([`LocalFs`] implementa [`FileStore`], ISSUE-013), guardado atómico y
//! prevención de sobrescrituras silenciosas (ISSUE-019), y snapshots internos
//! separados del documento real (§3.6).
//!
//! Promesas normativas que estas interfaces deben cumplir:
//! - §12.5: solo UTF-8; error claro, sin conversiones silenciosas.
//! - §12.6: LF permanece LF, CRLF permanece CRLF.
//! - §12.7: cambios externos se notifican; nunca se recarga ni se sobrescribe
//!   silenciosamente.
//! - §13.5–13.7: snapshots con límite configurable, detención al alcanzarlo y
//!   gestión manual.
//! - §46: watching con notificaciones nativas; nunca rescan continuo.
//! - §48: guardado atómico y validación de rutas.

mod atomic;
pub mod fuzzy;
mod ignore;
mod index;
mod local;
mod ops;
mod paths;
pub mod resolve;
pub mod search;
mod snapshots;
#[cfg(test)]
mod test_dir;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod watch;

pub use atomic::{stage_write, write_atomic, StagedWrite};
pub use ignore::IgnorePatterns;
pub use index::{IndexEntry, IndexHeading, ProjectIndex, RefreshStats};
pub use local::{FileStamp, LocalFs};
pub use ops::{
    apply_path_change, create_folder, create_markdown, duplicate, import_markdown, move_entry,
    plan_path_change, relative_path, trash, trash_to, wiki_links_active, ChangeOptions, PathChange,
};
pub use paths::{canonicalize, read_dir_entries, resolve_in_project, walk_tree, TreeEntry};
pub use resolve::{LinkTarget, OpenBuffers, Resolver, TargetKind};
pub use snapshots::{default_dir, SnapshotStore};
#[cfg(target_os = "linux")]
pub use watch::InotifyWatcher;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use watch::NotifyWatcher;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub use watch::{ChangeEvent, ChangeKind, NativeWatcher, Watcher};

use minmark_document::Document;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Errores de filesystem del core.
#[derive(Debug)]
pub enum FsError {
    /// El archivo no es UTF-8 válido (§12.5: error claro, sin conversión).
    NotUtf8 { path: PathBuf },
    /// Error de E/S del sistema.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// `save` sobre un documento sin archivo asignado: la UI debe abrir
    /// Save As (§12.4).
    MissingPath,
    /// La ruta resuelta sale del proyecto (§48: path traversal).
    PathOutsideProject { path: PathBuf },
    /// El archivo cambió en disco desde que se cargó (§12.7, §48): no se
    /// sobrescribe sin confirmación del usuario.
    ExternalModification { path: PathBuf },
    /// El destino ya existe (§3.7): nunca sobrescritura silenciosa.
    AlreadyExists { path: PathBuf },
    /// Referencia conocida no actualizable con garantías: el rename/movimiento
    /// se aborta completo, sin efectos (§16).
    UnsafeReference { path: PathBuf, dest: String },
    /// Operación inválida (p. ej. mover una carpeta dentro de sí misma).
    InvalidOperation { reason: &'static str },
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 { path } => {
                write!(
                    f,
                    "{} no es UTF-8 válido (solo se admite UTF-8, §12.5)",
                    path.display()
                )
            }
            Self::Io { path, source } => write!(f, "error de E/S en {}: {source}", path.display()),
            Self::MissingPath => {
                write!(
                    f,
                    "el documento no tiene archivo asignado; use Save As (§12.4)"
                )
            }
            Self::PathOutsideProject { path } => {
                write!(f, "la ruta {} sale del proyecto (§48)", path.display())
            }
            Self::ExternalModification { path } => write!(
                f,
                "{} cambió externamente desde que se cargó (§12.7)",
                path.display()
            ),
            Self::AlreadyExists { path } => {
                write!(
                    f,
                    "el destino {} ya existe (§3.7: sin sobrescritura)",
                    path.display()
                )
            }
            Self::UnsafeReference { path, dest } => write!(
                f,
                "la referencia «{dest}» en {} no se puede actualizar con garantías; \
                 operación cancelada completa (§16)",
                path.display()
            ),
            Self::InvalidOperation { reason } => write!(f, "operación inválida: {reason}"),
        }
    }
}

impl std::error::Error for FsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Almacenamiento de documentos sobre el buffer único (§6.1).
///
/// [`LocalFs`] lo implementa con `std::fs`, con guardado atómico (§48) y una
/// [`FileStamp`] por archivo abierto/guardado para detectar cambios externos
/// (§12.7). La notificación al usuario se monta sobre [`crate::Watcher`] (§46).
pub trait FileStore {
    /// Documento nuevo, vacío y sin path (§12.1).
    fn create_new(&mut self) -> Document;
    /// Lee y valida UTF-8 (§12.5); el buffer preserva LF/CRLF (§12.6).
    fn open(&mut self, path: &Path) -> Result<Document, FsError>;
    /// Guardado explícito en el path del documento (§12.3). Sin autosave.
    ///
    /// Falla con [`FsError::MissingPath`] si el documento no tiene archivo y
    /// con [`FsError::ExternalModification`] si el archivo cambió en disco
    /// desde que se cargó (nunca sobrescritura silenciosa, §12.7).
    fn save(&mut self, document: &mut Document) -> Result<(), FsError>;
    /// Guardado a un path nuevo (§12.4).
    fn save_as(&mut self, document: &mut Document, path: &Path) -> Result<(), FsError>;
}

/// Metadata de un snapshot interno para la gestión manual (§13.7).
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotMeta {
    pub id: u64,
    /// Documento asociado; `None` si nunca se guardó el archivo real.
    pub document: Option<PathBuf>,
    pub timestamp: SystemTime,
    pub size_bytes: u64,
}

/// ¿El archivo se puede abrir como documento Markdown (§12.2)?
///
/// Extensiones exactas `.md` y `.markdown` (sensibles a mayúsculas, como el
/// filesystem del soporte principal).
pub fn is_markdown_path(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("md") | Some("markdown") => true,
        _ => false,
    }
}

/// Uso total del almacenamiento de snapshots (§13.5).
pub fn snapshots_size(snapshots: &[SnapshotMeta]) -> u64 {
    snapshots.iter().map(|s| s.size_bytes).sum()
}

/// ¿Se ha alcanzado el límite de snapshots (§13.5)?
///
/// Al alcanzarlo se detiene la creación de snapshots nuevos y se muestra un
/// aviso persistente (§13.6); el guardado real no se ve afectado.
pub fn at_snapshot_limit(snapshots: &[SnapshotMeta], limit_bytes: u64) -> bool {
    snapshots_size(snapshots) >= limit_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensiones_markdown_de_12_2() {
        assert!(is_markdown_path(Path::new("notas.md")));
        assert!(is_markdown_path(Path::new(
            "/proyecto/carpeta/informe.markdown"
        )));
        assert!(!is_markdown_path(Path::new("datos.txt")));
        assert!(!is_markdown_path(Path::new("sin_extension")));
        assert!(!is_markdown_path(Path::new("README.MD")));
    }

    #[test]
    fn limite_de_snapshots_en_el_borde() {
        let snapshot = |size_bytes| SnapshotMeta {
            id: 1,
            document: None,
            timestamp: SystemTime::UNIX_EPOCH,
            size_bytes,
        };
        let list = vec![snapshot(999)];
        assert!(!at_snapshot_limit(&list, 1_000));

        let list = vec![snapshot(600), snapshot(400)];
        assert_eq!(snapshots_size(&list), 1_000);
        // §13.5: al alcanzar el límite se detiene la creación.
        assert!(at_snapshot_limit(&list, 1_000));
    }
}
