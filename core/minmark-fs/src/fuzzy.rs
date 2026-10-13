//! Búsqueda difusa y Quick Open (ISSUE-029, §41): matcher propio std-only
//! (§4.1; decisión §4.4 registrada en la ISSUE: sin dependencias nuevas) y
//! consultas de la paleta `Ctrl+P` (§39) sobre el [`ProjectIndex`] de
//! ISSUE-025 (§45).
//!
//! Modos de consulta §41 dentro de la misma paleta (detectados por prefijo en
//! [`parse_query`]):
//!
//! - texto → fuzzy sobre los archivos del proyecto;
//! - `:N` → salta a la línea N del archivo activo (o del elegido con un
//!   filtro previo, `docs:42`);
//! - `@heading` → headings del documento actual;
//! - `#heading` → fuzzy sobre los headings de todo el proyecto.
//!
//! Ranking documentado (fórmula exacta en [`quickopen_query`]): score de
//! coincidencia (con bonus de inicio de palabra, prefijo de segmento y
//! camelCase) + recencia de apertura + preferencia por el nombre del archivo
//! sobre la ruta completa − penalización por profundidad; desempate estable
//! por orden del índice. El resultado se corta en [`MAX_RESULTS`] para no
//! bloquear la UI con miles de entradas (§47.5).
//!
//! Los headings del índice §45 guardan su línea y su ancla (§26); al abrir,
//! la línea del heading elegido se re-resuelve sobre el fuente actual con
//! [`find_heading_line`] / [`resolve_heading_line`], de modo que la consulta
//! es pura y no hace E/S por pulsación (§47.5).

use crate::index::ProjectIndex;
use minmark_document::{find_heading_nodes, Heading};
use std::path::Path;

/// Techo de resultados por consulta (§47.5: miles de entradas no deben
/// bloquear la UI).
pub const MAX_RESULTS: usize = 100;

/// Puntos por carácter coincidente.
const BONUS_MATCH: i64 = 10;
/// Puntos extra si el carácter coincidente inicia texto, palabra o segmento
/// (tras un separador de ruta): inicio de palabra / prefijo de segmento.
const BONUS_BOUNDARY: i64 = 20;
/// Puntos extra si el carácter coincide en una transición minúscula→Mayúscula
/// (camelCase).
const BONUS_CAMEL: i64 = 15;
/// Puntos extra si el carácter coincide justo después del anterior.
const BONUS_CONSECUTIVE: i64 = 8;
/// Penalización por carácter saltado entre coincidencias (con tope).
const PENALTY_GAP: i64 = -1;
const PENALTY_GAP_CAP: i64 = -20;
/// Penalización por carácter previo a la primera coincidencia (con tope).
const PENALTY_LEADING: i64 = -1;
const PENALTY_LEADING_CAP: i64 = -10;
/// Preferencia por nombres de archivo sobre rutas completas (§41): se suma
/// cuando la consulta también casa con el nombre del archivo solo.
const BONUS_FILENAME: i64 = 25;
/// Recencia de apertura (§41): `recent[0]` es el más reciente; los 8
/// primeros reciben de +[`RECENCY_MAX`] a +[`RECENCY_STEP`] (los demás, 0).
const RECENCY_MAX: i64 = 40;
const RECENCY_STEP: i64 = 5;
/// Penalización por profundidad de la ruta: −[`PENALTY_DEPTH`] por cada `/`.
const PENALTY_DEPTH: i64 = 5;

/// Extensiones de los tipos documentales visibles por defecto (§14.4):
/// Markdown, imágenes (§18.1) y recursos `.bib`/`.csl`/`.toml`. Es el mismo
/// criterio que el explorador (ISSUE-022); `Show All Files` lo desactiva.
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "md", "markdown", "png", "jpg", "jpeg", "svg", "webp", "gif", "bib", "csl", "toml",
];

/// Modo de la consulta §41, según el prefijo tecleado en la paleta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickOpenMode {
    /// Fuzzy sobre los archivos del proyecto (§41).
    Files,
    /// `:N` (o `filtro:N`): salta a la línea N (§41).
    Line,
    /// `@heading`: headings del documento actual (§41).
    DocHeadings,
    /// `#heading`: headings de todo el proyecto (§41).
    ProjectHeadings,
}

/// Consulta §41 ya interpretada: modo, filtro textual y línea opcional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedQuery<'a> {
    pub mode: QuickOpenMode,
    /// Texto de búsqueda sin el prefijo (para [`Line`], el filtro de archivo).
    pub filter: &'a str,
    /// Línea pedida en modo [`Line`] (1-based; `None` si es inválida).
    pub line: Option<u32>,
}

/// Interpreta la consulta tecleada §41 a partir de su prefijo.
///
/// - `@…` → [`QuickOpenMode::DocHeadings`]; `#…` →
///   [`QuickOpenMode::ProjectHeadings`];
/// - `:N` → [`QuickOpenMode::Line`] sobre el archivo activo;
/// - `filtro:N` (con `N` dígitos tras el **último** `:`) →
///   [`QuickOpenMode::Line`] sobre el archivo elegido por fuzzy `filtro`;
/// - el resto → [`QuickOpenMode::Files`].
pub fn parse_query(query: &str) -> ParsedQuery<'_> {
    if let Some(filter) = query.strip_prefix('@') {
        return ParsedQuery {
            mode: QuickOpenMode::DocHeadings,
            filter,
            line: None,
        };
    }
    if let Some(filter) = query.strip_prefix('#') {
        return ParsedQuery {
            mode: QuickOpenMode::ProjectHeadings,
            filter,
            line: None,
        };
    }
    if query.starts_with(':') {
        let (filter, line) = match split_line_query(query) {
            Some((filter, line)) => (filter, Some(line)),
            None => ("", None),
        };
        return ParsedQuery {
            mode: QuickOpenMode::Line,
            filter,
            line,
        };
    }
    if query.contains(':') {
        if let Some((filter, line)) = split_line_query(query) {
            return ParsedQuery {
                mode: QuickOpenMode::Line,
                filter,
                line: Some(line),
            };
        }
    }
    ParsedQuery {
        mode: QuickOpenMode::Files,
        filter: query,
        line: None,
    }
}

/// Divide una consulta de línea §41 en `(filtro de archivo, línea)`; `None`
/// si no hay una línea válida (dígitos, ≥ 1).
///
/// Acepta `:N`, `N` y `filtro:N` (la línea va tras el último `:`).
fn split_line_query(query: &str) -> Option<(&str, u32)> {
    let body = query.strip_prefix(':').unwrap_or(query);
    if body.is_empty() {
        return None;
    }
    let (filter, digits) = match body.rfind(':') {
        Some(pos) => (&body[..pos], &body[pos + 1..]),
        None => ("", body),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match digits.parse::<u32>() {
        Ok(line) if line >= 1 => Some((filter, line)),
        _ => None,
    }
}

/// Coincidencia difusa encontrada por [`fuzzy_match`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyHit {
    /// Puntuación según los bonus/penalizaciones del módulo (ver docs).
    pub score: i64,
    /// Offsets en bytes de los caracteres coincidentes dentro del texto.
    pub positions: Vec<usize>,
}

/// ¿Es un separador de palabra / inicio de segmento?
fn is_separator(ch: char) -> bool {
    matches!(ch, '/' | '\\' | ' ' | '\t' | '-' | '_' | '.')
}

/// Plegado de mayúsculas/minúsculas (Unicode, std-only, sin normalización de
/// acentos: «í» ≠ «i»).
fn fold(ch: char) -> char {
    ch.to_lowercase().next().unwrap_or(ch)
}

/// Matcher fuzzy: subsecuencia case-insensitive con bonus de inicio de
/// palabra/prefijo de segmento, camelCase y coincidencias consecutivas, y
/// penalizaciones por huecos y arrastre (§4.1).
///
/// La búsqueda es greedy izquierda→derecha (determinista y lineal): cada
/// carácter de la consulta se empareja en la primera posición libre válida.
/// La consulta vacía casa con todo con score 0.
pub fn fuzzy_match(text: &str, query: &str) -> Option<FuzzyHit> {
    let mut positions = Vec::new();
    let mut score: i64 = 0;
    let mut pending = query.chars();
    let mut next = pending.next();
    let mut prev: Option<char> = None;
    let mut prev_end: Option<usize> = None;
    let mut leading: i64 = 0;
    let mut gap: i64 = 0;
    for (offset, ch) in text.char_indices() {
        let Some(needle) = next else { break };
        if fold(ch) == fold(needle) {
            let mut bonus = BONUS_MATCH;
            match prev {
                None => bonus += BONUS_BOUNDARY,
                Some(before) if is_separator(before) => bonus += BONUS_BOUNDARY,
                Some(before) if before.is_lowercase() && ch.is_uppercase() => bonus += BONUS_CAMEL,
                _ => {}
            }
            if prev_end == Some(offset) {
                bonus += BONUS_CONSECUTIVE;
            }
            score += bonus;
            positions.push(offset);
            prev_end = Some(offset + ch.len_utf8());
            next = pending.next();
        } else if positions.is_empty() {
            leading += 1;
        } else {
            gap += 1;
        }
        prev = Some(ch);
    }
    if next.is_some() {
        return None; // no es subsecuencia
    }
    score += (PENALTY_LEADING * leading).max(PENALTY_LEADING_CAP);
    score += (PENALTY_GAP * gap).max(PENALTY_GAP_CAP);
    Some(FuzzyHit { score, positions })
}

/// Resultado de [`quickopen_query`]: una fila de la paleta §41.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Etiqueta visible (ya formateada para el modo).
    pub label: String,
    /// Path relativo a la raíz del proyecto (§17, forma posix).
    pub rel_path: String,
    /// Línea objetivo (1-based) para `:N`; los headings la resuelven al
    /// abrir con [`resolve_heading_line`] sobre el fuente actual.
    pub line: Option<u32>,
    /// Puntuación del ranking documentado en [`quickopen_query`].
    pub score: i64,
    /// Heading objetivo en los modos `@`/`#` (§41).
    pub heading: Option<Heading>,
}

/// ¿`name` es un tipo documental visible por defecto (§14.4)?
///
/// Misma regla que el explorador (ISSUE-022): extensión tras el último `.`
/// (un tallo vacío, `.md`, no cuenta), insensible a mayúsculas.
pub fn is_document_type(name: &str) -> bool {
    let file = basename(name);
    let Some((_, ext)) = file.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) else {
        return false;
    };
    DOCUMENT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
}

/// Nombre del archivo dentro de un path posix (§17).
fn basename(rel_path: &str) -> &str {
    rel_path
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(rel_path)
}

/// Recencia de apertura (§41): `recent[0]` es el más reciente.
fn recency_bonus(recent: &[String], rel_path: &str) -> i64 {
    recent
        .iter()
        .position(|opened| opened == rel_path)
        .map(|pos| (RECENCY_MAX - RECENCY_STEP * pos as i64).max(0))
        .unwrap_or(0)
}

/// Penalización por profundidad: −[`PENALTY_DEPTH`] por cada `/` del path.
fn depth_penalty(rel_path: &str) -> i64 {
    PENALTY_DEPTH * rel_path.matches('/').count() as i64
}

/// Ranking base de una entrada de archivo (§41):
///
/// ```text
/// score = score_fuzzy(rel_path, filtro)
///       + 25 si el filtro también casa con el nombre del archivo
///       + recencia (hasta +40 si está entre los 8 últimos abiertos)
///       − 5 × profundidad (nº de `/` del rel_path)
/// ```
///
/// El desempate es estable por orden del índice (§45: rel_path ascendente).
fn file_score(rel_path: &str, hit: &FuzzyHit, filter: &str, recent: &[String]) -> i64 {
    let bonus_name = if !filter.is_empty() && fuzzy_match(basename(rel_path), filter).is_some() {
        BONUS_FILENAME
    } else {
        0
    };
    hit.score + bonus_name + recency_bonus(recent, rel_path) - depth_penalty(rel_path)
}

/// Consulta §41 sobre el índice del proyecto (§45): entrada principal de la
/// paleta Quick Open (`Ctrl+P`, §39).
///
/// - [`QuickOpenMode::Files`]: fuzzy sobre los archivos indexados;
/// - [`QuickOpenMode::Line`]: `:N` sobre el archivo activo
///   (`current_rel_path`) o `filtro:N` sobre los archivos elegidos por fuzzy;
/// - [`QuickOpenMode::DocHeadings`]: fuzzy sobre los headings del documento
///   actual;
/// - [`QuickOpenMode::ProjectHeadings`]: fuzzy sobre los headings de todo el
///   proyecto.
///
/// `query` es el texto tecleado crudo (con o sin prefijo); `kind` es el modo
/// autoritativo ([`parse_query`] lo deriva del prefijo). `recent` lista las
/// rutas abiertas, la más reciente primero (§41: recencia de apertura).
///
/// Ranking documentado por modo (desempate estable por orden del índice):
///
/// ```text
/// archivos / línea:  score_fuzzy(rel_path, filtro) + 25 si casa el nombre
///                    del archivo + recencia (hasta +40) − 5 × profundidad
/// headings (@/#):    score_fuzzy(texto, filtro) + recencia del archivo
///                    − 5 × profundidad del archivo
/// ```
///
/// Con filtro vacío `score_fuzzy` vale 0 y manda el orden natural (índice;
/// documento para `@`). Con `show_all_files == false` solo entran tipos
/// documentales §14.4 (en el índice §45 son todos: solo indexa Markdown);
/// queda para fuentes de entrada ampliadas. El resultado se corta en
/// [`MAX_RESULTS`] (§47.5).
pub fn quickopen_query(
    kind: QuickOpenMode,
    query: &str,
    index: &ProjectIndex,
    current_rel_path: Option<&str>,
    recent: &[String],
    show_all_files: bool,
) -> Vec<Entry> {
    match kind {
        QuickOpenMode::Files => {
            let filter = query;
            files_mode(filter, index, recent, show_all_files)
        }
        QuickOpenMode::Line => line_mode(query, index, current_rel_path, recent, show_all_files),
        QuickOpenMode::DocHeadings => {
            let filter = query.strip_prefix('@').unwrap_or(query);
            doc_headings_mode(filter, index, current_rel_path, recent)
        }
        QuickOpenMode::ProjectHeadings => {
            let filter = query.strip_prefix('#').unwrap_or(query);
            project_headings_mode(filter, index, recent)
        }
    }
}

/// Modo archivos §41: fuzzy sobre `rel_path` con el ranking documentado.
fn files_mode(
    filter: &str,
    index: &ProjectIndex,
    recent: &[String],
    show_all_files: bool,
) -> Vec<Entry> {
    let mut results: Vec<Entry> = Vec::new();
    for indexed in index.entries() {
        if !show_all_files && !is_document_type(&indexed.rel_path) {
            continue;
        }
        let Some(hit) = fuzzy_match(&indexed.rel_path, filter) else {
            continue;
        };
        results.push(Entry {
            label: indexed.rel_path.clone(),
            rel_path: indexed.rel_path.clone(),
            line: None,
            score: file_score(&indexed.rel_path, &hit, filter, recent),
            heading: None,
        });
    }
    finish(results)
}

/// Modo línea §41 (`:N` o `filtro:N`).
fn line_mode(
    query: &str,
    index: &ProjectIndex,
    current_rel_path: Option<&str>,
    recent: &[String],
    show_all_files: bool,
) -> Vec<Entry> {
    let Some((filter, line)) = split_line_query(query) else {
        return Vec::new();
    };
    if filter.is_empty() {
        // Sin filtro, la línea es del archivo activo (§41).
        let Some(rel_path) = current_rel_path else {
            return Vec::new();
        };
        return vec![Entry {
            label: format!("{rel_path}:{line}"),
            rel_path: rel_path.to_owned(),
            line: Some(line),
            score: recency_bonus(recent, rel_path),
            heading: None,
        }];
    }
    // Con filtro, cada archivo fuzzy elegido recibe la misma línea.
    files_mode(filter, index, recent, show_all_files)
        .into_iter()
        .map(|mut entry| {
            entry.label = format!("{}:{line}", entry.rel_path);
            entry.line = Some(line);
            entry
        })
        .collect()
}

/// Modo `@heading` §41: headings del documento actual (orden del documento
/// con filtro vacío).
fn doc_headings_mode(
    filter: &str,
    index: &ProjectIndex,
    current_rel_path: Option<&str>,
    recent: &[String],
) -> Vec<Entry> {
    let Some(indexed) = current_rel_path.and_then(|rel_path| index.get(rel_path)) else {
        return Vec::new();
    };
    let mut results: Vec<Entry> = Vec::new();
    for heading in indexed.headings.iter().filter(|h| h.level > 0) {
        let Some(hit) = fuzzy_match(&heading.text, filter) else {
            continue;
        };
        results.push(Entry {
            label: format!("{} {}", "#".repeat(heading.level as usize), heading.text),
            rel_path: indexed.rel_path.clone(),
            line: None,
            score: hit.score + recency_bonus(recent, &indexed.rel_path)
                - depth_penalty(&indexed.rel_path),
            heading: Some(Heading {
                level: heading.level,
                text: heading.text.clone(),
            }),
        });
    }
    finish(results)
}

/// Modo `#heading` §41: fuzzy sobre los headings de todo el proyecto.
fn project_headings_mode(filter: &str, index: &ProjectIndex, recent: &[String]) -> Vec<Entry> {
    let mut results: Vec<Entry> = Vec::new();
    for indexed in index.entries() {
        for heading in indexed.headings.iter().filter(|h| h.level > 0) {
            let Some(hit) = fuzzy_match(&heading.text, filter) else {
                continue;
            };
            results.push(Entry {
                label: format!(
                    "{} {} — {}",
                    "#".repeat(heading.level as usize),
                    heading.text,
                    indexed.rel_path
                ),
                rel_path: indexed.rel_path.clone(),
                line: None,
                score: hit.score + recency_bonus(recent, &indexed.rel_path)
                    - depth_penalty(&indexed.rel_path),
                heading: Some(Heading {
                    level: heading.level,
                    text: heading.text.clone(),
                }),
            });
        }
    }
    finish(results)
}

/// Ordena por score descendente (estable: empate por orden del índice) y
/// aplica el techo [`MAX_RESULTS`] (§47.5).
fn finish(mut results: Vec<Entry>) -> Vec<Entry> {
    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(MAX_RESULTS);
    results
}

/// Línea (1-based) de `target` en `source`, o `None` si ese heading no existe
/// en el documento. Resolución perezosa al abrir (§41: posiciona el cursor
/// §8.6 en el heading elegido).
///
/// Localiza el primer heading de [`find_heading_nodes`] (única fuente de
/// verdad de la gramática §27.1, compartida con las anclas §26) con el mismo
/// nivel y texto que `target`: nunca se salta a ciegas y los duplicados
/// resuelven al primero, como el índice.
pub fn find_heading_line(source: &str, target: &Heading) -> Option<u32> {
    find_heading_nodes(source)
        .into_iter()
        .find(|node| node.level == target.level && node.text == target.text)
        .map(|node| node.line)
}

/// Línea (1-based) del heading `target` dentro de `root/rel_path` (§41): lee
/// el fichero al abrir —una sola lectura en Enter— y delega en
/// [`find_heading_line`].
pub fn resolve_heading_line(root: &Path, rel_path: &str, target: &Heading) -> Option<u32> {
    let source = std::fs::read_to_string(root.join(rel_path)).ok()?;
    find_heading_line(&source, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Índice sintético sobre el formato `minmark-index 1` (§45): evita
    /// tocar el filesystem y construye N entradas al instante (rendimiento).
    fn synth_index(root: &str, docs: &[(&str, &[(u8, &str)])]) -> ProjectIndex {
        let mut text = String::from("minmark-index 3\n");
        text.push_str(&format!("root {root}\n"));
        for (rel_path, headings) in docs {
            let payload: Vec<String> = headings
                .iter()
                .enumerate()
                .map(|(i, (level, text))| format!("h={level}:{}:ancla-{i}:{text}", i + 1))
                .collect();
            text.push_str(&format!("{rel_path}\t1\t1\t{}\n", payload.join("\u{1e}")));
        }
        ProjectIndex::parse(&text, Path::new(root)).expect("índice sintético legible")
    }

    fn heading(level: u8, text: &str) -> Heading {
        Heading {
            level,
            text: text.to_owned(),
        }
    }

    // --- Matcher fuzzy ------------------------------------------------------

    #[test]
    fn subsecuencia_case_insensitive_con_posiciones() {
        let hit = fuzzy_match("docs/Guia-Rapida.md", "grm").expect("subsecuencia");
        // g@5 (inicio de segmento), R@10 (camelCase… prev es '-', boundary),
        // m@17 (final).
        assert_eq!(hit.positions, vec![5, 10, 17]);
        assert!(fuzzy_match("abc", "ABC").is_some());
        assert!(fuzzy_match("abc", "cba").is_none());
        assert_eq!(fuzzy_match("lo que sea", "").unwrap().score, 0);
    }

    #[test]
    fn bonus_inicio_palabra_segmento_y_camel() {
        // «sm»: ambas letras en inicio de texto/segmento superan a la variante
        // con arrastre.
        let boundary = fuzzy_match("src/main.rs", "sm").unwrap().score;
        let dragged = fuzzy_match("xsrc/main.rs", "sm").unwrap().score;
        assert!(boundary > dragged, "{boundary} !> {dragged}");

        // «fb»: transición camelCase (fooBar) supera a minúscula interna.
        let camel = fuzzy_match("fooBar.md", "fb").unwrap().score;
        let lower = fuzzy_match("fooxbar.md", "fb").unwrap().score;
        assert!(camel > lower, "{camel} !> {lower}");
    }

    #[test]
    fn bonus_consecutivas_y_penalizacion_por_huecos() {
        let contiguous = fuzzy_match("ab_x", "ab").unwrap().score;
        let spread = fuzzy_match("a_xb", "ab").unwrap().score;
        assert!(contiguous > spread, "{contiguous} !> {spread}");

        // La penalización por huecos está topada: no hunde el score sin límite.
        let long_gap = fuzzy_match(&format!("a{}b", "x".repeat(500)), "ab")
            .unwrap()
            .score;
        assert_eq!(long_gap, 10 + BONUS_BOUNDARY + 10 + PENALTY_GAP_CAP);
    }

    #[test]
    fn sin_resultados_cuando_no_es_subsecuencia() {
        let index = synth_index("/proy", &[("docs/a.md", &[]), ("docs/b.md", &[])]);
        assert!(fuzzy_match("docs/a.md", "zzz").is_none());
        let results = quickopen_query(QuickOpenMode::Files, "zzz", &index, None, &[], true);
        assert!(results.is_empty());
    }

    // --- Ranking documentado ------------------------------------------------

    #[test]
    fn ranking_prefiere_nombre_de_archivo_y_penaliza_profundidad() {
        let index = synth_index(
            "/proy",
            &[
                ("readme.md", &[]),
                ("docs/a/readme.md", &[]),
                ("docs/b/otro.md", &[]),
            ],
        );
        let results = quickopen_query(QuickOpenMode::Files, "readme", &index, None, &[], true);
        assert_eq!(
            results
                .iter()
                .map(|entry| entry.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["readme.md", "docs/a/readme.md"],
        );
    }

    #[test]
    fn ranking_recencia_supera_a_profundidad() {
        let index = synth_index("/proy", &[("a/b/c/ficha.md", &[]), ("ficha.md", &[])]);
        let recent = vec!["a/b/c/ficha.md".to_owned()];
        let results = quickopen_query(QuickOpenMode::Files, "ficha", &index, None, &recent, true);
        // +40 de recencia (aunque −15 de profundidad) deja al más reciente
        // por delante del más plano.
        assert_eq!(results[0].rel_path, "a/b/c/ficha.md");
        assert_eq!(results[1].rel_path, "ficha.md");
    }

    #[test]
    fn desempate_estable_por_orden_del_indice() {
        let index = synth_index("/proy", &[("b.md", &[]), ("a.md", &[]), ("c.md", &[])]);
        let results = quickopen_query(QuickOpenMode::Files, "", &index, None, &[], true);
        // Mismo score (sin filtro, sin recencia): orden del índice (§45).
        assert_eq!(
            results
                .iter()
                .map(|entry| entry.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["a.md", "b.md", "c.md"],
        );
    }

    #[test]
    fn techo_de_resultados_100_47_5() {
        let paths: Vec<String> = (0..250).map(|i| format!("f{i:03}/doc.md")).collect();
        let sin_headings: &[(u8, &str)] = &[];
        let borrowed: Vec<(&str, &[(u8, &str)])> = paths
            .iter()
            .map(|path| (path.as_str(), sin_headings))
            .collect();
        let index = synth_index("/proy", &borrowed);
        let results = quickopen_query(QuickOpenMode::Files, "doc", &index, None, &[], true);
        assert_eq!(results.len(), MAX_RESULTS);
    }

    #[test]
    fn rendimiento_con_10k_entradas_47_5() {
        let paths: Vec<String> = (0..10_000)
            .map(|i| format!("d{i:04}/doc_{i:04}.md"))
            .collect();
        let sin_headings: &[(u8, &str)] = &[];
        let borrowed: Vec<(&str, &[(u8, &str)])> = paths
            .iter()
            .map(|path| (path.as_str(), sin_headings))
            .collect();
        let index = synth_index("/proy", &borrowed);

        let started = std::time::Instant::now();
        let results = quickopen_query(QuickOpenMode::Files, "doc", &index, None, &[], true);
        let elapsed = started.elapsed();
        assert_eq!(results.len(), MAX_RESULTS);
        // Atado generoso: el matcher greedy es lineal y no hace E/S; el
        // objetivo es que la pulsación nunca bloquee (§47.5).
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "{elapsed:?}"
        );
    }

    #[test]
    fn filtros_14_4_con_show_all_files() {
        let index = synth_index(
            "/proy",
            &[("doc.md", &[]), ("figura.png", &[]), ("datos.bin", &[])],
        );
        let por_defecto = quickopen_query(QuickOpenMode::Files, "", &index, None, &[], false);
        assert_eq!(
            por_defecto
                .iter()
                .map(|entry| entry.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["doc.md", "figura.png"], // tipos documentales §14.4
        );
        let todos = quickopen_query(QuickOpenMode::Files, "", &index, None, &[], true);
        assert_eq!(
            todos
                .iter()
                .map(|entry| entry.rel_path.as_str())
                .collect::<Vec<_>>(),
            ["datos.bin", "doc.md", "figura.png"],
        );
    }

    // --- Modos §41 ----------------------------------------------------------

    #[test]
    fn modo_linea_sobre_archivo_activo_y_elegido_41() {
        let index = synth_index("/proy", &[("docs/gui.md", &[]), ("docs/glosario.md", &[])]);
        let activo = quickopen_query(
            QuickOpenMode::Line,
            ":42",
            &index,
            Some("docs/gui.md"),
            &[],
            true,
        );
        assert_eq!(activo.len(), 1);
        assert_eq!(activo[0].rel_path, "docs/gui.md");
        assert_eq!(activo[0].line, Some(42));
        assert_eq!(activo[0].label, "docs/gui.md:42");

        // `gui:7`: el filtro elige el archivo; la línea se aplica a los
        // candidatos fuzzy.
        let elegido = quickopen_query(
            QuickOpenMode::Line,
            "gui:7",
            &index,
            Some("docs/glosario.md"),
            &[],
            true,
        );
        assert_eq!(elegido.len(), 1);
        assert_eq!(elegido[0].rel_path, "docs/gui.md");
        assert_eq!(elegido[0].line, Some(7));

        // Líneas inválidas o sin archivo activo: sin resultados (§41 `:42` es
        // 1-based).
        assert!(quickopen_query(
            QuickOpenMode::Line,
            ":0",
            &index,
            Some("docs/gui.md"),
            &[],
            true
        )
        .is_empty());
        assert!(quickopen_query(
            QuickOpenMode::Line,
            ":x",
            &index,
            Some("docs/gui.md"),
            &[],
            true
        )
        .is_empty());
        assert!(quickopen_query(QuickOpenMode::Line, ":42", &index, None, &[], true).is_empty());
    }

    #[test]
    fn modo_headings_del_documento_actual_41() {
        let index = synth_index(
            "/proy",
            &[
                (
                    "docs/gui.md",
                    &[(1, "Guia"), (2, "Instalacion"), (2, "Uso")],
                ),
                ("docs/otro.md", &[(1, "Otra guia")]),
            ],
        );
        let filtrado = quickopen_query(
            QuickOpenMode::DocHeadings,
            "@inst",
            &index,
            Some("docs/gui.md"),
            &[],
            true,
        );
        assert_eq!(filtrado.len(), 1);
        assert_eq!(filtrado[0].label, "## Instalacion");
        assert_eq!(filtrado[0].heading, Some(heading(2, "Instalacion")));

        // Sin filtro: todos los headings del documento actual, en orden.
        let todos = quickopen_query(
            QuickOpenMode::DocHeadings,
            "@",
            &index,
            Some("docs/gui.md"),
            &[],
            true,
        );
        assert_eq!(
            todos
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            ["# Guia", "## Instalacion", "## Uso"],
        );

        // Sin documento actual: sin resultados.
        assert!(
            quickopen_query(QuickOpenMode::DocHeadings, "@g", &index, None, &[], true).is_empty()
        );
    }

    #[test]
    fn modo_headings_de_todo_el_proyecto_41() {
        let index = synth_index(
            "/proy",
            &[
                ("cap/a.md", &[(1, "Cap A"), (2, "Detalle")]),
                ("cap/b.md", &[(2, "Detalle final")]),
                ("index.md", &[(1, "Inicio")]),
            ],
        );
        let results = quickopen_query(
            QuickOpenMode::ProjectHeadings,
            "#det",
            &index,
            Some("index.md"),
            &[],
            true,
        );
        let paths: Vec<&str> = results
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        assert_eq!(paths, ["cap/a.md", "cap/b.md"]);
        assert_eq!(results[0].heading, Some(heading(2, "Detalle")));
        assert_eq!(results[1].heading, Some(heading(2, "Detalle final")));
        // La línea no se resuelve en la consulta (§47.5: sin E/S por
        // pulsación); se resuelve al abrir.
        assert!(results.iter().all(|entry| entry.line.is_none()));

        let todos = quickopen_query(QuickOpenMode::ProjectHeadings, "#", &index, None, &[], true);
        assert_eq!(todos.len(), 4);
    }

    #[test]
    fn parse_query_detecta_modos_y_linea_41() {
        assert_eq!(
            parse_query("gui"),
            ParsedQuery {
                mode: QuickOpenMode::Files,
                filter: "gui",
                line: None
            }
        );
        assert_eq!(
            parse_query(":42"),
            ParsedQuery {
                mode: QuickOpenMode::Line,
                filter: "",
                line: Some(42)
            }
        );
        assert_eq!(
            parse_query("docs/gui:7"),
            ParsedQuery {
                mode: QuickOpenMode::Line,
                filter: "docs/gui",
                line: Some(7)
            }
        );
        // Sin línea válida el prefijo `:` sigue siendo modo línea (sin salto).
        assert_eq!(
            parse_query(":x"),
            ParsedQuery {
                mode: QuickOpenMode::Line,
                filter: "",
                line: None
            }
        );
        // Dos puntos internos sin dígitos: búsqueda de archivos normal.
        assert_eq!(
            parse_query("a:b"),
            ParsedQuery {
                mode: QuickOpenMode::Files,
                filter: "a:b",
                line: None
            }
        );
        assert_eq!(
            parse_query("@inst"),
            ParsedQuery {
                mode: QuickOpenMode::DocHeadings,
                filter: "inst",
                line: None
            }
        );
        assert_eq!(
            parse_query("#det"),
            ParsedQuery {
                mode: QuickOpenMode::ProjectHeadings,
                filter: "det",
                line: None
            }
        );
    }

    #[test]
    fn tipos_documentales_14_4() {
        assert!(is_document_type("notas.md"));
        assert!(is_document_type("docs/FIGURA.PNG"));
        assert!(is_document_type("refs.bib"));
        assert!(is_document_type("estilo.csl"));
        assert!(is_document_type("config.toml"));
        assert!(!is_document_type("datos.bin"));
        assert!(!is_document_type("carpeta.md/"));
        assert!(!is_document_type("doc.md.bak"));
    }

    // --- Línea de heading ---------------------------------------------------

    #[test]
    fn linea_de_heading_con_fences_y_front_matter() {
        let source = "---\ntitulo: X\n# comentario yaml\n---\n# Real\n\ntexto\n\n```\n# Falsa\n```\n\n## Dos\n";
        // find_headings (ISSUE-025/§27.1): Real(5), Dos(13).
        assert_eq!(find_heading_line(source, &heading(1, "Real")), Some(5));
        assert_eq!(find_heading_line(source, &heading(2, "Dos")), Some(13));
        // Ni el comentario YAML ni la del fence existen como headings.
        assert_eq!(
            find_heading_line(source, &heading(1, "comentario yaml")),
            None
        );
        assert_eq!(find_heading_line(source, &heading(1, "Falsa")), None);
    }

    #[test]
    fn linea_de_heading_distingue_por_nivel() {
        let source = "# A\n\n## A\n";
        assert_eq!(find_heading_line(source, &heading(1, "A")), Some(1));
        assert_eq!(find_heading_line(source, &heading(2, "A")), Some(3));
    }

    #[test]
    fn resolve_heading_line_desde_disco() {
        let root = std::env::temp_dir().join(format!(
            "minmark-fuzzy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("cap")).unwrap();
        std::fs::write(root.join("cap/a.md"), "# Cap A\n\n## Detalle\n").unwrap();
        assert_eq!(
            resolve_heading_line(&root, "cap/a.md", &heading(2, "Detalle")),
            Some(3)
        );
        assert_eq!(
            resolve_heading_line(&root, "cap/nope.md", &heading(1, "X")),
            None
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
