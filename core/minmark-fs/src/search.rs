//! Búsqueda en el proyecto y reemplazo masivo deshacible (ISSUE-028, §40.2).
//!
//! - §40.2: búsqueda sobre el proyecto siguiendo el modelo funcional de
//!   VS Code — scope por defecto Markdown (`.md`/`.markdown`) ampliable a
//!   otras extensiones o a «todos» —, con contexto de línea por coincidencia
//!   para la UI y **reemplazo masivo deshacible en una sola acción**.
//! - §14.6: los archivos ignorados por el `.gitignore` de la raíz se
//!   excluyen por defecto de la búsqueda (reglas de [`crate::IgnorePatterns`],
//!   ISSUE-023/025).
//! - §12.5: solo UTF-8; un archivo que no lo es se salta con nota
//!   ([`SkipReason::NotUtf8`]) y nunca rompe la búsqueda.
//! - §47.1/§47.5: la búsqueda incremental corre en el hilo del consumidor
//!   (la UI la lanza en una tarea aparte) y emite resultados parciales por
//!   lotes sobre un canal; la cancelación es cooperativa y sin efectos
//!   parciales visibles.
//! - §48: guardado atómico ([`crate::write_atomic`]) para cada escritura del
//!   reemplazo; §3.7: all-or-nothing con rollback (estilo [`crate::ops`]) y
//!   resumen de diagnóstico.
//! - §12.7: si un archivo cambió en disco desde que se buscó, el reemplazo se
//!   aborta completo ([`ReplaceError::Stale`]); nunca sobrescritura
//!   silenciosa.
//!
//! El matching reutiliza [`Matcher`] de `minmark_document::search` (contrato
//! compartido con ISSUE-027): Match Case / Whole Word / Regex con la misma
//! semántica que la búsqueda del archivo actual (§40.1).

use crate::index::ProjectIndex;
use crate::ignore::IgnorePatterns;
use crate::paths::{canonicalize, walk_tree};
use crate::{relative_path, resolve_in_project, write_atomic, FsError};
use minmark_document::{Matcher, TextMatch};
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

/// Archivos por lote antes de emitir resultados parciales (§47.5).
const BATCH_FILES: usize = 32;

/// Coincidencias por lote antes de emitir resultados parciales (§47.5).
const BATCH_MATCHES: usize = 256;

/// Alcance de la búsqueda (§40.2: por defecto solo `.md`, ampliable).
///
/// La comparación de extensiones es exacta y sensible a mayúsculas (mismo
/// criterio que [`crate::is_markdown_path`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchScope {
    /// `None` = «todos los archivos»; `Some(lista)` = solo esas extensiones
    /// (sin punto inicial).
    exts: Option<Vec<String>>,
}

impl Default for SearchScope {
    /// Alcance por defecto §40.2: solo Markdown.
    fn default() -> Self {
        Self::markdown()
    }
}

impl SearchScope {
    /// Solo Markdown (`.md` y `.markdown`), el alcance por defecto §40.2.
    pub fn markdown() -> Self {
        Self {
            exts: Some(vec!["md".to_owned(), "markdown".to_owned()]),
        }
    }

    /// «Todos los archivos»: sin filtro de extensión.
    pub fn all() -> Self {
        Self { exts: None }
    }

    /// Extensiones personalizadas (con o sin punto inicial; se normalizan).
    /// Una lista vacía no coincide con ningún archivo.
    pub fn extensions<'a>(exts: impl IntoIterator<Item = &'a str>) -> Self {
        let normalized = exts
            .into_iter()
            .map(|e| e.trim().trim_start_matches('.').to_owned())
            .filter(|e| !e.is_empty())
            .collect();
        Self { exts: Some(normalized) }
    }

    /// ¿El archivo `path` entra en el alcance?
    pub fn includes(&self, path: &Path) -> bool {
        match &self.exts {
            None => true,
            Some(list) => path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| list.iter().any(|want| want == ext)),
        }
    }
}

/// Una coincidencia con su contexto de línea (§40.2: «contexto»).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    /// Número de línea de la coincidencia, **1-based**.
    pub line: usize,
    /// Columna del inicio de la coincidencia dentro de `line_text`,
    /// **1-based y en caracteres Unicode** (§8.6: la columna cuenta
    /// caracteres): `Position::new(line - 1, col - 1)` sitúa el cursor.
    pub col: usize,
    /// Texto completo de la línea, sin terminador (se recorta el `\r` de los
    /// archivos CRLF, §12.6).
    pub line_text: String,
    /// Offset byte `[start, end)` de la coincidencia en el contenido total
    /// del archivo (clave de selección para el reemplazo).
    pub start: usize,
    /// Fin byte exclusivo de la coincidencia.
    pub end: usize,
}

/// Coincidencias de un archivo, en orden de aparición.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatches {
    /// Path relativo a la raíz del proyecto (§17, forma posix).
    pub rel_path: String,
    pub matches: Vec<LineMatch>,
}

/// Motivo por el que un archivo se saltó (§12.5: con nota, nunca rompe).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// El archivo no es UTF-8 válido (§12.5).
    NotUtf8,
    /// El archivo no se pudo leer (borrado en caliente, permisos…).
    Unreadable,
}

/// Archivo saltado durante la búsqueda, con su nota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub rel_path: String,
    pub reason: SkipReason,
}

/// Diagnóstico de una búsqueda (§3.7: observable).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchStats {
    /// Archivos intentados (incluye los saltados).
    pub files_scanned: usize,
    /// Archivos con al menos una coincidencia.
    pub files_matched: usize,
    /// Coincidencias totales.
    pub matches: usize,
    /// Archivos saltados con nota (§12.5).
    pub skipped: Vec<SkippedFile>,
}

/// Resultado completo de una búsqueda síncrona ([`search`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchReport {
    pub files: Vec<FileMatches>,
    pub stats: SearchStats,
}

/// Evento de la búsqueda incremental (§47.5: resultados parciales).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchEvent {
    /// Lote de resultados parciales, en orden de aparición.
    Batch(Vec<FileMatches>),
    /// Nota de archivo saltado (§12.5); también acumulada en
    /// [`SearchStats::skipped`].
    Skipped(SkippedFile),
    /// Fin de la búsqueda (no se emite si hubo cancelación).
    Done(SearchStats),
}

/// Origen de los candidatos de búsqueda (§40.2 sobre el índice ISSUE-025).
#[derive(Debug, Clone, Copy)]
pub enum SearchSource<'a> {
    /// Recorrido del árbol con [`crate::walk_tree`], excluyendo los ignorados
    /// por `.gitignore` (§14.6). Cubre cualquier alcance.
    Project,
    /// Entradas del [`ProjectIndex`] (ISSUE-025, §45): ya filtradas por
    /// `.gitignore` y solo Markdown — para alcances amplios use
    /// [`SearchSource::Project`].
    Index(&'a ProjectIndex),
}

/// Búsqueda completa y síncrona (tests y planificación).
///
/// Equivale a [`search_streaming`] recogiendo los lotes; la UI usa la forma
/// incremental (§47.5).
pub fn search(
    root: &Path,
    source: SearchSource<'_>,
    matcher: &Matcher,
    scope: &SearchScope,
) -> Result<SearchReport, FsError> {
    let never = AtomicBool::new(false);
    let mut files = Vec::new();
    let stats = drive(root, source, matcher, scope, &never, |event| {
        if let SearchEvent::Batch(batch) = event {
            files.extend(batch);
        }
        true
    })?;
    Ok(SearchReport { files, stats })
}

/// Búsqueda incremental (§47.1/§47.5): corre en el hilo del consumidor (la
/// UI la lanza en una tarea aparte) y emite [`SearchEvent::Batch`] por lotes,
/// [`SearchEvent::Skipped`] al saltar un archivo (§12.5) y
/// [`SearchEvent::Done`] al terminar con las estadísticas.
///
/// `cancel` es cooperativo: al fijarse la búsqueda se detiene lo antes
/// posible **sin** emitir `Done` (el consumidor relanzó). Si el canal cae
/// (consumidor canceló), se detiene igualmente sin error.
pub fn search_streaming(
    root: &Path,
    source: SearchSource<'_>,
    matcher: &Matcher,
    scope: &SearchScope,
    cancel: &AtomicBool,
    tx: &Sender<SearchEvent>,
) -> Result<SearchStats, FsError> {
    let stats = drive(root, source, matcher, scope, cancel, |event| {
        tx.send(event).is_ok()
    })?;
    if !cancel.load(Ordering::Relaxed) {
        let _ = tx.send(SearchEvent::Done(stats.clone()));
    }
    Ok(stats)
}

/// Motor común: enumera candidatos y emite eventos por lotes.
///
/// `emit` devuelve `false` para detenerse (canal caído). La cancelación por
/// `cancel` o por `emit` no es un error: se devuelven las estadísticas
/// parciales.
fn drive(
    root: &Path,
    source: SearchSource<'_>,
    matcher: &Matcher,
    scope: &SearchScope,
    cancel: &AtomicBool,
    mut emit: impl FnMut(SearchEvent) -> bool,
) -> Result<SearchStats, FsError> {
    let root = canonicalize(root).map_err(|source| FsError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let candidates = candidates(&root, source, scope)?;
    let mut stats = SearchStats::default();
    let mut batch: Vec<FileMatches> = Vec::new();
    let mut batch_matches = 0usize;
    for path in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Ok(stats);
        }
        stats.files_scanned += 1;
        let rel_path = relative_path(&root, &path);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(source) => {
                let reason = if source.kind() == std::io::ErrorKind::InvalidData {
                    SkipReason::NotUtf8
                } else {
                    SkipReason::Unreadable
                };
                stats.skipped.push(SkippedFile { rel_path, reason });
                let keep = emit(SearchEvent::Skipped(
                    stats.skipped.last().expect("recién insertado").clone(),
                ));
                if !keep {
                    return Ok(stats);
                }
                continue;
            }
        };
        let matches = line_matches(&text, matcher);
        if matches.is_empty() {
            continue;
        }
        stats.files_matched += 1;
        stats.matches += matches.len();
        batch_matches += matches.len();
        batch.push(FileMatches { rel_path, matches });
        if batch.len() >= BATCH_FILES || batch_matches >= BATCH_MATCHES {
            if !emit(SearchEvent::Batch(std::mem::take(&mut batch))) {
                return Ok(stats);
            }
            batch_matches = 0;
        }
    }
    if !batch.is_empty() {
        emit(SearchEvent::Batch(batch));
    }
    Ok(stats)
}

/// Candidatos de la búsqueda: entradas del índice (ISSUE-025) o recorrido del
/// árbol con `.gitignore` (§14.6), filtrados por el alcance (§40.2).
fn candidates(
    root: &Path,
    source: SearchSource<'_>,
    scope: &SearchScope,
) -> Result<Vec<PathBuf>, FsError> {
    match source {
        SearchSource::Index(index) => Ok(index
            .entries()
            .map(|entry| entry.rel_path.as_str())
            .filter(|rel_path| scope.includes(Path::new(rel_path)))
            .map(|rel_path| root.join(rel_path))
            .collect()),
        SearchSource::Project => {
            let ignore = IgnorePatterns::from_root(root);
            let mut out = Vec::new();
            for entry in walk_tree(root)? {
                if entry.is_dir {
                    continue;
                }
                let rel_path = relative_path(root, &entry.path);
                // §14.6: los ignorados por `.gitignore` quedan fuera de la
                // búsqueda global (visibles en el explorador, nunca aquí).
                if ignore.is_ignored(Path::new(&rel_path), false) {
                    continue;
                }
                if scope.includes(&entry.path) {
                    out.push(entry.path);
                }
            }
            Ok(out)
        }
    }
}

/// Coincidencias de `text` con contexto de línea (§40.2).
fn line_matches(text: &str, matcher: &Matcher) -> Vec<LineMatch> {
    let starts = line_starts(text);
    let mut out = Vec::new();
    for hit in matcher.find_all_in_text(text) {
        let (line, line_start) = locate_line(&starts, hit.start);
        let line_end = line_end(text, &starts, line);
        let line_text = text[line_start..line_end].trim_end_matches('\r');
        let col = text[line_start..hit.start].chars().count() + 1;
        out.push(LineMatch {
            line,
            col,
            line_text: line_text.to_owned(),
            start: hit.start,
            end: hit.end,
        });
    }
    out
}

/// Offsets byte del inicio de cada línea (la 1-based `n` empieza en `n-1`).
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// `(número de línea 1-based, offset byte de su inicio)` para un offset byte.
fn locate_line(starts: &[usize], offset: usize) -> (usize, usize) {
    let index = starts.partition_point(|start| *start <= offset).saturating_sub(1);
    (index + 1, starts[index])
}

/// Fin byte de la línea `line` (1-based), sin el `\n`.
fn line_end(text: &str, starts: &[usize], line: usize) -> usize {
    match starts.get(line) {
        Some(next) => next.saturating_sub(1),
        None => text.len(),
    }
}

// --- Reemplazo masivo deshacible (§40.2) ------------------------------------

/// Selección de reemplazo de un archivo: rangos byte `[start, end)` tal y
/// como devolvió la búsqueda (las coincidencias excluidas en la UI
/// simplemente no viajan aquí).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceFile {
    pub rel_path: String,
    pub ranges: Vec<(usize, usize)>,
}

/// Resumen de un reemplazo (§3.7 diagnóstico): ficheros y coincidencias
/// afectadas. Tras [`MassReplace::undo`] describe lo revertido.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplaceSummary {
    /// Paths relativos (§17) de los archivos afectados, en orden.
    pub files: Vec<String>,
    /// Coincidencias afectadas.
    pub matches: usize,
}

/// Plan de reemplazo validado (estilo [`crate::ops::plan_path_change`]): no
/// tiene efectos hasta [`apply_replace`].
#[derive(Debug)]
pub struct ReplacePlan {
    files: Vec<PlannedFile>,
    summary: ReplaceSummary,
}

#[derive(Debug)]
struct PlannedFile {
    abs: PathBuf,
    /// Contenido en disco al planificar; con él se detecta §12.7.
    original: String,
    /// Contenido resultante de aplicar el reemplazo.
    replaced: String,
}

/// Error de reemplazo masivo.
#[derive(Debug)]
pub enum ReplaceError {
    /// El archivo no es UTF-8 válido (§12.5).
    NotUtf8 { path: PathBuf },
    /// El contenido en disco ya no corresponde a los resultados (§12.7):
    /// aborta completo, nunca sobrescritura silenciosa.
    Stale { path: PathBuf },
    /// Error de E/S.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Operación inválida (rangos solapados, ruta fuera del proyecto §48…).
    InvalidOperation(&'static str),
}

impl fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 { path } => {
                write!(f, "{} no es UTF-8 válido (solo se admite UTF-8, §12.5)", path.display())
            }
            Self::Stale { path } => write!(
                f,
                "{} cambió desde la búsqueda: reemplazo cancelado completo (§12.7)",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "error de E/S en {}: {source}", path.display()),
            Self::InvalidOperation(reason) => write!(f, "operación inválida: {reason}"),
        }
    }
}

impl std::error::Error for ReplaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Planifica el reemplazo de `files` (§40.2: individual, por archivo o
/// global — la UI solo filtra qué rangos viajan).
///
/// Cada archivo se re-busca con `matcher` y **todos** los rangos solicitados
/// deben corresponder a una coincidencia viva; si alguno no lo hace el
/// contenido cambió y se aborta con [`ReplaceError::Stale`] (§12.7). El texto
/// de reemplazo se materializa con
/// [`Matcher::expand_replacement`](minmark_document::Matcher) (regex: `$0`…`$9`,
/// `$$`; literal: solo `$$`).
pub fn plan_replace(
    root: &Path,
    matcher: &Matcher,
    template: &str,
    files: &[ReplaceFile],
) -> Result<ReplacePlan, ReplaceError> {
    let root = canonicalize(root).map_err(|source| ReplaceError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut planned = Vec::new();
    let mut summary = ReplaceSummary::default();
    for file in files {
        let selected: BTreeSet<(usize, usize)> = file.ranges.iter().copied().collect();
        if selected.is_empty() {
            continue;
        }
        let abs = resolve_in_project(&root, Path::new(&file.rel_path)).map_err(|_| {
            ReplaceError::InvalidOperation("la ruta seleccionada sale del proyecto (§48)")
        })?;
        let original = read_text(&abs)?;
        let hits: Vec<TextMatch> = matcher
            .find_all_in_text(&original)
            .into_iter()
            .filter(|hit| selected.contains(&(hit.start, hit.end)))
            .collect();
        if hits.len() != selected.len() {
            // Un rango solicitado ya no es coincidencia viva: contenido
            // cambiado o resultados obsoletos (§12.7).
            return Err(ReplaceError::Stale { path: abs });
        }
        let mut replaced = String::with_capacity(original.len());
        let mut at = 0usize;
        let mut count = 0usize;
        for hit in &hits {
            if hit.start < at {
                return Err(ReplaceError::InvalidOperation("rangos solapados"));
            }
            replaced.push_str(&original[at..hit.start]);
            replaced.push_str(&matcher.expand_replacement(&original, hit, template));
            at = hit.end;
            count += 1;
        }
        replaced.push_str(&original[at..]);
        summary.files.push(file.rel_path.clone());
        summary.matches += count;
        planned.push(PlannedFile {
            abs,
            original,
            replaced,
        });
    }
    Ok(ReplacePlan { files: planned, summary })
}

impl ReplacePlan {
    /// Resumen previo a la escritura (§3.7 diagnóstico).
    pub fn summary(&self) -> ReplaceSummary {
        self.summary.clone()
    }
}

/// Aplica un plan con escritura atómica por archivo (§48) y all-or-nothing:
/// ante **cualquier** fallo se restaura todo lo ya escrito (rollback estilo
/// [`crate::ops::apply_path_change`]) y el error se propaga sin efectos.
///
/// Antes de escribir cada archivo se re-verifica su contenido (§12.7): un
/// cambio en disco desde el plan aborta el reemplazo completo. Devuelve el
/// registro [`MassReplace`] con los contenidos originales para
/// [`MassReplace::undo`] — **un solo undo revierte todos los archivos** (§40.2).
pub fn apply_replace(plan: &ReplacePlan) -> Result<MassReplace, ReplaceError> {
    let mut originals: Vec<(PathBuf, String)> = Vec::new();
    for planned in &plan.files {
        let current = match read_text(&planned.abs) {
            Ok(current) => current,
            Err(error) => {
                rollback(&originals);
                return Err(error);
            }
        };
        if current != planned.original {
            rollback(&originals);
            return Err(ReplaceError::Stale { path: planned.abs.clone() });
        }
        if let Err(source) = write_atomic(&planned.abs, planned.replaced.as_bytes()) {
            rollback(&originals);
            return Err(ReplaceError::Io {
                path: planned.abs.clone(),
                source,
            });
        }
        originals.push((planned.abs.clone(), current));
    }
    Ok(MassReplace {
        summary: plan.summary.clone(),
        originals,
    })
}

/// Registro de un reemplazo masivo aplicado (§40.2): conserva **todos** los
/// contenidos originales para revertirlo de una sola vez.
#[derive(Debug)]
pub struct MassReplace {
    summary: ReplaceSummary,
    /// Contenido previo a la escritura, por archivo, en orden de aplicación.
    originals: Vec<(PathBuf, String)>,
}

impl MassReplace {
    /// Resumen del reemplazo aplicado (§3.7 diagnóstico).
    pub fn summary(&self) -> &ReplaceSummary {
        &self.summary
    }

    /// Revierte **todos** los archivos afectados en una sola acción (§40.2),
    /// con escritura atómica (§48) y all-or-nothing con rollback: si una
    /// restauración falla, las ya hechas vuelven al contenido reemplazado.
    ///
    /// Es idempotente por contenido: reejecutarla reescribe los mismos
    /// originales. Devuelve el resumen de lo revertido.
    pub fn undo(&self) -> Result<ReplaceSummary, ReplaceError> {
        let mut journal: Vec<(PathBuf, String)> = Vec::new();
        for (path, original) in &self.originals {
            let current = match read_text(path) {
                Ok(current) => current,
                Err(error) => {
                    rollback(&journal);
                    return Err(error);
                }
            };
            if let Err(source) = write_atomic(path, original.as_bytes()) {
                rollback(&journal);
                return Err(ReplaceError::Io {
                    path: path.clone(),
                    source,
                });
            }
            journal.push((path.clone(), current));
        }
        Ok(self.summary.clone())
    }
}

/// Rollback de un journal `(ruta, contenido previo)` (estilo [`crate::ops`]).
fn rollback(journal: &[(PathBuf, String)]) {
    for (path, content) in journal {
        let _ = write_atomic(path, content.as_bytes());
    }
}

/// Lectura UTF-8 estricta (§12.5) con error tipado.
fn read_text(path: &Path) -> Result<String, ReplaceError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(source) if source.kind() == std::io::ErrorKind::InvalidData => {
            Err(ReplaceError::NotUtf8 { path: path.to_path_buf() })
        }
        Err(source) => Err(ReplaceError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_document::SearchOptions;
    use std::sync::mpsc::channel;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    /// Directorio temporal de test, eliminado al salir (mismo patrón que
    /// [`crate::paths`]).
    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("minmark-search-{}-{}", std::process::id(), nanos));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, rel: &str, content: impl AsRef<[u8]>) {
            let path = self.0.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }

        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.0.join(rel)).unwrap()
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn matcher(pattern: &str, options: SearchOptions) -> Matcher {
        Matcher::new(pattern, options).unwrap()
    }

    /// §40.2: el alcance por defecto es solo Markdown y es ampliable.
    #[test]
    fn alcance_por_defecto_solo_markdown_y_ampliable() {
        let por_defecto = SearchScope::default();
        assert!(por_defecto.includes(Path::new("a/b.md")));
        assert!(por_defecto.includes(Path::new("a/b.markdown")));
        assert!(!por_defecto.includes(Path::new("a/b.txt")));
        assert!(!por_defecto.includes(Path::new("a/sin_extension")));

        let todos = SearchScope::all();
        assert!(todos.includes(Path::new("a/b.txt")));
        assert!(todos.includes(Path::new("a/sin_extension")));

        let custom = SearchScope::extensions([".txt", "log", "  "]);
        assert!(custom.includes(Path::new("notas.txt")));
        assert!(custom.includes(Path::new("run.log")));
        assert!(!custom.includes(Path::new("a.md")));
        assert!(!SearchScope::extensions(Vec::<&str>::new()).includes(Path::new("a.md")));

        // El alcance filtra los candidatos, no solo el matcher.
        let dir = TestDir::new();
        dir.write("uno.md", "objetivo\n");
        dir.write("dos.txt", "objetivo\n");
        let found = search(
            &dir.0,
            SearchSource::Project,
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::default(),
        )
        .unwrap();
        assert_eq!(
            found.files.iter().map(|f| f.rel_path.as_str()).collect::<Vec<_>>(),
            ["uno.md"]
        );
    }

    /// §14.6: los ignorados por `.gitignore` se excluyen de la búsqueda.
    #[test]
    fn excluye_gitignore_por_defecto_14_6() {
        let dir = TestDir::new();
        dir.write(".gitignore", "generado/\n*.bak\n");
        dir.write("visible.md", "objetivo aquí\n");
        dir.write("generado/salida.md", "objetivo aquí\n");
        dir.write("copias/respaldo.bak", "objetivo aquí\n");
        dir.write("generado/no_importa.txt", "objetivo aquí\n");

        let found = search(
            &dir.0,
            SearchSource::Project,
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::all(),
        )
        .unwrap();
        assert_eq!(
            found.files.iter().map(|f| f.rel_path.as_str()).collect::<Vec<_>>(),
            ["visible.md"]
        );
        assert_eq!(found.stats.files_matched, 1);
    }

    /// §40.2 «contexto»: línea 1-based, columna en caracteres, texto de línea
    /// y offsets byte.
    #[test]
    fn resultados_con_contexto_de_linea_y_offsets() {
        let dir = TestDir::new();
        let contenido = "primera línea\nsegunda objetivo tercera\n\núltima\n";
        dir.write("ctx.md", contenido);
        let found = search(
            &dir.0,
            SearchSource::Project,
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::default(),
        )
        .unwrap();
        let hits = &found.files[0].matches;
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.line, 2);
        // «segunda » = 8 caracteres → columna 9 (1-based, Unicode).
        assert_eq!(hit.col, 9);
        assert_eq!(hit.line_text, "segunda objetivo tercera");
        let esperado = contenido.find("objetivo").unwrap();
        assert_eq!((hit.start, hit.end), (esperado, esperado + "objetivo".len()));

        // CRLF: el `\r` nunca forma parte del contexto de línea (§12.6).
        dir.write("crlf.md", "uno\r\nobjetivo dos\r\n");
        let crlf = search(
            &dir.0,
            SearchSource::Project,
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::default(),
        )
        .unwrap();
        let hit = &crlf.files[0].matches[0];
        assert_eq!(hit.line_text, "objetivo dos");
        assert_eq!((hit.line, hit.col), (2, 1));
    }

    /// §40.2: Match Case / Whole Word / Regex reutilizan el [`Matcher`] de
    /// ISSUE-027 (misma semántica que §40.1).
    #[test]
    fn opciones_case_word_y_regex_usan_matcher_40_2() {
        let dir = TestDir::new();
        dir.write("opciones.md", "Foo foo\nscatter cat catalog\n");

        let case = SearchOptions { case_sensitive: true, ..Default::default() };
        let found = search(&dir.0, SearchSource::Project, &matcher("Foo", case), &SearchScope::default()).unwrap();
        assert_eq!(found.stats.matches, 1);
        assert_eq!(found.files[0].matches[0].line, 1);

        let word = SearchOptions { whole_word: true, ..Default::default() };
        let found = search(&dir.0, SearchSource::Project, &matcher("cat", word), &SearchScope::default()).unwrap();
        assert_eq!(found.stats.matches, 1);
        assert_eq!(found.files[0].matches[0].line, 2);

        let regex = SearchOptions { regex: true, ..Default::default() };
        let re = matcher(r"(\w+)-(\w+)", regex);
        dir.write("regex.md", "alpha-beta gamma-delta\n");
        let found = search(&dir.0, SearchSource::Project, &re, &SearchScope::default()).unwrap();
        let hits = &found.files[0].matches;
        assert_eq!(hits.len(), 2);
        // Los grupos se materializan sobre el texto fuente (contrato ISSUE-027).
        let texto = dir.read("regex.md");
        let m = re.find_all_in_text(&texto);
        assert_eq!(re.expand_replacement(&texto, &m[0], "$2/$1"), "beta/alpha");
    }

    /// §12.5: los archivos no UTF-8 se saltan con nota y no rompen nada.
    #[test]
    fn no_utf8_se_salta_con_nota_12_5() {
        let dir = TestDir::new();
        dir.write("a.md", "objetivo uno\n");
        dir.write("binario.md", [0xFFu8, 0xFE, 0x00, b'x']);
        dir.write("c.md", "objetivo dos\n");

        let found = search(
            &dir.0,
            SearchSource::Project,
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::default(),
        )
        .unwrap();
        assert_eq!(found.stats.files_scanned, 3);
        assert_eq!(found.stats.files_matched, 2);
        assert_eq!(
            found.stats.skipped,
            vec![SkippedFile {
                rel_path: "binario.md".to_owned(),
                reason: SkipReason::NotUtf8
            }]
        );
    }

    /// §40.2 sobre el índice (ISSUE-025): los candidatos son sus entradas.
    #[test]
    fn search_index_usa_las_entradas_del_indice() {
        let dir = TestDir::new();
        dir.write("en_indice.md", "objetivo\n");
        dir.write("fuera.txt", "objetivo\n");
        let mut index = ProjectIndex::new(dir.0.clone());
        index.refresh().unwrap();

        let found = search(
            &dir.0,
            SearchSource::Index(&index),
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::default(),
        )
        .unwrap();
        assert_eq!(
            found.files.iter().map(|f| f.rel_path.as_str()).collect::<Vec<_>>(),
            ["en_indice.md"]
        );
    }

    /// §40.2: reemplazo individual, por archivo y global sobre el mismo motor.
    #[test]
    fn reemplazo_individual_por_fichero_y_global() {
        let dir = TestDir::new();
        dir.write("uno.md", "x uno\nx dos\n");
        dir.write("dos.md", "x tres\n");
        let matcher = matcher("x", SearchOptions::default());

        // Individual: un solo rango (la primera coincidencia de `uno.md`).
        let plan = plan_replace(
            &dir.0,
            &matcher,
            "I",
            &[ReplaceFile { rel_path: "uno.md".to_owned(), ranges: vec![(0, 1)] }],
        )
        .unwrap();
        assert_eq!(plan.summary(), ReplaceSummary { files: vec!["uno.md".into()], matches: 1 });
        let applied = apply_replace(&plan).unwrap();
        assert_eq!(dir.read("uno.md"), "I uno\nx dos\n");
        assert_eq!(*applied.summary(), plan.summary());
        applied.undo().unwrap();

        // Por archivo: todas las coincidencias de `uno.md`, ninguna de `dos.md`.
        let plan = plan_replace(
            &dir.0,
            &matcher,
            "I",
            &[ReplaceFile {
                rel_path: "uno.md".to_owned(),
                ranges: vec![(0, 1), (6, 7)],
            }],
        )
        .unwrap();
        let por_fichero = apply_replace(&plan).unwrap();
        assert_eq!(dir.read("uno.md"), "I uno\nI dos\n");
        assert_eq!(dir.read("dos.md"), "x tres\n");
        por_fichero.undo().unwrap();

        // Global: todos los archivos seleccionados de una vez.
        let plan = plan_replace(
            &dir.0,
            &matcher,
            "G",
            &[
                ReplaceFile { rel_path: "uno.md".to_owned(), ranges: vec![(0, 1), (6, 7)] },
                ReplaceFile { rel_path: "dos.md".to_owned(), ranges: vec![(0, 1)] },
            ],
        )
        .unwrap();
        let applied = apply_replace(&plan).unwrap();
        assert_eq!(dir.read("uno.md"), "G uno\nG dos\n");
        assert_eq!(dir.read("dos.md"), "G tres\n");
        assert_eq!(
            *applied.summary(),
            ReplaceSummary {
                files: vec!["uno.md".into(), "dos.md".into()],
                matches: 3
            }
        );
    }

    /// §40.2: **un solo undo** revierte todos los archivos afectados.
    #[test]
    fn undo_masivo_revierte_todos_los_ficheros_de_una_vez() {
        let dir = TestDir::new();
        let originales = [
            ("uno.md", "x uno\nx dos\n"),
            ("dos.md", "x tres\n"),
            ("sub/tres.md", "sin coincidencias\nx final\n"),
        ];
        for (rel, contenido) in originales {
            dir.write(rel, contenido);
        }
        let matcher = matcher("x", SearchOptions::default());
        let plan = plan_replace(
            &dir.0,
            &matcher,
            "Y",
            &[
                ReplaceFile { rel_path: "uno.md".to_owned(), ranges: vec![(0, 1), (6, 7)] },
                ReplaceFile { rel_path: "dos.md".to_owned(), ranges: vec![(0, 1)] },
                ReplaceFile { rel_path: "sub/tres.md".to_owned(), ranges: vec![(18, 19)] },
            ],
        )
        .unwrap();
        let applied = apply_replace(&plan).unwrap();
        assert_eq!(dir.read("uno.md"), "Y uno\nY dos\n");
        assert_eq!(dir.read("dos.md"), "Y tres\n");
        assert_eq!(dir.read("sub/tres.md"), "sin coincidencias\nY final\n");

        // Una sola acción: los tres ficheros vuelven byte a byte.
        let resumen = applied.undo().unwrap();
        assert_eq!(resumen, applied.summary().clone());
        for (rel, contenido) in originales {
            assert_eq!(dir.read(rel), contenido, "{rel} no se restauró");
        }
    }

    /// §3.7/§48: all-or-nothing — un fallo de escritura a mitad de lista
    /// restaura los archivos ya escritos y no deja nada a medias.
    #[test]
    fn apply_es_all_or_nothing_ante_error_de_escritura() {
        let dir = TestDir::new();
        dir.write("a.md", "x a\n");
        dir.write("b.md", "x b\n");
        let matcher = matcher("x", SearchOptions::default());
        let plan = plan_replace(
            &dir.0,
            &matcher,
            "Z",
            &[
                ReplaceFile { rel_path: "a.md".to_owned(), ranges: vec![(0, 1)] },
                ReplaceFile { rel_path: "b.md".to_owned(), ranges: vec![(0, 1)] },
            ],
        )
        .unwrap();

        // `b.md` pasa a ser un directorio no vacío: la lectura de `apply`
        // falla DESPUÉS de haber escrito `a.md` → rollback completo.
        std::fs::remove_file(dir.0.join("b.md")).unwrap();
        std::fs::create_dir(dir.0.join("b.md")).unwrap();
        std::fs::write(dir.0.join("b.md/dentro"), "y").unwrap();

        let error = apply_replace(&plan).unwrap_err();
        assert!(matches!(error, ReplaceError::Io { .. }), "{error}");
        assert_eq!(dir.read("a.md"), "x a\n", "rollback no restauró a.md");
    }

    /// §12.7/§3.7: si el contenido cambió desde la búsqueda, el reemplazo se
    /// aborta completo (sin escritura previa visible).
    #[test]
    fn resultados_obsoletos_no_se_sobrescriben_12_7() {
        let dir = TestDir::new();
        dir.write("a.md", "x a\n");
        dir.write("b.md", "x b\n");
        let matcher = matcher("x", SearchOptions::default());
        let plan = plan_replace(
            &dir.0,
            &matcher,
            "Z",
            &[
                ReplaceFile { rel_path: "a.md".to_owned(), ranges: vec![(0, 1)] },
                ReplaceFile { rel_path: "b.md".to_owned(), ranges: vec![(0, 1)] },
            ],
        )
        .unwrap();

        // Cambio externo en `b.md` tras planificar (§12.7).
        dir.write("b.md", "x b\nx cambiado\n");

        let error = apply_replace(&plan).unwrap_err();
        assert!(matches!(error, ReplaceError::Stale { .. }), "{error}");
        assert_eq!(dir.read("a.md"), "x a\n");
        assert_eq!(dir.read("b.md"), "x b\nx cambiado\n");

        // Un rango que ya no es coincidencia viva tampoco se aplica.
        let error = plan_replace(
            &dir.0,
            &matcher,
            "Z",
            &[ReplaceFile { rel_path: "a.md".to_owned(), ranges: vec![(0, 1), (5, 6)] }],
        )
        .unwrap_err();
        assert!(matches!(error, ReplaceError::Stale { .. }), "{error}");
    }

    /// §47.5: ~1000 archivos se recorren sin bloquear, con resultados
    /// parciales por lotes y cierre con `Done`.
    #[test]
    fn busqueda_incremental_sobre_1000_ficheros() {
        let dir = TestDir::new();
        for d in 0..10 {
            for f in 0..100 {
                let mut contenido = String::new();
                for linea in 0..10 {
                    if linea % 4 == 0 {
                        contenido.push_str("línea con objetivo\n");
                    } else {
                        contenido.push_str(&format!("línea {d}/{f}/{linea}\n"));
                    }
                }
                dir.write(&format!("dir{d}/f{f:03}.md"), contenido);
            }
        }
        let (tx, rx) = channel();
        let cancel = AtomicBool::new(false);
        let root = dir.0.clone();
        let inicio = Instant::now();
        let hilo = std::thread::spawn(move || {
            search_streaming(
                &root,
                SearchSource::Project,
                &Matcher::new("objetivo", SearchOptions::default()).unwrap(),
                &SearchScope::default(),
                &cancel,
                &tx,
            )
            .unwrap()
        });

        let mut lotes = 0usize;
        let mut primer_evento_fue_lote = false;
        let mut done = None;
        while let Ok(event) = rx.recv() {
            match event {
                SearchEvent::Batch(_) => {
                    if done.is_none() && lotes == 0 {
                        primer_evento_fue_lote = true;
                    }
                    lotes += 1;
                }
                SearchEvent::Skipped(_) => panic!("ningún archivo debería saltarse"),
                SearchEvent::Done(stats) => done = Some(stats),
            }
        }
        let stats = hilo.join().unwrap();
        let elapsed = inicio.elapsed();

        assert!(primer_evento_fue_lote, "los resultados parciales llegan antes de Done");
        assert!(lotes >= 2, "la búsqueda debe emitir lotes intermedios, no uno solo");
        assert_eq!(done, Some(stats.clone()));
        assert_eq!(stats.files_scanned, 1000);
        assert_eq!(stats.files_matched, 1000);
        assert_eq!(stats.matches, 3000);
        assert!(elapsed.as_secs() < 10, "1000 ficheros pequeños deben ser rápidos: {elapsed:?}");
    }

    /// §47.5: la cancelación (relanzar) detiene la búsqueda sin `Done` y sin
    /// efectos parciales.
    #[test]
    fn cancelacion_detiene_la_busqueda_sin_done() {
        let dir = TestDir::new();
        dir.write("a.md", "objetivo\n");
        let (tx, rx) = channel();
        let cancel = AtomicBool::new(true); // cancelada antes de empezar (relanzar)
        let stats = search_streaming(
            &dir.0,
            SearchSource::Project,
            &matcher("objetivo", SearchOptions::default()),
            &SearchScope::default(),
            &cancel,
            &tx,
        )
        .unwrap();
        assert_eq!(stats.files_scanned, 0);
        drop(tx);
        assert!(rx.try_recv().is_err(), "cancelación sin eventos ni Done");
    }
}
