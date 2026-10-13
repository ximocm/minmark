//! Resolución unificada de destinos y anclas (§17, §19, §26).
//!
//! Un único punto de verdad para responder «¿adónde apunta este enlace?» sobre
//! el [`ProjectIndex`] (§45), reutilizado por navegación, diagnósticos,
//! wiki links y referencias cruzadas. Reglas:
//!
//! - los destinos relativos resuelven desde el archivo que contiene la
//!   referencia (§17); los absolutos (`/ruta`) desde la raíz del proyecto;
//! - `#fragmento` y `archivo.md#fragmento` resuelven contra las anclas del
//!   documento destino con la política única de
//!   [`minmark_document::document_anchors`] — la misma que emite la Preview,
//!   de modo que índice y Preview producen los mismos destinos;
//! - los enlaces por referencia (§16) resuelven por su definición
//!   `[id]: dest`, con la primera definición por etiqueta normalizada;
//! - los destinos externos (`https:`, `mailto:`, `//…`) **nunca** se
//!   consultan por red (§3.4): solo se clasifican;
//! - los archivos ignorados (§14.6), los inexistentes y los que salen del
//!   proyecto (§48) son [`TargetKind::Missing`]; los nombres wiki (§19.3) con
//!   varios candidatos son [`TargetKind::Ambiguous`];
//! - los buffers abiertos con cambios sin guardar tienen prioridad sobre la
//!   copia indexada en disco; resolver es de solo lectura: no modifica
//!   documentos ni su historial (§6.1, §8.5).
//!
//! Los destinos percent-encoded (`mi%20archivo.md`) y las formas con espacios
//! (`<mi archivo.md>`) se resuelven sin alterar nunca la forma original del
//! Markdown: aquí no hay reescritura (eso es [`crate::ops`], §16).

use crate::ignore::IgnorePatterns;
use crate::index::{IndexHeading, ProjectIndex};
use crate::paths::resolve_in_project;
use minmark_document::{
    document_anchors, find_anchor, find_heading_nodes, find_link_definitions, normalize_label,
    Anchor,
};
use std::collections::BTreeMap;
use std::path::Path;

/// Clase de destino de un enlace (§19).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Destino dentro del proyecto resuelto a un archivo (y a su ancla si el
    /// destino lleva `#fragmento`).
    Local,
    /// Destino externo (esquema de URL o `//…`): nunca se consulta la red.
    External,
    /// No resoluble: archivo inexistente o ignorado (§14.6), destino fuera
    /// del proyecto (§48) o fragmento sin ancla en el documento destino.
    Missing,
    /// Nombre wiki (§19.3) con varios candidatos en el proyecto.
    Ambiguous,
}

/// Destino resuelto de una referencia (§19, §26).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkTarget {
    pub kind: TargetKind,
    /// Path relativo posix (§17) del destino local, si se pudo determinar.
    pub rel_path: Option<String>,
    /// Fragmento `#…` sin la almohadilla, si el destino lo lleva.
    pub fragment: Option<String>,
    /// Ancla alcanzada en el documento destino (§26), si la hay.
    pub anchor: Option<Anchor>,
    /// Candidatos cuando [`TargetKind::Ambiguous`] (paths relativos).
    pub candidates: Vec<String>,
}

impl LinkTarget {
    fn external() -> Self {
        Self {
            kind: TargetKind::External,
            rel_path: None,
            fragment: None,
            anchor: None,
            candidates: Vec::new(),
        }
    }

    fn missing(fragment: Option<String>) -> Self {
        Self {
            kind: TargetKind::Missing,
            rel_path: None,
            fragment,
            anchor: None,
            candidates: Vec::new(),
        }
    }
}

/// Buffers abiertos cuyo contenido prioriza sobre el índice (§6.1): la fuente
/// tal cual está en memoria, por path relativo posix (§17).
#[derive(Debug, Default, Clone)]
pub struct OpenBuffers<'a> {
    sources: BTreeMap<String, &'a str>,
}

impl<'a> OpenBuffers<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra el buffer abierto de `rel_path` con su fuente actual.
    pub fn insert(&mut self, rel_path: &str, source: &'a str) {
        self.sources.insert(rel_path.to_owned(), source);
    }

    pub fn get(&self, rel_path: &str) -> Option<&'a str> {
        self.sources.get(rel_path).copied()
    }
}

/// Resolutor de destinos sobre un [`ProjectIndex`].
pub struct Resolver<'a> {
    index: &'a ProjectIndex,
    open: &'a OpenBuffers<'a>,
    ignore: IgnorePatterns,
}

impl<'a> Resolver<'a> {
    /// Resolutor sin buffers abiertos: solo el índice y el disco.
    pub fn new(index: &'a ProjectIndex, open: &'a OpenBuffers<'a>) -> Self {
        let ignore = IgnorePatterns::from_root(index.root());
        Self { index, open, ignore }
    }

    /// Resuelve el destino `dest` de una referencia contenida en `from_rel`
    /// (§17): ruta relativa, `#fragmento`, `archivo.md#fragmento` o destino
    /// externo, preservando espacios/Unicode y la forma original del Markdown.
    pub fn resolve(&self, dest: &str, from_rel: &str) -> LinkTarget {
        let dest = dest.trim();
        // Tolerancia a la forma `<...>` (§19.1): el destino extraído por
        // [`minmark_document::find_markdown_references`] ya va sin llaves.
        let dest = dest
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
            .unwrap_or(dest);
        if has_scheme(dest) || dest.starts_with("//") {
            return LinkTarget::external();
        }
        let (path_part, fragment) = split_fragment(dest);
        let fragment = fragment.strip_prefix('#');
        let fragment_owned = fragment.map(str::to_owned);
        if path_part.is_empty() {
            // Fragmento del propio documento (§19.2).
            return self.resolve_in_doc(from_rel, fragment);
        }
        for rel in path_candidates(path_part, from_rel) {
            if let Some(target) = self.resolve_file(&rel, fragment) {
                return target;
            }
        }
        LinkTarget::missing(fragment_owned)
    }

    /// Resuelve un uso por referencia `[texto][id]` / `[texto][]` (§16):
    /// busca la definición `[id]: dest` en `from_rel` y resuelve su destino.
    /// La primera definición por etiqueta normalizada es la que gana.
    pub fn resolve_usage(&self, label: &str, from_rel: &str) -> LinkTarget {
        let Some(source) = self.source_of(from_rel) else {
            return LinkTarget::missing(None);
        };
        let label = normalize_label(label);
        let dest = find_link_definitions(&source)
            .into_iter()
            .find(|definition| definition.label == label)
            .map(|definition| definition.dest);
        match dest {
            Some(dest) => self.resolve(&dest, from_rel),
            None => LinkTarget::missing(None),
        }
    }

    /// Resuelve un wiki link `[[destino]]` / `[[destino#frag]]` (§19.3) por
    /// nombre de proyecto: tallo del archivo o ruta sin extensión
    /// `.md`/`.markdown`. Con varios candidatos el destino es
    /// [`TargetKind::Ambiguous`].
    pub fn resolve_wiki(&self, target: &str) -> LinkTarget {
        let (name, fragment) = split_fragment(target.trim());
        let fragment = fragment.strip_prefix('#');
        let fragment_owned = fragment.map(str::to_owned);
        if name.is_empty() || has_scheme(name) {
            return LinkTarget::missing(fragment_owned);
        }
        let mut candidates: Vec<String> = self
            .index
            .entries()
            .map(|entry| entry.rel_path.clone())
            .filter(|rel_path| wiki_name_matches(Path::new(rel_path), name))
            .collect();
        candidates.sort();
        match candidates.len() {
            0 => LinkTarget::missing(fragment_owned),
            1 => {
                let mut target = self
                    .resolve_file(&candidates[0], fragment)
                    .unwrap_or_else(|| LinkTarget::missing(fragment_owned.clone()));
                if target.rel_path.is_none() {
                    target.rel_path = Some(candidates[0].clone());
                }
                target
            }
            _ => LinkTarget {
                kind: TargetKind::Ambiguous,
                rel_path: None,
                fragment: fragment_owned,
                anchor: None,
                candidates,
            },
        }
    }

    /// Resolución dentro de un documento ya determinado (`#fragmento`).
    fn resolve_in_doc(&self, rel_path: &str, fragment: Option<&str>) -> LinkTarget {
        let fragment_owned = fragment.map(str::to_owned);
        if self.ignore.is_ignored(Path::new(rel_path), false)
            || resolve_in_project(self.index.root(), &self.index.root().join(rel_path)).is_err()
            || !(self.index.get(rel_path).is_some()
                || self.open.get(rel_path).is_some()
                || self.on_disk(rel_path))
        {
            return LinkTarget::missing(fragment_owned);
        }
        match fragment {
            None => LinkTarget {
                kind: TargetKind::Local,
                rel_path: Some(rel_path.to_owned()),
                fragment: None,
                anchor: None,
                candidates: Vec::new(),
            },
            Some(fragment) => {
                let Some(anchors) = self.doc_anchors(rel_path) else {
                    return LinkTarget::missing(fragment_owned);
                };
                match find_anchor(&anchors, fragment)
                    .or_else(|| percent_decode(fragment).and_then(|decoded| find_anchor(&anchors, &decoded)))
                {
                    Some(anchor) => LinkTarget {
                        kind: TargetKind::Local,
                        rel_path: Some(rel_path.to_owned()),
                        fragment: fragment_owned,
                        anchor: Some(anchor.clone()),
                        candidates: Vec::new(),
                    },
                    None => LinkTarget {
                        kind: TargetKind::Missing,
                        rel_path: Some(rel_path.to_owned()),
                        fragment: fragment_owned,
                        anchor: None,
                        candidates: Vec::new(),
                    },
                }
            }
        }
    }

    /// Resolución de un archivo candidato: `None` = ese archivo no existe y
    /// se prueba la siguiente interpretación del destino.
    fn resolve_file(&self, rel_path: &str, fragment: Option<&str>) -> Option<LinkTarget> {
        if self.ignore.is_ignored(Path::new(rel_path), false) {
            // §14.6: ignorado = fuera del índice y de las consultas.
            return None;
        }
        let known = self.index.get(rel_path).is_some()
            || self.open.get(rel_path).is_some()
            || self.on_disk(rel_path);
        if !known {
            return None;
        }
        Some(self.resolve_in_doc(rel_path, fragment))
    }

    /// Anclas del documento destino: buffer abierto primero (§6.1), después
    /// el índice (§45) y, si el archivo cambió sin reindexar, el disco.
    fn doc_anchors(&self, rel_path: &str) -> Option<Vec<Anchor>> {
        if let Some(source) = self.open.get(rel_path) {
            return Some(document_anchors(&find_heading_nodes(source)));
        }
        if let Some(entry) = self.index.get(rel_path) {
            return Some(entry.headings.iter().map(anchor_of_heading).collect());
        }
        let source = std::fs::read_to_string(self.index.root().join(rel_path)).ok()?;
        Some(document_anchors(&find_heading_nodes(&source)))
    }

    /// Fuente del documento de origen (para definiciones §16): buffer abierto
    /// primero; si no, disco.
    fn source_of(&self, rel_path: &str) -> Option<String> {
        if self.ignore.is_ignored(Path::new(rel_path), false) {
            return None;
        }
        let path = resolve_in_project(self.index.root(), &self.index.root().join(rel_path)).ok()?;
        if let Some(source) = self.open.get(rel_path) {
            return Some(source.to_owned());
        }
        std::fs::read_to_string(path).ok()
    }

    /// ¿Existe el archivo en disco dentro del proyecto (§48) y sin ignorar
    /// (§14.6)?
    fn on_disk(&self, rel_path: &str) -> bool {
        match resolve_in_project(self.index.root(), &self.index.root().join(rel_path)) {
            Ok(path) => path.exists(),
            Err(_) => false,
        }
    }
}

fn anchor_of_heading(heading: &IndexHeading) -> Anchor {
    Anchor {
        id: heading.anchor.clone(),
        level: heading.level,
        text: heading.text.clone(),
        line: heading.line,
    }
}

/// Destino y fragmento `#…` (el `#` literal en nombre no se admite en
/// destinos planos: con fragmento el primer `#` siempre separa).
pub fn split_fragment(dest: &str) -> (&str, &str) {
    match dest.find('#') {
        Some(pos) => (&dest[..pos], &dest[pos..]),
        None => (dest, ""),
    }
}

/// ¿Tiene esquema de URL (`https:`, `mailto:`…)? No es ruta interna.
pub fn has_scheme(dest: &str) -> bool {
    let Some((prefix, _)) = dest.split_once(':') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// ¿El wiki dest `name` resuelve a `path` (por tallo o por ruta sin `.md`)?
///
/// `path` es relativo a la raíz del proyecto (§17). Es la misma regla que
/// usa la reescritura de renombrados ([`crate::ops`], §16): una única
/// respuesta a «¿a qué archivo apunta `[[name]]`?».
pub(crate) fn wiki_name_matches(path: &Path, name: &str) -> bool {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let stem = match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => stem.to_owned(),
        _ => file_name.clone(),
    };
    if name == stem {
        return true;
    }
    let mut without_ext = path.to_string_lossy().replace('\\', "/");
    for ext in [".markdown", ".md"] {
        if without_ext.ends_with(ext) {
            without_ext.truncate(without_ext.len() - ext.len());
            break;
        }
    }
    name == without_ext.trim_start_matches('/')
}

/// Interpretaciones del path de un destino relativo a `from_rel` (§17):
/// percent-decodificado primero (URI) y literal después. `None` si el destino
/// sale del proyecto (§48) y no hay interpretación segura.
fn path_candidates(path_part: &str, from_rel: &str) -> Vec<String> {
    let base = match from_rel.rsplit_once('/') {
        Some((dir, _)) => dir.to_owned(),
        None => String::new(),
    };
    let absolute = path_part.starts_with('/');
    let components: Vec<String> = path_part
        .trim_start_matches('/')
        .split('/')
        .map(|component| percent_decode(component).unwrap_or_else(|| component.to_owned()))
        .collect();
    let mut decoded = normalize_rel(if absolute { "" } else { &base }, &components);
    let mut out = Vec::new();
    if let Some(rel) = decoded.take() {
        out.push(rel);
    }
    if path_part.contains('%') {
        let raw: Vec<String> = path_part
            .trim_start_matches('/')
            .split('/')
            .map(str::to_owned)
            .collect();
        if let Some(rel) = normalize_rel(if absolute { "" } else { &base }, &raw) {
            if !out.contains(&rel) {
                out.push(rel);
            }
        }
    }
    out
}

/// Compone `base` con `components` y normaliza lexically (`.`, `..`); `None`
/// si `..` sale de la raíz del proyecto (§48).
fn normalize_rel(base: &str, components: &[String]) -> Option<String> {
    let mut parts: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').filter(|part| !part.is_empty() && *part != ".").collect()
    };
    for component in components {
        match component.as_str() {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return None;
                }
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Decodificación de un segmento percent-encoded (`mi%20archivo.md` →
/// `mi archivo.md`); `None` si la secuencia no es UTF-8 válido.
fn percent_decode(value: &str) -> Option<String> {
    if !value.contains('%') {
        return None;
    }
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_tree(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("minmark-resolve-{tag}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Proyecto base: `index.md` con enlaces, `cap/a.md` y `cap/mi archivo.md`.
    fn project(tag: &str) -> PathBuf {
        let root = temp_tree(tag);
        fs::create_dir(root.join("cap")).unwrap();
        fs::write(
            root.join("index.md"),
            "# Inicio {#intro}\n\n## Base de datos\n\n## Base de datos\n",
        )
        .unwrap();
        fs::write(root.join("cap/a.md"), "# Cap A\n\n## Detalle {#detalle}\n").unwrap();
        fs::write(root.join("cap/mi archivo.md"), "# Con espacio\n").unwrap();
        root
    }

    fn resolver<'a>(index: &'a ProjectIndex, open: &'a OpenBuffers<'a>) -> Resolver<'a> {
        Resolver::new(index, open)
    }

    #[test]
    fn rutas_relativas_desde_el_archivo_que_contiene_la_referencia_17() {
        let root = project("rutas");
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        // Desde cap/a.md: el mismo directorio y la raíz.
        assert_eq!(r.resolve("mi archivo.md", "cap/a.md").rel_path.as_deref(), Some("cap/mi archivo.md"));
        assert_eq!(r.resolve("../index.md", "cap/a.md").rel_path.as_deref(), Some("index.md"));
        // Desde la raíz: el subdirectorio.
        assert_eq!(r.resolve("cap/a.md", "index.md").rel_path.as_deref(), Some("cap/a.md"));
        // Percent-encoded (§19.1) resuelve al mismo destino que la forma con
        // espacio; la forma original del Markdown nunca se reescribe aquí.
        assert_eq!(r.resolve("mi%20archivo.md", "cap/a.md").kind, TargetKind::Local);
        // Path absoluto del proyecto: desde la raíz.
        assert_eq!(r.resolve("/cap/a.md", "index.md").rel_path.as_deref(), Some("cap/a.md"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fragmentos_y_archivo_con_fragmento_19_2() {
        let root = project("fragmentos");
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        // `#fragmento` del propio documento, con ID explícito y automático.
        let target = r.resolve("#intro", "index.md");
        assert_eq!(target.kind, TargetKind::Local);
        assert_eq!(target.anchor.as_ref().map(|a| a.line), Some(1));
        let target = r.resolve("#base-de-datos", "index.md");
        assert_eq!(target.anchor.as_ref().map(|a| a.line), Some(3));
        // `archivo.md#fragmento`.
        let target = r.resolve("a.md#detalle", "cap/mi archivo.md");
        assert_eq!(target.kind, TargetKind::Local);
        assert_eq!(target.rel_path.as_deref(), Some("cap/a.md"));
        assert_eq!(target.anchor.as_ref().map(|a| a.line), Some(3));
        // Coincidencia exacta del ancla: sin plegado de mayúsculas (§26).
        assert_eq!(r.resolve("#Intro", "index.md").kind, TargetKind::Missing);
        // Fragmento sin ancla en un documento que sí existe: Missing.
        let target = r.resolve("cap/a.md#no-esta", "index.md");
        assert_eq!(target.kind, TargetKind::Missing);
        assert_eq!(target.rel_path.as_deref(), Some("cap/a.md"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ids_duplicados_y_setext_26_1() {
        let root = temp_tree("duplicados");
        fs::write(
            root.join("d.md"),
            "# Introducción\n\n# Introducción\n\nTítulo\n=====\n\npárrafo\ncontinuación\n---\n",
        )
        .unwrap();
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        assert_eq!(r.resolve("#introducción", "d.md").anchor.as_ref().map(|a| a.line), Some(1));
        assert_eq!(r.resolve("#introducción-1", "d.md").anchor.as_ref().map(|a| a.line), Some(3));
        // Setext: ATX y setext producen ancla igual que la Preview.
        assert_eq!(r.resolve("#título", "d.md").kind, TargetKind::Local);
        assert_eq!(r.resolve("#párrafo-continuación", "d.md").anchor.as_ref().map(|a| a.line), Some(8));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unicode_espacios_y_crlf() {
        let root = temp_tree("unicode");
        fs::write(root.join("ñ.md"), "# Ñandú 見出し\r\n\r\n## Otra sección\r\n").unwrap();
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        assert_eq!(r.resolve("ñ.md", "index.md").kind, TargetKind::Local);
        let target = r.resolve("ñ.md#ñandú-見出し", "index.md");
        assert_eq!(target.anchor.as_ref().map(|a| a.line), Some(1), "línea con CRLF");
        assert_eq!(r.resolve("ñ.md#otra-sección", "index.md").kind, TargetKind::Local);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn enlaces_por_referencia_16() {
        let root = project("referencia");
        fs::write(
            root.join("ref.md"),
            "Ver [texto][id] y [otro][].\n\n[id]: cap/a.md#detalle\n[otro]: <cap/mi archivo.md>\n",
        )
        .unwrap();
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        let target = r.resolve_usage("id", "ref.md");
        assert_eq!(target.kind, TargetKind::Local);
        assert_eq!(target.rel_path.as_deref(), Some("cap/a.md"));
        assert_eq!(target.anchor.as_ref().map(|a| a.id.as_str()), Some("detalle"));
        // Etiquetas: sin distinguir mayúsculas ni espacios redundantes.
        assert_eq!(r.resolve_usage("OTRO", "ref.md").rel_path.as_deref(), Some("cap/mi archivo.md"));
        // Sin definición: no resoluble.
        assert_eq!(r.resolve_usage("no-existe", "ref.md").kind, TargetKind::Missing);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn definiciones_de_un_buffer_sin_guardar() {
        let root = project("ref-buffer");
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        // En disco no hay definición; el buffer abierto sí (cambios sin guardar).
        let source = "Ver [texto][id].\n\n[id]: cap/a.md#detalle\n";
        let mut open = OpenBuffers::new();
        open.insert("ref.md", source);
        let r = resolver(&index, &open);
        let target = r.resolve_usage("id", "ref.md");
        assert_eq!(target.kind, TargetKind::Local);
        assert_eq!(target.anchor.as_ref().map(|a| a.id.as_str()), Some("detalle"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn destinos_externos_no_consultan_la_red_3_4() {
        let root = project("externos");
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);
        for dest in ["https://example.com/x", "mailto:a@b.com", "//cdn.example/x"] {
            assert_eq!(r.resolve(dest, "index.md").kind, TargetKind::External, "{dest}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn inexistentes_fuera_del_proyecto_e_ignorados_14_6_48() {
        let root = project("missing");
        fs::write(root.join(".gitignore"), "generado.md\n").unwrap();
        fs::write(root.join("generado.md"), "# G\n").unwrap();
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        assert!(index.get("generado.md").is_none(), "§14.6: fuera del índice");
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        assert_eq!(r.resolve("borrado.md", "index.md").kind, TargetKind::Missing);
        assert_eq!(r.resolve("generado.md", "index.md").kind, TargetKind::Missing, "§14.6");
        assert_eq!(r.resolve("../../etc/passwd", "cap/a.md").kind, TargetKind::Missing, "§48");
        assert_eq!(r.resolve("/../fuera.md", "index.md").kind, TargetKind::Missing, "§48");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archivos_binarios_sin_fragmento_son_destinos_locales() {
        let root = project("binarios");
        fs::write(root.join("imagen.png"), [0x89, 0x50, 0x4e, 0x47, 0xff]).unwrap();
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        assert_eq!(r.resolve("imagen.png", "index.md").kind, TargetKind::Local);
        assert_eq!(r.resolve("imagen.png#titulo", "index.md").kind, TargetKind::Missing);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn porcentajes_literales_junto_a_unicode_no_provocan_panic() {
        let root = project("porcentaje-unicode");
        fs::write(root.join("100%año.md"), "# Título\n").unwrap();
        let index = ProjectIndex::new(root.clone());
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        assert_eq!(r.resolve("100%año.md", "index.md").kind, TargetKind::Local);
        assert_eq!(r.resolve("index.md#%año", "index.md").kind, TargetKind::Missing);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn buffers_y_fragmentos_respetan_limites_e_ignorados_14_6_48() {
        let root = project("buffer-limites");
        fs::write(root.join(".gitignore"), "generado.md\n").unwrap();
        let index = ProjectIndex::new(root.clone());
        let mut open = OpenBuffers::new();
        let source = "# Secreto\n\n[id]: /index.md\n";
        open.insert("../fuera.md", source);
        open.insert("generado.md", source);
        let r = resolver(&index, &open);

        for from in ["../fuera.md", "generado.md"] {
            assert_eq!(r.resolve("#secreto", from).kind, TargetKind::Missing, "{from}");
            assert_eq!(r.resolve_usage("id", from).kind, TargetKind::Missing, "{from}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn symlinks_externos_no_reutilizan_anclas_del_indice_48() {
        let root = project("symlink-indice");
        let outside = temp_tree("symlink-fuera");
        fs::write(outside.join("fuera.md"), "# Fuera\n").unwrap();
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        fs::remove_file(root.join("cap/a.md")).unwrap();
        std::os::unix::fs::symlink(outside.join("fuera.md"), root.join("cap/a.md")).unwrap();
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        assert_eq!(r.resolve("cap/a.md", "index.md").kind, TargetKind::Missing);
        assert_eq!(r.resolve("cap/a.md#detalle", "index.md").kind, TargetKind::Missing);
        assert_eq!(r.resolve("#detalle", "cap/a.md").kind, TargetKind::Missing);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn wiki_por_nombre_con_candidato_unico_o_ambiguo_19_3() {
        let root = temp_tree("wiki");
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("otro")).unwrap();
        fs::write(root.join("docs/arch.md"), "# Arch docs\n").unwrap();
        fs::write(root.join("suelto.md"), "# Suelto\n").unwrap();
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);

        // Tallo único.
        let target = r.resolve_wiki("suelto#inicio");
        assert_eq!(target.kind, TargetKind::Missing, "el fragmento no existe");
        assert_eq!(target.rel_path.as_deref(), Some("suelto.md"));
        // Ruta sin extensión.
        assert_eq!(r.resolve_wiki("docs/arch").kind, TargetKind::Local);
        // Ambiguo: dos candidatos con el mismo tallo.
        fs::write(root.join("otro/arch.md"), "# Arch otro\n").unwrap();
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        let r = resolver(&index, &open);
        let target = r.resolve_wiki("arch");
        assert_eq!(target.kind, TargetKind::Ambiguous);
        assert_eq!(target.candidates, ["docs/arch.md", "otro/arch.md"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn buffers_abiertos_con_cambios_sin_guardar_prioridad() {
        let root = project("buffers");
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        // El buffer de cap/a.md tiene un heading nuevo sin guardar.
        let source = "# Cap A\n\n## Nuevo sin guardar\n";
        let mut open = OpenBuffers::new();
        open.insert("cap/a.md", source);
        let r = resolver(&index, &open);

        let target = r.resolve("cap/a.md#nuevo-sin-guardar", "index.md");
        assert_eq!(target.kind, TargetKind::Local, "el buffer manda sobre el índice");
        assert_eq!(target.anchor.as_ref().map(|a| a.line), Some(3));
        assert_eq!(r.resolve("cap/a.md#detalle", "index.md").kind, TargetKind::Missing, "ya no está");
        // La copia indexada sigue resolviendo su propio contenido.
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);
        assert_eq!(r.resolve("cap/a.md#detalle", "index.md").kind, TargetKind::Local);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn refleja_renombrados_y_borrados_tras_refresh() {
        let root = project("cambios");
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        let open = OpenBuffers::new();
        let r = resolver(&index, &open);
        assert_eq!(r.resolve("cap/a.md#detalle", "index.md").kind, TargetKind::Local);

        // Renombrado: el índice lo refleja al refrescar (§45).
        fs::rename(root.join("cap/a.md"), root.join("cap/renombrada.md")).unwrap();
        let stats = index.refresh().unwrap();
        assert_eq!(stats.removed, 1);
        assert_eq!(stats.reindexed, 1);
        let r = resolver(&index, &open);
        assert_eq!(r.resolve("cap/a.md#detalle", "index.md").kind, TargetKind::Missing);
        assert_eq!(r.resolve("renombrada.md#detalle", "cap/x.md").kind, TargetKind::Local);

        // Borrado.
        fs::remove_file(root.join("cap/renombrada.md")).unwrap();
        index.refresh().unwrap();
        let r = resolver(&index, &open);
        assert_eq!(r.resolve("renombrada.md#detalle", "cap/x.md").kind, TargetKind::Missing);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_no_modifica_documentos_6_1() {
        let root = project("solo-lectura");
        let mut index = ProjectIndex::new(root.clone());
        index.refresh().unwrap();
        let source = "Ver [texto][id] y [roto](#no-esta).\n\n[id]: cap/a.md#detalle\n";
        let mut open = OpenBuffers::new();
        open.insert("doc.md", source);
        let r = resolver(&index, &open);
        let _ = r.resolve_usage("id", "doc.md");
        let _ = r.resolve("#no-esta", "doc.md");
        let _ = r.resolve_wiki("cap/a");

        assert_eq!(open.get("doc.md"), Some(source), "el buffer queda intacto");
        for rel in ["index.md", "cap/a.md", "cap/mi archivo.md"] {
            let mut path = root.clone();
            for part in rel.split('/') {
                path.push(part);
            }
            let antes = fs::read(&path).unwrap();
            let _ = r.resolve("cap/a.md#detalle", "index.md");
            assert_eq!(fs::read(&path).unwrap(), antes, "{rel} intacto en disco");
        }
        let _ = fs::remove_dir_all(&root);
    }
}
