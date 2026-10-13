//! Índice persistente e incremental del proyecto (ISSUE-025, §45).
//!
//! - §45: acelera búsqueda, headings, links, backlinks, referencias y
//!   autocompletado; **solo** reindexa archivos con stamp cambiado; no
//!   bloquea la UI (el consumidor lo usa desde una tarea aparte, §47.1); es
//!   completamente reconstruible (borrar el índice no pierde nada) y nunca
//!   contiene la única copia de información del usuario.
//! - §45.1: persistencia **fuera del proyecto**, bajo el directorio de datos
//!   de la aplicación (mismo patrón que [`crate::SnapshotStore::default_dir`],
//!   §3.6): `$XDG_DATA_HOME/minmark/index/<clave>.index`.
//! - §14.6: los archivos ignorados por `.gitignore` de la raíz quedan fuera
//!   del índice y de las consultas (visibles en el árbol, ISSUE-022).
//!
//! Formato (serialización propia std-only, decisión §4.4 registrada en
//! ISSUE-025: sin dependencias nuevas), texto línea a línea:
//!
//! ```text
//! minmark-index 3
//! root <raíz canónica, percent-encoded>
//! <path-rel>\t<mtime-nanos>\t<size>\t<payload>
//! ```
//!
//! `payload` = registros separados por `\x1e`: `h=<nivel>:<línea>:<ancla>:<texto>`
//! (heading §27.1 con su ancla final §26 y su línea) o `l=<destino>`
//! (referencia §16). Cada valor va percent-encoded (solo
//! `[A-Za-z0-9-._~/=]` a secas); la lectura es tolerante: una línea o un
//! archivo ilegible se descarta y se reconstruye (§45). Las anclas se calculan
//! con la política única de [`minmark_document::document_anchors`], la misma
//! que usa la Preview: índice y Preview producen los mismos destinos.

use crate::ignore::IgnorePatterns;
use crate::paths::walk_tree;
use crate::{relative_path, write_atomic, FsError};
use minmark_document::find_markdown_references;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Firma del formato de índice.
const SIGNATURE: &str = "minmark-index 3";

/// Heading del índice con su ancla final y posición (§26, §45).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexHeading {
    pub level: u8,
    /// Texto del heading (sin `#`, cierre ni atributos, como en el AST).
    pub text: String,
    /// Ancla final (§26): ID explícito o automático con sufijo de unicidad.
    pub anchor: String,
    /// Línea (1-based) del heading en el fuente indexado.
    pub line: u32,
}

/// Entrada del índice para un archivo del proyecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Path relativo a la raíz (§17), en forma posix.
    pub rel_path: String,
    /// Stamp: nanosegundos de mtime + tamaño (mismo espíritu que
    /// [`crate::FileStamp`], ISSUE-013).
    pub mtime_nanos: u64,
    pub size: u64,
    /// Destinos en orden, con ancla y línea: headings H1–H6 y objetos
    /// con ID explícito (nivel 0, excluidos de las consultas de headings).
    pub headings: Vec<IndexHeading>,
    /// Destinos referenciados (§16), en orden.
    pub links: Vec<String>,
}

/// Resultado de [`ProjectIndex::refresh`]: qué se tocó (diagnóstico §3.7 y
/// tests de incrementalidad §45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshStats {
    /// Archivos (re)indexados por stamp nuevo o cambiado.
    pub reindexed: usize,
    /// Archivos ya no presentes: entradas purgadas.
    pub removed: usize,
    /// Archivos sin cambios: no releídos (§45: solo se reindexa lo modificado).
    pub unchanged: usize,
}

/// Índice del proyecto: entradas por path relativo §17.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIndex {
    root: PathBuf,
    entries: BTreeMap<String, IndexEntry>,
}

impl ProjectIndex {
    /// Índice vacío de `root` (§45: siempre reconstruible).
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            entries: BTreeMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn entries(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.values()
    }

    pub fn get(&self, rel_path: &str) -> Option<&IndexEntry> {
        self.entries.get(rel_path)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ruta por defecto del índice de `root` (§45.1): fuera del proyecto,
    /// `$XDG_DATA_HOME/minmark/index/<clave>.index` (o `~/.local/share/…`),
    /// con la clave derivada de la raíz canónica.
    pub fn default_path(root: &Path) -> Result<PathBuf, FsError> {
        let base = data_dir_from_env(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))?;
        let canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let key = canon.to_string_lossy();
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a: std-only, estable.
        for byte in key.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        Ok(base.join(format!("{hash:016x}.index")))
    }

    /// Rehidrata el índice de `root` desde su ruta por defecto (§45.1); sin
    /// archivo válido, índice vacío que se reconstruye (§45).
    pub fn load_or_new(root: &Path) -> Self {
        let path = Self::default_path(root).ok();
        path.and_then(|path| Self::load_from(&path, root))
            .unwrap_or_else(|| Self::new(root.to_path_buf()))
    }

    /// Lee un índice guardado; `None` si la cabecera no corresponde a `root`
    /// o el archivo no es legible (se reconstruye, §45).
    pub fn load_from(path: &Path, root: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        Self::parse(&text, root)
    }

    /// Guarda en su ruta por defecto (§45.1; escritura atómica §48).
    pub fn save(&self) -> Result<(), FsError> {
        let path = Self::default_path(&self.root)?;
        self.save_to(&path)
    }

    /// [`Self::save`] sobre una ruta explícita (tests).
    pub fn save_to(&self, path: &Path) -> Result<(), FsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| FsError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        write_atomic(path, self.serialize().as_bytes()).map_err(|source| FsError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Serializa en el formato `minmark-index 3` (ver módulo).
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str(SIGNATURE);
        out.push('\n');
        out.push_str(&format!(
            "root {}\n",
            pct_encode(&self.root.to_string_lossy())
        ));
        for entry in self.entries.values() {
            let mut payload: Vec<String> = Vec::new();
            for heading in &entry.headings {
                payload.push(format!(
                    "h={}:{}:{}:{}",
                    heading.level,
                    heading.line,
                    pct_encode(&heading.anchor),
                    pct_encode(&heading.text)
                ));
            }
            for link in &entry.links {
                payload.push(format!("l={}", pct_encode(link)));
            }
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                pct_encode(&entry.rel_path),
                entry.mtime_nanos,
                entry.size,
                payload.join("\u{1e}")
            ));
        }
        out
    }

    /// Parsea el formato `minmark-index 3`; `None` si no es un índice de
    /// `root` (tolerante con líneas individuales corruptas).
    pub fn parse(text: &str, root: &Path) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next() != Some(SIGNATURE) {
            return None;
        }
        let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        for line in lines.by_ref() {
            if let Some(encoded) = line.strip_prefix("root ") {
                if pct_decode(encoded) != root_canon.to_string_lossy() {
                    return None;
                }
                break;
            }
            return None;
        }
        let mut index = Self::new(root_canon);
        for line in lines {
            if let Some((rel_path, entry)) = Self::parse_entry(line) {
                index.entries.insert(rel_path, entry);
            }
        }
        Some(index)
    }

    fn parse_entry(line: &str) -> Option<(String, IndexEntry)> {
        let mut fields = line.splitn(4, '\t');
        let rel_path = pct_decode(fields.next()?);
        let mtime_nanos = fields.next()?.parse().ok()?;
        let size = fields.next()?.parse().ok()?;
        let payload = fields.next().unwrap_or("");
        let mut headings = Vec::new();
        let mut links = Vec::new();
        for record in payload.split('\u{1e}').filter(|r| !r.is_empty()) {
            let (key, value) = record.split_once('=')?;
            match key {
                "h" => {
                    let mut parts = value.splitn(4, ':');
                    let level = parts.next()?.parse().ok()?;
                    let line = parts.next()?.parse().ok()?;
                    let anchor = pct_decode(parts.next()?);
                    let text = pct_decode(parts.next()?);
                    headings.push(IndexHeading {
                        level,
                        text,
                        anchor,
                        line,
                    });
                }
                "l" => links.push(pct_decode(value)),
                _ => return None,
            }
        }
        let entry = IndexEntry {
            rel_path: rel_path.clone(),
            mtime_nanos,
            size,
            headings,
            links,
        };
        Some((rel_path, entry))
    }

    /// Refresco incremental (§45): solo los archivos con stamp cambiado o
    /// nuevos se re-leen; los borrados se purgan (§45 criterio 2).
    ///
    /// Los archivos no UTF-8 o ilegibles se omiten del índice (§12.5: no son
    /// documentos abribles; el índice nunca es la única copia §45).
    pub fn refresh(&mut self) -> Result<RefreshStats, FsError> {
        let root = std::fs::canonicalize(&self.root).map_err(|source| FsError::Io {
            path: self.root.clone(),
            source,
        })?;
        self.root = root.clone();
        let ignore = IgnorePatterns::from_root(&root);

        let mut stats = RefreshStats::default();
        let mut seen = BTreeSet::new();
        for entry in walk_tree(&root)? {
            if entry.is_dir {
                continue;
            }
            let is_markdown = entry
                .path
                .extension()
                .is_some_and(|ext| ext == "md" || ext == "markdown");
            if !is_markdown {
                continue;
            }
            let rel_path = relative_path(&root, &entry.path);
            if ignore.is_ignored(Path::new(&rel_path), false) {
                continue; // §14.6
            }
            seen.insert(rel_path.clone());

            let meta = match std::fs::metadata(&entry.path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            let mtime_nanos = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let size = meta.len();
            let unchanged = self
                .entries
                .get(&rel_path)
                .is_some_and(|indexed| indexed.mtime_nanos == mtime_nanos && indexed.size == size);
            if unchanged {
                stats.unchanged += 1;
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&entry.path) else {
                continue;
            };
            let headings = minmark_parser::destination_anchors(&source)
                .into_iter()
                .map(|anchor| IndexHeading {
                    level: anchor.level,
                    text: anchor.text,
                    anchor: anchor.id,
                    line: anchor.line,
                })
                .collect();
            let links = find_markdown_references(&source)
                .iter()
                .map(|reference| reference.dest.clone())
                .collect();
            self.entries.insert(
                rel_path.clone(),
                IndexEntry {
                    rel_path,
                    mtime_nanos,
                    size,
                    headings,
                    links,
                },
            );
            stats.reindexed += 1;
        }

        let stale: Vec<String> = self
            .entries
            .keys()
            .filter(|rel_path| !seen.contains(*rel_path))
            .cloned()
            .collect();
        stats.removed = stale.len();
        for rel_path in stale {
            self.entries.remove(&rel_path);
        }
        Ok(stats)
    }
}

/// Directorio de datos de la aplicación con subcarpeta `tail` (§45.1): mismo
/// patrón que [`crate::SnapshotStore::default_dir`] (§3.6), fuera del
/// proyecto. Un valor no absoluto no es utilizable (§45.1).
fn data_dir_from_env(
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, FsError> {
    if let Some(dir) = xdg_data_home.map(PathBuf::from) {
        if dir.is_absolute() {
            return Ok(dir.join("minmark").join("index"));
        }
    }
    if let Some(dir) = home.map(PathBuf::from) {
        if dir.is_absolute() {
            return Ok(dir
                .join(".local")
                .join("share")
                .join("minmark")
                .join("index"));
        }
    }
    Err(FsError::Io {
        path: PathBuf::new(),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "sin XDG_DATA_HOME ni HOME absolutos no hay directorio de datos fuera del proyecto (§45.1)",
        ),
    })
}

/// Percent-encoding de un campo: solo `[A-Za-z0-9-._~/=]` literal.
fn pct_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b'=' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Decodificación inversa de [`pct_encode`]; `%XX` ilegible se conserva tal
/// cual (tolerancia de lectura §45).
fn pct_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &value[i + 1..i + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
        let dir = std::env::temp_dir().join(format!(
            "minmark-index-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn project(tag: &str) -> PathBuf {
        let root = temp_tree(tag);
        fs::create_dir(root.join("cap")).unwrap();
        fs::write(root.join("index.md"), "# Inicio\n\n[a](cap/a.md)\n").unwrap();
        fs::write(
            root.join("cap/a.md"),
            "# Cap A\n\n## Detalle\n\n![i](i.png)\n",
        )
        .unwrap();
        root
    }

    /// Cambia el stamp de `path` sin dormir (append determinista).
    fn touch(path: &Path) {
        let mut content = fs::read_to_string(path).unwrap();
        content.push_str("\n<!-- v2 -->\n");
        fs::write(path, content).unwrap();
    }

    #[test]
    fn construccion_con_headings_y_links_45() {
        let root = project("build");
        let mut index = ProjectIndex::new(root.clone());
        let stats = index.refresh().unwrap();
        assert_eq!(
            stats,
            RefreshStats {
                reindexed: 2,
                removed: 0,
                unchanged: 0
            }
        );
        assert_eq!(index.len(), 2);

        let a = index.get("cap/a.md").unwrap();
        assert_eq!(
            a.headings,
            [
                IndexHeading {
                    level: 1,
                    text: "Cap A".to_owned(),
                    anchor: "cap-a".to_owned(),
                    line: 1,
                },
                IndexHeading {
                    level: 2,
                    text: "Detalle".to_owned(),
                    anchor: "detalle".to_owned(),
                    line: 3,
                },
            ]
        );
        assert_eq!(a.links, ["i.png"]);
        assert_eq!(index.get("index.md").unwrap().links, ["cap/a.md"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn incremental_solo_relee_lo_cambiado_45() {
        let root = project("incremental");
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();

        touch(&root.join("cap/a.md"));
        let stats = index.refresh().unwrap();
        assert_eq!(
            stats,
            RefreshStats {
                reindexed: 1,
                removed: 0,
                unchanged: 1
            }
        );

        // Nada cambiado: cero lecturas de más.
        let stats = index.refresh().unwrap();
        assert_eq!(
            stats,
            RefreshStats {
                reindexed: 0,
                removed: 0,
                unchanged: 2
            }
        );

        // Alta y baja: solo lo nuevo se indexa y lo borrado se purga (§45).
        fs::write(root.join("nuevo.md"), "# Nuevo\n").unwrap();
        fs::remove_file(root.join("cap/a.md")).unwrap();
        let stats = index.refresh().unwrap();
        assert_eq!(
            stats,
            RefreshStats {
                reindexed: 1,
                removed: 1,
                unchanged: 1
            }
        );
        assert!(index.get("cap/a.md").is_none());
        assert!(index.get("nuevo.md").is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn persistencia_entre_sesiones_y_reconstruccion_45_45_1() {
        let root = project("persist");
        let store = root.parent().unwrap().join("fuera-del-proyecto.index");
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        index.save_to(&store).unwrap();
        // §45.1: el índice no vive dentro del proyecto.
        assert!(!store.starts_with(&root));

        let rehidratado = ProjectIndex::load_from(&store, &root).expect("índice legible");
        assert_eq!(rehidratado.len(), 2);
        assert_eq!(rehidratado.get("cap/a.md"), index.get("cap/a.md"));

        // Reconstrucción total: borrar el índice no pierde nada (§45).
        fs::remove_file(&store).unwrap();
        let mut reconstruido = ProjectIndex::new(root.clone());
        assert_eq!(reconstruido.refresh().unwrap().reindexed, 2);
        assert_eq!(reconstruido.get("cap/a.md"), index.get("cap/a.md"));

        // Un índice de otra raíz se descarta (cabecera).
        let otra = project("persist-otra");
        assert!(ProjectIndex::load_from(&store, &otra).is_none());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&otra);
        let _ = fs::remove_file(&store);
    }

    #[test]
    fn serializacion_tolerante_y_caracteres_especiales() {
        let root = temp_tree("serial");
        let mut index = ProjectIndex::new(root.clone());
        let entry = IndexEntry {
            rel_path: "con espacio/tab\t#raro.md".to_owned(),
            mtime_nanos: 42,
            size: 7,
            headings: vec![IndexHeading {
                level: 3,
                text: "Con \ttab y %".to_owned(),
                anchor: "con-3".to_owned(),
                line: 5,
            }],
            links: vec!["a b.md#frag".to_owned()],
        };
        index.entries.insert(entry.rel_path.clone(), entry.clone());
        let text = index.serialize();
        let parsed = ProjectIndex::parse(&text, &root).expect("roundtrip");
        assert_eq!(parsed.get(&entry.rel_path), Some(&entry));

        // Línea corrupta: se ignora sin tumbar el resto (§45: tolerante).
        let mut lineas: Vec<&str> = text.lines().collect();
        lineas.insert(3, "esta línea no\tsigue el\tformato");
        let parsed = ProjectIndex::parse(&lineas.join("\n"), &root).expect("tolerante");
        assert_eq!(parsed.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn los_ignorados_por_gitignore_no_se_indexan_14_6() {
        let root = temp_tree("ignore");
        fs::write(root.join(".gitignore"), "generado.md\ncache/\n").unwrap();
        fs::write(root.join("generado.md"), "# G\n").unwrap();
        fs::write(root.join("doc.md"), "# D\n").unwrap();
        fs::create_dir(root.join("cache")).unwrap();
        fs::write(root.join("cache/oculto.md"), "# C\n").unwrap();

        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        assert_eq!(index.len(), 1, "solo doc.md (§14.6)");
        assert!(index.get("doc.md").is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rendimiento_miles_de_archivos_47_5() {
        let root = temp_tree("miles");
        for i in 0..1000 {
            fs::write(root.join(format!("doc-{i:04}.md")), format!("# Doc {i}\n")).unwrap();
        }
        let mut index = ProjectIndex::new(root.clone());
        let inicio = std::time::Instant::now();
        let stats = index.refresh().unwrap();
        // §47.5: el índice de un proyecto de miles de archivos se construye en
        // un tiempo modesto (y en la app, fuera del hilo de UI, §47.1).
        assert_eq!(stats.reindexed, 1000);
        assert!(
            inicio.elapsed() < std::time::Duration::from_secs(10),
            "1000 archivos tardaron {:?}",
            inicio.elapsed()
        );

        // Refresco incremental sobre miles de archivos: solo lo tocado.
        touch(&root.join("doc-0500.md"));
        let inicio = std::time::Instant::now();
        let stats = index.refresh().unwrap();
        assert_eq!(
            stats,
            RefreshStats {
                reindexed: 1,
                removed: 0,
                unchanged: 999
            }
        );
        assert!(
            inicio.elapsed() < std::time::Duration::from_secs(5),
            "refresco incremental tardó {:?}",
            inicio.elapsed()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn default_path_fuera_del_proyecto_45_1() {
        let root = temp_tree("path");
        let path = ProjectIndex::default_path(&root).unwrap();
        assert!(
            !path.starts_with(&root),
            "el índice vive fuera del proyecto (§45.1)"
        );
        assert!(path.to_string_lossy().ends_with(".index"));
        let _ = fs::remove_dir_all(&root);
    }
}
