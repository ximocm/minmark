//! Backend [`Watcher`] de Windows y macOS (§46) sobre el crate `notify`.
//!
//! `notify` envuelve las APIs nativas de notificación — `ReadDirectoryChangesW`
//! en Windows y FSEvents en macOS — con un canal hacia este proceso. No existe
//! API de `std` para ellas y §46 exige notificaciones nativas (nunca rescaneo
//! continuo), por lo que la dependencia está justificada según el precedente
//! de ADR-003 / §4.4 (ver `Cargo.toml`): resuelve funcionalidad crítica, con
//! licencia limpia y mantenimiento activo. Solo se compila en estos dos SO;
//! en Linux el backend sigue siendo el `inotify` directo de §50.1.
//!
//! Mapeo a la interfaz única (el contrato de [`super::Watcher`] es idéntico
//! en todos los backends):
//!
//! - `Create` → alta en un árbol ([`ChangeKind::Created`]); sobre un fichero
//!   registrado cuenta como reemplazo ([`ChangeKind::Modified`], paridad con
//!   `IN_CREATE` del backend Linux).
//! - `Remove` → [`ChangeKind::Deleted`]; si la ruta es un directorio padre
//!   vigilado, todos sus ficheros registrados cuentan como borrados (paridad
//!   con `IN_DELETE_SELF`).
//! - rename → `ReadDirectoryChangesW` empareja origen/destino y FSEvents no,
//!   así que no se distingue la dirección: la ruta se comprueba al consumir
//!   el evento, después de terminar la operación → si existe es alta
//!   (árbol) o modificación (fichero); si ya no existe es baja.
//! - `Access` y `Other` (p. ej. el aviso de rescan de FSEvents) se descartan;
//!   un error o desbordamiento del backend también, como la cola saturada de
//!   inotify: la red de seguridad de §12.7/§48 es la huella de
//!   [`crate::LocalFs::changed_externally`].
//!
//! Namespace de rutas: al crear un watch (`watch`, `watch_tree`) la raíz se
//! canónica (`std::fs::canonicalize`), y la clave de cada evento se
//! normaliza a ese mismo namespace aunque el directorio del evento ya no
//! exista (ancestro existente más profundo + cola perdida). Así la síntesis
//! de bajas cuando un directorio vigilado desaparece (`Remove` sobre un
//! directorio) y la protección de la raíz de un árbol en `unwatch` comparan
//! siempre canónico contra canónico, sea cual sea el namespace que el
//! llamante o el backend pasaran — en macOS `temp_dir()` es `/var/folders/…`
//! (enlace) y `std::fs::canonicalize`/FSEvents resuelven
//! `/private/var/folders/…`.
//!
//! `poll` solo drena lo disponible en el canal sin bloquear: la entrega llega
//! desde el hilo nativo del SO (§12.7: notificar, nunca recargar).

use super::{coalesce, ChangeEvent, ChangeKind, Watcher};
use crate::FsError;
use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};
use std::collections::{HashMap, HashSet};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// Backend de [`Watcher`] para Windows y macOS tras `notify`.
///
/// Estado espejo del backend Linux: ficheros registrados por clave canónica
/// (directorio canónico + nombre), directorios padre con refcount para el
/// alta/baja de watches y raíces de [`Watcher::watch_tree`]. Un directorio
/// cubierto por un árbol no se registra aparte (el watch recursivo lo
/// cubre), y la baja de un último fichero nunca toca la raíz de un árbol.
pub struct NotifyWatcher {
    /// Watcher nativo de `notify` (su hilo entrega en `eventos`).
    watcher: RecommendedWatcher,
    eventos: Receiver<notify::Result<Event>>,
    /// Ruta registrada (clave canónica) → directorio canónico padre.
    registrados: HashMap<PathBuf, PathBuf>,
    /// Directorio canónico → claves registradas en él.
    por_directorio: HashMap<PathBuf, HashSet<PathBuf>>,
    /// Directorios padre registrados en `notify` (uno por refcount).
    en_notify: HashSet<PathBuf>,
    /// Raíces registradas con [`Watcher::watch_tree`] (canónicas).
    arboles: HashSet<PathBuf>,
}

impl NotifyWatcher {
    pub fn new() -> Result<Self, FsError> {
        let (tx, rx) = mpsc::channel();
        let watcher = notify::recommended_watcher(move |resultado| {
            let _ = tx.send(resultado);
        })
        .map_err(|source| FsError::Io {
            path: PathBuf::from("notify"),
            source: io_error(source),
        })?;
        Ok(Self {
            watcher,
            eventos: rx,
            registrados: HashMap::new(),
            por_directorio: HashMap::new(),
            en_notify: HashSet::new(),
            arboles: HashSet::new(),
        })
    }

    /// Normaliza una ruta registrada: (clave = dir canónico + nombre, dir
    /// canónico), la misma normalización que el backend Linux, de modo que
    /// `watch`/`unwatch` y las rutas de los eventos coincidan aunque la ruta
    /// se escriba de otra forma.
    fn normalizar(path: &Path) -> io::Result<(PathBuf, PathBuf)> {
        let nombre = path
            .file_name()
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "sin nombre de fichero"))?;
        let padre = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let dir = std::fs::canonicalize(padre)?;
        Ok((dir.join(nombre), dir))
    }

    /// ¿La ruta queda bajo alguna raíz vigilada con [`Watcher::watch_tree`]?
    fn bajo_arbol(&self, ruta: &Path) -> bool {
        self.arboles.iter().any(|raiz| ruta.starts_with(raiz))
    }

    /// Clave canónica de una ruta de evento: directorio padre resuelto al
    /// namespace canónico + nombre, el mismo namespace en que se registró el
    /// watch al crearse. Si el padre ya no existe (borrado en cascada), se
    /// resuelve el ancestro existente más profundo y se re-appenda la cola
    /// ([`canonica_con_cola`]), en vez de dejar la ruta tal cual, para que la
    /// clave siga coincidiendo con las de registro sin importar el namespace
    /// que el backend entregue.
    fn clave_de(ruta: &Path) -> PathBuf {
        let Some(nombre) = ruta.file_name() else {
            return ruta.to_path_buf();
        };
        let padre = match ruta.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => return ruta.to_path_buf(),
        };
        match canonica_con_cola(padre) {
            Some(dir) => dir.join(nombre),
            None => ruta.to_path_buf(),
        }
    }

    /// Baja masiva de todos los ficheros registrados en un directorio que
    /// desaparece (paridad con `IN_DELETE_SELF`/`IN_IGNORED` de inotify).
    fn quitar_directorio(&mut self, dir: &Path, out: &mut Vec<ChangeEvent>) {
        let Some(hijas) = self.por_directorio.remove(dir) else {
            return;
        };
        let registrado = self.en_notify.remove(dir);
        // La raíz de un árbol vigila el mismo directorio: no se toca (§46).
        if registrado && !self.bajo_arbol(dir) {
            let _ = self.watcher.unwatch(dir);
        }
        for hija in hijas {
            self.registrados.remove(&hija);
            out.push(ChangeEvent {
                path: hija,
                kind: ChangeKind::Deleted,
            });
        }
    }

    /// Traduce un evento del backend a [`ChangeEvent`] sobre rutas vigiliadas.
    fn traducir(&mut self, evento: Event, out: &mut Vec<ChangeEvent>) {
        if matches!(evento.kind, EventKind::Access(_) | EventKind::Other) {
            return;
        }
        for ruta in &evento.paths {
            let clave = Self::clave_de(ruta);
            if matches!(evento.kind, EventKind::Remove(_)) {
                self.quitar_directorio(&clave, out);
            }
            let registrado = self.registrados.contains_key(&clave);
            let en_arbol = self.bajo_arbol(&clave);
            if !registrado && !en_arbol {
                continue; // nombre no registrado en este directorio.
            }
            // `existe` distingue la dirección del rename sin depender de que
            // el backend empareje origen/destino: el `poll` ocurre después de
            // que la operación termine, cuando el filesystem ya refleja el
            // resultado. Si existe, en un árbol es alta y en un fichero
            // registrado, modificación (reemplazo); si ya no existe, baja.
            let existe = ruta.exists();
            let empujar = |out: &mut Vec<ChangeEvent>, kind| {
                out.push(ChangeEvent {
                    path: clave.clone(),
                    kind,
                })
            };
            match &evento.kind {
                EventKind::Create(_) => {
                    if registrado {
                        empujar(out, ChangeKind::Modified);
                    }
                    if en_arbol {
                        empujar(out, ChangeKind::Created);
                    }
                }
                EventKind::Remove(_) => {
                    if registrado || en_arbol {
                        empujar(out, ChangeKind::Deleted);
                    }
                }
                EventKind::Modify(ModifyKind::Name(_)) => {
                    if registrado {
                        empujar(
                            out,
                            if existe {
                                ChangeKind::Modified
                            } else {
                                ChangeKind::Deleted
                            },
                        );
                    }
                    if en_arbol {
                        empujar(
                            out,
                            if existe {
                                ChangeKind::Created
                            } else {
                                ChangeKind::Deleted
                            },
                        );
                    }
                }
                // Contenido, metadata o modificación sin especificar (y el
                // `Any` de modo impreciso, que estos backends no emiten).
                _ => {
                    if registrado || en_arbol {
                        empujar(
                            out,
                            if existe {
                                ChangeKind::Modified
                            } else {
                                ChangeKind::Deleted
                            },
                        );
                    }
                }
            }
        }
    }
}

impl Watcher for NotifyWatcher {
    fn watch(&mut self, path: &Path) -> Result<(), FsError> {
        let (clave, dir) = Self::normalizar(path).map_err(|source| FsError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if self.registrados.contains_key(&clave) {
            return Ok(()); // idempotente
        }
        // Un directorio ya cubierto por un árbol no necesita watch aparte.
        if !self.bajo_arbol(&dir) && !self.en_notify.contains(&dir) {
            self.watcher
                .watch(&dir, RecursiveMode::NonRecursive)
                .map_err(|source| FsError::Io {
                    path: dir.clone(),
                    source: io_error(source),
                })?;
            self.en_notify.insert(dir.clone());
        }
        self.registrados.insert(clave.clone(), dir.clone());
        self.por_directorio
            .entry(dir.clone())
            .or_default()
            .insert(clave);
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> Result<(), FsError> {
        let Ok((clave, dir)) = Self::normalizar(path) else {
            return Ok(());
        };
        if self.registrados.remove(&clave).is_none() {
            return Ok(());
        }
        let vacio = self
            .por_directorio
            .get_mut(&dir)
            .map(|hijas| {
                hijas.remove(&clave);
                hijas.is_empty()
            })
            .unwrap_or(false);
        if vacio {
            self.por_directorio.remove(&dir);
            // La baja del último fichero nunca interrumpe la raíz de un
            // árbol que cubre el mismo directorio (§46); errores de un watch
            // ya desaparecido se ignoran (paridad con `inotify_rm_watch`).
            if self.en_notify.remove(&dir) && !self.bajo_arbol(&dir) {
                let _ = self.watcher.unwatch(&dir);
            }
        }
        Ok(())
    }

    fn watch_tree(&mut self, root: &Path) -> Result<(), FsError> {
        let raiz = std::fs::canonicalize(root).map_err(|source| FsError::Io {
            path: root.to_path_buf(),
            source,
        })?;
        let cubierta = self.bajo_arbol(&raiz);
        if !self.arboles.insert(raiz.clone()) {
            return Ok(()); // idempotente
        }
        if cubierta {
            return Ok(()); // ya la vigila una raíz ancestra recursiva
        }
        self.watcher
            .watch(&raiz, RecursiveMode::Recursive)
            .map_err(|source| FsError::Io {
                path: raiz,
                source: io_error(source),
            })
    }

    fn poll(&mut self) -> Result<Vec<ChangeEvent>, FsError> {
        let mut events = Vec::new();
        loop {
            match self.eventos.try_recv() {
                Ok(Ok(evento)) => self.traducir(evento, &mut events),
                // Errores del backend (p. ej. desbordamiento del buffer de
                // ReadDirectoryChangesW): descartados como la cola saturada
                // de inotify (§48 sostiene la garantía, no el watcher).
                Ok(Err(_)) => continue,
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        Ok(coalesce(events))
    }
}

/// `notify::Error` → [`std::io::Error`] para envolverlo en [`FsError::Io`]
/// con la semántica de error de E/S del resto de la interfaz (§46).
fn io_error(source: notify::Error) -> io::Error {
    io::Error::other(source)
}

/// Resuelve `ruta` al namespace canónico (`std::fs::canonicalize`) aunque ya
/// no exista: canónica el ancestro existente más profundo y re-appenda la
/// cola perdida (mismo criterio que `crate::paths` para destinos aún
/// inexistentes). `None` si no hay ancestro canónico (p. ej. relativa).
fn canonica_con_cola(ruta: &Path) -> Option<PathBuf> {
    let mut cola: Vec<std::ffi::OsString> = Vec::new();
    let mut actual = ruta;
    loop {
        match std::fs::canonicalize(actual) {
            Ok(mut base) => {
                for parte in cola.iter().rev() {
                    base.push(parte);
                }
                return Some(base);
            }
            Err(_) => {
                cola.push(actual.file_name()?.to_os_string());
                actual = actual.parent()?;
            }
        }
    }
}
