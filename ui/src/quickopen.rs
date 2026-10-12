//! Quick Open: paleta modal `Ctrl+P` (ISSUE-029, §41 y §39).
//!
//! - §41: fuzzy sobre los archivos del proyecto y tres modos más en la misma
//!   paleta: `:N` (salto a línea), `@heading` (documento actual) y `#heading`
//!   (todo el proyecto). La consulta se resuelve con
//!   [`minmark_fs::fuzzy::quickopen_query`] sobre el índice §45 (ISSUE-025):
//!   la paleta no hace E/S por pulsación (§47.5).
//! - §39: el comando `win.quick-open` con acelerador `<Control>p` la presenta
//!   (el cableado vive en `main.rs`; ver handoff ISSUE-029).
//! - Interacción: campo de texto + lista de resultados; ↑/↓ navega, Enter
//!   abre (doble click también), Esc cierra; un indicador muestra el modo
//!   según el prefijo (`:`, `@`, `#`).
//! - Al abrir se dispara `on_open(rel_path, línea)` con la línea resuelta
//!   (`:N` directa; heading vía [`minmark_fs::fuzzy::resolve_heading_line`],
//!   el índice §45 no guarda líneas) para que el llamante abra la pestaña
//!   con `OpenTarget` (ISSUE-021) y posicione cursor/vista (§8.6).
//!
//! Módulo autocontenido: solo necesita `gtk4` y `minmark-fs`; el `mod` se
//! declara en `main.rs` con el cableado final.

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use minmark_fs::fuzzy::{self, Entry, QuickOpenMode};
use minmark_fs::ProjectIndex;
use std::cell::RefCell;
use std::rc::Rc;

/// Fuente de resultados de la paleta (§41).
enum Source {
    /// Consulta por pulsación sobre el índice del proyecto (§45).
    Index(ProjectIndex),
    /// Lista fija ya preparada por el llamante (se difumina por etiqueta).
    Entries(Vec<Entry>),
}

/// Estado mutable compartido entre la paleta y sus callbacks GTK.
#[derive(Default)]
struct State {
    source: Option<Source>,
    /// `rel_path` (§17) del documento activo: destino de `@heading` y `:N`.
    current: Option<String>,
    /// Rutas abiertas, la más reciente primero (§41: recencia de apertura).
    recent: Vec<String>,
    /// `Show All Files` (§14.4): sin filtro de tipos documentales.
    show_all_files: bool,
    /// Resultados mostrados, en orden.
    results: Vec<Entry>,
    /// Fila seleccionada (↑/↓ o click).
    selected: usize,
    /// Callback de apertura: `(rel_path, línea)`.
    on_open: Option<Rc<dyn Fn(String, Option<u32>)>>,
}

/// Paleta Quick Open (§41) con su ventana modal propia.
pub struct QuickOpen {
    window: gtk::Window,
    input: gtk::Entry,
    mode_label: gtk::Label,
    list: gtk::ListBox,
    state: Rc<RefCell<State>>,
}

/// Texto del indicador de modo según el prefijo tecleado (§41).
fn mode_name(query: &str) -> &'static str {
    match fuzzy::parse_query(query).mode {
        QuickOpenMode::Files => "Files",
        QuickOpenMode::Line => "Go to line (:N)",
        QuickOpenMode::DocHeadings => "Headings — document (@)",
        QuickOpenMode::ProjectHeadings => "Headings — project (#)",
    }
}

/// Filtra una lista fija por fuzzy sobre la etiqueta (empate: orden dado).
fn filter_static(entries: &[Entry], filter: &str) -> Vec<Entry> {
    let mut results: Vec<Entry> = entries
        .iter()
        .filter_map(|entry| {
            let hit = fuzzy::fuzzy_match(&entry.label, filter)?;
            let mut entry = entry.clone();
            entry.score = hit.score;
            Some(entry)
        })
        .collect();
    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(fuzzy::MAX_RESULTS);
    results
}

/// Calcula los resultados para el texto tecleado (§41).
fn recompute(state: &State, query: &str) -> Vec<Entry> {
    let parsed = fuzzy::parse_query(query);
    match &state.source {
        Some(Source::Index(index)) => fuzzy::quickopen_query(
            parsed.mode,
            query,
            index,
            state.current.as_deref(),
            &state.recent,
            state.show_all_files,
        ),
        Some(Source::Entries(entries)) => filter_static(entries, parsed.filter),
        None => Vec::new(),
    }
}

/// Repinta la lista de resultados y marca la fila seleccionada.
fn repaint(list: &gtk::ListBox, state: &State) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
    for entry in &state.results {
        let label = gtk::Label::new(Some(&entry.label));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
    if !state.results.is_empty() {
        select_row(list, state.selected);
    }
}

/// Marca la fila `index` como seleccionada en la lista.
fn select_row(list: &gtk::ListBox, index: usize) {
    list.select_row(list.row_at_index(index as i32).as_ref());
}

/// Abre la fila seleccionada (§41): resuelve la línea del heading si hace
/// falta (una lectura al abrir, nunca por pulsación §47.5) y dispara
/// `on_open(rel_path, línea)`.
fn open_selected(state: &RefCell<State>, window: &gtk::Window) {
    let (entry, line, on_open) = {
        let state = state.borrow();
        let Some(entry) = state.results.get(state.selected).cloned() else { return };
        let line = match (&entry.line, &entry.heading) {
            (Some(line), _) => Some(*line),
            (None, Some(heading)) => match &state.source {
                Some(Source::Index(index)) => {
                    fuzzy::resolve_heading_line(index.root(), &entry.rel_path, heading)
                }
                _ => None,
            },
            (None, None) => None,
        };
        (entry, line, state.on_open.clone())
    };
    window.set_visible(false);
    if let Some(on_open) = on_open {
        on_open(entry.rel_path.clone(), line);
    }
}

impl QuickOpen {
    /// Crea la paleta (oculta hasta [`QuickOpen::present`]).
    pub fn new() -> Self {
        let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::default()));

        let window = gtk::Window::new();
        window.set_title(Some("Quick Open"));
        window.set_modal(true);
        window.set_default_size(640, 420);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        root.set_margin_start(8);
        root.set_margin_end(8);

        let mode_label = gtk::Label::new(Some(mode_name("")));
        mode_label.set_xalign(0.0);
        root.append(&mode_label);

        let input = gtk::Entry::new();
        input.set_placeholder_text(Some("File, :line, @heading, #project heading"));
        root.append(&input);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_child(Some(&list));
        scroll.set_vexpand(true);
        scroll.set_min_content_height(320);
        root.append(&scroll);

        window.set_child(Some(&root));

        let palette = QuickOpen {
            window: window.clone(),
            input: input.clone(),
            mode_label: mode_label.clone(),
            list: list.clone(),
            state: state.clone(),
        };

        // Recalcula y repinta la lista ante cada pulsación (§41).
        {
            let state = state.clone();
            let input = input.clone();
            let mode_label = mode_label.clone();
            let list = list.clone();
            input.connect_changed(move |entry| {
                let mut state = state.borrow_mut();
                state.selected = 0;
                state.results = recompute(&state, &entry.text());
                mode_label.set_text(mode_name(&entry.text()));
                repaint(&list, &state);
            });
        }

        // ↑/↓ navega; Esc cierra (§41).
        {
            let state = state.clone();
            let list = list.clone();
            let window = window.clone();
            let controller = gtk::EventControllerKey::new();
            controller.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Up {
                    let mut state = state.borrow_mut();
                    if state.selected > 0 {
                        state.selected -= 1;
                        select_row(&list, state.selected);
                    }
                    glib::Propagation::Stop
                } else if key == gtk4::gdk::Key::Down {
                    let mut state = state.borrow_mut();
                    if state.selected + 1 < state.results.len() {
                        state.selected += 1;
                        select_row(&list, state.selected);
                    }
                    glib::Propagation::Stop
                } else if key == gtk4::gdk::Key::Escape {
                    window.set_visible(false);
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
            input.add_controller(controller);
        }

        // Enter abre la fila seleccionada (§41).
        {
            let state = state.clone();
            let window = window.clone();
            input.connect_activate(move |_| {
                open_selected(&state, &window);
            });
        }

        // Doble click sobre una fila: abre esa fila.
        {
            let state = state.clone();
            let window = window.clone();
            let list = list.clone();
            list.connect_row_activated(move |list, row| {
                state.borrow_mut().selected = row.index().max(0) as usize;
                let selected = state.borrow().selected;
                select_row(list, selected);
                open_selected(&state, &window);
            });
        }

        // El click simple sincroniza la selección con ↑/↓ (best-effort: el
        // repintado ya tiene el `RefCell` prestado).
        {
            let state = state.clone();
            list.connect_selected_rows_changed(move |list| {
                if let Some(row) = list.selected_row() {
                    if let Ok(mut state) = state.try_borrow_mut() {
                        state.selected = row.index().max(0) as usize;
                    }
                }
            });
        }

        // Cerrar (Esc o ✕) solo oculta: la paleta es reusable (§41).
        window.connect_close_request(move |window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });

        palette
    }

    /// Muestra la paleta con el campo limpio (§41: fuzzy desde cero).
    pub fn present(&self) {
        self.input.set_text("");
        self.input.grab_focus();
        self.refresh();
        self.window.present();
    }

    /// Fija la lista de entradas ya preparada (fuente fija; alternativa a
    /// [`QuickOpen::set_index`]: gana la última llamada).
    pub fn set_entries(&self, entries: Vec<Entry>) {
        self.state.borrow_mut().source = Some(Source::Entries(entries));
        self.refresh();
    }

    /// Fija el índice del proyecto (§45) como fuente de resultados (§41:
    /// archivos, `:N`, `@heading` y `#heading`).
    pub fn set_index(&self, index: &ProjectIndex) {
        self.state.borrow_mut().source = Some(Source::Index(index.clone()));
        self.refresh();
    }

    /// Documento activo (`rel_path` §17) para `:N` y `@heading`.
    pub fn set_current_path(&self, rel_path: Option<String>) {
        self.state.borrow_mut().current = rel_path;
        self.refresh();
    }

    /// Recencia de apertura (§41): rutas abiertas, la más reciente primero.
    pub fn set_recent(&self, recent: Vec<String>) {
        self.state.borrow_mut().recent = recent;
        self.refresh();
    }

    /// `Show All Files` (§14.4): sin filtro de tipos documentales en las
    /// consultas sobre el índice.
    pub fn set_show_all_files(&self, show_all_files: bool) {
        self.state.borrow_mut().show_all_files = show_all_files;
        self.refresh();
    }

    /// Callback de apertura (§41): `(rel_path, línea)`; la línea ya está
    /// resuelta (`:N` o heading §8.6) y puede ser `None` si no se pudo.
    pub fn set_on_open(&self, on_open: Rc<dyn Fn(String, Option<u32>)>) {
        self.state.borrow_mut().on_open = Some(on_open);
    }

    /// Ancla la paleta a su ventana (modal §36); opcional.
    pub fn set_transient_for(&self, parent: &gtk::Window) {
        self.window.set_transient_for(Some(parent));
    }

    /// Recalcula y repinta con el texto actual.
    fn refresh(&self) {
        let mut state = self.state.borrow_mut();
        state.results = recompute(&state, &self.input.text());
        state.selected = state.selected.min(state.results.len().saturating_sub(1));
        self.mode_label.set_text(mode_name(&self.input.text()));
        repaint(&self.list, &state);
    }
}

impl Default for QuickOpen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicador_de_modo_segun_prefijo_41() {
        assert_eq!(mode_name("gui"), "Files");
        assert_eq!(mode_name(":42"), "Go to line (:N)");
        assert_eq!(mode_name("docs/gui:7"), "Go to line (:N)");
        assert_eq!(mode_name("@inst"), "Headings — document (@)");
        assert_eq!(mode_name("#det"), "Headings — project (#)");
    }

    #[test]
    fn filtro_de_lista_fija_respeta_el_techo_y_el_orden() {
        let entries: Vec<Entry> = (0..150)
            .map(|i| Entry {
                label: format!("doc_{i:03}.md"),
                rel_path: format!("doc_{i:03}.md"),
                line: None,
                score: 0,
                heading: None,
            })
            .collect();

        // Sin filtro: el techo §47.5 conserva el orden dado.
        let todos = filter_static(&entries, "");
        assert_eq!(todos.len(), fuzzy::MAX_RESULTS);
        assert_eq!(todos[0].label, "doc_000.md");

        // Filtro único y sin resultados para una consulta no subsecuente.
        let uno = filter_static(&entries, "doc_001");
        assert_eq!(uno.iter().map(|entry| entry.label.as_str()).collect::<Vec<_>>(), ["doc_001.md"]);
        assert!(filter_static(&entries, "zzz").is_empty());
    }
}
