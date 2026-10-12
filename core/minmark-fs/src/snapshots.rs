//! Snapshots internos del documento, fuera del proyecto (§3.6, §45.1).
//!
//! El historial de versiones vive en un almacén separado del `.md` real
//! (§3.6): crear, listar o eliminar snapshots nunca escribe en el documento.
//! La gestión es manual (§13.6) y toda eliminación es permanente, sin
//! papelera (§3.7, §13.7).
//!
//! Límite configurable de espacio (1 GB por defecto, §13.5): al alcanzarlo se
//! detiene la creación de snapshots nuevos —los existentes quedan intactos y
//! el guardado real sigue disponible—.
//!
//! Formato en disco, sin serialización externa: un subdirectorio por snapshot
//! cuyo nombre es el id decimal relleno de ceros a 10 dígitos
//! (`0000000001`), con `content.md` (bytes del snapshot) y `document`
//! (UTF-8 del path asociado, vacío si no hay).

use crate::{at_snapshot_limit, snapshots_size, FsError, SnapshotMeta};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Fichero con los bytes del snapshot dentro de su subdirectorio.
const CONTENT_FILE: &str = "content.md";

/// Fichero con el path del documento asociado (UTF-8; vacío = sin documento).
const DOCUMENT_FILE: &str = "document";

/// Longitud máxima del nombre de id de un subdirectorio de snapshot.
const MAX_ID_LEN: usize = 20;

/// Directorio por defecto de snapshots, fuera del proyecto (§3.6, §45.1).
///
/// `$XDG_DATA_HOME/minmark/snapshots` si `XDG_DATA_HOME` está definido y es
/// absoluto; si no, `$HOME/.local/share/minmark/snapshots`. Nunca dentro del
/// proyecto: sin variables de entorno utilizables no hay destino seguro y se
/// devuelve [`FsError::Io`] con [`ErrorKind::NotFound`].
pub fn default_dir() -> Result<PathBuf, FsError> {
    dir_from_env(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Lógica de [`default_dir`] con el entorno inyectado.
///
/// Separada para poder validar todas las ramas sin mutar variables de
/// proceso (compartidas entre los tests en paralelo). Un valor no absoluto
/// no es utilizable: un destino relativo caería dentro del proyecto, fuera
/// del almacén externo que exige §45.1.
fn dir_from_env(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf, FsError> {
    if let Some(dir) = xdg_data_home.map(PathBuf::from) {
        if dir.is_absolute() {
            return Ok(dir.join("minmark").join("snapshots"));
        }
    }
    if let Some(dir) = home.map(PathBuf::from) {
        if dir.is_absolute() {
            return Ok(dir
                .join(".local")
                .join("share")
                .join("minmark")
                .join("snapshots"));
        }
    }
    Err(FsError::Io {
        path: PathBuf::new(),
        source: std::io::Error::new(
            ErrorKind::NotFound,
            "sin XDG_DATA_HOME ni HOME absolutos no hay directorio de snapshots fuera del proyecto (§45.1)",
        ),
    })
}

/// Nombre del subdirectorio de un snapshot: id decimal relleno a 10 dígitos.
fn snapshot_dir_name(id: u64) -> String {
    format!("{:010}", id)
}

/// Id desde el nombre de un subdirectorio: solo dígitos, hasta 20 (§13).
fn parse_id(name: &str) -> Option<u64> {
    if name.is_empty() || name.len() > MAX_ID_LEN || !name.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    name.parse().ok()
}

/// Escribe los ficheros de un snapshot recién creado.
fn write_snapshot_files(
    snap_dir: &Path,
    document_text: Option<&str>,
    content: &str,
) -> Result<(), FsError> {
    let content_path = snap_dir.join(CONTENT_FILE);
    fs::write(&content_path, content.as_bytes())
        .map_err(|source| FsError::Io {
            path: content_path,
            source,
        })?;
    let doc_path = snap_dir.join(DOCUMENT_FILE);
    fs::write(&doc_path, document_text.unwrap_or("").as_bytes()).map_err(|source| FsError::Io {
        path: doc_path,
        source,
    })
}

/// Documento asociado desde el fichero `document`; `None` si está vacío.
fn document_of(snap_dir: &Path) -> Result<Option<PathBuf>, FsError> {
    let path = snap_dir.join(DOCUMENT_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        // Ausente: resto de un crash durante la creación del snapshot; se
        // trata como "sin documento asociado" (§13.7) en vez de fallar.
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(FsError::Io {
                path: path.clone(),
                source,
            })
        }
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    // UTF-8 estricto (§12.5): sin conversiones silenciosas.
    let text = String::from_utf8(bytes).map_err(|_| FsError::NotUtf8 { path })?;
    Ok(Some(PathBuf::from(text)))
}

/// Metadata de un snapshot según su subdirectorio en disco (§13.5, §13.7).
fn meta_of(id: u64, snap_dir: &Path) -> Result<SnapshotMeta, FsError> {
    let content_path = snap_dir.join(CONTENT_FILE);
    let content_meta = fs::metadata(&content_path).map_err(|source| FsError::Io {
        path: content_path.clone(),
        source,
    })?;
    let timestamp = content_meta.modified().map_err(|source| FsError::Io {
        path: content_path,
        source,
    })?;
    // Espacio ocupado real del snapshot (§13.5): suma de los ficheros del
    // subdirectorio, coherente con `snapshots_size` y `at_snapshot_limit`.
    let mut size_bytes = 0;
    let entries = fs::read_dir(snap_dir).map_err(|source| FsError::Io {
        path: snap_dir.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| FsError::Io {
            path: snap_dir.to_path_buf(),
            source,
        })?;
        let file_meta = entry.metadata().map_err(|source| FsError::Io {
            path: entry.path(),
            source,
        })?;
        if file_meta.is_file() {
            size_bytes += file_meta.len();
        }
    }
    Ok(SnapshotMeta {
        id,
        document: document_of(snap_dir)?,
        timestamp,
        size_bytes,
    })
}

/// Almacén de snapshots internos (§13).
///
/// Un subdirectorio por snapshot fuera del proyecto (§45.1); nunca toca el
/// documento real (§3.6).
pub struct SnapshotStore {
    /// Directorio del almacén.
    dir: PathBuf,
    /// Id del próximo snapshot; dentro de una sesión nunca se reutilizan ids.
    next_id: u64,
}

impl SnapshotStore {
    /// Abre (o crea) el almacén en `dir`; crea el directorio si no existe.
    ///
    /// Los ids se recalculan de los subdirectorios existentes (siguiente =
    /// máximo + 1, mínimo 1): reabrir el almacén nunca sobrescribe snapshots
    /// previos.
    pub fn open(dir: PathBuf) -> Result<Self, FsError> {
        fs::create_dir_all(&dir).map_err(|source| FsError::Io {
            path: dir.clone(),
            source,
        })?;
        let mut store = Self { dir, next_id: 1 };
        let max = store.snapshot_dirs()?.into_iter().map(|(id, _)| id).max();
        store.next_id = max.map_or(1, |id| id.saturating_add(1));
        Ok(store)
    }

    /// Directorio del almacén (fuera del proyecto, §45.1).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Crea un snapshot del `content` actual (§3.6: nunca escribe en el
    /// documento real).
    ///
    /// `Ok(None)` si se alcanzó el límite (§13.5: se detiene la creación y
    /// los snapshots existentes quedan intactos). Si la escritura falla a
    /// mitad se elimina el subdirectorio parcial y se devuelve [`FsError::Io`]:
    /// un snapshot roto nunca contamina el almacén.
    pub fn create(
        &mut self,
        document: Option<&Path>,
        content: &str,
        limit_bytes: u64,
    ) -> Result<Option<SnapshotMeta>, FsError> {
        if at_snapshot_limit(&self.list()?, limit_bytes) {
            return Ok(None);
        }
        // El path se guarda como UTF-8 (§12.5), sin conversión silenciosa.
        let document_text = match document {
            Some(path) => {
                let text = path.to_str().ok_or_else(|| FsError::NotUtf8 {
                    path: path.to_path_buf(),
                })?;
                Some(text)
            }
            None => None,
        };
        let id = self.next_id;
        let snap_dir = self.dir.join(snapshot_dir_name(id));
        fs::create_dir(&snap_dir).map_err(|source| FsError::Io {
            path: snap_dir.clone(),
            source,
        })?;
        let written =
            write_snapshot_files(&snap_dir, document_text, content).and_then(|()| {
                meta_of(id, &snap_dir)
            });
        let meta = match written {
            Ok(meta) => meta,
            Err(err) => {
                let _ = fs::remove_dir_all(&snap_dir);
                return Err(err);
            }
        };
        self.next_id = id.saturating_add(1);
        Ok(Some(meta))
    }

    /// Lista los snapshots con su metadata (§13.6), orden ascendente por id.
    ///
    /// Robusto frente a entradas ajenas y a restos de un crash durante la
    /// creación: ignora lo que no sea un directorio con nombre de id válido
    /// (dígitos, hasta 20) y con `content.md` presente.
    pub fn list(&self) -> Result<Vec<SnapshotMeta>, FsError> {
        let mut snapshots = Vec::new();
        for (id, snap_dir) in self.snapshot_dirs()? {
            if !snap_dir.join(CONTENT_FILE).is_file() {
                continue; // directorio parcial de un crash: no es un snapshot.
            }
            snapshots.push(meta_of(id, &snap_dir)?);
        }
        Ok(snapshots)
    }

    /// Espacio total ocupado por el almacén (§13.5).
    pub fn total_size(&self) -> Result<u64, FsError> {
        Ok(snapshots_size(&self.list()?))
    }

    /// Eliminación permanente, sin papelera (§13.7). Idempotente: `Ok(())`
    /// aunque el snapshot no exista.
    pub fn delete(&mut self, id: u64) -> Result<(), FsError> {
        let snap_dir = self.dir.join(snapshot_dir_name(id));
        match fs::remove_dir_all(&snap_dir) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(FsError::Io {
                path: snap_dir,
                source,
            }),
        }
    }

    /// Vacía todos los snapshots (§13.6: reset completo); conserva el
    /// directorio del almacén. Eliminación permanente (§13.7).
    pub fn clear(&mut self) -> Result<(), FsError> {
        for (id, _) in self.snapshot_dirs()? {
            self.delete(id)?;
        }
        Ok(())
    }

    /// Subdirectorios de snapshot existentes como `(id, ruta)`, id ascendente.
    fn snapshot_dirs(&self) -> Result<Vec<(u64, PathBuf)>, FsError> {
        let mut dirs = Vec::new();
        let entries = fs::read_dir(&self.dir).map_err(|source| FsError::Io {
            path: self.dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| FsError::Io {
                path: self.dir.clone(),
                source,
            })?;
            let file_type = entry.file_type().map_err(|source| FsError::Io {
                path: entry.path(),
                source,
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let id = match name.to_str().and_then(parse_id) {
                Some(id) => id,
                None => continue,
            };
            dirs.push((id, entry.path()));
        }
        dirs.sort_unstable_by_key(|(id, _)| *id);
        Ok(dirs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    /// Directorio temporal de test, eliminado al salir.
    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("minmark-fs-{}-{}", std::process::id(), nanos));
            fs::create_dir_all(&path).unwrap();
            Self(path)
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

    #[test]
    fn create_y_listar_snapshots() {
        let dir = TestDir::new();
        let mut store = SnapshotStore::open(dir.file("snaps")).unwrap();
        let antes = SystemTime::now();
        let documento = PathBuf::from("/tmp/notas/doc.md");
        let m1 = store
            .create(Some(&documento), "hola\n", u64::MAX)
            .unwrap()
            .unwrap();
        assert_eq!(m1.id, 1);
        assert_eq!(m1.document, Some(documento));
        assert!(m1.size_bytes > 0);
        assert!(m1.timestamp > UNIX_EPOCH, "timestamp debe estar poblado");
        assert!(
            m1.timestamp + Duration::from_secs(60) >= antes,
            "timestamp debe ser el mtime reciente de content.md"
        );

        let m2 = store.create(None, "segundo\n", u64::MAX).unwrap().unwrap();
        assert_eq!(m2.id, 2);
        assert_eq!(m2.document, None);
        assert!(m2.size_bytes > 0);

        let list = store.list().unwrap();
        assert_eq!(list, vec![m1.clone(), m2.clone()]);
        // El contenido del snapshot se puede leer de content.md y coincide.
        assert_eq!(
            fs::read(store.dir().join("0000000001").join(CONTENT_FILE)).unwrap(),
            b"hola\n"
        );
        assert_eq!(
            fs::read(store.dir().join("0000000002").join(CONTENT_FILE)).unwrap(),
            b"segundo\n"
        );
        // Con documento el fichero `document` guarda su path; sin él, vacío.
        assert_eq!(
            fs::read(store.dir().join("0000000001").join(DOCUMENT_FILE)).unwrap(),
            b"/tmp/notas/doc.md"
        );
        assert_eq!(
            fs::read(store.dir().join("0000000002").join(DOCUMENT_FILE)).unwrap(),
            b""
        );
    }

    #[test]
    fn limite_alcanzado_detiene_la_creacion() {
        let dir = TestDir::new();
        let mut store = SnapshotStore::open(dir.file("snaps")).unwrap();
        let m1 = store.create(None, "abcdef", u64::MAX).unwrap().unwrap();
        let total = store.total_size().unwrap();
        assert_eq!(total, m1.size_bytes);

        // Límite ya alcanzado: `Ok(None)` y nada nuevo se crea (§13.5).
        assert!(store.create(None, "nuevo\n", total).unwrap().is_none());
        assert!(store
            .create(Some(Path::new("/otro.md")), "x", total - 1)
            .unwrap()
            .is_none());

        // Los existentes quedan intactos (§13.5).
        let list = store.list().unwrap();
        assert_eq!(list, vec![m1]);
        assert_eq!(store.total_size().unwrap(), total);
        assert_eq!(fs::read_dir(store.dir()).unwrap().count(), 1);
        assert_eq!(
            fs::read(store.dir().join("0000000001").join(CONTENT_FILE)).unwrap(),
            b"abcdef"
        );
    }

    #[test]
    fn delete_permanente_idempotente_y_clear_completo() {
        let dir = TestDir::new();
        let mut store = SnapshotStore::open(dir.file("snaps")).unwrap();
        let m1 = store
            .create(Some(Path::new("/docs/a.md")), "uno\n", u64::MAX)
            .unwrap()
            .unwrap();
        let m2 = store.create(None, "dos\n", u64::MAX).unwrap().unwrap();
        assert_eq!(store.total_size().unwrap(), m1.size_bytes + m2.size_bytes);

        // Eliminación permanente, sin papelera (§13.7), idempotente.
        store.delete(m1.id).unwrap();
        store.delete(m1.id).unwrap();
        store.delete(99).unwrap(); // tampoco falla si no existía
        assert!(!store.dir().join("0000000001").exists());
        let list = store.list().unwrap();
        assert_eq!(list, vec![m2.clone()]);
        assert_eq!(store.total_size().unwrap(), m2.size_bytes);

        // Reset completo (§13.6): vacía todo y conserva `dir`.
        store.clear().unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.total_size().unwrap(), 0);
        assert!(store.dir().is_dir());
        store.clear().unwrap(); // vaciar de nuevo no falla
    }

    #[test]
    fn reapertura_continua_los_ids_sin_sobrescribir() {
        let dir = TestDir::new();
        let ruta = dir.file("snaps");
        let mut primera = SnapshotStore::open(ruta.clone()).unwrap();
        primera.create(None, "uno\n", u64::MAX).unwrap().unwrap();
        primera.create(None, "dos\n", u64::MAX).unwrap().unwrap();
        drop(primera);

        let mut segunda = SnapshotStore::open(ruta).unwrap();
        let m3 = segunda.create(None, "tres\n", u64::MAX).unwrap().unwrap();
        assert_eq!(m3.id, 3);
        let ids: Vec<u64> = segunda.list().unwrap().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        // Los previos siguen intactos.
        assert_eq!(
            fs::read(segunda.dir().join("0000000001").join(CONTENT_FILE)).unwrap(),
            b"uno\n"
        );
        assert_eq!(
            fs::read(segunda.dir().join("0000000002").join(CONTENT_FILE)).unwrap(),
            b"dos\n"
        );
        assert_eq!(
            fs::read(segunda.dir().join("0000000003").join(CONTENT_FILE)).unwrap(),
            b"tres\n"
        );
    }

    #[test]
    fn default_dir_es_absoluto_y_termina_en_minmark_snapshots() {
        match default_dir() {
            Ok(dir) => {
                assert!(dir.is_absolute(), "{:?} debe ser absoluta", dir);
                assert!(
                    dir.ends_with(Path::new("minmark").join("snapshots")),
                    "{:?} debe terminar en minmark/snapshots",
                    dir
                );
            }
            // Sin variables de entorno utilizables el error está especificado;
            // toda la lógica se valida de forma determinista abajo.
            Err(FsError::Io { source, .. }) => {
                assert_eq!(source.kind(), ErrorKind::NotFound)
            }
            Err(other) => panic!("error inesperado: {:?}", other),
        }
    }

    #[test]
    fn default_dir_logica_controlada() {
        // XDG_DATA_HOME absoluto manda (§45.1).
        assert_eq!(
            dir_from_env(
                Some(OsString::from("/tmp/xdg")),
                Some(OsString::from("/home/ana"))
            )
            .unwrap(),
            PathBuf::from("/tmp/xdg/minmark/snapshots")
        );
        // XDG_DATA_HOME no absoluto: se ignora y se usa HOME.
        assert_eq!(
            dir_from_env(
                Some(OsString::from("xdg-relativo")),
                Some(OsString::from("/home/ana"))
            )
            .unwrap(),
            PathBuf::from("/home/ana/.local/share/minmark/snapshots")
        );
        // Solo HOME.
        assert_eq!(
            dir_from_env(None, Some(OsString::from("/home/ana"))).unwrap(),
            PathBuf::from("/home/ana/.local/share/minmark/snapshots")
        );
        // Sin variables utilizables no hay destino fuera del proyecto.
        for (xdg, home) in [
            (None, None),
            (Some(OsString::from("rel")), None),
            (None, Some(OsString::from("rel"))),
            (Some(OsString::from("rel")), Some(OsString::from("rel"))),
        ] {
            match dir_from_env(xdg, home) {
                Err(FsError::Io { source, .. }) => {
                    assert_eq!(source.kind(), ErrorKind::NotFound)
                }
                other => panic!("esperaba Io NotFound, obtuvo {:?}", other),
            }
        }
    }

    #[test]
    fn list_ignora_entradas_que_no_son_snapshots() {
        let dir = TestDir::new();
        let mut store = SnapshotStore::open(dir.file("snaps")).unwrap();
        store.create(None, "x\n", u64::MAX).unwrap().unwrap();
        // Fichero con nombre de id, directorio sin nombre de id y directorio
        // con id no numérico: ninguno es un snapshot.
        fs::write(store.dir().join("0000000002"), "no soy un snapshot").unwrap();
        fs::create_dir(store.dir().join("notas")).unwrap();
        fs::create_dir(store.dir().join("123abc")).unwrap();
        fs::create_dir(store.dir().join("999999999999999999999")).unwrap(); // > 20 dígitos
        let ids: Vec<u64> = store.list().unwrap().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn document_no_utf8_es_error() {
        let dir = TestDir::new();
        let store = SnapshotStore::open(dir.file("snaps")).unwrap();
        let snap_dir = store.dir().join("0000000001");
        fs::create_dir(&snap_dir).unwrap();
        fs::write(snap_dir.join(CONTENT_FILE), "x").unwrap();
        fs::write(snap_dir.join(DOCUMENT_FILE), b"no-\xFF-utf8").unwrap();
        match store.list() {
            Err(FsError::NotUtf8 { path }) => {
                assert_eq!(path, snap_dir.join(DOCUMENT_FILE))
            }
            other => panic!("esperaba NotUtf8, obtuvo {:?}", other),
        }
    }

    #[test]
    fn restos_de_crash_no_bloquean_el_almacen() {
        // Un crash a mitad de `create` deja un directorio parcial (§13.6):
        // no debe impedir gestionar ni crear snapshots tras reabrir.
        let dir = TestDir::new();
        let snaps = dir.file("snaps");
        {
            let mut store = SnapshotStore::open(snaps.clone()).unwrap();
            store.create(None, "completo uno\n", u64::MAX).unwrap().unwrap();
        }
        let parcial = snaps.join("0000000002");
        fs::create_dir(&parcial).unwrap();
        fs::write(parcial.join(CONTENT_FILE), "a medias").unwrap();

        let mut store = SnapshotStore::open(snaps).unwrap();
        // Sin `document` la metadata sigue siendo utilizable…
        assert_eq!(store.list().unwrap()[1].document, None);
        // …y un directorio sin `content.md` no es un snapshot y se ignora.
        fs::remove_file(parcial.join(CONTENT_FILE)).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);

        // Los ids no se reutilizan y la creación continúa.
        let meta = store.create(None, "completo\n", u64::MAX).unwrap().unwrap();
        assert_eq!(meta.id, 3);
        assert_eq!(store.list().unwrap().len(), 2);
    }
}
