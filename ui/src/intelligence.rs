//! Consultas de Markdown sobre el resolutor común, sin GTK ni escrituras.
use minmark_document::{scan_markdown_links, RefKind, Span};
use minmark_fs::{LinkTarget, OpenBuffers, ProjectIndex, Resolver, TargetKind};

#[derive(Clone, Debug)]
pub struct Link {
    pub span: Span,
    pub destination: String,
    pub target: LinkTarget,
}

pub fn links(source: &str, from: &str, resolver: &Resolver<'_>, wiki: bool) -> Vec<Link> {
    let scan = scan_markdown_links(source);
    let mut result = Vec::new();
    for reference in scan.references {
        let target = match reference.kind {
            RefKind::MarkdownLink => resolver.resolve(&reference.dest, from),
            RefKind::WikiLink if wiki => resolver.resolve_wiki(&reference.dest),
            _ => continue,
        };
        let destination = if reference.kind == RefKind::WikiLink {
            format!("minmark-wiki:{}", reference.dest)
        } else {
            reference.dest
        };
        result.push(Link {
            span: reference.full_span,
            destination,
            target,
        });
    }
    for usage in scan.usages {
        if usage.kind == RefKind::MarkdownLink {
            let destination = scan
                .definitions
                .iter()
                .find(|d| d.label == usage.label)
                .map(|d| d.dest.clone())
                .unwrap_or_default();
            result.push(Link {
                span: usage.span,
                destination,
                target: resolver.resolve_usage(&usage.label, from),
            });
        }
    }
    result.sort_by_key(|link| link.span.start);
    result
}

pub fn problem(target: &LinkTarget) -> Option<String> {
    match target.kind {
        TargetKind::Missing if target.rel_path.is_some() && target.fragment.is_some() => {
            Some(format!(
                "No existe el fragmento #{} en {}",
                target.fragment.as_deref().unwrap_or_default(),
                target.rel_path.as_deref().unwrap_or_default()
            ))
        }
        TargetKind::Missing => {
            Some("No existe el archivo o está fuera del ámbito permitido".into())
        }
        TargetKind::Ambiguous => Some(format!("Destino ambiguo: {}", target.candidates.join(", "))),
        _ => None,
    }
}

/// Copia de consulta enviada a un worker. Los buffers siempre prevalecen.
pub struct Snapshot {
    pub index: ProjectIndex,
    pub buffers: Vec<(String, String)>,
    pub from: String,
    pub source: String,
    pub wiki: bool,
}
impl Snapshot {
    pub fn query<T>(&self, query: impl FnOnce(&Resolver<'_>) -> T) -> T {
        let mut open = OpenBuffers::new();
        for (path, text) in &self.buffers {
            open.insert(path, text);
        }
        open.insert(&self.from, &self.source);
        query(&Resolver::new(&self.index, &open))
    }
    pub fn links(&self) -> Vec<Link> {
        self.query(|r| links(&self.source, &self.from, r, self.wiki))
    }
}

pub struct Backlink {
    pub entry: minmark_fs::fuzzy::Entry,
    pub position: minmark_document::Position,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub span: Span,
    pub text: String,
    pub label: String,
}

/// Reutiliza el highlighting del parser para excluir código, escapes y comentarios.
pub fn citation_completions(
    source: &str,
    cursor: usize,
    bibliography: &minmark_renderer::Bibliography,
) -> Vec<Completion> {
    let Some(prefix) = source.get(..cursor) else {
        return Vec::new();
    };
    let Some(at) = prefix.rfind('@') else {
        return Vec::new();
    };
    let Some(open) = prefix[..at].rfind('[') else {
        return Vec::new();
    };
    let before = &prefix[open + 1..at];
    if (!before.trim().is_empty() && !before.trim_end().ends_with(';'))
        || prefix[at + 1..].contains([',', ';', '[', ']', '\n', '\r'])
    {
        return Vec::new();
    }
    let start = at + 1;
    let mut end = cursor;
    for (offset, ch) in source[cursor..].char_indices() {
        if ch.is_alphanumeric() || matches!(ch, ':' | '.' | '_' | '-' | '*' | '+' | '/' | '!') {
            end = cursor + offset + ch.len_utf8();
        } else {
            break;
        }
    }
    let mut probe = source.to_owned();
    probe.replace_range(start..end, "mmprobe");
    let after = start + "mmprobe".len();
    if !probe[after..]
        .split('\n')
        .next()
        .unwrap_or("")
        .contains(']')
    {
        probe.insert(after, ']');
    }
    if !minmark_parser::highlight(&probe).iter().any(|token| {
        token.kind == minmark_parser::HighlightKind::Citation
            && token.span.start as usize <= at
            && at < token.span.end as usize
    }) {
        return Vec::new();
    }
    bibliography
        .suggestions(&source[start..cursor])
        .into_iter()
        .map(|(text, label)| Completion {
            span: Span {
                start: start as u32,
                end: end as u32,
            },
            text,
            label,
        })
        .collect()
}

/// El scanner conserva el contexto (código/comentarios/escapes). El carácter
/// provisional permite consultar también destinos vacíos e incompletos.
pub fn completion_context(
    source: &str,
    cursor: usize,
    wiki: bool,
) -> Option<minmark_document::Reference> {
    if !source.is_char_boundary(cursor) {
        return None;
    }
    let mut probe = source.to_owned();
    probe.insert(cursor, 'x');
    // Un wiki link incompleto necesita su delimitador de cierre.
    if wiki && !probe[cursor..].contains("]]") {
        probe.push_str("]]");
    }
    let mut r = scan_markdown_links(&probe)
        .references
        .into_iter()
        .find(|r| {
            (r.kind == RefKind::MarkdownLink || (wiki && r.kind == RefKind::WikiLink))
                && r.span.start as usize <= cursor
                && cursor < r.span.end as usize
        })?;
    r.span.end = r.span.end.saturating_sub(1).min(source.len() as u32);
    r.dest = source
        .get(r.span.start as usize..r.span.end as usize)?
        .to_owned();
    Some(r)
}

impl Snapshot {
    pub fn completions(&self, cursor: usize) -> Vec<Completion> {
        let Some(context) = completion_context(&self.source, cursor, self.wiki) else {
            return Vec::new();
        };
        let prefix = &self.source[context.span.start as usize..cursor];
        let mut results = Vec::new();
        if let Some((file, filter)) = prefix.split_once('#') {
            let target = self.query(|r| {
                if context.kind == RefKind::WikiLink {
                    r.resolve_wiki(file)
                } else {
                    r.resolve(file, &self.from)
                }
            });
            let rel = if file.is_empty() {
                Some(self.from.clone())
            } else {
                target.rel_path
            };
            if let Some(rel) = rel {
                let headings =
                    if let Some((_, source)) = self.buffers.iter().find(|(p, _)| p == &rel) {
                        minmark_document::document_anchors(&minmark_document::find_heading_nodes(
                            source,
                        ))
                    } else if rel == self.from {
                        minmark_document::document_anchors(&minmark_document::find_heading_nodes(
                            &self.source,
                        ))
                    } else {
                        self.index
                            .get(&rel)
                            .map(|e| {
                                e.headings
                                    .iter()
                                    .map(|h| minmark_document::Anchor {
                                        id: h.anchor.clone(),
                                        text: h.text.clone(),
                                        line: h.line,
                                        level: h.level,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                for h in headings {
                    if minmark_fs::fuzzy::fuzzy_match(&h.id, filter).is_some()
                        || minmark_fs::fuzzy::fuzzy_match(&h.text, filter).is_some()
                    {
                        results.push(Completion {
                            span: context.span,
                            text: format!("{file}#{}", h.id),
                            label: h.text,
                        });
                    }
                }
            }
        } else {
            let parent = std::path::Path::new(&self.from)
                .parent()
                .unwrap_or(std::path::Path::new(""));
            for e in self.index.entries() {
                let dest = if context.kind == RefKind::WikiLink {
                    e.rel_path
                        .strip_suffix(".markdown")
                        .or_else(|| e.rel_path.strip_suffix(".md"))
                        .unwrap_or(&e.rel_path)
                        .to_owned()
                } else {
                    minmark_fs::relative_path(parent, std::path::Path::new(&e.rel_path))
                };
                if minmark_fs::fuzzy::fuzzy_match(&dest, prefix).is_some() {
                    let text = if context.form == minmark_document::DestForm::Angle
                        || context.kind == RefKind::WikiLink
                    {
                        dest.clone()
                    } else {
                        dest.replace('%', "%25")
                            .replace(' ', "%20")
                            .replace('(', "%28")
                            .replace(')', "%29")
                    };
                    results.push(Completion {
                        span: context.span,
                        text,
                        label: dest,
                    });
                }
            }
        }
        results.truncate(50);
        results
    }

    pub fn backlinks(&self) -> Vec<Backlink> {
        self.query(|resolver| {
            let mut entries = Vec::new();
            for entry in self.index.entries() {
                if resolver
                    .resolve(&format!("/{}", entry.rel_path), &self.from)
                    .kind
                    != TargetKind::Local
                {
                    continue;
                }
                let source = self
                    .buffers
                    .iter()
                    .find(|(p, _)| p == &entry.rel_path)
                    .map(|(_, s)| s.clone())
                    .or_else(|| {
                        std::fs::read_to_string(self.index.root().join(&entry.rel_path)).ok()
                    });
                let Some(source) = source else { continue };
                for link in links(&source, &entry.rel_path, resolver, self.wiki) {
                    if link.target.kind != TargetKind::Local
                        || link.target.rel_path.as_deref() != Some(&self.from)
                    {
                        continue;
                    }
                    let line = source[..link.span.start as usize]
                        .bytes()
                        .filter(|&b| b == b'\n')
                        .count() as u32
                        + 1;
                    let context = source.lines().nth(line as usize - 1).unwrap_or("").trim();
                    let column = source[..link.span.start as usize]
                        .rsplit('\n')
                        .next()
                        .unwrap_or("")
                        .chars()
                        .count() as u32;
                    entries.push(Backlink {
                        entry: minmark_fs::fuzzy::Entry {
                            label: format!("{}:{line}:{} — {context}", entry.rel_path, column + 1),
                            rel_path: entry.rel_path.clone(),
                            line: Some(line),
                            heading: None,
                            score: 0,
                        },
                        position: minmark_document::Position::new(line - 1, column),
                    });
                }
            }
            entries
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "minmark-intelligence-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(root.join("docs")).unwrap();
            std::fs::write(root.join(".gitignore"), "ignorado.md\n").unwrap();
            std::fs::write(root.join("ignorado.md"), "# Oculto").unwrap();
            std::fs::write(root.join("docs/destino.md"), "# Antiguo\r\n").unwrap();
            std::fs::write(root.join("origen.md"), "[ir](docs/destino.md)\n").unwrap();
            Self(root)
        }
        fn snapshot(&self, source: &str) -> Snapshot {
            let mut index = ProjectIndex::new(self.0.clone());
            index.refresh().unwrap();
            Snapshot {
                index,
                buffers: vec![(
                    "docs/destino.md".into(),
                    "# Nuevo á\r\n\r\n## Detalle\r\n".into(),
                )],
                from: "origen.md".into(),
                source: source.into(),
                wiki: false,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn activacion_desde_etiqueta_y_referencia_usa_buffer_sin_guardar() {
        let f = Fixture::new();
        let source = "á [etiqueta](docs/destino.md#nuevo-á) [otra][id]\r\n\r\n[id]: docs/destino.md#detalle\r\n";
        let snapshot = f.snapshot(source);
        let links = snapshot.links();
        let offset = source.find("etiqueta").unwrap() as u32;
        let link = links
            .iter()
            .find(|l| l.span.start <= offset && offset < l.span.end)
            .unwrap();
        assert_eq!(link.target.kind, TargetKind::Local);
        assert_eq!(link.target.anchor.as_ref().unwrap().line, 1);
        let offset = source.find("otra").unwrap() as u32;
        assert_eq!(
            links
                .iter()
                .find(|l| l.span.start <= offset && offset < l.span.end)
                .unwrap()
                .target
                .anchor
                .as_ref()
                .unwrap()
                .line,
            3
        );
        assert_eq!(snapshot.source, source);
        assert_eq!(
            std::fs::read_to_string(f.0.join("docs/destino.md")).unwrap(),
            "# Antiguo\r\n"
        );
    }
    #[test]
    fn diagnosticos_archivo_fragmento_externo_y_wiki() {
        let f = Fixture::new();
        let mut s = f.snapshot(
            "[a](nada.md) [b](docs/destino.md#nada) [c](https://example.com) [[destino]]",
        );
        let links = s.links();
        assert_eq!(links.len(), 3);
        assert!(problem(&links[0].target).unwrap().contains("archivo"));
        assert!(problem(&links[1].target).unwrap().contains("fragmento"));
        assert!(problem(&links[2].target).is_none());
        s.wiki = true;
        assert_eq!(s.links().last().unwrap().target.kind, TargetKind::Local);
        std::fs::write(f.0.join("destino.md"), "# Otro").unwrap();
        s.index.refresh().unwrap();
        assert_eq!(s.links().last().unwrap().target.kind, TargetKind::Ambiguous);
    }
    #[test]
    fn contexto_de_completado_excluye_codigo_comentarios_escapes_y_titulos() {
        for source in [
            "`[a](dest`",
            "```md\n[a](dest",
            "    [a](dest",
            "<!-- [a](dest",
            "\\[a](dest",
            "[a](ruta \"dest",
        ] {
            let cursor = source.find("dest").unwrap() + 4;
            assert!(
                completion_context(source, cursor, true).is_none(),
                "{source}"
            );
        }
        for source in ["[a](", "á [a](de", "[a](uno) [b](de", "[a](<mi archivo>)"] {
            let cursor = source.find('>').unwrap_or(source.len());
            assert!(
                completion_context(source, cursor, false).is_some(),
                "{source}"
            );
        }
        assert!(completion_context("[[dest", 6, false).is_none());
        assert!(completion_context("[[dest", 6, true).is_some());
    }
    #[test]
    fn completar_solo_destino_y_una_operacion_undo_crlf() {
        use minmark_document::{Document, Editor, Range};
        let f = Fixture::new();
        let source = "á [etiqueta](docs/de \"título\")\r\n";
        let s = f.snapshot(source);
        let cursor = source.find("de \"").unwrap() + 2;
        let completion = s
            .completions(cursor)
            .into_iter()
            .find(|c| c.text == "docs/destino.md")
            .unwrap();
        let mut editor = Editor::new(Document::from_text(None, source.into()));
        let range = Range::new(
            editor.position_of(completion.span.start as usize),
            editor.position_of(completion.span.end as usize),
        );
        editor.replace(range, &completion.text).unwrap();
        assert_eq!(
            editor.document().content(),
            "á [etiqueta](docs/destino.md \"título\")\r\n"
        );
        editor.undo();
        assert_eq!(editor.document().content(), source);
        editor.redo();
        assert!(editor.document().content().contains("docs/destino.md"));
    }
    #[test]
    fn sugerencias_de_headings_actuales_y_archivos_ignorados() {
        let f = Fixture::new();
        let s = f.snapshot("[a](docs/destino.md#nu)");
        assert_eq!(
            s.completions(s.source.len() - 1)[0].text,
            "docs/destino.md#nuevo-á"
        );
        let s = f.snapshot("[a](");
        assert!(s
            .completions(s.source.len())
            .iter()
            .all(|c| !c.text.contains("ignorado")));
    }
    #[test]
    fn backlinks_reflejan_overlay_borrado_y_cada_ocurrencia() {
        let f = Fixture::new();
        let mut s = f.snapshot("# Nuevo á");
        s.from = "docs/destino.md".into();
        s.buffers.push((
            "origen.md".into(),
            "[uno](docs/destino.md) y [dos](docs/destino.md#nuevo-á)".into(),
        ));
        assert_eq!(s.backlinks().len(), 2);
        s.buffers.last_mut().unwrap().1 = "Sin enlaces".into();
        assert!(s.backlinks().is_empty());
        s.buffers.pop();
        std::fs::remove_file(f.0.join("origen.md")).unwrap();
        s.index.refresh().unwrap();
        assert!(s.backlinks().is_empty());
    }
}

#[cfg(test)]
mod citation_tests {
    use super::*;
    fn bibliography() -> minmark_renderer::Bibliography {
        minmark_renderer::Bibliography::from_sources(
            &[(
                "refs.bib".into(),
                include_str!("../../docs/qa/fixtures/references.bib").into(),
            )],
            None,
        )
    }
    #[test]
    fn citation_contexts_exclude_code_escapes_comments_and_locators() {
        let bibliography = bibliography();
        for source in [
            "`[@Sm`",
            "```md\n[@Sm",
            "\\[@Sm",
            "<!-- [@Sm -->",
            "mail@Sm",
            "[@smith2025, p. Sm",
        ] {
            let cursor = source.find("Sm").unwrap() + 2;
            assert!(
                citation_completions(source, cursor, &bibliography).is_empty(),
                "{source}"
            );
        }
        for source in ["[@Sm", "[@Sm]", "[@jones2024; @Sm]", "[@2025]", "[@Energy]"] {
            let cursor = source.trim_end_matches(']').len();
            assert_eq!(
                citation_completions(source, cursor, &bibliography)[0].text,
                "smith2025",
                "{source}"
            );
        }
    }
    #[test]
    fn completion_preserves_unicode_crlf_locator_and_single_undo() {
        use minmark_document::{Document, Editor, Range};
        let source = "β [@Sm, p. 42]\r\n";
        let cursor = source.find(", p.").unwrap();
        let completion = citation_completions(source, cursor, &bibliography()).remove(0);
        let mut editor = Editor::new(Document::from_text(None, source.into()));
        editor
            .replace(
                Range::new(
                    editor.position_of(completion.span.start as usize),
                    editor.position_of(completion.span.end as usize),
                ),
                &completion.text,
            )
            .unwrap();
        assert_eq!(editor.document().content(), "β [@smith2025, p. 42]\r\n");
        editor.undo();
        assert_eq!(editor.document().content(), source);
        editor.redo();
        assert_eq!(editor.document().content(), "β [@smith2025, p. 42]\r\n");
    }
}
