//! Backend `inotify` de [`Watcher`] (Linux, §50.1): notificaciones nativas
//! sin dependencias externas (§4.1 Standard library first).
//!
//! Vigila los directorios padres de los ficheros registrados y filtra los
//! eventos por nombre, de modo que también detecta los reemplazos atómicos
//! por rename (en los que el inode vigilado cambia de nombre). Nunca hace un
//! rescan continuo del proyecto (§46).
//!
//! El watcher solo EMITE eventos ([`ChangeEvent`]): nunca recarga el buffer ni
//! escribe nada (§12.7: sin recarga automática; §3.6: sin escrituras
//! silenciosas). La UI notifica al usuario y este decide (ISSUE-020).
//!
//! Pérdida de eventos: ante desbordamiento de la cola se descartan; la
//! garantía de no sobrescritura no depende del watcher sino de la huella de
//! [`crate::LocalFs::changed_externally`] (§48).

use super::{coalesce, ChangeEvent, ChangeKind, Watcher};
use crate::FsError;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, CString, OsString};
use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};

const IN_MODIFY: u32 = 0x0000_0002;
const IN_ATTRIB: u32 = 0x0000_0004;
const IN_CLOSE_WRITE: u32 = 0x0000_0008;
const IN_MOVED_FROM: u32 = 0x0000_0040;
const IN_MOVED_TO: u32 = 0x0000_0080;
const IN_CREATE: u32 = 0x0000_0100;
const IN_DELETE: u32 = 0x0000_0200;
const IN_DELETE_SELF: u32 = 0x0000_0400;
const IN_MOVE_SELF: u32 = 0x0000_0800;
const IN_IGNORED: u32 = 0x0000_8000;
/// El evento afecta a un directorio (flag de `inotify_event::mask`).
const IN_ISDIR: u32 = 0x4000_0000;

/// Contenido, metadata, reemplazos (rename) y bajas.
const WATCH_MASK: u32 = IN_MODIFY
    | IN_ATTRIB
    | IN_CLOSE_WRITE
    | IN_MOVED_FROM
    | IN_MOVED_TO
    | IN_CREATE
    | IN_DELETE
    | IN_DELETE_SELF
    | IN_MOVE_SELF;

const IN_NONBLOCK: i32 = 0o4000;
const IN_CLOEXEC: i32 = 0o2000000;

const EVENT_HEADER_LEN: usize = 16;
const READ_BUF: usize = 4096;

extern "C" {
    fn inotify_init1(flags: i32) -> i32;
    fn inotify_add_watch(fd: i32, pathname: *const c_char, mask: u32) -> i32;
    fn inotify_rm_watch(fd: i32, wd: i32) -> i32;
}

#[derive(Debug)]
struct DirWatch {
    wd: i32,
    /// Nombre de fichero → ruta registrada.
    files: HashMap<OsString, PathBuf>,
}

/// Backend nativo `inotify` de [`Watcher`] (Linux, §50.1).
///
/// Un watch por directorio padre (refcounted por nombre de fichero), filtrado
/// por nombre: los reemplazos por rename sobre la ruta vigilada se detectan
/// como [`ChangeKind::Modified`], aunque el inode cambie.
#[derive(Debug)]
pub struct InotifyWatcher {
    file: File,
    /// Directorio canónico → watch.
    dirs: HashMap<PathBuf, DirWatch>,
    /// wd de inotify → directorio canónico.
    by_wd: HashMap<i32, PathBuf>,
    /// Ruta registrada (normalizada) → (directorio canónico, nombre).
    watched: HashMap<PathBuf, (PathBuf, OsString)>,
    /// Raíces registradas con [`Watcher::watch_tree`] (canónicas).
    tree_roots: HashSet<PathBuf>,
    /// Directorio del subárbol (canónico) → wd.
    tree_dirs: HashMap<PathBuf, i32>,
    /// wd → directorio del subárbol (canónico).
    tree_by_wd: HashMap<i32, PathBuf>,
}

impl InotifyWatcher {
    pub fn new() -> Result<Self, FsError> {
        // SAFETY: inotify_init1 no tiene precondiciones; el fd devuelto se
        // envuelve inmediatamente en un `File` que lo cierra al soltarse.
        let fd = unsafe { inotify_init1(IN_NONBLOCK | IN_CLOEXEC) };
        if fd < 0 {
            return Err(FsError::Io {
                path: PathBuf::from("inotify"),
                source: io::Error::last_os_error(),
            });
        }
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(Self {
            file,
            dirs: HashMap::new(),
            by_wd: HashMap::new(),
            watched: HashMap::new(),
            tree_roots: HashSet::new(),
            tree_dirs: HashMap::new(),
            tree_by_wd: HashMap::new(),
        })
    }

    /// Normaliza una ruta registrada: (clave = dir canónico + nombre, nombre,
    /// dir canónico). Así `watch`/`unwatch`/eventos coinciden aunque la ruta
    /// se escriba de otra forma.
    fn normalized(path: &Path) -> io::Result<(PathBuf, OsString, PathBuf)> {
        let name = path
            .file_name()
            .map(OsString::from)
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "sin nombre de fichero"))?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let dir = std::fs::canonicalize(parent)?;
        Ok((dir.join(&name), name, dir))
    }

    fn add_dir_watch(&mut self, dir: &Path) -> Result<i32, FsError> {
        let c_dir = CString::new(dir.as_os_str().as_bytes()).map_err(|source| FsError::Io {
            path: dir.to_path_buf(),
            source: io::Error::new(ErrorKind::InvalidInput, source),
        })?;
        // SAFETY: `c_dir` es un C string válido y el fd pertenece a `self.file`.
        let wd = unsafe { inotify_add_watch(self.file.as_raw_fd(), c_dir.as_ptr(), WATCH_MASK) };
        if wd < 0 {
            return Err(FsError::Io {
                path: dir.to_path_buf(),
                source: io::Error::last_os_error(),
            });
        }
        Ok(wd)
    }

    /// Registra `dir` y sus descendientes como subárbol vigilado (ISSUE-026).
    /// Los fallos puntuales (permisos, carreras de borrado) se ignoran: un
    /// directorio ilegible simplemente no emite.
    fn add_tree_dir_recursive(&mut self, dir: &Path) {
        let Ok(dir) = std::fs::canonicalize(dir) else {
            return;
        };
        if self.tree_dirs.contains_key(&dir) {
            return;
        }
        if let Ok(wd) = self.add_dir_watch(&dir) {
            self.tree_dirs.insert(dir.clone(), wd);
            self.tree_by_wd.insert(wd, dir.clone());
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            return;
        };
        for entry in read.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                self.add_tree_dir_recursive(&entry.path());
            }
        }
    }

    /// Da de baja `dir` y sus descendientes del subárbol vigilado.
    fn drop_tree_dir_recursive(&mut self, dir: &Path) {
        let stale: Vec<PathBuf> = self
            .tree_dirs
            .keys()
            .filter(|d| *d == dir || d.starts_with(dir))
            .cloned()
            .collect();
        for stale_dir in stale {
            if let Some(wd) = self.tree_dirs.remove(&stale_dir) {
                self.tree_by_wd.remove(&wd);
            }
        }
    }

    /// Traduce un evento sobre un directorio del subárbol (ISSUE-026):
    /// altas [`ChangeKind::Created`], bajas [`ChangeKind::Deleted`],
    /// modificaciones [`ChangeKind::Modified`]; los directorios nuevos
    /// quedan vigilados (su subárbol, en el momento del alta).
    fn collect_tree(
        &mut self,
        wd: i32,
        mask: u32,
        name: Option<OsString>,
        out: &mut Vec<ChangeEvent>,
    ) {
        let Some(dir) = self.tree_by_wd.get(&wd).cloned() else {
            return;
        };
        match name {
            Some(name) => {
                let path = dir.join(&name);
                let is_dir = mask & IN_ISDIR != 0;
                if mask & (IN_CREATE | IN_MOVED_TO) != 0 {
                    if is_dir {
                        self.add_tree_dir_recursive(&path);
                    }
                    out.push(ChangeEvent {
                        path,
                        kind: ChangeKind::Created,
                    });
                } else if mask & (IN_DELETE | IN_MOVED_FROM) != 0 {
                    if is_dir {
                        self.drop_tree_dir_recursive(&path);
                    }
                    out.push(ChangeEvent {
                        path,
                        kind: ChangeKind::Deleted,
                    });
                } else if mask & (IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE) != 0 {
                    out.push(ChangeEvent {
                        path,
                        kind: ChangeKind::Modified,
                    });
                }
            }
            None => {
                if mask & (IN_DELETE_SELF | IN_MOVE_SELF | IN_IGNORED) != 0 {
                    self.drop_tree_dir_recursive(&dir);
                    out.push(ChangeEvent {
                        path: dir,
                        kind: ChangeKind::Deleted,
                    });
                }
            }
        }
    }

    /// Traduce un evento del kernel a [`ChangeEvent`]s sobre las rutas
    /// registradas y mantiene los mapas coherentes.
    fn collect(&mut self, wd: i32, mask: u32, name: Option<OsString>, out: &mut Vec<ChangeEvent>) {
        let Some(dir) = self.by_wd.get(&wd).cloned() else {
            return; // wd desconocido o desbordamiento de cola: descartar.
        };
        match name {
            Some(name) => {
                let Some(path) = self
                    .dirs
                    .get(&dir)
                    .and_then(|dw| dw.files.get(&name))
                    .cloned()
                else {
                    return; // nombre no registrado en este directorio.
                };
                if mask & (IN_DELETE | IN_MOVED_FROM) != 0 {
                    out.push(ChangeEvent {
                        path,
                        kind: ChangeKind::Deleted,
                    });
                } else if mask & (IN_MODIFY | IN_ATTRIB | IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE)
                    != 0
                {
                    out.push(ChangeEvent {
                        path,
                        kind: ChangeKind::Modified,
                    });
                }
            }
            None => {
                // Evento sobre el propio directorio: desaparece o se mueve →
                // todos sus ficheros vigilados cuentan como borrados (§12.7:
                // el contenido sigue en memoria y se notifica al usuario).
                if mask & (IN_DELETE_SELF | IN_MOVE_SELF | IN_IGNORED) != 0 {
                    if let Some(dw) = self.dirs.remove(&dir) {
                        self.by_wd.remove(&dw.wd);
                        for (name, path) in dw.files {
                            self.watched.remove(&path);
                            let _ = name;
                            out.push(ChangeEvent {
                                path,
                                kind: ChangeKind::Deleted,
                            });
                        }
                    }
                }
            }
        }
    }
}

impl Watcher for InotifyWatcher {
    fn watch(&mut self, path: &Path) -> Result<(), FsError> {
        let (key, name, dir) = Self::normalized(path).map_err(|source| FsError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !self.dirs.contains_key(&dir) {
            let wd = self.add_dir_watch(&dir)?;
            self.by_wd.insert(wd, dir.clone());
            self.dirs.insert(
                dir.clone(),
                DirWatch {
                    wd,
                    files: HashMap::new(),
                },
            );
        }
        if let Some(dw) = self.dirs.get_mut(&dir) {
            dw.files.insert(name.clone(), key.clone());
        }
        self.watched.insert(key, (dir, name));
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> Result<(), FsError> {
        let Ok((key, name, dir)) = Self::normalized(path) else {
            return Ok(());
        };
        if self.watched.remove(&key).is_none() {
            return Ok(());
        }
        let mut empty = false;
        if let Some(dw) = self.dirs.get_mut(&dir) {
            dw.files.remove(&name);
            empty = dw.files.is_empty();
        }
        if empty {
            if let Some(dw) = self.dirs.remove(&dir) {
                self.by_wd.remove(&dw.wd);
                // SAFETY: el wd pertenece a este fd; si el watch ya
                // desapareció (directorio borrado) el error se ignora.
                unsafe {
                    inotify_rm_watch(self.file.as_raw_fd(), dw.wd);
                }
            }
        }
        Ok(())
    }

    fn watch_tree(&mut self, root: &Path) -> Result<(), FsError> {
        let root = std::fs::canonicalize(root).map_err(|source| FsError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        if !self.tree_roots.insert(root.clone()) {
            return Ok(()); // idempotente
        }
        self.add_tree_dir_recursive(&root);
        if !self.tree_dirs.contains_key(&root) {
            return Err(FsError::Io {
                path: root,
                source: io::Error::new(ErrorKind::NotFound, "raíz de watch_tree ilegible"),
            });
        }
        Ok(())
    }

    fn poll(&mut self) -> Result<Vec<ChangeEvent>, FsError> {
        let mut buf = [0u8; READ_BUF];
        let mut events = Vec::new();
        loop {
            // inotify solo devuelve eventos completos: un buffer lleno nunca
            // parte un evento por la mitad.
            let n = match self.file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(source) => {
                    return Err(FsError::Io {
                        path: PathBuf::from("inotify"),
                        source,
                    })
                }
            };
            for (wd, mask, name) in parse_events(&buf[..n]) {
                self.collect(wd, mask, name.clone(), &mut events);
                self.collect_tree(wd, mask, name, &mut events);
            }
        }
        Ok(coalesce(events))
    }
}

/// Cabecera `struct inotify_event` (16 bytes: wd, mask, cookie, len) más el
/// nombre NUL-terminado de longitud `len`.
fn parse_events(buf: &[u8]) -> Vec<(i32, u32, Option<OsString>)> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + EVENT_HEADER_LEN <= buf.len() {
        let wd = i32::from_ne_bytes(buf[off..off + 4].try_into().expect("cabecera completa"));
        let mask = u32::from_ne_bytes(buf[off + 4..off + 8].try_into().expect("cabecera completa"));
        let len = u32::from_ne_bytes(
            buf[off + 12..off + 16]
                .try_into()
                .expect("cabecera completa"),
        ) as usize;
        let name_start = off + EVENT_HEADER_LEN;
        let name_end = name_start.saturating_add(len).min(buf.len());
        let raw = &buf[name_start..name_end];
        let trimmed: Vec<u8> = raw.iter().copied().take_while(|&b| b != 0).collect();
        let name = if trimmed.is_empty() {
            None
        } else {
            Some(OsString::from_vec(trimmed))
        };
        out.push((wd, mask, name));
        off = name_start + len;
    }
    out
}
