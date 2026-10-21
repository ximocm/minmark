//! Filesystem watching (§46): notificaciones nativas tras una abstracción
//! ligera ([`Watcher`]).
//!
//! Un mecanismo por plataforma detrás de una interfaz única:
//!
//! - **Linux** (§50.1): [`InotifyWatcher`] usa `inotify` directamente, la API
//!   del sistema operativo, sin dependencias externas (§4.1 Standard library
//!   first).
//! - **Windows y macOS**: [`NotifyWatcher`] envuelve `ReadDirectoryChangesW`
//!   y FSEvents mediante el crate `notify`. No existe API de `std` para esas
//!   notificaciones y escribirlas a mano sería una superficie `unsafe` enorme;
//!   la alternativa sería polling, que §46 prohíbe («notificaciones nativas,
//!   nunca rescan continuo»). Justificación de la dependencia estratégica:
//!   precedente ADR-003 / §4.4 — resuelve funcionalidad crítica exigida por
//!   SRS, con licencia limpia (CC0-1.0) y mantenimiento activo; solo se
//!   compila en esos dos SO (ver `Cargo.toml`).
//!
//! El watcher solo EMITE eventos ([`ChangeEvent`]): nunca recarga el buffer ni
//! escribe nada (§12.7: sin recarga automática; §3.6: sin escrituras
//! silenciosas). La UI notifica al usuario y este decide (ISSUE-020).
//!
//! Pérdida de eventos: ante desbordamiento de la cola se descartan; la
//! garantía de no sobrescritura no depende del watcher sino de la huella de
//! [`crate::LocalFs::changed_externally`] (§48).

use crate::FsError;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod inotify;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod notify_backend;

#[cfg(target_os = "linux")]
pub use inotify::InotifyWatcher;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use notify_backend::NotifyWatcher;

/// Backend nativo del sistema en ejecución: `inotify` en Linux,
/// `ReadDirectoryChangesW`/FSEvents en Windows/macOS (§46).
#[cfg(target_os = "linux")]
pub type NativeWatcher = InotifyWatcher;
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub type NativeWatcher = NotifyWatcher;

/// Naturaleza del cambio detectado (§12.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Contenido o metadata cambiados; cubre también el reemplazo por rename
    /// (p. ej. la escritura atómica de otro programa).
    Modified,
    /// El fichero fue borrado o movido fuera de su ruta.
    Deleted,
    /// Alta dentro de un árbol vigilado con [`Watcher::watch_tree`]
    /// (ISSUE-026): creado o movido/dentro.
    Created,
}

/// Cambio detectado en un fichero vigilado (§12.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeEvent {
    /// Ruta registrada en [`Watcher::watch`].
    pub path: PathBuf,
    pub kind: ChangeKind,
}

/// Abstracción ligera de filesystem watching (§46): un mecanismo por
/// plataforma tras una interfaz única.
pub trait Watcher {
    /// Registra `path` (debe tener nombre de fichero). Repetir `watch` sobre
    /// la misma ruta la registra una sola vez.
    fn watch(&mut self, path: &Path) -> Result<(), FsError>;
    /// Deja de vigilar `path`. Idempotente.
    fn unwatch(&mut self, path: &Path) -> Result<(), FsError>;
    /// Vigila `root` y **todo su subárbol** (ISSUE-026, §46): altas, bajas,
    /// movimientos y modificaciones de cualquier nombre dentro del árbol,
    /// con [`ChangeKind::Created`] para las altas. Los directorios que
    /// aparecen después se registran solos (sin rescan continuo: solo se
    /// lista el subárbol nuevo al llegar su evento).
    fn watch_tree(&mut self, root: &Path) -> Result<(), FsError>;
    /// Eventos acumulados desde la última llamada, sin bloquear. Coalescidos
    /// por fichero (un `Deleted` gana a cualquier `Modified` previo). Solo
    /// notifica: no recarga ni escribe nada (§12.7, §3.7).
    fn poll(&mut self) -> Result<Vec<ChangeEvent>, FsError>;
}

/// Un evento por ruta y por `poll` (ISSUE-026): `Deleted` gana a todo salvo
/// que el mismo lote traiga un `Created` posterior (delete + create =
/// reemplazo, `Modified` §12.7); un `Created` seguido de `Modified` sigue
/// siendo `Created` (un fichero nuevo escribiéndose es un alta).
fn coalesce(events: Vec<ChangeEvent>) -> Vec<ChangeEvent> {
    let mut out: Vec<ChangeEvent> = Vec::new();
    for event in events {
        match out.iter_mut().find(|e| e.path == event.path) {
            Some(slot) => {
                slot.kind = match (slot.kind, event.kind) {
                    (ChangeKind::Created, ChangeKind::Modified) => ChangeKind::Created,
                    (_, ChangeKind::Deleted) => ChangeKind::Deleted,
                    (ChangeKind::Deleted, _) => ChangeKind::Modified,
                    (_, other) => other,
                };
            }
            None => out.push(event),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    /// Cota total de espera de eventos: inotify entrega ya en el primer `poll`
    /// y los backends de Windows/macOS entregan desde su hilo nativo casi de
    /// inmediato (latencia FSEvents 0); el margen cubre cargas de CI, donde
    /// FSEvents puede arrastrar la entrega más de lo habitual. Siempre
    /// finita: ningún test duerme sin cota.
    const ESPERA: Duration = Duration::from_secs(20);

    /// Cota de cada intento de [`esperar_reintentando`]; el resto de la cota
    /// total `ESPERA` queda para los reintentos.
    const INTENTO: Duration = Duration::from_secs(5);

    /// Ventana de quietud de [`drenar`]: sin eventos durante este tiempo se
    /// considera al watcher vacío de verdad.
    const TRANQUILO: Duration = Duration::from_millis(500);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            Self(crate::test_dir::crear("minmark-watch"))
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn single(events: Vec<ChangeEvent>, name: &str) -> ChangeKind {
        assert_eq!(events.len(), 1, "eventos: {events:?}");
        assert_eq!(events[0].path.file_name(), Some(name.as_ref()));
        events[0].kind
    }

    /// Un intento de espera hasta `limite`: acumula (coalescidos por ruta
    /// como en un `poll`) hasta que `espera` se cumple o se agota la cota,
    /// con lo acumulado y si se cumplió.
    fn intento(
        watcher: &mut NativeWatcher,
        limite: Instant,
        espera: impl Fn(&[ChangeEvent]) -> bool,
    ) -> (Vec<ChangeEvent>, bool) {
        let mut acumulados: Vec<ChangeEvent> = Vec::new();
        loop {
            acumulados.extend(watcher.poll().unwrap());
            acumulados = coalesce(acumulados);
            if espera(&acumulados) {
                return (acumulados, true);
            }
            if Instant::now() >= limite {
                return (acumulados, false);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// `poll` hasta que `espera` se cumple sobre los eventos acumulados
    /// (coalescidos por ruta como en un `poll`), o falla con lo acumulado.
    ///
    /// Los backends asíncronos (ReadDirectoryChangesW/FSEvents) entregan desde
    /// un hilo propio: la espera es de test, no un rescan (§46 se cumple en el
    /// backend, que solo emite notificaciones nativas).
    fn esperar(
        watcher: &mut NativeWatcher,
        espera: impl Fn(&[ChangeEvent]) -> bool,
    ) -> Vec<ChangeEvent> {
        let (acumulados, cumplida) = intento(watcher, Instant::now() + ESPERA, espera);
        assert!(
            cumplida,
            "sin los eventos esperados; acumulados: {acumulados:?}"
        );
        acumulados
    }

    /// Como [`esperar`] pero con reintento acotado de `operacion`: en
    /// macOS/FSEvents puede arrastrarse un `Created` tardío de la operación
    /// anterior y `coalesce` lo fundiría con el `Modified` posterior (el alta
    /// gana, §12.7/ISSUE-026), en cuyo caso ni ampliar la espera bastaría.
    /// Cada intento descarta sus acumulados — lo tardío queda consumido — y
    /// repite la operación, siempre dentro de la cota total `ESPERA`
    /// (acotado, sin esperas libres).
    fn esperar_reintentando(
        watcher: &mut NativeWatcher,
        mut operacion: impl FnMut(),
        espera: impl Fn(&[ChangeEvent]) -> bool,
    ) -> Vec<ChangeEvent> {
        let fin = Instant::now() + ESPERA;
        loop {
            operacion();
            let (acumulados, cumplida) =
                intento(watcher, (Instant::now() + INTENTO).min(fin), &espera);
            if cumplida {
                return acumulados;
            }
            assert!(
                Instant::now() < fin,
                "sin los eventos esperados; acumulados: {acumulados:?}"
            );
            // Cota del intento agotada: se consume lo tardío y se repite.
            drenar(watcher);
        }
    }

    /// Drena el watcher hasta una ventana de quietud ([`TRANQUILO`]), no hasta
    /// el primer `poll` vacío: los backends asíncronos pueden tener entregas
    /// en vuelo y un `poll` vacío no significa que no lleguen más. Lo tardío
    /// que se cuela entre fases se funde en [`coalesce`] (el alta gana) y
    /// arruina el siguiente intento. Siempre acotado por `ESPERA`.
    fn drenar(watcher: &mut NativeWatcher) {
        let fin = Instant::now() + ESPERA;
        let mut quieto: Option<Instant> = None;
        loop {
            let events = watcher.poll().unwrap();
            if events.is_empty() {
                if quieto.get_or_insert_with(Instant::now).elapsed() >= TRANQUILO {
                    return;
                }
            } else {
                quieto = None;
            }
            assert!(Instant::now() < fin, "el watcher no se vacía: {events:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn poll_sin_eventos_es_vacio_y_no_bloquea() {
        let dir = TestDir::new();
        let path = dir.file("doc.md");
        fs::write(&path, "uno\n").unwrap();

        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch(&path).unwrap();
        assert!(watcher.poll().unwrap().is_empty());
    }

    #[test]
    fn detecta_modificacion_externa_coalescida() {
        // §12.7: la notificación existe; el watcher nunca recarga.
        let dir = TestDir::new();
        let path = dir.file("doc.md");
        fs::write(&path, "uno\n").unwrap();

        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch(&path).unwrap();

        fs::write(&path, "dos\n").unwrap();
        fs::write(&path, "tres\n").unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter().any(|e| e.kind == ChangeKind::Modified)
        });
        assert_eq!(single(events, "doc.md"), ChangeKind::Modified);
        // Un solo evento por poll por fichero; el backend termina vacío.
        drenar(&mut watcher);
    }

    #[test]
    fn detecta_borrado_externo() {
        let dir = TestDir::new();
        let path = dir.file("doc.md");
        fs::write(&path, "uno\n").unwrap();

        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch(&path).unwrap();

        fs::remove_file(&path).unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter().any(|e| e.kind == ChangeKind::Deleted)
        });
        assert_eq!(single(events, "doc.md"), ChangeKind::Deleted);
    }

    #[test]
    fn detecta_reemplazo_atomico_por_rename() {
        // Otro programa guarda con temporal + rename (§48): el inode cambia y
        // el watch por nombre sigue detectándolo.
        let dir = TestDir::new();
        let path = dir.file("doc.md");
        fs::write(&path, "uno\n").unwrap();

        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch(&path).unwrap();

        let tmp = dir.file(".doc.md.tmp");
        fs::write(&tmp, "dos\n").unwrap();
        fs::rename(&tmp, &path).unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter().any(|e| e.kind == ChangeKind::Modified)
        });
        assert_eq!(single(events, "doc.md"), ChangeKind::Modified);
    }

    #[test]
    fn unwatch_deja_de_notificar() {
        let dir = TestDir::new();
        let path = dir.file("doc.md");
        fs::write(&path, "uno\n").unwrap();

        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch(&path).unwrap();
        watcher.watch(&path).unwrap(); // idempotente
        watcher.unwatch(&path).unwrap();
        watcher.unwatch(&path).unwrap(); // idempotente

        fs::write(&path, "dos\n").unwrap();
        assert!(watcher.poll().unwrap().is_empty());
    }

    /// Solo Linux: la desaparición del directorio propio de un watch no se
    /// notifica igual en todos los SO (inotify sí emite `IN_IGNORED` sobre el
    /// watch; `ReadDirectoryChangesW`/FSEvents no garantizan reportar el
    /// directorio vigilado como tal). En Windows/macOS la garantía de §12.7
    /// sobre rutas borradas la sostiene la huella de
    /// [`crate::LocalFs::changed_externally`] (§48).
    #[cfg(target_os = "linux")]
    #[test]
    fn borro_de_los_ficheros_del_directorio_tras_desaparecer_el_directorio() {
        let dir = TestDir::new();
        let path = dir.file("doc.md");
        fs::write(&path, "uno\n").unwrap();

        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch(&path).unwrap();
        fs::remove_dir_all(&dir.0).unwrap();

        let events = esperar(&mut watcher, |evs| {
            evs.iter()
                .any(|e| e.kind == ChangeKind::Deleted && e.path.ends_with("doc.md"))
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Deleted && e.path.ends_with("doc.md")),
            "eventos: {events:?}"
        );
    }

    #[test]
    fn watch_tree_detecta_altas_bajas_y_modificaciones() {
        // ISSUE-026: crear, borrar y modificar dentro del subárbol.
        let dir = TestDir::new();
        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch_tree(&dir.0).unwrap();
        watcher.watch_tree(&dir.0).unwrap(); // idempotente
        assert!(watcher.poll().unwrap().is_empty());

        let path = dir.file("nuevo.md");
        fs::write(&path, "hola\n").unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("nuevo.md"))
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("nuevo.md")),
            "eventos: {events:?}"
        );
        drenar(&mut watcher);

        // Reintento acotado: un `Created` tardío de la creación se fundiría
        // con este `Modified` en `coalesce` (el alta gana, ISSUE-026), así
        // que, si una cota de intento se agota, se repite la escritura con
        // acumulados propios. La primera escritura la hace el propio helper.
        let events = esperar_reintentando(
            &mut watcher,
            || fs::write(&path, "cambiado\n").unwrap(),
            |evs| {
                evs.iter()
                    .any(|e| e.kind == ChangeKind::Modified && e.path.ends_with("nuevo.md"))
            },
        );
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Modified && e.path.ends_with("nuevo.md")),
            "eventos: {events:?}"
        );
        drenar(&mut watcher);

        fs::remove_file(&path).unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter()
                .any(|e| e.kind == ChangeKind::Deleted && e.path.ends_with("nuevo.md"))
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Deleted && e.path.ends_with("nuevo.md")),
            "eventos: {events:?}"
        );
    }

    #[test]
    fn watch_tree_vigila_los_subdirectorios_nuevos() {
        // ISSUE-026: un directorio creado después se registra solo (sin
        // rescan continuo §46) y sus ficheros generan eventos.
        let dir = TestDir::new();
        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch_tree(&dir.0).unwrap();

        let sub = dir.file("cap");
        fs::create_dir(&sub).unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("cap"))
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("cap")),
            "eventos: {events:?}"
        );

        fs::write(sub.join("a.md"), "a\n").unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("a.md"))
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("a.md")),
            "eventos: {events:?}"
        );
    }

    #[test]
    fn watch_tree_renombrar_dentro_es_baja_mas_alta() {
        // Un rename dentro del árbol: baja del nombre antiguo + alta del
        // nuevo, coalescidos por ruta; los consumidores (árbol/índice)
        // refrescan. Los dos lados pueden llegar en `poll`s distintos
        // (ReadDirectoryChangesW emite dos eventos sin emparejar), así que se
        // esperan sobre el acumulado.
        let dir = TestDir::new();
        fs::write(dir.file("a.md"), "a\n").unwrap();
        let mut watcher = NativeWatcher::new().unwrap();
        watcher.watch_tree(&dir.0).unwrap();

        fs::rename(dir.file("a.md"), dir.file("b.md")).unwrap();
        let events = esperar(&mut watcher, |evs| {
            evs.iter()
                .any(|e| e.kind == ChangeKind::Deleted && e.path.ends_with("a.md"))
                && evs
                    .iter()
                    .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("b.md"))
        });
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Deleted && e.path.ends_with("a.md")),
            "eventos: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| e.kind == ChangeKind::Created && e.path.ends_with("b.md")),
            "eventos: {events:?}"
        );
    }

    #[test]
    fn coalesce_un_evento_por_ruta_y_baja_gana() {
        let modificado = |nombre: &str, kind| ChangeEvent {
            path: PathBuf::from(nombre),
            kind,
        };
        // Un solo evento por ruta, en orden de llegada.
        let lote = coalesce(vec![
            modificado("a.md", ChangeKind::Modified),
            modificado("a.md", ChangeKind::Modified),
            modificado("b.md", ChangeKind::Deleted),
        ]);
        assert_eq!(lote.len(), 2, "{lote:?}");
        assert_eq!(lote[0].path, PathBuf::from("a.md"));
        // Created + Modified sigue siendo Created (alta en curso).
        let lote = coalesce(vec![
            modificado("a.md", ChangeKind::Created),
            modificado("a.md", ChangeKind::Modified),
        ]);
        assert_eq!(single(lote, "a.md"), ChangeKind::Created);
        // Borrado + reescritura del mismo nombre: Modified (reemplazo).
        let lote = coalesce(vec![
            modificado("a.md", ChangeKind::Deleted),
            modificado("a.md", ChangeKind::Modified),
        ]);
        assert_eq!(single(lote, "a.md"), ChangeKind::Modified);
        // Borrado gana a lo previo.
        let lote = coalesce(vec![
            modificado("a.md", ChangeKind::Modified),
            modificado("a.md", ChangeKind::Deleted),
        ]);
        assert_eq!(single(lote, "a.md"), ChangeKind::Deleted);
    }
}
