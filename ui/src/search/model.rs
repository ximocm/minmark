//! Modelo puro del panel de búsqueda de proyecto (ISSUE-028, §40.2).
//!
//! Sin GTK (mismo patrón que [`crate::find::FindState`]): agrupación de
//! resultados por archivo, expandir/contraer, **exclusión individual de
//! coincidencias** (§40.2) y selección de alcance del reemplazo. Los tests
//! ejercen la lógica sin inicializar widgets.

use minmark_fs::search::{FileMatches, LineMatch, ReplaceFile, SearchScope};
use std::collections::BTreeSet;

/// Alcance de un reemplazo sobre la selección (§40.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceScope {
    /// «Reemplazar»: solo la coincidencia activa (aunque esté excluida: es
    /// una acción explícita sobre ella).
    Current,
    /// «Reemplazar en fichero»: las incluidas del grupo activo.
    Group,
    /// «Reemplazar todo»: las incluidas de todos los grupos.
    All,
}

/// Resultados de un archivo con su estado de UI (§40.2).
#[derive(Debug, Clone)]
pub struct ResultGroup {
    pub rel_path: String,
    pub matches: Vec<LineMatch>,
    /// El grupo está expandido (§40.2: expandir/contraer).
    pub expanded: bool,
    /// Índices de coincidencias **excluidas** (§40.2: exclusión individual).
    excluded: BTreeSet<usize>,
}

impl ResultGroup {
    /// ¿La coincidencia `match_index` está excluida del reemplazo?
    pub fn is_excluded(&self, match_index: usize) -> bool {
        self.excluded.contains(&match_index)
    }

    /// Coincidencias incluidas (no excluidas).
    pub fn included_count(&self) -> usize {
        self.matches.len() - self.excluded.len()
    }

    /// Rangos byte `[start, end)` de las coincidencias incluidas.
    pub fn included_ranges(&self) -> Vec<(usize, usize)> {
        (0..self.matches.len())
            .filter(|index| !self.excluded.contains(index))
            .map(|index| (self.matches[index].start, self.matches[index].end))
            .collect()
    }
}

/// Estado de resultados del panel (§40.2): agrupación, exclusión y activa.
#[derive(Debug, Default)]
pub struct SearchModel {
    groups: Vec<ResultGroup>,
    current: Option<(usize, usize)>,
}

impl SearchModel {
    /// Sustituye todos los resultados (nueva búsqueda). Los grupos nuevos
    /// entran expandidos y sin exclusiones; se limpia la activa.
    #[cfg(test)]
    pub fn set_results(&mut self, files: Vec<FileMatches>) {
        self.groups = files.into_iter().map(new_group).collect();
        self.current = None;
    }

    /// Acumula un lote parcial (§47.5) conservando el estado de los grupos
    /// ya presentes.
    pub fn append_batch(&mut self, batch: Vec<FileMatches>) {
        for file in batch {
            match self.groups.iter_mut().find(|g| g.rel_path == file.rel_path) {
                Some(group) => group.matches.extend(file.matches),
                None => self.groups.push(new_group(file)),
            }
        }
    }

    /// Vacía resultados y estado.
    pub fn clear(&mut self) {
        self.groups.clear();
        self.current = None;
    }

    pub fn groups(&self) -> &[ResultGroup] {
        &self.groups
    }

    /// Expande/contrae el grupo `group`; devuelve el nuevo estado.
    pub fn toggle_expanded(&mut self, group: usize) -> bool {
        let Some(target) = self.groups.get_mut(group) else {
            return false;
        };
        target.expanded = !target.expanded;
        target.expanded
    }

    /// Incluye/excluye la coincidencia (§40.2); devuelve el nuevo estado de
    /// inclusión.
    pub fn toggle_excluded(&mut self, group: usize, match_index: usize) -> bool {
        let Some(target) = self.groups.get_mut(group) else {
            return false;
        };
        if match_index >= target.matches.len() {
            return false;
        }
        if target.excluded.remove(&match_index) {
            true
        } else {
            target.excluded.insert(match_index);
            false
        }
    }

    /// Fija la coincidencia activa (la del foco de «Reemplazar»).
    pub fn set_current(&mut self, group: usize, match_index: usize) -> bool {
        let valida = self
            .groups
            .get(group)
            .is_some_and(|target| match_index < target.matches.len());
        if valida {
            self.current = Some((group, match_index));
        }
        valida
    }

    /// Coincidencia activa `(grupo, índice)`.
    pub fn current(&self) -> Option<(usize, usize)> {
        self.current
    }

    /// Coincidencia activa con su grupo y sus datos, si la hay.
    pub fn current_match(&self) -> Option<(&ResultGroup, &LineMatch)> {
        let (group, match_index) = self.current?;
        let target = self.groups.get(group)?;
        Some((target, &target.matches[match_index]))
    }

    /// Ficheros con al menos una coincidencia (§40.2 contadores).
    pub fn total_files(&self) -> usize {
        self.groups.len()
    }

    /// Coincidencias totales.
    pub fn total_matches(&self) -> usize {
        self.groups.iter().map(|g| g.matches.len()).sum()
    }

    /// Coincidencias incluidas (las que «Reemplazar todo» tocaría).
    pub fn included_matches(&self) -> usize {
        self.groups.iter().map(ResultGroup::included_count).sum()
    }

    /// Coincidencias incluidas del grupo `group`.
    pub fn included_in(&self, group: usize) -> usize {
        self.groups
            .get(group)
            .map(ResultGroup::included_count)
            .unwrap_or(0)
    }

    /// Selección de reemplazo para `scope` (§40.2), lista para
    /// [`minmark_fs::search::plan_replace`]. Sin selección válida, vacía.
    pub fn replace_files(&self, scope: ReplaceScope) -> Vec<ReplaceFile> {
        match scope {
            ReplaceScope::Current => {
                let Some((group, match_index)) = self.current else {
                    return Vec::new();
                };
                let target = &self.groups[group];
                let hit = &target.matches[match_index];
                vec![ReplaceFile {
                    rel_path: target.rel_path.clone(),
                    ranges: vec![(hit.start, hit.end)],
                }]
            }
            ReplaceScope::Group => {
                let Some((group, _)) = self.current else {
                    return Vec::new();
                };
                let target = &self.groups[group];
                let ranges = target.included_ranges();
                if ranges.is_empty() {
                    return Vec::new();
                }
                vec![ReplaceFile {
                    rel_path: target.rel_path.clone(),
                    ranges,
                }]
            }
            ReplaceScope::All => self
                .groups
                .iter()
                .filter_map(|target| {
                    let ranges = target.included_ranges();
                    (!ranges.is_empty()).then(|| ReplaceFile {
                        rel_path: target.rel_path.clone(),
                        ranges,
                    })
                })
                .collect(),
        }
    }
}

fn new_group(file: FileMatches) -> ResultGroup {
    ResultGroup {
        rel_path: file.rel_path,
        matches: file.matches,
        expanded: true,
        excluded: BTreeSet::new(),
    }
}

/// Alcance §40.2 desde el selector de la UI: `0` = Markdown (por defecto),
/// `1` = todos los archivos, `2` = extensiones personalizadas (texto
/// separado por `,` o `;`; vacío cae al alcance por defecto).
pub fn scope_from(index: u32, custom: &str) -> SearchScope {
    match index {
        1 => SearchScope::all(),
        2 => {
            let exts: Vec<&str> = custom.split([',', ';']).collect();
            if exts.iter().all(|e| e.trim().is_empty()) {
                SearchScope::markdown()
            } else {
                SearchScope::extensions(exts)
            }
        }
        _ => SearchScope::markdown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(line: usize, start: usize, end: usize) -> LineMatch {
        LineMatch {
            line,
            col: 1,
            line_text: "objetivo".to_owned(),
            start,
            end,
        }
    }

    fn file(rel_path: &str, spans: &[(usize, usize)]) -> FileMatches {
        FileMatches {
            rel_path: rel_path.to_owned(),
            matches: spans
                .iter()
                .enumerate()
                .map(|(index, (start, end))| hit(index + 1, *start, *end))
                .collect(),
        }
    }

    /// §40.2: agrupación por fichero, contadores y exclusión individual.
    #[test]
    fn agrupacion_contadores_y_exclusion_individual() {
        let mut model = SearchModel::default();
        model.set_results(vec![
            file("uno.md", &[(0, 3), (10, 13)]),
            file("dos.md", &[(5, 8)]),
        ]);
        assert_eq!(model.total_files(), 2);
        assert_eq!(model.total_matches(), 3);
        assert_eq!(model.included_matches(), 3);

        // Exclusión individual: desmarcar una coincidencia la saca del
        // reemplazo, sin tocar las demás.
        assert!(!model.toggle_excluded(0, 1));
        assert!(model.groups()[0].is_excluded(1));
        assert_eq!(model.included_matches(), 2);
        assert_eq!(model.included_in(0), 1);
        assert!(model.toggle_excluded(0, 1), "reincluye");
        assert_eq!(model.included_matches(), 3);

        // Índices fuera de rango no cambian nada.
        assert!(!model.toggle_excluded(9, 0));
        assert!(!model.toggle_excluded(0, 9));
        assert_eq!(model.included_matches(), 3);
    }

    /// §40.2: expandir/contraer por grupo y lotes parciales que conservan el
    /// estado (§47.5).
    #[test]
    fn expandir_contraer_y_lotes_conservan_estado() {
        let mut model = SearchModel::default();
        model.set_results(vec![file("uno.md", &[(0, 3)])]);
        assert!(model.groups()[0].expanded);
        assert!(!model.toggle_expanded(0));
        model.toggle_excluded(0, 0);

        // Lote parcial: mismo fichero crece, fichero nuevo se añade y el
        // estado (contraído, excluido) sobrevive.
        model.append_batch(vec![file("uno.md", &[(20, 23)]), file("dos.md", &[(0, 3)])]);
        assert_eq!(model.total_files(), 2);
        assert_eq!(model.total_matches(), 3);
        assert!(!model.groups()[0].expanded);
        assert!(model.groups()[0].is_excluded(0));
        assert!(!model.groups()[0].is_excluded(1));
        assert_eq!(model.included_matches(), 2);

        model.clear();
        assert_eq!(model.total_files(), 0);
        assert!(model.current().is_none());
    }

    /// §40.2: reemplazo individual (activa, aunque esté excluida), por
    /// fichero (solo incluidas) y global (solo incluidas).
    #[test]
    fn seleccion_de_reemplazo_individual_por_fichero_y_global() {
        let mut model = SearchModel::default();
        model.set_results(vec![
            file("uno.md", &[(0, 3), (10, 13)]),
            file("dos.md", &[(5, 8)]),
        ]);
        assert_eq!(
            model.replace_files(ReplaceScope::All),
            vec![
                ReplaceFile {
                    rel_path: "uno.md".into(),
                    ranges: vec![(0, 3), (10, 13)]
                },
                ReplaceFile {
                    rel_path: "dos.md".into(),
                    ranges: vec![(5, 8)]
                },
            ]
        );

        model.toggle_excluded(0, 1);
        assert!(model.set_current(0, 1));
        // Individual: la activa va aunque esté excluida (acción explícita).
        assert_eq!(
            model.replace_files(ReplaceScope::Current),
            vec![ReplaceFile {
                rel_path: "uno.md".into(),
                ranges: vec![(10, 13)]
            }]
        );
        // Por fichero: solo las incluidas del grupo activo.
        assert_eq!(
            model.replace_files(ReplaceScope::Group),
            vec![ReplaceFile {
                rel_path: "uno.md".into(),
                ranges: vec![(0, 3)]
            }]
        );
        // Global: solo incluidas, de todos los grupos.
        assert_eq!(
            model.replace_files(ReplaceScope::All),
            vec![
                ReplaceFile {
                    rel_path: "uno.md".into(),
                    ranges: vec![(0, 3)]
                },
                ReplaceFile {
                    rel_path: "dos.md".into(),
                    ranges: vec![(5, 8)]
                },
            ]
        );

        // Sin activa no hay reemplazo individual ni por fichero.
        model.clear();
        assert!(model.replace_files(ReplaceScope::Current).is_empty());
        assert!(model.replace_files(ReplaceScope::Group).is_empty());
    }

    /// §40.2: el alcance es `.md` por defecto, ampliable a extensiones o a
    /// «todos».
    #[test]
    fn alcance_desde_el_selector() {
        assert_eq!(scope_from(0, ""), SearchScope::markdown());
        assert_eq!(scope_from(3, ""), SearchScope::markdown());
        assert_eq!(scope_from(1, "txt"), SearchScope::all());
        assert_eq!(
            scope_from(2, "txt, .log"),
            SearchScope::extensions(["txt", ".log"])
        );
        // Personalizado vacío cae al alcance por defecto (§40.2).
        assert_eq!(scope_from(2, " , ;"), SearchScope::markdown());
    }
}
