//! minmark — aplicación de escritorio GTK4 + WebKitGTK 6.0 (ISSUE-020, ADR-001).
//!
//! Integra el core (`core/`) con la UI decidida en ADR-001:
//! - menús §36 (File, Edit, View, Insert, Format, Project, Help) y menú
//!   Insert/Format de los controles contextuales (fase 7, #29/#30/#31:
//!   Image y Table con #30, Equation y Citation con #31);
//! - atajos §39 (Ctrl+N/O/S, Ctrl+Shift+S, Ctrl+W, Ctrl+Alt+M, más los de
//!   la fase 7: Ctrl+Shift+B/C/X, Ctrl+I/K, Ctrl+1..6, Ctrl+Shift+P);
//! - Command Palette §38 (`Ctrl+Shift+P`) y popover de controles
//!   contextuales §44 (#29/#30/#31, con «Edit Image», «Table Attributes»,
//!   «Edit Equation», «Equation Attributes» y «Edit Citation»): mismas
//!   acciones `win.*` que menú y atajos, con destino único
//!   `TabManager::apply_format_active`;
//! - composiciones (§20, ISSUE-022/023): Preview combinado y PDF único desde
//!   el menú Project/File, con la cadena de ajustes §58 y la bibliografía
//!   unida de los archivos incluidos (ISSUE-024);
//! - editor Markdown Mode (ISSUE-015) y modo Visual §8.2 (ISSUE-030:
//!   representación semántica editable sobre el mismo buffer);
//! - Preview WebView continuo (ISSUE-016) con temas (ISSUE-017);
//! - pestañas §37 con indicador ● de modificado;
//! - ventanas múltiples (ISSUE-021): standalone tipo Notepad++ (archivos
//!   sueltos, nuevos o existentes) y ventanas de proyecto por carpeta (§48);
//! - explorador de archivos con árbol perezoso (ISSUE-022, §14.3–14.7) en las
//!   ventanas de proyecto;
//! - barra lateral estilo VS Code (§40.2): búsqueda en el proyecto con
//!   `Ctrl+Shift+F` (ISSUE-028) junto al explorador, una vista visible cada
//!   vez;
//! - Quick Open `Ctrl+P` (ISSUE-029, §41): paleta fuzzy sobre el índice §45
//!   con `:N`, `@heading`, `#heading` y MRU de abiertos;
//! - diálogos GTK4 y confirmaciones explícitas (§3.7, §12.7).
//!
//! Buffer único §6.1: el modelo vive en `minmark_document::Editor` dentro de
//! `TabManager`; el `TextBuffer` del TextView es el espejo sincronizado del
//! mismo documento en los dos modos §8 (Markdown y Visual, ISSUE-030).

// `pub` para que el binario de prueba `tests/qa_preview.rs` (ISSUE-57) pueda
// reexportarlos al incluir este archivo como módulo y las rutas `crate::…`
// resuelvan igual que en el binario.
pub mod academic;
pub mod command_palette;
pub mod composition;
pub mod editor;
pub mod explorer;
pub mod export;
pub mod find;
pub mod intelligence;
pub mod list_scroll;
pub mod menus;
pub mod preview;
pub mod quickopen;
pub mod runtime;
pub mod search;
pub mod settings;
pub mod tabs;

use editor::{EditSink, EditorPane};
use explorer::{load_state, save_state, ExplorerAction, ExplorerPane, ExplorerState};
use find::{FindAction, FindBar};
use menus::{actions, dialogs, install_actions, Commands};
use preview::{PreviewPane, ViewMode};
use quickopen::QuickOpen;
use search::{ReplaceSummary, SearchPanel};
use tabs::{OpenTarget, TabBar, TabManager, WindowScope};

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation, Paned, PopoverMenuBar,
    Widget,
};
use minmark_config::{
    detect_system_theme, resolve_theme, Composition, ResolvedTheme, Settings, VisualSyntax,
};
use minmark_document::{Position, Range, SearchOptions, Selection};
use minmark_fs::Watcher as _;
use minmark_renderer::Palette;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Duration;

/// Retardo de refresco de Preview/highlight tras teclear (§47.1: las tareas
/// secundarias nunca bloquean una pulsación de teclado).
const REFRESH_DEBOUNCE_MS: u64 = 120;

/// Periodo del bucle de eventos del watcher de proyecto (ISSUE-026, §46):
/// son eventos de inotify coalescidos, nunca un rescan del filesystem.
const WATCHER_POLL_MS: u64 = 250;

/// Periodo del timer local que integra los resultados de los hilos worker en
/// el bucle principal (patrón §47.5 de la búsqueda §40.2: canal `mpsc` +
/// timer GLib); solo la entrega a Quick Open, nunca la E/S.
const DELIVERY_POLL_MS: u64 = 60;

/// Tamaño del MRU de Quick Open (§41): el ranking solo bonifica los 8 últimos
/// abiertos (misma cota que `minmark_fs::fuzzy`).
const MRU_MAX: usize = 8;

/// Cambios de disco generados por nuestras propias operaciones §14.8
/// (ISSUE-026 criterio 3): se registran antes de escribir y los eventos del
/// watcher que coinciden no se marcan como cambio externo §12.7.
#[derive(Default)]
struct SelfOps {
    paths: RefCell<Vec<PathBuf>>,
}

impl SelfOps {
    fn register(&self, path: &Path) {
        self.paths.borrow_mut().push(path.to_path_buf());
    }

    /// `true` si `path` corresponde a una operación propia (una entrada por
    /// evento: un watcher duplicado no resucita el aviso).
    fn consume(&self, path: &Path) -> bool {
        let mut paths = self.paths.borrow_mut();
        let hit = paths
            .iter()
            .position(|own| own == path || path.starts_with(own) || own.starts_with(path));
        match hit {
            Some(index) => {
                paths.remove(index);
                true
            }
            None => false,
        }
    }
}

/// Refresco del índice §45 en una tarea aparte (§47.1/§47.5) con entrega al
/// hilo de UI al terminar. El refresco es incremental e idempotente
/// (ISSUE-025) y como mucho se pide una vez por tick del watcher; el índice
/// ya refrescado se reenvía a Quick Open (ISSUE-029, §41) para consultas
/// frescas.
fn spawn_index_refresh(root: PathBuf, slot: Rc<CmdSlot>) {
    let generation = slot
        .borrow()
        .as_ref()
        .and_then(Weak::upgrade)
        .map(|me| {
            let generation = me.index_generation.get().wrapping_add(1);
            me.index_generation.set(generation);
            generation
        })
        .unwrap_or(0);
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut index = minmark_fs::ProjectIndex::load_or_new(&root);
        if let Err(err) = index.refresh() {
            eprintln!("minmark: índice de proyecto no refrescado: {err}");
            return;
        }
        if let Err(err) = index.save() {
            eprintln!("minmark: no se pudo guardar la caché del índice: {err}");
        }
        let _ = tx.send((generation, index));
    });
    deliver_to_quickopen(rx, slot, |me, (generation, index)| {
        if generation != me.index_generation.get()
            || me.quickopen_base().as_deref() != Some(index.root())
        {
            return;
        }
        if me.scope.project_root().is_some() {
            me.quickopen.set_index(&index);
        }
        *me.index.borrow_mut() = Some(index);
        me.refresh_intelligence(false);
    });
}

/// Integra el resultado de un hilo worker en el bucle principal y lo entrega
/// a la paleta Quick Open (patrón §47.5 de la búsqueda §40.2: canal `mpsc` +
/// timer GLib). El timer se detiene al recibir o al cerrarse el canal.
fn deliver_to_quickopen<T: 'static>(
    rx: Receiver<T>,
    slot: Rc<CmdSlot>,
    apply: impl Fn(&AppCmds, T) + 'static,
) {
    glib::timeout_add_local(Duration::from_millis(DELIVERY_POLL_MS), move || {
        match rx.try_recv() {
            Ok(value) => {
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    apply(&me, value);
                }
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Lista de archivos de Quick Open en ventanas standalone (ISSUE-029, §41):
/// el `walk_tree` + `IgnorePatterns` del explorador, con la visibilidad por
/// defecto §14.4/§14.5 (tipos documentales, sin ocultos). Se construye en un
/// hilo aparte (§47.1/§47.5) y se entrega como fuente fija de la paleta
/// (el índice §45 solo existe en ventanas de proyecto).
fn spawn_quickopen_entries(base: PathBuf, slot: Rc<CmdSlot>) {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let _ = tx.send(quickopen_entries(&base));
    });
    deliver_to_quickopen(rx, slot, |me, entries| me.quickopen.set_entries(entries));
}

/// Contenido de [`spawn_quickopen_entries`]: los archivos del subárbol
/// `base` visibles por defecto en el explorador — tipos documentales §14.4,
/// sin segmentos ocultos §14.5 y sin los ignorados §14.6 (`.gitignore` de
/// `base`) — en orden estable por `rel_path` §17 (empate del ranking §41).
fn quickopen_entries(base: &Path) -> Vec<minmark_fs::fuzzy::Entry> {
    use minmark_fs::fuzzy::Entry;
    // Misma raíz canónica que `walk_tree` (§12.7: dos grafías, un fichero).
    let base = ruta_comparable(base);
    let ignore = minmark_fs::IgnorePatterns::from_root(&base);
    let mut entries: Vec<Entry> = Vec::new();
    let Ok(tree) = minmark_fs::walk_tree(&base) else {
        return entries;
    };
    for node in tree {
        if node.is_dir {
            continue;
        }
        let rel_path = minmark_fs::relative_path(&base, &node.path);
        if rel_path.split('/').any(|segment| segment.starts_with('.')) {
            continue; // §14.5: los ocultos no se listan
        }
        if ignore.is_ignored(Path::new(&rel_path), false) {
            continue; // §14.6
        }
        if !minmark_fs::fuzzy::is_document_type(&rel_path) {
            continue; // §14.4
        }
        entries.push(Entry {
            label: rel_path.clone(),
            rel_path,
            line: None,
            score: 0,
            heading: None,
        });
    }
    entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    entries
}

/// MRU de Quick Open (§41): `rel_path` al frente (lo más reciente primero),
/// sin duplicados y acotado a [`MRU_MAX`] (el ranking no bonifica más atrás).
fn mru_touch(recent: &mut Vec<String>, rel_path: &str) {
    recent.retain(|opened| opened != rel_path);
    recent.insert(0, rel_path.to_owned());
    recent.truncate(MRU_MAX);
}

/// Enlace tardío «widget → [`AppCmds`]»: los widgets se construyen antes que
/// el estado de la ventana, así que sus callbacks capturan esta celda.
///
/// Guarda un [`Weak`]`<AppCmds>` (no un `Rc`): los callbacks poseen la celda y
/// el `AppCmds` la referencia fuerte, sin ciclos de `Rc`.
type CmdSlot = RefCell<Option<Weak<AppCmds>>>;

/// Puente `EditSink` (TextView) → modelo (`TabManager` → core `Editor`).
struct Sink {
    tabs: Rc<RefCell<TabManager>>,
    /// Disparador de los controles contextuales §44 (fase 7 #29): cada
    /// cambio de cursor/selección (mark_set → aquí) reevalúa el popover
    /// con el estado vigente del modelo.
    on_selection: Rc<dyn Fn(Selection)>,
}

impl EditSink for Sink {
    fn insert_at(&self, at: Position, text: &str) {
        self.tabs.borrow_mut().insert_active(at, text);
    }

    fn delete_range(&self, from: Position, to: Position) {
        self.tabs.borrow_mut().delete_active(from, to);
    }

    fn set_selection(&self, selection: Selection) {
        self.tabs.borrow_mut().set_selection_active(selection);
        (self.on_selection)(selection);
    }
}

/// Estado de la ventana: widgets + modelo compartido.
struct AppCmds {
    app: Application,
    window: ApplicationWindow,
    /// Ámbito de esta ventana (ISSUE-021): standalone o de proyecto.
    scope: WindowScope,
    tabs: Rc<RefCell<TabManager>>,
    editor_pane: Rc<EditorPane>,
    preview: Rc<PreviewPane>,
    tab_bar: Rc<TabBar>,
    external_notice: GtkBox,
    /// Barra de búsqueda/reemplazo del archivo actual (§40.1, ISSUE-027).
    find_bar: Rc<FindBar>,
    /// Paleta Quick Open (§41, ISSUE-029, §39: `Ctrl+P`).
    quickopen: Rc<QuickOpen>,
    /// Paleta de comandos §38 (fase 7 #29, §39: `Ctrl+Shift+P`).
    command_palette: Rc<command_palette::CommandPalette>,
    index: RefCell<Option<minmark_fs::ProjectIndex>>,
    index_generation: Cell<u64>,
    intelligence_generation: Cell<u64>,
    intelligence_busy: Cell<bool>,
    intelligence_pending: Cell<bool>,
    intelligence_suggest: Cell<bool>,
    completion_source: RefCell<Option<(Option<PathBuf>, String, Position)>>,
    navigation_list: Rc<QuickOpen>,
    navigation_kind: Cell<bool>,
    navigation_positions: RefCell<std::collections::BTreeMap<String, (String, Position)>>,
    /// MRU de `rel_path` §17 abiertos (§41): el más reciente primero.
    recent: RefCell<Vec<String>>,
    /// Directorio base de la lista standalone de Quick Open (ISSUE-029) y de
    /// sus `rel_path`; `None` hasta abrir el primer fichero con ruta.
    quickopen_dir: RefCell<Option<PathBuf>>,
    /// Barra lateral §14.3/§40.2 (ISSUE-022/028): explorador + búsqueda de
    /// proyecto; solo en ventanas de proyecto.
    sidebar: Option<Rc<Sidebar>>,
    /// Watcher del subárbol del proyecto (ISSUE-026, §46); `None` en
    /// standalone.
    watcher: Option<Rc<RefCell<minmark_fs::NativeWatcher>>>,
    /// Composiciones del `.mdproject.toml` de la raíz (§20, ISSUE-022);
    /// vacías en ventanas standalone, donde las acciones de composición
    /// quedan deshabilitadas. Se redescubren al cambiar `.mdproject.toml`
    /// (B1, revisión PR #44) vía [`AppCmds::refresh_compositions`].
    compositions: RefCell<Vec<Composition>>,
    /// Grupo «win» insertado por `install_actions` (B1), en referencia
    /// DÉBIL: sus acciones capturan `Rc<dyn Commands>` de esta ventana, así
    /// que una referencia fuerte cerraría el ciclo `AppCmds → grupo →
    /// acciones → AppCmds` y la ventana no se liberaría al cerrarse. La
    /// referencia fuerte la retiene la propia ventana (`insert_action_group`);
    /// `None` hasta `install_actions`.
    action_group: RefCell<Option<gtk4::glib::WeakRef<gtk4::gio::SimpleActionGroup>>>,
    /// Ventana temporal del Preview combinado (§20.4, ISSUE-023),
    /// reutilizable con el patrón Outline/Backlinks.
    composition_preview: RefCell<Option<Rc<CompositionPreview>>>,
    /// Operaciones propias §14.8 (ISSUE-026 criterio 3).
    self_ops: Rc<SelfOps>,
    settings: RefCell<Settings>,
    academic_monitors: RefCell<Vec<gtk4::gio::FileMonitor>>,
    academic_paths: RefCell<Vec<String>>,
    view: Rc<Cell<ViewMode>>,
    refresh_scheduled: Rc<Cell<bool>>,
    slot: Rc<CmdSlot>,
}

/// Vista activa de la barra lateral (§40.2, estilo VS Code: el explorador y
/// la búsqueda de proyecto comparten sidebar, con una sola vista visible).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    /// Explorador de archivos §14.3 (ISSUE-022).
    Explorer,
    /// Búsqueda en el proyecto §40.2 (ISSUE-028).
    Search,
}

/// Barra lateral de una ventana de proyecto y su estado (ISSUE-022 §14.3,
/// ISSUE-028 §40.2): contenedor vertical `explorador | búsqueda` como
/// start-child del `Paned` exterior, con una sola vista visible cada vez y
/// ancho compartido.
struct Sidebar {
    /// Contenedor vertical `explorador | búsqueda` (§40.2).
    widget: GtkBox,
    /// Explorador §14.3 (ISSUE-022).
    explorer: Rc<ExplorerPane>,
    /// Búsqueda en el proyecto §40.2 (ISSUE-028).
    search: Rc<SearchPanel>,
    /// Vista visible (§40.2: una cada vez).
    view: Cell<SidebarView>,
    /// `Paned` exterior (barra lateral | editor+preview); su posición es el
    /// ancho de la barra lateral.
    paned: Paned,
    visible: Cell<bool>,
    state: RefCell<ExplorerState>,
}

/// Ventana temporal del Preview de una composición (§20.4, ISSUE-023), con
/// el patrón Outline/Backlinks: instancia reutilizable que Escape (o la ✕)
/// oculta sin destruir, y cuya WebView conserva la página entre aperturas.
struct CompositionPreview {
    window: gtk4::Window,
    pane: Rc<PreviewPane>,
    /// Generación del último render pedido: descarta los entregados tarde
    /// cuando se abre otra composición mientras este render sigue en vuelo.
    generation: Cell<u64>,
}

impl AppCmds {
    fn refresh_intelligence(&self, suggest: bool) {
        self.intelligence_suggest
            .set(self.intelligence_suggest.get() || suggest);
        if self.intelligence_busy.get() {
            self.intelligence_pending.set(true);
            return;
        }
        let Some(snapshot) = self.intelligence_snapshot() else {
            return;
        };
        self.intelligence_busy.set(true);
        self.intelligence_pending.set(false);
        let suggest = self.intelligence_suggest.replace(false)
            && self.tabs.borrow().active().editor.selection().is_empty();
        let generation = self.intelligence_generation.get().wrapping_add(1);
        self.intelligence_generation.set(generation);
        let cursor = self.tabs.borrow().active().editor.selection().head;
        let offset = self.tabs.borrow().active().editor.offset_of(cursor).ok();
        let path = self.tabs.borrow().active().path.clone();
        let source = snapshot.source.clone();
        let backlinks = self.navigation_list.is_visible() && self.navigation_kind.get();
        let settings = self.settings.borrow().clone();
        let root = self.scope.project_root();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let html =
                preview::render_document_at(&source, &settings, path.as_deref(), root.as_deref());
            let links = snapshot.links();
            let completions = if suggest {
                offset
                    .map(|o| {
                        let citations = intelligence::citation_completions(
                            &source,
                            o,
                            &academic::load(&settings),
                        );
                        if citations.is_empty() {
                            snapshot.completions(o)
                        } else {
                            citations
                        }
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let back = backlinks.then(|| snapshot.backlinks());
            let _ = tx.send((
                generation,
                path,
                source,
                cursor,
                html,
                links,
                completions,
                back,
            ));
        });
        deliver_to_quickopen(
            rx,
            self.slot.clone(),
            |me, (generation, path, source, cursor, html, links, completions, back)| {
                me.intelligence_busy.set(false);
                if me.intelligence_pending.replace(false) {
                    me.refresh_intelligence(false);
                    return;
                }
                if generation != me.intelligence_generation.get() {
                    return;
                }
                let tabs = me.tabs.borrow();
                if tabs.active().path != path || tabs.active().editor.document().content() != source
                {
                    return;
                }
                let same_cursor = tabs.active().editor.selection() == Selection::collapsed(cursor);
                let document_key = (tabs.active().path.clone(), tabs.active().untitled_number);
                drop(tabs);
                let base_uri = path.as_deref().and_then(Path::parent).map(|directory| {
                    format!(
                        "{}/",
                        gtk4::gio::File::for_path(ruta_comparable(directory))
                            .uri()
                            .trim_end_matches('/')
                    )
                });
                me.preview
                    .update_html(&html, base_uri.as_deref(), document_key);
                me.editor_pane.set_link_diagnostics(&source, &links);
                me.preview.set_link_diagnostics(&links);
                if same_cursor && !completions.is_empty() {
                    *me.completion_source.borrow_mut() = Some((path, source.clone(), cursor));
                    me.editor_pane.show_completions(completions);
                }
                if let Some(entries) = back {
                    let mut positions = me.navigation_positions.borrow_mut();
                    positions.clear();
                    let entries = entries
                        .into_iter()
                        .enumerate()
                        .map(|(i, mut item)| {
                            let key = format!("backlink-{i}");
                            positions.insert(key.clone(), (item.entry.rel_path, item.position));
                            item.entry.rel_path = key;
                            item.entry
                        })
                        .collect();
                    me.navigation_list.set_entries(entries);
                }
            },
        );
    }

    fn accept_completion(&self, completion: intelligence::Completion) {
        let Some((path, source, cursor)) = self.completion_source.borrow_mut().take() else {
            return;
        };
        let mut tabs = self.tabs.borrow_mut();
        let active = tabs.active();
        if active.path != path
            || active.editor.document().content() != source
            || active.editor.selection() != Selection::collapsed(cursor)
        {
            return;
        }
        let range = Range::new(
            active.editor.position_of(completion.span.start as usize),
            active.editor.position_of(completion.span.end as usize),
        );
        tabs.replace_active(range, &completion.text);
        drop(tabs);
        self.reload_active();
    }

    fn show_navigation(&self, backlinks: bool) {
        self.navigation_kind.set(backlinks);
        self.navigation_positions.borrow_mut().clear();
        self.navigation_list
            .set_title(if backlinks { "Backlinks" } else { "Outline" });
        self.navigation_list.set_entries(Vec::new());
        self.navigation_list.present();
        self.refresh_navigation();
    }

    fn refresh_navigation(&self) {
        if !self.navigation_list.is_visible() {
            return;
        }
        if self.navigation_kind.get() {
            if self.tabs.borrow().active().path.is_none() {
                self.navigation_list.set_entries(Vec::new());
            } else {
                self.refresh_intelligence(false);
            }
            return;
        }
        let tabs = self.tabs.borrow();
        let headings = minmark_document::document_anchors(&minmark_document::find_heading_nodes(
            tabs.active().editor.document().content(),
        ));
        self.navigation_list.set_entries(
            headings
                .into_iter()
                .map(|h| minmark_fs::fuzzy::Entry {
                    label: format!(
                        "{}H{} {}",
                        "  ".repeat(h.level.saturating_sub(1) as usize),
                        h.level,
                        h.text
                    ),
                    rel_path: String::new(),
                    line: Some(h.line),
                    heading: None,
                    score: 0,
                })
                .collect(),
        );
    }
    fn intelligence_snapshot(&self) -> Option<intelligence::Snapshot> {
        let root = self
            .quickopen_base()
            .or_else(|| std::env::current_dir().ok())?;
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        let from = tab
            .path
            .as_ref()
            .map(|p| minmark_fs::relative_path(&root, &ruta_comparable(p)))
            .unwrap_or_else(|| "__untitled__.md".into());
        let buffers = tabs
            .tabs()
            .iter()
            .filter_map(|tab| {
                tab.path.as_ref().map(|p| {
                    (
                        minmark_fs::relative_path(&root, &ruta_comparable(p)),
                        tab.editor.document().content().to_owned(),
                    )
                })
            })
            .collect();
        Some(intelligence::Snapshot {
            index: self
                .index
                .borrow()
                .as_ref()
                .filter(|i| i.root() == root)
                .cloned()
                .unwrap_or_else(|| minmark_fs::ProjectIndex::new(root)),
            buffers,
            from,
            source: tab.editor.document().content().to_owned(),
            wiki: self.settings.borrow().wiki_links,
        })
    }

    fn activate_link(&self, position: Option<Position>, destination: Option<String>) {
        let Some(snapshot) = self.intelligence_snapshot() else {
            return;
        };
        let offset = position.and_then(|p| self.tabs.borrow().active().editor.offset_of(p).ok());
        let path = self.tabs.borrow().active().path.clone();
        let source = snapshot.source.clone();
        let root = snapshot.index.root().to_path_buf();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let target = if let Some(dest) = destination {
                snapshot.query(|r| {
                    if let Some(wiki) = dest.strip_prefix("minmark-wiki:") {
                        snapshot.wiki.then(|| r.resolve_wiki(wiki))
                    } else {
                        Some(r.resolve(&dest, &snapshot.from))
                    }
                })
            } else {
                snapshot
                    .links()
                    .into_iter()
                    .find(|l| {
                        offset
                            .is_some_and(|o| l.span.start as usize <= o && o < l.span.end as usize)
                    })
                    .map(|l| l.target)
            };
            let _ = tx.send((path, source, root, target));
        });
        deliver_to_quickopen(rx, self.slot.clone(), |me, (path, source, root, target)| {
            let current = me.tabs.borrow();
            if current.active().path != path
                || current.active().editor.document().content() != source
            {
                return;
            }
            drop(current);
            let Some(target) = target else { return };
            if let Some(message) = intelligence::problem(&target) {
                show_error(&me.window, &message);
                return;
            }
            if target.kind != minmark_fs::TargetKind::Local {
                return;
            }
            let Some(rel) = target.rel_path else { return };
            let target_path = root.join(&rel);
            if path.is_none() && rel == "__untitled__.md" {
                if let Some(anchor) = target.anchor {
                    me.focus_line(anchor.line);
                }
                return;
            }
            me.activate_or_open(&target_path);
            if me
                .tabs
                .borrow()
                .active()
                .path
                .as_ref()
                .is_some_and(|p| ruta_comparable(p) == ruta_comparable(&target_path))
            {
                if let Some(anchor) = target.anchor {
                    me.focus_line(anchor.line);
                }
            }
        });
    }

    fn focus_line(&self, line: u32) {
        let at = Position::new(line.saturating_sub(1), 0);
        self.editor_pane.focus_find_match(Range::new(at, at));
        if let Ok(byte) = self.tabs.borrow().active().editor.offset_of(at) {
            self.preview.scroll_to_byte(byte as u32);
        }
        let pane = self.editor_pane.clone();
        glib::idle_add_local_once(move || pane.scroll_to_cursor());
    }

    fn slot(&self) -> Option<Rc<AppCmds>> {
        self.slot.borrow().as_ref().and_then(Weak::upgrade)
    }

    /// Sincroniza vistas y título con el tab activo (cambio de tab, open,
    /// undo/redo, save) y refresca Preview + highlighting.
    fn reload_active(&self) {
        {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            self.editor_pane.load(&tab.editor, tab.mode);
        }
        self.refresh_now();
        self.sync_quickopen_context();
        self.refresh_navigation();
    }

    /// Acción editorial contextual §44 (fase 7 #29/#30): ÚNICO dispatch de
    /// formato — menú Insert/Format, atajos, Command Palette, popover y los
    /// flujos de imagen/tabla #30 llegan aquí vía la acción `win.*` →
    /// [`TabManager::apply_format_active`] (UNA transacción §8.5 con la
    /// selección §8.6 registrada). Sin plan aplicable el documento no se
    /// toca. Tras aplicar se refresca la vista (`reload_active`, §8.6:
    /// `load` restaura cursor/selección desde el modelo) y se reevalúan los
    /// controles contextuales (actualizan o cierran el popover).
    ///
    /// `true` = aplicado; `false` = no aplicable o error de edición (el
    /// documento queda intacto — lo usa la importación #30 para retirar la
    /// copia si la edición no llegara a aplicarse).
    fn apply_format(&self, action: &minmark_document::format::FormatAction) -> bool {
        let applied = self.tabs.borrow_mut().apply_format_active(action);
        if applied {
            self.reload_active();
        }
        self.refresh_context_controls();
        applied
    }

    /// Reevalúa el popover de controles contextuales con el estado vigente
    /// del tab activo (§8.6).
    fn refresh_context_controls(&self) {
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        self.editor_pane.update_context(
            tab.editor.document().content(),
            tab.editor.selection(),
            tabs.active_index(),
            tab.mode,
        );
    }

    /// #30 §18.2: base de la importación de imagen — la raíz del proyecto
    /// (ventanas de proyecto) o, en standalone, la carpeta del documento
    /// activo — junto a la carpeta del documento para la ruta relativa §17
    /// (`None` si el documento aún no tiene ruta: se usa la raíz). `None` =
    /// no hay carpeta (documento sin guardar en standalone).
    fn image_import_scope(&self) -> Option<(PathBuf, Option<PathBuf>)> {
        let doc_dir = {
            let tabs = self.tabs.borrow();
            tabs.active()
                .path
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        };
        let root = match self.scope.project_root() {
            Some(root) => root,
            None => doc_dir.clone()?,
        };
        Some((root, doc_dir))
    }

    /// #30: formulario de campos de la imagen elegida en `win.insert-image` —
    /// Ruta (destino provisional `assets/<nombre>`, §18.2), Alt (nombre del
    /// fichero sin extensión, §UI) y ID/Ancho opcionales. El fichero se copia
    /// SOLO al aceptar: Cancelar/Escape en el formulario (o en el selector
    /// anterior) ⇒ cero cambios y NO hay copia (§UI).
    fn start_image_import(&self, source: PathBuf) {
        let Some((root, doc_dir)) = self.image_import_scope() else {
            show_error(
                &self.window,
                "No se puede importar la imagen: guarda el documento en una carpeta \
                 (se copia junto a él y el enlace se inserta con ruta relativa, §17).",
            );
            return;
        };
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let alt = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fields = vec![
            dialogs::FormField::new("Ruta", format!("assets/{name}")),
            dialogs::FormField::new("Alt", alt),
            dialogs::FormField::new("ID", ""),
            dialogs::FormField::new("Ancho %", ""),
        ];
        let slot = Rc::clone(&self.slot);
        dialogs::form_dialog(
            &self.window,
            actions::Command::InsertImage.label(),
            &fields,
            Some(validate_image_import_form),
            Rc::new(move |values| {
                // Cancelar/Escape ⇒ `None`: NO copia y NO edita (§UI #30).
                let Some(values) = values else { return };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.import_image_apply(&source, &root, doc_dir.as_deref(), values);
            }),
        );
    }

    /// #30: aceptado el formulario — copia el fichero a `<raíz>/<ruta>` con
    /// renombrado §18.3 (`diagram.png`, `diagram-2.png`, …) y aplica
    /// `ImageInsert` con la ruta relativa §17 a la carpeta del documento.
    /// Si la copia falla el error es visible (§3.7); si la edición no
    /// aplicara, la copia recién creada se retira (cero cambios).
    fn import_image_apply(
        &self,
        source: &Path,
        root: &Path,
        doc_dir: Option<&Path>,
        values: Vec<String>,
    ) {
        let Some((target, action)) = image_import_plan(root, doc_dir, Some(values)) else {
            return;
        };
        if let Some(parent) = target.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                show_error(
                    &self.window,
                    &format!("No se pudo crear la carpeta de imágenes: {error}"),
                );
                return;
            }
        }
        if let Err(error) = std::fs::copy(source, &target) {
            show_error(
                &self.window,
                &format!("No se pudo copiar la imagen: {error}"),
            );
            return;
        }
        if !self.apply_format(&action) {
            // La edición no aplicó: no se deja el huérfano copiado.
            let _ = std::fs::remove_file(&target);
        }
    }

    /// Contexto de Quick Open (§41, ISSUE-029) con el tab activo: `rel_path`
    /// §17 (`None` en tabs nuevos sin guardar; alimenta `:N` y `@heading`),
    /// MRU de abiertos (el activo el primero) y, en standalone, la lista de
    /// archivos del directorio activo (fuente fija, [`quickopen_entries`]).
    fn sync_quickopen_context(&self) {
        let path = self.tabs.borrow().active().path.clone();
        // Base de los `rel_path` (§17): la raíz del proyecto o, en
        // standalone, el directorio del primer fichero activo — estable al
        // navegar dentro de su subárbol (la lista no se estrecha) y con un
        // tab sin ruta (la lista cargada sigue valiendo).
        let mut recargar = false;
        if self.scope.project_root().is_none() {
            let activo = path.as_deref().and_then(Path::parent).map(ruta_comparable);
            let mut loaded = self.quickopen_dir.borrow_mut();
            let cambiar = match (&*loaded, &activo) {
                (_, None) => false,
                (None, Some(_)) => true,
                (Some(loaded), Some(activo)) => !activo.starts_with(loaded),
            };
            if cambiar {
                recargar = true;
                *loaded = activo;
            }
        }
        let base = self.quickopen_base();
        let rel_path = match (&base, &path) {
            (Some(base), Some(path)) => {
                Some(minmark_fs::relative_path(base, &ruta_comparable(path)))
            }
            _ => None,
        };
        self.quickopen.set_current_path(rel_path.clone());
        if let Some(rel_path) = &rel_path {
            let mut recent = self.recent.borrow_mut();
            mru_touch(&mut recent, rel_path);
            self.quickopen.set_recent(recent.clone());
        }
        // §45: en ventanas de proyecto la fuente es el índice y llega con
        // cada refresco; en standalone se reconstruye al cambiar de directorio.
        if recargar {
            match base {
                Some(base) => {
                    spawn_quickopen_entries(base.clone(), self.slot.clone());
                    spawn_index_refresh(base, self.slot.clone());
                }
                None => self.quickopen.set_entries(Vec::new()),
            }
        }
    }

    /// Base de los `rel_path` de Quick Open (§17, forma canónica como la del
    /// índice §45): la raíz del proyecto o, en standalone, el directorio
    /// cuya lista está cargada (ISSUE-029).
    fn quickopen_base(&self) -> Option<PathBuf> {
        match &self.scope {
            WindowScope::Project(root) => Some(ruta_comparable(root)),
            WindowScope::Standalone => self.quickopen_dir.borrow().clone(),
        }
    }

    /// Apertura desde Quick Open (§41): `rel_path` va a `OpenTarget`
    /// (ISSUE-021) y, si la entrada trae línea (`:N` o heading ya resuelto
    /// por la paleta), coloca cursor + scroll §8.6 (1-based → 0-based).
    fn open_quickopen_entry(&self, rel_path: &str, line: Option<u32>) {
        let Some(base) = self.quickopen_base() else {
            return;
        };
        let path = base.join(rel_path);
        // §41: `:N`/`@heading` saltan en el documento actual; se activa la
        // pestaña ya abierta en vez de duplicarla (mismo criterio que un
        // resultado §40.2: «abrir (o activar) la pestaña del fichero»).
        self.activate_or_open(&path);
        let Some(line) = line else { return };
        // El cursor solo se coloca si la pestaña activa es ese fichero
        // (`activate_or_open` pudo abrirlo en una ventana nueva o fallar).
        let activo = self
            .tabs
            .borrow()
            .active()
            .path
            .as_deref()
            .is_some_and(|opened| ruta_comparable(opened) == ruta_comparable(&path));
        if !activo {
            return;
        }
        self.focus_line(line);
    }

    /// Refresco completo del tab activo: highlighting, Preview y vista
    /// Visual comparten el mismo render del buffer (§6.1).
    fn refresh_now(&self) {
        let text = self
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        let root = self.scope.project_root();
        let document = self.tabs.borrow().active().path.clone();
        let effective = settings::load_at(root.as_deref(), &text, document.as_deref());
        self.watch_academic_resources(&effective);
        self.editor_pane.set_wiki_links(effective.wiki_links);
        {
            let mut settings = self.settings.borrow_mut();
            settings.wiki_links = effective.wiki_links;
            settings.language = effective.language;
            settings.heading_numbering = effective.heading_numbering;
            settings.counter_numbering = effective.counter_numbering;
            settings.toc_max_level = effective.toc_max_level;
            settings.bibliography = effective.bibliography;
            settings.bibliography_style = effective.bibliography_style;
            settings.pagination_enabled = effective.pagination_enabled;
        }
        self.editor_pane.refresh(&text);
        {
            let tabs = self.tabs.borrow();
            self.tab_bar.sync(&tabs);
        }
        self.update_title();
        // §40.1: con la barra abierta, el marcado sigue al contenido; el
        // ancla pasa a ser el cursor (edición, undo/redo o cambio de tab).
        let cursor = self.tabs.borrow().active().editor.selection().head;
        self.find_refresh(Some(cursor), false);
        self.refresh_intelligence(false);
    }

    fn watch_academic_resources(&self, settings: &Settings) {
        let mut paths = settings.bibliography.clone();
        if !settings.bibliography_style.eq_ignore_ascii_case("IEEE") {
            paths.push(settings.bibliography_style.clone());
        }
        if *self.academic_paths.borrow() == paths {
            return;
        }
        for monitor in self.academic_monitors.borrow_mut().drain(..) {
            monitor.cancel();
        }
        let mut monitors = self.academic_monitors.borrow_mut();
        for path in &paths {
            if let Ok(monitor) = gtk4::gio::File::for_path(path).monitor_file(
                gtk4::gio::FileMonitorFlags::NONE,
                None::<&gtk4::gio::Cancellable>,
            ) {
                let slot = self.slot.clone();
                monitor.connect_changed(move |_, _, _, _| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.schedule_refresh();
                    }
                });
                monitors.push(monitor);
            }
        }
        *self.academic_paths.borrow_mut() = paths;
    }

    /// Pinta el marcado de búsqueda y la barra desde [`find::FindState`]
    /// (sin recalcular). `focus` lleva la coincidencia activa a la vista
    /// (scroll + cursor, §8.6).
    fn find_repaint(&self, focus: bool) {
        if !self.find_bar.is_open() {
            return;
        }
        let (matches, active_index, active, counter, error) = {
            let state = self.find_bar.state().borrow();
            (
                state.matches().to_vec(),
                state.active_index(),
                state.active_match(),
                state.counter_text(),
                state.error().map(str::to_owned),
            )
        };
        self.editor_pane.set_find_marks(&matches, active_index);
        self.find_bar.set_counter(&counter);
        self.find_bar.set_error(error.as_deref());
        if focus {
            if let Some(range) = active {
                self.editor_pane.focus_find_match(range);
            }
        }
    }

    /// Recalcula la búsqueda sobre el tab activo (§40.1) y pinta.
    ///
    /// `anchor` fija el ancla (cursor al abrir o editar, o el final de un
    /// reemplazo); con `None` el ancla se conserva, de modo que teclear la
    /// consulta no la mueve.
    fn find_refresh(&self, anchor: Option<Position>, focus: bool) {
        if !self.find_bar.is_open() {
            return;
        }
        {
            let mut state = self.find_bar.state().borrow_mut();
            if let Some(position) = anchor {
                state.set_anchor(position);
            }
            let tabs = self.tabs.borrow();
            state.refresh(&tabs.active().editor);
        }
        self.find_repaint(focus);
    }

    /// Abre la barra de búsqueda (§39: `Ctrl+F`) o de reemplazo (`Ctrl+H`)
    /// sobre el tab activo y busca desde el cursor.
    fn find_open(&self, show_replace: bool) {
        self.find_bar.open(show_replace);
        let cursor = self.tabs.borrow().active().editor.selection().head;
        self.find_refresh(Some(cursor), true);
        self.find_bar.focus_search();
    }

    /// Acciones de la barra de búsqueda (§40.1).
    fn find_action(&self, action: FindAction) {
        match action {
            FindAction::Search => {
                // El ancla no se mueve al teclear la consulta.
                self.find_refresh(None, true);
            }
            FindAction::Next | FindAction::Previous => {
                {
                    let mut state = self.find_bar.state().borrow_mut();
                    if action == FindAction::Next {
                        state.next();
                    } else {
                        state.previous();
                    }
                }
                // La activa ya avanzó (con wrap §40.1): solo re-marcar y
                // llevarla a la vista.
                self.find_repaint(true);
            }
            FindAction::Replace => {
                let edit = self.find_bar.state().borrow().replace_edit();
                let Some((range, text)) = edit else { return };
                self.tabs.borrow_mut().replace_active(range, &text);
                // «Reemplazar» avanza a la siguiente: `reload_active` fija el
                // ancla en el cursor, que quedó al final del texto insertado
                // (§8.6), y recalcula.
                self.reload_active();
                self.find_repaint(true);
            }
            FindAction::ReplaceAll => {
                let edits = self.find_bar.state().borrow().replace_all_edits();
                if edits.is_empty() {
                    return;
                }
                // §8.5: una sola transacción; un solo undo la revierte entera.
                self.tabs.borrow_mut().replace_all_active(&edits);
                self.reload_active();
                self.find_repaint(true);
            }
            FindAction::Close => {
                self.find_bar.close();
                self.find_bar.state().borrow_mut().clear_results();
                self.editor_pane.clear_find_marks();
            }
        }
    }

    /// Título observable de la ventana (§3.7): tab activo + ámbito (ISSUE-021).
    fn update_title(&self) {
        let title = {
            let tabs = self.tabs.borrow();
            tabs.title(tabs.active_index())
        };
        let text = match &self.scope {
            WindowScope::Standalone => format!("{title} — minmark"),
            WindowScope::Project(root) => {
                let name = root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                format!("{title} — minmark · {name}")
            }
        };
        self.window.set_title(Some(&text));
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        self.external_notice.set_visible(tab.external_change);
        self.external_notice.set_tooltip_text(
            tab.path
                .as_ref()
                .map(|p| p.display().to_string())
                .as_deref(),
        );
    }

    fn reload_external(&self) {
        let Some(me) = self.slot() else { return };
        let (path, modified) = {
            let tabs = self.tabs.borrow();
            let Some(path) = tabs.active().path.clone() else {
                return;
            };
            (path, tabs.is_modified(tabs.active_index()))
        };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            if modified && !dialogs::confirm_discard(&window, &path.display().to_string()).await {
                return;
            }
            let result = me.tabs.borrow_mut().reload_active_from_disk();
            match result {
                Ok(()) => me.reload_active(),
                Err(err) => show_error(&window, &err.to_string()),
            }
        });
    }

    fn save_confirmed(&self) {
        // Soltar el préstamo del modelo antes de recargar las vistas.
        let saved = self.tabs.borrow_mut().save_active_confirmed();
        match saved {
            Ok(()) => self.reload_active(),
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Abre una ventana nueva con su propio ámbito y sus pestañas (ISSUE-021).
    fn spawn_window(&self, scope: WindowScope, paths: &[PathBuf]) {
        build_window(&self.app, scope, paths);
    }

    /// Abre `path` según el ámbito de la ventana (ISSUE-021, `OpenTarget`):
    /// dentro del ámbito, pestaña aquí; fuera del ámbito, ventana standalone
    /// nueva — nunca se rechaza un archivo.
    fn open_path(&self, path: &Path) {
        if self.scope.target_for(path) == OpenTarget::NewStandaloneWindow {
            self.spawn_window(WindowScope::Standalone, &[path.to_path_buf()]);
            return;
        }
        let opened = self.tabs.borrow_mut().open(path);
        match opened {
            Ok(index) => self.select_tab(index),
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Abre `path` en una pestaña o activa la que ya lo tiene abierto
    /// (ISSUE-028 §5: «abrir (o activar) la pestaña del fichero»). La
    /// comparación de rutas admite grafías distintas del mismo fichero.
    fn activate_or_open(&self, path: &Path) {
        let objetivo = ruta_comparable(path);
        let abierta = {
            let tabs = self.tabs.borrow();
            tabs.tabs().iter().position(|tab| {
                tab.path
                    .as_deref()
                    .is_some_and(|p| ruta_comparable(p) == objetivo)
            })
        };
        match abierta {
            Some(index) => self.select_tab(index),
            None => self.open_path(path),
        }
    }

    /// Persiste visibilidad y ancho de la barra lateral (§14.3: estado
    /// restaurable).
    fn persist_explorer_state(&self) {
        let Some(side) = &self.sidebar else { return };
        if side.visible.get() {
            let width = side.paned.position();
            if width > 0 {
                side.state.borrow_mut().width = ExplorerState::clamp_width(width);
            }
        }
        let state = *side.state.borrow();
        if let Err(err) = save_state(&state) {
            eprintln!("minmark: no se pudo guardar el estado del explorador: {err}");
        }
    }

    /// Operaciones §14.8 del explorador (ISSUE-023) sobre el nodo `target`
    /// (`None` = raíz del proyecto).
    fn explorer_op(&self, action: ExplorerAction, target: Option<PathBuf>) {
        let Some(me) = self.slot() else { return };
        let Some(root) = self.scope.project_root() else {
            return;
        };
        match action {
            ExplorerAction::CopyRelativePath => {
                let Some(target) = target else { return };
                // §14.8/§17: ruta relativa al proyecto.
                let rel = minmark_fs::relative_path(&root, &target);
                if let Some(display) = gtk4::gdk::Display::default() {
                    display.clipboard().set_text(&rel);
                }
            }
            ExplorerAction::Delete => {
                // §14.9: a la papelera del sistema, con confirmación §3.7.
                let Some(target) = target else { return };
                let window = self.window.clone();
                glib::spawn_future_local(async move {
                    let name = target
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| target.display().to_string());
                    if dialogs::confirm_trash(&window, &name).await {
                        me.trash_target(&target);
                    }
                });
            }
            ExplorerAction::Duplicate => {
                let Some(target) = target else { return };
                match minmark_fs::duplicate(&root, &target) {
                    Ok(copy) => {
                        self.self_ops.register(&copy);
                        self.refresh_explorer();
                    }
                    Err(err) => show_error(&self.window, &err.to_string()),
                }
            }
            ExplorerAction::NewMarkdown | ExplorerAction::NewFolder => {
                // §14.8: crear dentro del nodo seleccionado (archivo → su
                // carpeta; vacío → raíz).
                let parent = match &target {
                    Some(path) if path.is_dir() => path.clone(),
                    Some(path) => path.parent().unwrap_or(&root).to_path_buf(),
                    None => root.clone(),
                };
                let (title, initial) = match action {
                    ExplorerAction::NewMarkdown => ("New Markdown", "nuevo.md"),
                    _ => ("New Folder", "nueva-carpeta"),
                };
                let window = self.window.clone();
                let prompt_window = self.window.clone();
                dialogs::prompt_text(
                    &window,
                    title,
                    initial,
                    Rc::new(move |answer| {
                        let Some(name) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        let created = if action == ExplorerAction::NewMarkdown {
                            minmark_fs::create_markdown(&root, &parent, &name)
                        } else {
                            minmark_fs::create_folder(&root, &parent, &name)
                        };
                        match created {
                            Ok(path) => {
                                // Criterio 3 (ISSUE-026): alta propia, no externa.
                                me.self_ops.register(&path);
                                me.refresh_explorer();
                            }
                            Err(err) => show_error(&prompt_window, &err.to_string()),
                        }
                    }),
                );
            }
            ExplorerAction::Rename => {
                let Some(target) = target else { return };
                let initial = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let parent = target.parent().unwrap_or(&root).to_path_buf();
                let window = self.window.clone();
                dialogs::prompt_text(
                    &window,
                    "Rename",
                    &initial,
                    Rc::new(move |answer| {
                        let Some(name) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        me.rename_with_links(&target, &parent.join(name));
                    }),
                );
            }
            ExplorerAction::Move => {
                let Some(target) = target else { return };
                let initial = minmark_fs::relative_path(&root, &target);
                let window = self.window.clone();
                dialogs::prompt_text(
                    &window,
                    "Move",
                    &initial,
                    Rc::new(move |answer| {
                        let Some(dest) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        me.rename_with_links(&target, &root.join(dest));
                    }),
                );
            }
            ExplorerAction::ImportMarkdown => {
                // §15: el nodo del click derecho es la carpeta destino.
                let dest = match &target {
                    Some(path) if path.is_dir() => path.clone(),
                    Some(path) => path.parent().unwrap_or(&root).to_path_buf(),
                    None => root.clone(),
                };
                self.import_markdown_into(Some(dest));
            }
        }
    }

    /// Import Markdown (§15, ISSUE-024): copia un `.md` **externo** dentro del
    /// proyecto (el original queda intacto, colisión → renombrado §18.3).
    ///
    /// `dest_dir` = destino ya elegido (menú contextual del explorador) o
    /// `None` (menú File: se pide con diálogo). Origen (criterio 1): la
    /// pestaña activa si es un `.md` externo al ámbito; si no, el picker.
    fn import_markdown_into(&self, dest_dir: Option<PathBuf>) {
        let Some(me) = self.slot() else { return };
        let root = self.scope.project_root();
        let externo = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            tab.path.clone().filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext == "md" || ext == "markdown")
                    && root
                        .as_ref()
                        .is_none_or(|root| minmark_fs::resolve_in_project(root, path).is_err())
            })
        };
        match externo {
            Some(source) => me.import_source(source, dest_dir),
            None => {
                let window = self.window.clone();
                glib::spawn_future_local(async move {
                    let Some(source) = dialogs::open_file(&window).await else {
                        return;
                    };
                    me.import_source(source, dest_dir);
                });
            }
        }
    }

    /// Segundo paso del import (§15, criterio 1–2): elegir la carpeta destino.
    ///
    /// - destino ya elegido (menú contextual del explorador) → copiar ya;
    /// - ventana de proyecto → prompt relativo a la raíz;
    /// - ventana standalone → diálogo de carpeta (la carpeta elegida es la
    ///   raíz de validación §48 de la importación).
    fn import_source(&self, source: PathBuf, dest_dir: Option<PathBuf>) {
        if let Some(dest) = dest_dir {
            let root = self.scope.project_root().unwrap_or_else(|| dest.clone());
            self.import_copy(&source, &dest, &root);
            return;
        }
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        match self.scope.project_root() {
            Some(root) => {
                dialogs::prompt_text(
                    &window,
                    "Import Markdown — destino",
                    ".",
                    Rc::new(move |answer| {
                        let Some(dest) = answer.filter(|n| !n.trim().is_empty()) else {
                            return;
                        };
                        me.import_copy(&source, &root.join(dest), &root);
                    }),
                );
            }
            None => {
                glib::spawn_future_local(async move {
                    let Some(dest) = dialogs::open_folder(&window).await else {
                        return;
                    };
                    me.import_copy(&source, &dest, &dest);
                });
            }
        }
    }

    /// Copia la importación y re-objetiva la pestaña del externo a la copia
    /// (ISSUE-024 criterio 4).
    fn import_copy(&self, source: &Path, dest_dir: &Path, root: &Path) {
        match minmark_fs::import_markdown(root, source, dest_dir) {
            Ok(copy) => {
                self.self_ops.register(&copy);
                self.tabs.borrow_mut().retarget_path(source, &copy);
                self.reload_active();
                self.refresh_explorer();
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Rename/movimiento §16 (ISSUE-023): plan all-or-nothing, aplicación con
    /// rollback y retarget de las pestañas afectadas.
    fn rename_with_links(&self, from: &Path, to: &Path) {
        let Some(root) = self.scope.project_root() else {
            return;
        };
        // Criterio 3 (ISSUE-026): el movimiento propio no avisa como externo.
        self.self_ops.register(from);
        self.self_ops.register(to);
        let options = minmark_fs::ChangeOptions {
            wiki_links: self.settings.borrow().wiki_links,
        };
        let result = minmark_fs::plan_path_change(&root, from, to, options)
            .and_then(|plan| minmark_fs::apply_path_change(&plan));
        match result {
            Ok(()) => {
                self.tabs.borrow_mut().retarget_path(from, to);
                self.reload_active();
                self.refresh_explorer();
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    /// Delete §14.9: a la papelera; las pestañas afectadas se re-objetivan de
    /// forma observable (ISSUE-023 criterio 5).
    fn trash_target(&self, target: &Path) {
        // Criterio 3 (ISSUE-026): la baja propia no avisa como externa.
        self.self_ops.register(target);
        match minmark_fs::trash(target) {
            Ok(()) => {
                self.tabs.borrow_mut().path_deleted(target);
                self.reload_active();
                self.refresh_explorer();
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    fn refresh_explorer(&self) {
        if let Some(side) = &self.sidebar {
            side.explorer.refresh();
        }
    }

    /// #70 (§14.4/§14.5): alterna un toggle del menú View sobre el explorador
    /// de ESTA ventana. El explorador manda: su setter aplica el filtro y
    /// notifica a los consumidores, y el integrador refleja el valor en el
    /// estado booleano de la acción `win.*` — así la activación del menú y un
    /// set programático convergen en la misma sincronía y el estado nunca
    /// queda desfasado. Sin explorador (ventana standalone) es un sin-op
    /// documentado, como [`Commands::toggle_explorer`].
    fn toggle_explorer_filter(&self, show_all: bool) {
        let Some(side) = &self.sidebar else { return };
        if show_all {
            let siguiente = !side.explorer.show_all();
            side.explorer.set_show_all(siguiente);
        } else {
            let siguiente = !side.explorer.show_hidden();
            side.explorer.set_show_hidden(siguiente);
        }
    }

    /// #70 (revisión): refleja un filtro ya aplicado en el explorador en el
    /// estado booleano de la acción `win.<nombre>` de esta ventana, para que
    /// el menú View lo muestre marcado. Solo usa `set_state`, nunca
    /// `activate`: el cambio de estado no realimenta la acción, así que no
    /// hay bucle.
    fn sync_explorer_filter_action(&self, nombre: &str, valor: bool) {
        let Some(accion) = self
            .action_group
            .borrow()
            .as_ref()
            .and_then(|group| group.upgrade())
            .and_then(|group| group.lookup_action(nombre))
            .and_then(|accion| accion.downcast::<gtk4::gio::SimpleAction>().ok())
        else {
            return;
        };
        accion.set_state(&gtk4::glib::Variant::from(valor));
    }

    /// Texto seleccionado del tab activo para precargar la búsqueda (§4,
    /// estilo VS Code); solo selección de una línea.
    fn selection_seed(&self) -> Option<String> {
        let tabs = self.tabs.borrow();
        let tab = tabs.active();
        let range = tab.editor.selection().range();
        let start = tab.editor.offset_of(range.start).ok()?;
        let end = tab.editor.offset_of(range.end).ok()?;
        let text = &tab.editor.document().content()[start..end];
        if text.is_empty() || text.contains('\n') {
            None
        } else {
            Some(text.to_owned())
        }
    }

    /// Muestra `view` en la barra lateral (§40.2: una vista visible cada vez,
    /// estilo VS Code) montándola primero si estaba oculta (§14.3: el ancho
    /// compartido se restaura del estado guardado).
    fn show_sidebar_view(&self, view: SidebarView) {
        let Some(side) = &self.sidebar else { return };
        side.explorer
            .widget()
            .set_visible(view == SidebarView::Explorer);
        side.search
            .widget()
            .set_visible(view == SidebarView::Search);
        side.view.set(view);
        if !side.visible.get() {
            let width = side.state.borrow().width;
            side.paned.set_start_child(Some(&side.widget));
            side.paned.set_position(width);
            side.visible.set(true);
            side.state.borrow_mut().visible = true;
            self.persist_explorer_state();
        }
    }

    /// Bucle de eventos del watcher de proyecto (ISSUE-026): los eventos de
    /// inotify coalescidos refrescan el árbol y el índice (§45) y marcan las
    /// pestañas afectadas (§12.7, sin recarga del buffer; §3.7 observable).
    fn pump_watcher(&self) {
        let Some(watcher) = &self.watcher else { return };
        let events = match watcher.borrow_mut().poll() {
            Ok(events) => events,
            Err(err) => {
                eprintln!("minmark: watcher de proyecto: {err}");
                return;
            }
        };
        if events.is_empty() {
            return;
        }
        let mut touched_tabs = false;
        for event in &events {
            // Criterio 3: nuestras operaciones §14.8 no avisan como externas.
            if self.self_ops.consume(&event.path) {
                continue;
            }
            self.tabs.borrow_mut().mark_external(&event.path);
            touched_tabs = true;
        }
        self.refresh_explorer();
        if events
            .iter()
            .any(|e| e.path.file_name().is_some_and(|n| n == ".mdproject.toml"))
        {
            self.refresh_now();
            // B1 (revisión PR #44, #22): la lista de composiciones de la
            // ventana no se congela en la apertura: se redescubre y las
            // acciones quedan acordes (§20).
            self.refresh_compositions();
        }
        if let Some(root) = self.scope.project_root() {
            spawn_index_refresh(root, self.slot.clone());
        }
        if touched_tabs {
            self.tab_bar.sync(&self.tabs.borrow());
            self.update_title();
        }
    }

    /// Refresco diferido tras cada edición real (§47.1).
    fn schedule_refresh(self: &Rc<Self>) {
        if self.refresh_scheduled.get() {
            return;
        }
        self.refresh_scheduled.set(true);
        let me = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(REFRESH_DEBOUNCE_MS), move || {
            me.refresh_scheduled.set(false);
            me.refresh_now();
            let byte = {
                let tabs = me.tabs.borrow();
                let editor = &tabs.active().editor;
                editor.offset_of(editor.selection().head).ok()
            };
            if let Some(byte) = byte {
                me.preview.follow_edit(byte as u32);
            }
            me.refresh_intelligence(true);
            me.refresh_navigation();
        });
    }

    fn select_tab(&self, index: usize) {
        self.tabs.borrow_mut().set_active(index);
        self.reload_active();
    }

    /// ISSUE-52: cierra la pestaña `index` desde su «X» (Ctrl+W y File →
    /// Close Tab pasan la activa con [`Commands::close_tab`]). El tab activo
    /// solo cambia si se cierra él (§37) y siempre queda ≥1 pestaña.
    fn close_tab_at(&self, index: usize) {
        let (modified, title) = {
            let tabs = self.tabs.borrow();
            if index >= tabs.len() {
                return;
            }
            (tabs.is_modified(index), tabs.title(index))
        };
        if !modified {
            self.tabs.borrow_mut().close(index);
            self.reload_active();
            return;
        }
        // §3.7: descartar cambios solo con confirmación explícita, la misma
        // que Close Tab.
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            if dialogs::confirm_discard(&window, &title).await {
                me.tabs.borrow_mut().close(index);
                me.reload_active();
            }
        });
    }

    fn apply_view(&self) {
        let (show_editor, show_preview) = match self.view.get() {
            ViewMode::Editor => (true, false),
            ViewMode::Split => (true, true),
            ViewMode::Preview => (false, true),
        };
        self.editor_pane.widget().set_visible(show_editor);
        self.preview.widget().set_visible(show_preview);
    }

    // --- Composiciones (§20, ISSUE-022/023) ---------------------------------

    /// Contenido actual de los buffers abiertos con ruta (ISSUE-023): la
    /// composición usa el buffer sin guardar en lugar del disco (§6.1).
    fn open_buffers(&self) -> Vec<(PathBuf, String)> {
        self.tabs
            .borrow()
            .tabs()
            .iter()
            .filter_map(|tab| {
                Some((
                    ruta_comparable(tab.path.as_ref()?),
                    tab.editor.document().content().to_owned(),
                ))
            })
            .collect()
    }

    /// Redescubre las composiciones del proyecto (B1, revisión PR #44, #22):
    /// relée `.mdproject.toml` y reevalúa el `enabled` de
    /// `win.preview-composition` / `win.export-composition-pdf` (§20). Lo
    /// pide el watcher al cambiar ese archivo; al abrir la ventana ya
    /// descubre `build_window`. `build` vuelve a leer `.mdproject.toml` para
    /// la cadena §58 (`load_composition_at`), de modo que `files` y
    /// `overrides` son de lecturas próximas —eventualmente consistentes con
    /// el watcher—, no de una misma lectura atómica.
    fn refresh_compositions(&self) {
        let root = match &self.scope {
            WindowScope::Project(root) => Some(root.as_path()),
            WindowScope::Standalone => None,
        };
        let disponible = composition::refresh(&mut self.compositions.borrow_mut(), root);
        let grupo = self
            .action_group
            .borrow()
            .as_ref()
            .and_then(|grupo| grupo.upgrade());
        if let Some(group) = grupo {
            actions::refresh_composition_actions(&group, disponible);
        }
    }

    /// Ejecuta `then` con la composición elegida (ISSUE-022): con una sola
    /// no hay diálogo; con varias se presenta la selección (§20.1). Sin
    /// composiciones no hace nada (la acción está deshabilitada).
    fn with_composition(&self, then: Rc<dyn Fn(&AppCmds, Composition)>) {
        let nombres: Vec<String> = self
            .compositions
            .borrow()
            .iter()
            .map(|comp| comp.name.clone())
            .collect();
        match nombres.len() {
            0 => {}
            1 => {
                let comp = self.compositions.borrow()[0].clone();
                then(self, comp);
            }
            _ => {
                let Some(me) = self.slot() else { return };
                dialogs::choose_composition(
                    &self.window,
                    &nombres,
                    Rc::new(move |elegida| {
                        let Some(elegida) = elegida else { return };
                        let comp = me
                            .compositions
                            .borrow()
                            .iter()
                            .find(|comp| comp.name == elegida)
                            .cloned();
                        if let Some(comp) = comp {
                            then(&me, comp);
                        }
                    }),
                );
            }
        }
    }

    /// Ventana temporal del Preview combinado (patrón Outline/Backlinks):
    /// se crea en el primer uso y después se reutiliza; Escape o la ✕ la
    /// ocultan sin destruir la WebView.
    fn composition_window(&self) -> Rc<CompositionPreview> {
        if let Some(preview) = self.composition_preview.borrow().as_ref() {
            return Rc::clone(preview);
        }
        let window = gtk4::Window::builder()
            .transient_for(&self.window)
            .default_width(1000)
            .default_height(800)
            .build();
        let pane = Rc::new(PreviewPane::new());
        // El clic en un enlace no navega fuera del documento combinado
        // (§3.4: la WebView conserva la página, igual que el Preview normal);
        // un destino interno `#fragmento` (S1, ISSUE-024, revisión PR #44) sí
        // desplaza in-page hasta su elemento (§26).
        // Captura DÉBIL: el gestor de contenido de la WebView retiene este
        // callback y `pane` retiene la WebView, así que una captura fuerte
        // cerraría el ciclo y la ventana del Preview no se liberaría al
        // cerrarse (misma razón que el grupo «win», B1).
        let pane_interna = Rc::downgrade(&pane);
        pane.connect_link_activation(Rc::new(move |destino| {
            if let Some(pane) = pane_interna.upgrade() {
                pane.scroll_to_fragment(&destino);
            }
        }));
        window.set_child(Some(&pane.widget()));
        {
            let on_escape = window.clone();
            let controller = gtk4::EventControllerKey::new();
            controller.connect_key_pressed(move |_, key, _, _| {
                if key == gtk4::gdk::Key::Escape {
                    on_escape.set_visible(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(controller);
        }
        window.connect_close_request(move |window| {
            window.set_visible(false);
            glib::Propagation::Stop
        });
        let preview = Rc::new(CompositionPreview {
            window,
            pane,
            generation: Cell::new(0),
        });
        *self.composition_preview.borrow_mut() = Some(Rc::clone(&preview));
        preview
    }

    /// Abre la ventana temporal con el Preview COMBINADO de la composición
    /// (§20.4, ISSUE-023): continuo o paginado según el `pagination`
    /// efectivo; el render corre en un hilo y se entrega al hilo de UI
    /// (patrón §47.5). Consultar no escribe los fuentes (§20).
    fn show_composition_preview(&self, comp: &Composition) {
        let Some(root) = self.scope.project_root() else {
            return;
        };
        let combined = composition::build(&root, comp, &self.open_buffers());
        let preview = self.composition_window();
        preview
            .window
            .set_title(Some(&format!("Preview — {}", combined.name)));
        preview.window.present();
        let generation = preview.generation.get().wrapping_add(1);
        preview.generation.set(generation);
        let base = composition::base_uri(&root);
        // Misma composición ⇒ misma página (actualización in-place §10.2);
        // otra composición ⇒ recarga completa de la WebView.
        let clave = (
            Some(root.join(format!(".mdproject-composition:{}", combined.name))),
            0u64,
        );
        let pane = Rc::clone(&preview.pane);
        let slot = self.slot.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let html = composition::render(combined);
            let _ = tx.send(html);
        });
        let ventana = Rc::clone(&preview);
        deliver_to_quickopen(rx, slot, move |_me, html| {
            if ventana.generation.get() != generation {
                return; // otro render más reciente ya ocupó la ventana
            }
            pane.update_html(&html, Some(&base), clave.clone());
        });
    }

    /// Exporta la composición elegida como un único PDF (§20.4, ISSUE-023)
    /// por el MISMO flujo que la exportación individual: diálogo de
    /// destino, sobrescritura y errores idénticos (§32.1).
    fn export_composition(&self, comp: &Composition) {
        let Some(root) = self.scope.project_root() else {
            return;
        };
        let combined = composition::build(&root, comp, &self.open_buffers());
        let suggested = format!("{}.pdf", combined.name);
        let crate::composition::Combined {
            composed,
            settings,
            meta,
            root,
            ..
        } = combined;
        self.run_pdf_export(suggested, move || {
            export::export_composed(composed.ast, &settings, &root, &meta)
        });
    }

    /// Flujo de exportación PDF único (§32.1): diálogo de destino con su
    /// confirmación de sobrescritura (`dialogs::save_pdf`), escritura del
    /// bytes y errores visibles con el mismo patrón que siempre. `produce`
    /// genera los bytes por el camino que corresponda (individual o
    /// composición) — la lógica del diálogo no se duplica.
    fn run_pdf_export(
        &self,
        suggested: String,
        produce: impl FnOnce() -> Result<Vec<u8>, minmark_pdf::PdfError> + 'static,
    ) {
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(path) = dialogs::save_pdf(&window, &suggested).await else {
                return;
            };
            match produce() {
                Ok(bytes) => {
                    if let Err(err) = std::fs::write(&path, &bytes) {
                        show_error(&window, &err.to_string());
                    }
                }
                Err(err) => show_error(&window, &err.to_string()),
            }
        });
    }
}

impl Commands for AppCmds {
    fn outline(&self) {
        self.show_navigation(false);
    }
    fn follow_link(&self) {
        let cursor = self.tabs.borrow().active().editor.selection().head;
        self.activate_link(Some(cursor), None);
    }
    fn backlinks(&self) {
        self.show_navigation(true);
    }
    fn new(&self) {
        self.tabs.borrow_mut().new_tab();
        self.reload_active();
    }

    fn new_window(&self) {
        // ISSUE-021: un archivo nuevo fuera de proyecto vive en una ventana
        // standalone nueva (como Notepad++), sin restricción de carpeta.
        self.spawn_window(WindowScope::Standalone, &[]);
    }

    fn open(&self) {
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(path) = dialogs::open_file(&window).await else {
                return;
            };
            // ISSUE-021: dentro del ámbito de la ventana, pestaña aquí; fuera
            // del ámbito (archivo ajeno al proyecto), ventana standalone
            // nueva — nunca se rechaza el archivo.
            me.open_path(&path);
        });
    }

    fn quick_open(&self) {
        // §41/§39: `Ctrl+P` presenta la paleta modal sobre esta ventana
        // (§36) con el contexto del tab activo (`present` repinta la
        // consulta desde cero, fuzzy limpio §41).
        self.quickopen.present();
    }

    fn open_folder(&self) {
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(folder) = dialogs::open_folder(&window).await else {
                return;
            };
            // ISSUE-021: cada carpeta se abre en su propia ventana de proyecto.
            me.spawn_window(WindowScope::Project(folder), &[]);
        });
    }

    fn import_markdown(&self) {
        // §15: sin proyecto no hay carpeta destino (decisión registrada en
        // ISSUE-024: Import vive en ventanas de proyecto).
        self.import_markdown_into(None);
    }

    fn save(&self) {
        let result = self.tabs.borrow_mut().save_active();
        match result {
            Ok(()) => self.reload_active(),
            // §12.4: sin archivo asignado, Save As.
            Err(minmark_fs::FsError::MissingPath) => self.save_as(),
            // §12.7: nunca sobrescritura silenciosa; solo con confirmación
            // explícita del usuario (§3.7) se sobrescribe el cambio externo.
            Err(minmark_fs::FsError::ExternalModification { path }) => {
                let Some(me) = self.slot() else { return };
                let window = self.window.clone();
                glib::spawn_future_local(async move {
                    if dialogs::confirm_overwrite_external(&window, &path).await {
                        me.save_confirmed();
                    }
                });
            }
            Err(err) => show_error(&self.window, &err.to_string()),
        }
    }

    fn save_as(&self) {
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        let suggested = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            match &tab.path {
                Some(path) => path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "sin-titulo.md".to_owned()),
                None => format!("sin-titulo-{}.md", tab.untitled_number),
            }
        };
        glib::spawn_future_local(async move {
            let Some(path) = dialogs::save_file_as(&window, &suggested).await else {
                return;
            };
            let saved = me.tabs.borrow_mut().save_active_as(&path);
            match saved {
                Ok(()) => me.reload_active(),
                Err(err) => show_error(&window, &err.to_string()),
            }
        });
    }

    fn export_pdf(&self) {
        // §32.1 (ISSUE-035): exporta el documento activo tal cual está en el
        // buffer (§6.1), con los ajustes `pdf_*` vigentes. El motor ya está
        // probado en `minmark-pdf` (ISSUE-018).
        let Some(_me) = self.slot() else { return };
        // El front matter del snapshot puede preceder al refresco diferido.
        self.refresh_now();
        let root = self.scope.project_root();
        let (text, settings, suggested, document_path) = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            let suggested = match &tab.path {
                Some(path) => path
                    .file_stem()
                    .map(|stem| format!("{}.pdf", stem.to_string_lossy()))
                    .unwrap_or_else(|| "documento.pdf".to_owned()),
                None => format!("sin-titulo-{}.pdf", tab.untitled_number),
            };
            (
                tab.editor.document().content().to_owned(),
                self.settings.borrow().clone(),
                suggested,
                tab.path.clone(),
            )
        };
        let meta = minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse(&text));
        self.run_pdf_export(suggested, move || {
            export::export_document_at(
                &text,
                &settings,
                document_path.as_deref(),
                root.as_deref(),
                &meta,
            )
        });
    }

    fn export_composition_pdf(&self) {
        // §20.4/ISSUE-023: selección si hay varias y después el MISMO
        // flujo de diálogo que `export_pdf` (`run_pdf_export`).
        self.with_composition(Rc::new(|me, comp| me.export_composition(&comp)));
    }

    fn preview_composition(&self) {
        // §20.4/ISSUE-023: selección si hay varias y ventana temporal con
        // el Preview combinado (patrón Outline/Backlinks).
        self.with_composition(Rc::new(|me, comp| me.show_composition_preview(&comp)));
    }

    fn compositions_available(&self) -> bool {
        // ISSUE-022: sin proyecto con `[[composition]]` las acciones de
        // composición quedan deshabilitadas en menú y paleta.
        !self.compositions.borrow().is_empty()
    }

    fn close_tab(&self) {
        let index = self.tabs.borrow().active_index();
        self.close_tab_at(index);
    }

    fn undo(&self) {
        self.tabs.borrow_mut().undo_active();
        self.reload_active();
    }

    fn redo(&self) {
        self.tabs.borrow_mut().redo_active();
        self.reload_active();
    }

    fn find(&self) {
        // §40.1/§39: Ctrl+F, modo búsqueda.
        self.find_open(false);
    }

    fn replace(&self) {
        // §40.1/§39: Ctrl+H, con la fila de reemplazo visible.
        self.find_open(true);
    }

    fn project_search(&self) {
        // §40.2/§39: `Ctrl+Shift+F` muestra la vista de búsqueda de la barra
        // lateral y enfoca el campo. En ventanas standalone no hay panel: el
        // comando es sin-op (§40.2).
        let Some(side) = &self.sidebar else { return };
        // Una vista cada vez (modelo VS Code §40.2): manda la búsqueda y el
        // explorador queda oculto.
        self.show_sidebar_view(SidebarView::Search);
        // Precarga desde la selección del editor activo (§4, estilo VS Code):
        // con selección de una línea fija la consulta, precarga el campo de
        // reemplazo y relanza.
        if let Some(seed) = self.selection_seed() {
            side.search
                .run_search(&seed, SearchOptions::default(), Some(String::new()));
        }
        side.search.focus_search();
    }

    fn toggle_mode(&self) {
        self.tabs.borrow_mut().toggle_mode_active();
        self.reload_active();
    }

    /// §8.3 (ISSUE-030): alterna la visibilidad de la sintaxis en Visual
    /// entre «ocultar siempre» y «revelar la del elemento editado».
    fn toggle_visual_syntax(&self) {
        {
            let mut settings = self.settings.borrow_mut();
            settings.visual_syntax = match settings.visual_syntax {
                VisualSyntax::Hidden => VisualSyntax::RevealCurrent,
                VisualSyntax::RevealCurrent => VisualSyntax::Hidden,
            };
        }
        self.editor_pane
            .set_visual_syntax(self.settings.borrow().visual_syntax);
        self.reload_active();
    }

    fn toggle_explorer(&self) {
        // §14.3: la barra lateral solo existe en ventanas de proyecto
        // (ISSUE-022); en standalone Ctrl+B es un no-op documentado.
        let Some(side) = &self.sidebar else { return };
        // §40.2 (estilo VS Code): si manda la búsqueda, Ctrl+B es lo inverso
        // de `win.project-search` y vuelve al explorador; si ya manda el
        // explorador, muestra/oculta la barra lateral (§14.3).
        if side.view.get() == SidebarView::Search {
            self.show_sidebar_view(SidebarView::Explorer);
            return;
        }
        let visible = !side.visible.get();
        if visible {
            let width = side.state.borrow().width;
            side.paned.set_start_child(Some(&side.widget));
            side.paned.set_position(width);
        } else {
            // El ancho actual se guarda antes de ocultar (§14.3).
            self.persist_explorer_state();
            side.paned.set_start_child(Option::<&Widget>::None);
        }
        side.visible.set(visible);
        side.state.borrow_mut().visible = visible;
        self.persist_explorer_state();
    }

    fn set_view(&self, mode: ViewMode) {
        self.view.set(mode);
        self.apply_view();
    }

    /// #70 (§14.4): `win.show-all-files` desde el menú View.
    fn show_all_files(&self) {
        self.toggle_explorer_filter(true);
    }

    /// #70 (§14.5): `win.show-hidden-files` desde el menú View.
    fn show_hidden_files(&self) {
        self.toggle_explorer_filter(false);
    }

    // Controles contextuales §44 (fase 7 #29): menú, atajos, paleta y
    // popover comparten este camino win.* → apply_format.

    fn format_strong(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Strong);
    }

    fn format_emphasis(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Emphasis);
    }

    fn format_strikethrough(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Strikethrough);
    }

    fn format_code(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Code);
    }

    fn format_heading1(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Heading(1));
    }

    fn format_heading2(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Heading(2));
    }

    fn format_heading3(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Heading(3));
    }

    fn format_heading4(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Heading(4));
    }

    fn format_heading5(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Heading(5));
    }

    fn format_heading6(&self) {
        self.apply_format(&minmark_document::format::FormatAction::Heading(6));
    }

    fn insert_link(&self) {
        // Diálogo de URL prellenado con la del enlace bajo el cursor si lo
        // hay («Edit link» del popover reutiliza esta misma acción); Cancelar
        // (botón, Escape o cierre) ⇒ cero cambios (§UI #29).
        let prefill = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            minmark_document::format::context(
                tab.editor.document().content(),
                tab.editor.selection(),
            )
            .link
            .map(|link| link.url)
            .unwrap_or_default()
        };
        let slot = Rc::clone(&self.slot);
        dialogs::prompt_text(&self.window, "Link", &prefill, {
            Rc::new(move |url| {
                let Some(url) = url else { return };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.apply_format(&minmark_document::format::FormatAction::Link { url });
            })
        });
    }

    fn insert_callout(&self, tone: &str) {
        self.apply_format(&minmark_document::format::FormatAction::Callout {
            callout_type: tone.to_owned(),
        });
    }

    // Flujos de imágenes y tablas (fase 7 #30): menú, paleta y popover §38/
    // §44 activan estas MISMAS acciones `win.*` con destino único
    // `TabManager::apply_format_active`; Cancelar/Escape en cualquier
    // diálogo ⇒ cero cambios.

    fn insert_image(&self) {
        // §18.2: selector de fichero (§18.1) → formulario de campos → copia
        // a `<raíz>/assets/` §18.3 → `ImageInsert` con ruta relativa §17.
        // Cancelar en CUALQUIER punto ⇒ cero cambios y NO hay copia.
        let Some(me) = self.slot() else { return };
        let window = self.window.clone();
        glib::spawn_future_local(async move {
            let Some(source) = dialogs::open_image(&window).await else {
                return;
            };
            me.start_image_import(source);
        });
    }

    fn insert_table(&self) {
        // §35/#30: filas 1–50 y columnas 1–8; fuera de rango el diálogo no
        // se cierra (validación) y Cancelar/Escape ⇒ cero cambios.
        let slot = Rc::clone(&self.slot);
        dialogs::form_dialog(
            &self.window,
            actions::Command::InsertTable.label(),
            &[
                dialogs::FormField::new("Filas", "2"),
                dialogs::FormField::new("Columnas", "2"),
            ],
            Some(validate_table_dims),
            Rc::new(move |values| {
                let Some(values) = values else { return };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                if let Some(action) = table_insert_action(Some(values)) {
                    me.apply_format(&action);
                }
            }),
        );
    }

    fn edit_image(&self) {
        // Precarga Ruta/Alt/ID/Ancho desde `format::context` (§UI #30). Sin
        // constructo imagen bajo la selección no hay diálogo (el popover solo
        // ofrece «Edit Image» dentro de la imagen y `ImageEdit` fuera del
        // constructo no aplica → cero cambios).
        let imagen = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            minmark_document::format::context(
                tab.editor.document().content(),
                tab.editor.selection(),
            )
            .image
        };
        let Some(imagen) = imagen else { return };
        let slot = Rc::clone(&self.slot);
        dialogs::form_dialog(
            &self.window,
            actions::Command::EditImage.label(),
            &[
                dialogs::FormField::new("Ruta", imagen.destination),
                dialogs::FormField::new("Alt", imagen.alt),
                dialogs::FormField::new("ID", imagen.id.unwrap_or_default()),
                dialogs::FormField::new(
                    "Ancho %",
                    imagen
                        .width_percent
                        .map(|width| width.to_string())
                        .unwrap_or_default(),
                ),
            ],
            Some(validate_image_form),
            Rc::new(move |values| {
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                if let Some(action) = image_edit_action(values) {
                    me.apply_format(&action);
                }
            }),
        );
    }

    fn edit_table_attrs(&self) {
        // Precarga ID/Caption de la tabla bajo la selección (§UI #30); sin
        // constructo tabla no hay diálogo (cero cambios).
        let tabla = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            minmark_document::format::context(
                tab.editor.document().content(),
                tab.editor.selection(),
            )
            .table
        };
        let Some(tabla) = tabla else { return };
        let slot = Rc::clone(&self.slot);
        dialogs::form_dialog(
            &self.window,
            actions::Command::EditTableAttrs.label(),
            &[
                dialogs::FormField::new("ID", tabla.id.unwrap_or_default()),
                dialogs::FormField::new("Caption", tabla.caption.unwrap_or_default()),
            ],
            Some(validate_table_attrs_form),
            Rc::new(move |values| {
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                if let Some(action) = table_attrs_action(values) {
                    me.apply_format(&action);
                }
            }),
        );
    }

    // Flujos de ecuaciones y citas (fase 7 #31): menú Insert, paleta §38 y
    // popover §44 activan estas MISMAS acciones `win.*` con destino único
    // `TabManager::apply_format_active`; Cancelar/Escape en cualquier
    // diálogo ⇒ cero cambios.

    fn insert_equation(&self) {
        // §24/#31: con selección no vacía el texto seleccionado es el LaTeX
        // (semántica de `MathInsert`): se envuelve SIN diálogo — monolínea →
        // `$…$` y multilínea → `$$…$$` (la forma canónica sigue al
        // contenido). Sin selección se pide el LaTeX en el diálogo
        // multilinea con diagnóstico ADR-002 y SIN atributos: los edita
        // `win.equation-attrs` (§UI #31).
        let seleccion = {
            let tabs = self.tabs.borrow();
            tabs.active().editor.selection()
        };
        if !seleccion.is_empty() {
            let latex = {
                let tabs = self.tabs.borrow();
                let tab = tabs.active();
                let (Ok(anchor), Ok(head)) = (
                    tab.editor.offset_of(seleccion.anchor),
                    tab.editor.offset_of(seleccion.head),
                ) else {
                    return;
                };
                let (start, end) = (anchor.min(head), anchor.max(head));
                tab.editor
                    .document()
                    .content()
                    .get(start..end)
                    .map(str::to_owned)
            };
            let Some(latex) = latex else { return };
            if latex.trim().is_empty() {
                return;
            }
            let block = latex.contains('\n');
            self.apply_format(&minmark_document::format::FormatAction::MathInsert {
                block,
                latex,
            });
            return;
        }
        let slot = Rc::clone(&self.slot);
        dialogs::equation_dialog(
            &self.window,
            actions::Command::InsertEquation.label(),
            "",
            Rc::new(move |latex| {
                let Some(action) = equation_insert_action(latex) else {
                    return;
                };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.apply_format(&action);
            }),
        );
    }

    fn edit_equation(&self) {
        // Precarga el LaTeX de `format::context` (§UI #31); sin ecuación
        // bajo la selección no hay diálogo (cero cambios). ADR-002: el
        // marcador de «math no soportado» en el diálogo es un aviso NO
        // bloqueante — aceptar sigue siendo válido y el contenido se
        // conserva visible y editable (§24).
        let latex = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            minmark_document::format::context(
                tab.editor.document().content(),
                tab.editor.selection(),
            )
            .math
            .map(|math| math.latex)
        };
        let Some(latex) = latex else { return };
        let slot = Rc::clone(&self.slot);
        dialogs::equation_dialog(
            &self.window,
            actions::Command::EditEquation.label(),
            &latex,
            Rc::new(move |latex| {
                let Some(action) = equation_edit_action(latex) else {
                    return;
                };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.apply_format(&action);
            }),
        );
    }

    fn equation_attrs(&self) {
        // Atributos SOLO de ecuación de BLOQUE (§26.4/#31): en inline — o
        // fuera de ecuación — no hay diálogo ⇒ cero cambios. Precarga
        // ID/Caption como en Table Attributes (#30, mismo formulario y
        // validador §7.4.3).
        let attrs = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            minmark_document::format::context(
                tab.editor.document().content(),
                tab.editor.selection(),
            )
            .math
            .filter(|math| math.block)
            .map(|math| (math.id, math.caption))
        };
        let Some((id, caption)) = attrs else { return };
        let slot = Rc::clone(&self.slot);
        dialogs::form_dialog(
            &self.window,
            actions::Command::EquationAttrs.label(),
            &[
                dialogs::FormField::new("ID", id.unwrap_or_default()),
                dialogs::FormField::new("Caption", caption.unwrap_or_default()),
            ],
            Some(validate_table_attrs_form),
            Rc::new(move |values| {
                let Some(action) = math_attrs_action(values) else {
                    return;
                };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.apply_format(&action);
            }),
        );
    }

    fn insert_citation(&self) {
        // Diálogo de items (§25.2) con autocompletado §25.3 sobre la MISMA
        // bibliografía que la inteligencia de la fase 5 (`academic::load`);
        // arranca con una fila vacía. Aceptar ⇒ CitationInsert; Cancelar ⇒
        // cero cambios.
        let bibliography = Rc::new(academic::load(&self.settings.borrow()));
        let slot = Rc::clone(&self.slot);
        dialogs::citation_dialog(
            &self.window,
            actions::Command::InsertCitation.label(),
            &[],
            bibliography,
            Rc::new(move |items| {
                let Some(action) = citation_insert_action(items) else {
                    return;
                };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.apply_format(&action);
            }),
        );
    }

    fn edit_citation(&self) {
        // Precarga los items de `format::context` (§UI #31) en el MISMO
        // diálogo; sin cita bajo la selección no hay diálogo (cero cambios).
        let items = {
            let tabs = self.tabs.borrow();
            let tab = tabs.active();
            minmark_document::format::context(
                tab.editor.document().content(),
                tab.editor.selection(),
            )
            .citation
            .map(|cita| cita.items)
        };
        let Some(items) = items else { return };
        let bibliography = Rc::new(academic::load(&self.settings.borrow()));
        let slot = Rc::clone(&self.slot);
        dialogs::citation_dialog(
            &self.window,
            actions::Command::EditCitation.label(),
            &items,
            bibliography,
            Rc::new(move |items| {
                let Some(action) = citation_edit_action(items) else {
                    return;
                };
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                me.apply_format(&action);
            }),
        );
    }

    fn command_palette(&self) {
        // §38/§39: `Ctrl+Shift+P` presenta la paleta modal; aceptar activa
        // la MISMA acción `win.*` del comando elegido.
        self.command_palette.present();
    }

    fn about(&self) {
        let about = gtk4::AboutDialog::builder()
            .program_name("minmark")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("Editor Markdown académico — Privacy & Control Promise (§3)")
            .license("Apache-2.0 — ver LICENSE")
            .website("https://minmark.local")
            .modal(true)
            .transient_for(&self.window)
            .build();
        about.present();
    }

    fn shortcuts(&self) {
        let mut detail = String::new();
        for (action, accel) in menus::ACCELERATORS {
            detail.push_str(&format!("{accel}\t{action}\n"));
        }
        let alert = gtk4::AlertDialog::builder()
            .message("Atajos (§39)")
            .detail(detail.trim_end())
            .buttons(["Cerrar"])
            .modal(true)
            .build();
        alert.choose(
            Some(&self.window),
            std::option::Option::<&gtk4::gio::Cancellable>::None,
            |_| {},
        );
    }
}

/// Ruta comparable: canónica si se resuelve y la propia si no (§12.7: dos
/// grafías de un mismo fichero son el mismo fichero).
fn ruta_comparable(path: &Path) -> PathBuf {
    minmark_fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Diálogo de error modal (§3.7: acciones observables).
fn show_error(parent: &ApplicationWindow, message: &str) {
    let alert = gtk4::AlertDialog::builder()
        .message("Error")
        .detail(message)
        .buttons(["Aceptar"])
        .modal(true)
        .build();
    alert.choose(
        Some(parent),
        std::option::Option::<&gtk4::gio::Cancellable>::None,
        |_| {},
    );
}

/// Color del separador del split editor↔preview (§9.1): barra vertical
/// visible — gris sobre tema claro y negro sobre tema oscuro (el negro sobre
/// fondo claro se perdería y el gris claro sobre oscuro, también).
fn split_separator_color(theme: ResolvedTheme) -> &'static str {
    match theme {
        ResolvedTheme::Light => "#767676",
        ResolvedTheme::Dark => "#050505",
    }
}

/// CSS de la UI GTK desde la [`Palette`] del tema resuelto (ISSUE-017):
/// lógica pura y testeable. Incluye la barra vertical del split §9.1, que
/// solo se aplica al `Paned` con clase `mm-split` (el del explorador no).
fn theme_css(palette: &Palette, theme: ResolvedTheme) -> String {
    format!(
        "window {{ background-color: {bg}; color: {fg}; }}\n\
         textview, textview > text {{ font-family: monospace; background-color: {bg}; color: {fg}; }}\n\
         .mm-tab-bar {{ background-color: {code}; }}\n\
         .mm-tab-bar .mm-tab {{ padding: 2px 4px 0 4px; }}\n\
         .mm-tab-bar .mm-tab button {{ background-image: none; background-color: transparent; box-shadow: none; border-width: 0; padding: 2px 8px; min-height: 24px; }}\n\
         .mm-tab-bar .mm-tab button:hover {{ background-color: rgba(128, 128, 128, 0.3); }}\n\
         .mm-tab-bar .mm-tab button:focus {{ border-width: 2px; border-color: {fg}; }}\n\
         .mm-tab-bar .mm-tab button.mm-tab-close {{ padding: 2px 4px; min-width: 24px; }}\n\
         .mm-tab-bar .mm-tab.active {{ background-image: none; background-color: {bg}; color: {fg}; border-bottom: 2px solid {fg}; font-weight: bold; }}\n\
         .mm-explorer {{ background-color: {bg}; color: {fg}; }}\n\
         .mm-explorer listview {{ background-color: {bg}; color: {fg}; }}\n\
         .mm-explorer listview button {{ background-color: {bg}; color: {fg}; }}\n\
         .mm-explorer listview label {{ color: {fg}; }}\n\
         paned.mm-split > separator {{ min-width: 4px; background: {split}; }}\n",
        bg = palette.bg,
        fg = palette.fg,
        code = palette.code_bg,
        split = split_separator_color(theme),
    )
}

/// Estiliza la UI GTK con la [`Palette`] del tema resuelto (ISSUE-017).
fn install_theme_css(palette: &Palette, theme: ResolvedTheme) {
    let css = theme_css(palette, theme);
    let provider = CssProvider::new();
    provider.load_from_data(&css);
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

// ---------------------------------------------------------------------------
// Fase 7 #30: imágenes y tablas (§18, §26.3, §35) — lógica pura
// ---------------------------------------------------------------------------

/// #30 §18.3: `diagram.png` libre o `diagram-2.png`, `diagram-3.png`, … en
/// `dir` — nunca sobrescribe (§3.7), con el mismo criterio que `minmark_fs`
/// para Markdown (#15). `dir` puede no existir aún (la copia lo crea).
fn free_asset_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let render = |n: u32| {
        let name = if n <= 1 {
            stem.to_owned()
        } else {
            format!("{stem}-{n}")
        };
        dir.join(format!("{name}.{ext}"))
    };
    let mut n = 1u32;
    while render(n).exists() || render(n).is_symlink() {
        n += 1;
    }
    render(n)
}

/// #30: nombre con extensión obligatoria para la ruta de una imagen (p. ej.
/// `diagram.png`); `None` si no la tiene.
fn split_stem_ext(name: &str) -> Option<(&str, &str)> {
    let (stem, ext) = name.rsplit_once('.')?;
    match stem {
        "" | "." => None,
        _ if ext.is_empty() => None,
        _ => Some((stem, ext)),
    }
}

/// #30: ID limpio del formulario — se admite teclear el `#` inicial, que la
/// gramática de atributos §7.4.3 no incluye; vacío ⇒ `None` (sin id).
fn parse_form_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    let id = id.strip_prefix('#').unwrap_or(id);
    (!id.is_empty()).then(|| id.to_owned())
}

/// #30: ¿el id encaja con la gramática de atributos §7.4.3
/// (`[A-Za-z0-9:._-]+`)? Es la misma regla que valida `format::plan`.
fn valid_form_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'))
}

/// #30: ancho del formulario → porcentaje §18.5 (1–100); vacío ⇒ `None`
/// (sin ancho). No distingue «vacío» de «inválido»: eso lo hace el
/// validador del formulario antes de entregar los valores.
fn parse_form_width(raw: &str) -> Option<u8> {
    let width = raw.trim().parse::<u8>().ok()?;
    (1..=100).contains(&width).then_some(width)
}

/// #30: ID/Ancho compartidos por los dos formularios de imagen — vacío
/// admitido, con gramática §7.4.3 y rango §18.5. Devuelve el mensaje de
/// error o `None` si todo encaja.
fn validate_image_id_width(values: &[String]) -> Option<String> {
    let id = values.get(2)?;
    let id = id.trim();
    let id = id.strip_prefix('#').unwrap_or(id);
    if !id.is_empty() && !valid_form_id(id) {
        return Some("ID: solo letras, dígitos, «:», «.», «_» y «-» (§7.4.3)".to_owned());
    }
    let width = values.get(3)?;
    if !width.trim().is_empty() && parse_form_width(width).is_none() {
        return Some("Ancho %: entero entre 1 y 100 (§18.5)".to_owned());
    }
    None
}

/// #30: formulario de edición de imagen (`win.edit-image`) — Ruta obligatoria
/// más ID/Ancho §7.4.3/§18.5. La ruta editada puede ser relativa, absoluta o
/// un destino externo: `ImageEdit` solo la sustituye en el constructo.
fn validate_image_form(values: &[String]) -> Option<String> {
    if values.first().is_none_or(|ruta| ruta.trim().is_empty()) {
        return Some("La ruta es obligatoria".to_owned());
    }
    validate_image_id_width(values)
}

/// #30 §48: mensaje si `ruta` no es relativa sin salirse de la raíz (ni
/// absoluta ni con `..`/componentes extraños); `None` si es admisible.
fn reject_non_relative(ruta: &str) -> Option<String> {
    let path = Path::new(ruta);
    let invalid = path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)));
    invalid.then(|| "La ruta debe ser relativa y no salirse de la raíz (§48)".to_owned())
}

/// #30: formulario de importación (`win.insert-image`) — además de ID/Ancho
/// exige una ruta relativa a la raíz que apunte a un fichero con extensión:
/// la copia va a `<raíz>/<ruta>` (renombrado §18.3) y el enlace se inserta
/// con la ruta relativa §17 a la carpeta del documento.
fn validate_image_import_form(values: &[String]) -> Option<String> {
    let Some(ruta) = values.first() else {
        return Some("La ruta es obligatoria".to_owned());
    };
    let ruta = ruta.trim();
    if ruta.is_empty() {
        return Some("La ruta es obligatoria".to_owned());
    }
    if let Some(message) = reject_non_relative(ruta) {
        return Some(message);
    }
    if split_stem_ext(Path::new(ruta).file_name()?.to_str()?).is_none() {
        return Some(
            "La ruta debe apuntar a un fichero con extensión (p. ej. assets/diagram.png)"
                .to_owned(),
        );
    }
    validate_image_id_width(values)
}

/// #30: formulario de atributos de tabla (`win.edit-table-attrs`) — ID con
/// la gramática §7.4.3 si no está vacío; Caption es texto libre.
fn validate_table_attrs_form(values: &[String]) -> Option<String> {
    let id = values.first()?;
    let id = id.trim();
    let id = id.strip_prefix('#').unwrap_or(id);
    if !id.is_empty() && !valid_form_id(id) {
        return Some("ID: solo letras, dígitos, «:», «.», «_» y «-» (§7.4.3)".to_owned());
    }
    None
}

/// Dimensión de tabla dentro de `limites` (#35); `None` si no es un entero
/// del rango.
fn parse_dimension(raw: &str, limites: std::ops::RangeInclusive<usize>) -> Option<usize> {
    let value = raw.trim().parse::<usize>().ok()?;
    limites.contains(&value).then_some(value)
}

/// #30: diálogo de inserción de tabla — filas 1–50 y columnas 1–8 (§35,
/// límites del contrato); fuera de rango `format::plan` devuelve `None`.
fn validate_table_dims(values: &[String]) -> Option<String> {
    if values
        .first()
        .is_none_or(|rows| parse_dimension(rows, 1..=50).is_none())
    {
        return Some("Filas: entero entre 1 y 50 (§35)".to_owned());
    }
    if values
        .get(1)
        .is_none_or(|cols| parse_dimension(cols, 1..=8).is_none())
    {
        return Some("Columnas: entero entre 1 y 8 (§35)".to_owned());
    }
    None
}

/// #30: valores aceptados del formulario de imagen (Ruta, Alt, ID, Ancho %)
/// → `ImageEdit`; `None` = cancelación (el formulario entregó `None`) o
/// valores inválidos: cero cambios en el documento.
fn image_edit_action(form: Option<Vec<String>>) -> Option<minmark_document::format::FormatAction> {
    let values = form?;
    let [ruta, alt, id, width] = values.as_slice() else {
        return None;
    };
    let destination = ruta.trim().to_owned();
    if destination.is_empty() {
        return None;
    }
    let id = parse_form_id(id);
    if id.as_deref().is_some_and(|id| !valid_form_id(id)) {
        return None;
    }
    if !width.trim().is_empty() && parse_form_width(width).is_none() {
        return None;
    }
    Some(minmark_document::format::FormatAction::ImageEdit {
        destination,
        // El alt es texto libre: se lee tal cual se precargó/escribió para
        // que un valor intacto siga siendo no-op byte a byte.
        alt: alt.clone(),
        id,
        width_percent: parse_form_width(width),
    })
}

/// #30/#31: campos (ID, Caption) del formulario de atributos →
/// `Some((id, caption))` con la gramática §7.4.3; `None` = cancelación o id
/// inválido. Un campo vacío llega como `None` y quita ese atributo (§26.3/
/// §26.4). El caption se entrega VERBATIM —los espacios del valor precargado
/// son bytes del documento, así que un valor intacto sigue siendo no-op byte
/// a byte (hallazgo 7)—; solo un valor vacío o de solo espacios lo quita.
fn parse_attrs_form(
    form: Option<Vec<String>>,
) -> Option<(Option<String>, Option<String>)> {
    let values = form?;
    let [id, caption] = values.as_slice() else {
        return None;
    };
    let id = parse_form_id(id);
    if id.as_deref().is_some_and(|id| !valid_form_id(id)) {
        return None;
    }
    let caption = (!caption.trim().is_empty()).then(|| caption.clone());
    Some((id, caption))
}

/// #30: valores aceptados del formulario de atributos (ID, Caption) →
/// `TableAttrs`; `None` = cancelación o id inválido: cero cambios.
fn table_attrs_action(
    form: Option<Vec<String>>,
) -> Option<minmark_document::format::FormatAction> {
    let (id, caption) = parse_attrs_form(form)?;
    Some(minmark_document::format::FormatAction::TableAttrs { id, caption })
}

/// #31: mismos campos que Table Attributes (§26.4, mismo formulario) →
/// `MathAttrs`; `None` = cancelación o id inválido: cero cambios. En inline
/// el plan núcleo lo rechaza (los atributos son de bloque).
fn math_attrs_action(
    form: Option<Vec<String>>,
) -> Option<minmark_document::format::FormatAction> {
    let (id, caption) = parse_attrs_form(form)?;
    Some(minmark_document::format::FormatAction::MathAttrs { id, caption })
}

/// #31 §24: LaTeX aceptado del diálogo de edición → `MathEdit` VERBATIM;
/// `None` = cancelación (cero cambios) o LaTeX vacío (el diálogo lo
/// retiene y el plan núcleo también lo rechaza: doble red de seguridad).
fn equation_edit_action(
    latex: Option<String>,
) -> Option<minmark_document::format::FormatAction> {
    let latex = latex?;
    if latex.trim().is_empty() {
        return None;
    }
    Some(minmark_document::format::FormatAction::MathEdit { latex })
}

/// #31 §24: LaTeX aceptado del diálogo de inserción → `MathInsert` con la
/// forma canónica derivada del contenido — multilínea ⇒ bloque `$$…$$`,
/// monolínea ⇒ inline `$…$` (el inline nunca lleva salto de línea); `None`
/// = cancelación o LaTeX vacío: cero cambios.
fn equation_insert_action(
    latex: Option<String>,
) -> Option<minmark_document::format::FormatAction> {
    let latex = latex?;
    if latex.trim().is_empty() {
        return None;
    }
    let block = latex.contains('\n');
    Some(minmark_document::format::FormatAction::MathInsert { block, latex })
}

/// §25.2: los items aceptados pasan la gramática consumidora que valida el
/// diálogo ([`dialogs::validate_citation_items`], espejo de
/// `try_citation`/del núcleo); `None` de las builders ⇒ cero cambios.
fn citation_items_validos(items: &[minmark_document::CitationItem]) -> bool {
    dialogs::validate_citation_items(items).is_none()
}

/// #31 §25.2: items aceptados del diálogo → `CitationInsert`;
/// `None` = cancelación (cero cambios) o items inválidos.
fn citation_insert_action(
    items: Option<Vec<minmark_document::CitationItem>>,
) -> Option<minmark_document::format::FormatAction> {
    let items = items?;
    if !citation_items_validos(&items) {
        return None;
    }
    Some(minmark_document::format::FormatAction::CitationInsert { items })
}

/// #31 §25.2: items aceptados del diálogo → `CitationEdit`;
/// `None` = cancelación (cero cambios) o items inválidos.
fn citation_edit_action(
    items: Option<Vec<minmark_document::CitationItem>>,
) -> Option<minmark_document::format::FormatAction> {
    let items = items?;
    if !citation_items_validos(&items) {
        return None;
    }
    Some(minmark_document::format::FormatAction::CitationEdit { items })
}

/// #30: valores aceptados del diálogo de dimensiones (Filas, Columnas) →
/// `TableInsert` dentro de los límites §35; `None` = cancelación o fuera de
/// rango: cero cambios.
fn table_insert_action(
    form: Option<Vec<String>>,
) -> Option<minmark_document::format::FormatAction> {
    let values = form?;
    let [rows, cols] = values.as_slice() else {
        return None;
    };
    let rows = parse_dimension(rows, 1..=50)?;
    let cols = parse_dimension(cols, 1..=8)?;
    Some(minmark_document::format::FormatAction::TableInsert { rows, cols })
}

/// #30 §18.2/§18.3: plan de importación SIN tocar el disco — `None` =
/// cancelación del formulario (cero cambios, NO hay copia) o valores
/// inválidos. Devuelve `(copia, acción)` con el destino libre §18.3 en
/// `<raíz>/<ruta>` y el destino de markdown RELATIVO §17 desde la carpeta
/// del documento (`doc_dir`), o desde la raíz si el documento aún no tiene
/// ruta. La copia la hace el llamador (y la retira si la edición no
/// aplicara), para que aceptar el formulario sea el único punto con efectos.
fn image_import_plan(
    root: &Path,
    doc_dir: Option<&Path>,
    form: Option<Vec<String>>,
) -> Option<(PathBuf, minmark_document::format::FormatAction)> {
    let values = form?;
    let [ruta, alt, id, width] = values.as_slice() else {
        return None;
    };
    if validate_image_import_form(&values).is_some() {
        return None;
    }
    let ruta = ruta.trim();
    let name = Path::new(ruta).file_name()?.to_str()?;
    let (stem, ext) = split_stem_ext(name)?;
    let target = free_asset_path(root.join(ruta).parent()?, stem, ext);
    let destination = minmark_fs::relative_path(doc_dir.unwrap_or(root), &target);
    let id = parse_form_id(id);
    Some((
        target,
        minmark_document::format::FormatAction::ImageInsert {
            destination,
            alt: alt.clone(),
            id,
            width_percent: parse_form_width(width),
        },
    ))
}

fn build_window(app: &Application, scope: WindowScope, open_paths: &[PathBuf]) -> Rc<AppCmds> {
    let settings = Settings::default();
    let theme = resolve_theme(settings.theme, detect_system_theme());
    let palette = match theme {
        ResolvedTheme::Light => Palette::LIGHT,
        ResolvedTheme::Dark => Palette::DARK,
    };
    install_theme_css(&palette, theme);

    let window = ApplicationWindow::builder()
        .application(app)
        .title("minmark")
        .default_width(1280)
        .default_height(800)
        .build();

    // Modelo + vistas (§6.1: el modelo vive en el core).
    let slot: Rc<CmdSlot> = Rc::new(RefCell::new(None));
    let tabs = Rc::new(RefCell::new(TabManager::new()));
    tabs.borrow_mut().set_scope(scope.clone());
    for path in open_paths {
        let opened = tabs.borrow_mut().open(path);
        match opened {
            Ok(index) => tabs.borrow_mut().set_active(index),
            Err(err) => eprintln!("minmark: no se pudo abrir {}: {err}", path.display()),
        }
    }

    let sink = {
        let slot = slot.clone();
        Rc::new(Sink {
            tabs: tabs.clone(),
            // Controles contextuales §44 (fase 7 #29): cada marca de
            // cursor/selección reevalúa el popover con el estado vigente
            // (la guardia de pestaña vive en el popover, §UI).
            on_selection: Rc::new(move |selection| {
                let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                    return;
                };
                let tabs = me.tabs.borrow();
                let tab = tabs.active();
                me.editor_pane.update_context(
                    tab.editor.document().content(),
                    selection,
                    tabs.active_index(),
                    tab.mode,
                );
            }),
        })
    };
    let on_edited = {
        let slot = slot.clone();
        Rc::new(move || {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.schedule_refresh();
            }
        })
    };
    let editor_pane = Rc::new(EditorPane::new(sink, on_edited, &palette));
    // §8.3 (ISSUE-030): visibilidad de la sintaxis en Visual según ajuste.
    editor_pane.set_visual_syntax(settings.visual_syntax);
    let preview = Rc::new(PreviewPane::new());
    editor_pane.connect_link_activation({
        let slot = slot.clone();
        Rc::new(move |at| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.activate_link(Some(at), None);
            }
        })
    });
    preview.connect_link_activation({
        let slot = slot.clone();
        Rc::new(move |dest| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.activate_link(None, Some(dest));
            }
        })
    });
    editor_pane.connect_completion({
        let slot = slot.clone();
        Rc::new(move |item| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.accept_completion(item);
            }
        })
    });
    // Controles contextuales #29: el popover fija el tono de
    // `win.insert-callout` sobre el grupo «win» que `install_actions`
    // instaló (referencia débil B1) y luego activa la misma acción.
    editor_pane.connect_context_tone({
        let slot = slot.clone();
        Rc::new(move |tone: String| {
            let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                return;
            };
            let Some(group) = me
                .action_group
                .borrow()
                .as_ref()
                .and_then(|group| group.upgrade())
            else {
                return;
            };
            if let Some(action) = group
                .lookup_action("insert-callout")
                .and_then(|action| action.downcast::<gtk4::gio::SimpleAction>().ok())
            {
                action.set_state(&gtk4::glib::Variant::from(tone.as_str()));
            }
        })
    });
    let tab_bar = Rc::new(TabBar::new(
        {
            let slot = slot.clone();
            Rc::new(move |index| {
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    me.select_tab(index);
                }
            })
        },
        {
            let slot = slot.clone();
            Rc::new(move |index| {
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    // ISSUE-52: la «X» cierra esa pestaña concreta.
                    me.close_tab_at(index);
                }
            })
        },
    ));

    // Barra de búsqueda/reemplazo del archivo actual (§40.1, ISSUE-027):
    // acciones en tardío por [`CmdSlot`], como el resto de widgets.
    let find_bar = Rc::new(FindBar::new({
        let slot = slot.clone();
        Rc::new(move |action| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.find_action(action);
            }
        })
    }));

    // Paleta Quick Open (§41, ISSUE-029): modal sobre esta ventana (§36) y
    // con la apertura en tardío por [`CmdSlot`], como el resto de widgets.
    let quickopen = Rc::new(QuickOpen::new());
    let navigation_list = Rc::new(QuickOpen::new());
    navigation_list.set_transient_for(window.upcast_ref::<gtk4::Window>());
    navigation_list.set_on_open({
        let slot = slot.clone();
        Rc::new(move |path, line| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                if path.is_empty() {
                    if let Some(line) = line {
                        me.focus_line(line);
                    }
                } else {
                    let target = me.navigation_positions.borrow().get(&path).cloned();
                    if let Some((path, at)) = target {
                        me.open_quickopen_entry(&path, Some(at.line + 1));
                        if me
                            .tabs
                            .borrow()
                            .active()
                            .path
                            .as_ref()
                            .zip(me.quickopen_base())
                            .is_some_and(|(p, base)| {
                                minmark_fs::relative_path(&base, &ruta_comparable(p)) == path
                            })
                        {
                            me.editor_pane.focus_find_match(Range::new(at, at));
                        }
                    }
                }
            }
        })
    });
    quickopen.set_transient_for(window.upcast_ref::<gtk4::Window>());
    quickopen.set_on_open({
        let slot = slot.clone();
        Rc::new(move |rel_path: String, line: Option<u32>| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.open_quickopen_entry(&rel_path, line);
            }
        })
    });

    // Paleta de comandos §38 (fase 7 #29, `Ctrl+Shift+P`): modal sobre esta
    // ventana; al aceptar activa la acción `win.*` elegida sobre ella.
    let command_palette = Rc::new(command_palette::CommandPalette::new());
    command_palette.set_transient_for(window.upcast_ref::<gtk4::Window>());

    // Barra lateral §14.3/§40.2 (ISSUE-022/028): el explorador y la búsqueda
    // de proyecto solo existen en ventanas de proyecto y comparten sidebar
    // (§40.2); el estado (visibilidad/ancho) se restaura al abrir.
    let explorer_state = load_state();
    let lateral = match &scope {
        WindowScope::Project(root) => {
            let on_open: Rc<dyn Fn(PathBuf)> = {
                let slot = slot.clone();
                Rc::new(move |path: PathBuf| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.open_path(&path);
                    }
                })
            };
            let on_action: Rc<dyn Fn(ExplorerAction, Option<PathBuf>)> = {
                let slot = slot.clone();
                Rc::new(move |action, target| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.explorer_op(action, target);
                    }
                })
            };
            let explorer = Rc::new(ExplorerPane::new(root, on_open, on_action));

            // §14.4 (ISSUE-029, #70): `Show All Files` alimenta la consulta
            // de Quick Open y refleja el valor en su acción `win.*` (menú
            // marcado); el reflejo vive en el notificador para que un set
            // programático del panel también sincronice el menú (revisión #70).
            explorer.set_on_show_all_changed({
                let slot = slot.clone();
                let quickopen = quickopen.clone();
                Rc::new(move |show_all: bool| {
                    quickopen.set_show_all_files(show_all);
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.sync_explorer_filter_action("show-all-files", show_all);
                    }
                })
            });
            // §14.5 (#70): mismo reflejo para `Show Hidden Files` por su
            // notificador propio (Quick Open no consume este filtro).
            explorer.set_on_show_hidden_changed({
                let slot = slot.clone();
                Rc::new(move |show_hidden: bool| {
                    if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                        me.sync_explorer_filter_action("show-hidden-files", show_hidden);
                    }
                })
            });

            // Búsqueda en el proyecto (§40.2, ISSUE-028) con su raíz (§14.1) y
            // callbacks en tardío por [`CmdSlot`], como el resto de widgets.
            let search = Rc::new(SearchPanel::new());
            search.set_root(root);

            // Activar un resultado (ISSUE-028 §5): abrir (o activar) la
            // pestaña del fichero y colocar el cursor (§8.6: las posiciones
            // del callback son 1-based y las del editor 0-based; la columna
            // cuenta caracteres Unicode).
            search.on_open_result({
                let slot = slot.clone();
                move |rel_path: String, line: usize, col: usize| {
                    let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                        return;
                    };
                    let Some(root) = me.scope.project_root() else {
                        return;
                    };
                    let path = root.join(&rel_path);
                    me.activate_or_open(&path);
                    // El cursor solo se coloca si la pestaña activa es ese
                    // fichero (`open_path` pudo abrir una ventana nueva).
                    let activo = me
                        .tabs
                        .borrow()
                        .active()
                        .path
                        .as_deref()
                        .is_some_and(|p| ruta_comparable(p) == ruta_comparable(&path));
                    if !activo {
                        return;
                    }
                    let position =
                        Position::new(line.saturating_sub(1) as u32, col.saturating_sub(1) as u32);
                    me.tabs
                        .borrow_mut()
                        .set_selection_active(Selection::collapsed(position));
                    me.reload_active();
                }
            });

            // Reemplazo aplicado o deshecho (ISSUE-028 §5): orden exacto del
            // criterio — escrituras propias, pestañas, derivados §45 y
            // repintado del activo.
            search.on_replaced({
                let slot = slot.clone();
                move |summary: ReplaceSummary| {
                    let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) else {
                        return;
                    };
                    let Some(root) = me.scope.project_root() else {
                        return;
                    };
                    // 1. Escrituras PROPIAS §14.8 (ISSUE-026 criterio 3):
                    //    registradas antes de que el watcher (§46) las vea.
                    for rel in &summary.files {
                        me.self_ops.register(&root.join(rel));
                    }
                    // 2. Sincronizar las pestañas abiertas (§12.7): la limpia
                    //    se recarga en silencio (la escritura es nuestra); la
                    //    modificada NUNCA se recarga — solo se marca y el
                    //    usuario decide al guardar (§3.7). Las operaciones de
                    //    `TabManager` son por path (puede haber varias pestañas
                    //    del mismo fichero): primero se marca el path si alguna
                    //    de sus pestañas está modificada y después la recarga
                    //    limpia el banderín de las pestañas limpias.
                    let objetivos: Vec<PathBuf> = summary
                        .files
                        .iter()
                        .map(|rel| ruta_comparable(&root.join(rel)))
                        .collect();
                    let mut active_reloaded = false;
                    {
                        let mut tabs = me.tabs.borrow_mut();
                        let active = tabs.active_index();
                        let mut grupos: Vec<(PathBuf, Vec<usize>)> = Vec::new();
                        for index in 0..tabs.len() {
                            let Some(path) = tabs.tabs()[index].path.clone() else {
                                continue;
                            };
                            if !objetivos.contains(&ruta_comparable(&path)) {
                                continue;
                            }
                            match grupos.iter_mut().find(|(p, _)| *p == path) {
                                Some((_, indices)) => indices.push(index),
                                None => grupos.push((path, vec![index])),
                            }
                        }
                        for (path, indices) in grupos {
                            if indices.iter().any(|&i| tabs.is_modified(i)) {
                                tabs.mark_external(&path);
                            }
                            if indices.iter().any(|&i| !tabs.is_modified(i)) {
                                if let Err(err) = tabs.reload_from_disk(&path) {
                                    show_error(&me.window, &err.to_string());
                                    continue;
                                }
                                active_reloaded |= indices.contains(&active);
                            }
                        }
                    }
                    // 3. Derivados §45 (headings y referencias cambian).
                    spawn_index_refresh(root, slot.clone());
                    // 4. Preview/activo: repintar solo si la activa se recargó.
                    if active_reloaded {
                        me.reload_active();
                    }
                }
            });

            Some((explorer, search))
        }
        WindowScope::Standalone => None,
    };

    // §45/§46 (ISSUE-025/026): índice del proyecto y watcher del subárbol en
    // ventanas de proyecto. El índice se construye en una tarea aparte
    // (§47.1/§47.5); el watcher solo EMITE eventos (§46: sin rescan continuo,
    // §12.7: sin recarga automática).
    let watcher = match &scope {
        WindowScope::Project(root) => {
            spawn_index_refresh(root.clone(), slot.clone());
            match minmark_fs::NativeWatcher::new() {
                Ok(mut watcher) => {
                    if let Err(err) = minmark_fs::Watcher::watch_tree(&mut watcher, root) {
                        eprintln!("minmark: sin watching de proyecto: {err}");
                    }
                    Some(Rc::new(RefCell::new(watcher)))
                }
                Err(err) => {
                    eprintln!("minmark: sin watching de proyecto: {err}");
                    None
                }
            }
        }
        WindowScope::Standalone => None,
    };

    // Layout §36: barra de menús superior sin toolbar permanente.
    let menubar = PopoverMenuBar::from_model(Some(&menus::build_menubar()));
    menubar.set_hexpand(true);
    let paned = Paned::new(Orientation::Horizontal);
    // §9.1: separador editor|preview como barra vertical visible (`mm-split`;
    // el `Paned` del explorador no lleva la clase y queda como estaba).
    paned.add_css_class("mm-split");
    paned.set_start_child(Some(&editor_pane.widget()));
    paned.set_end_child(Some(&preview.widget()));
    paned.set_position(640);
    paned.set_wide_handle(true);

    // §14.3/§40.2: `barra lateral | editor+preview`; la barra lateral
    // (`explorador | búsqueda`, una vista visible cada vez) es el start-child
    // del `Paned` exterior y su posición es el ancho compartido, restaurado
    // del estado guardado.
    let (content, sidebar) = match lateral {
        Some((explorer, search)) => {
            let sidebar_box = GtkBox::new(Orientation::Vertical, 0);
            sidebar_box.append(&explorer.widget());
            sidebar_box.append(search.widget());
            // Al abrir manda el explorador (§40.2).
            search.widget().set_visible(false);
            let outer = Paned::new(Orientation::Horizontal);
            outer.set_wide_handle(true);
            // #70/#67: el mínimo del panel es el de la lista (elipsizada),
            // pero NO se asigna por debajo del mínimo: en Windows un ancho
            // restaurado pequeño desplazaba el contenido fuera del borde de
            // la ventana. El mínimo de la ventana lo marcan editor+preview,
            // no el explorador.
            outer.set_shrink_start_child(false);
            outer.set_end_child(Some(&paned));
            if explorer_state.visible {
                outer.set_start_child(Some(&sidebar_box));
                outer.set_position(explorer_state.width);
            }
            let panel = Rc::new(Sidebar {
                widget: sidebar_box,
                explorer,
                search,
                view: Cell::new(SidebarView::Explorer),
                paned: outer.clone(),
                visible: Cell::new(explorer_state.visible),
                state: RefCell::new(explorer_state),
            });
            (outer.upcast::<Widget>(), Some(panel))
        }
        None => (paned.clone().upcast(), None),
    };

    let external_notice = GtkBox::new(Orientation::Horizontal, 8);
    external_notice.set_visible(false);
    external_notice.set_margin_top(6);
    external_notice.set_margin_bottom(6);
    external_notice.set_margin_start(8);
    external_notice.set_margin_end(8);
    external_notice.append(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
    let notice_text = gtk4::Label::new(Some(
        "El archivo cambió o se eliminó fuera de minmark. Tu versión sigue abierta.",
    ));
    notice_text.set_wrap(true);
    notice_text.set_xalign(0.0);
    notice_text.set_hexpand(true);
    external_notice.append(&notice_text);
    let reload = gtk4::Button::with_label("Recargar desde disco");
    reload.connect_clicked({
        let slot = slot.clone();
        move |_| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.reload_external();
            }
        }
    });
    external_notice.append(&reload);
    let save_copy = gtk4::Button::with_label("Guardar como…");
    save_copy.connect_clicked({
        let slot = slot.clone();
        move |_| {
            if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                me.save_as();
            }
        }
    });
    external_notice.append(&save_copy);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&menubar);
    vbox.append(&tab_bar.widget());
    vbox.append(&external_notice);
    vbox.append(&find_bar.widget());
    vbox.append(&content);
    content.set_vexpand(true);
    vbox.set_vexpand(true);
    window.set_child(Some(&vbox));

    // §20 (ISSUE-022): composiciones del `.mdproject.toml` de la raíz del
    // proyecto activo; en standalone la lista queda vacía y `install_actions`
    // deshabilita las acciones de composición (`compositions_available`). El
    // redescubrimiento en caliente (B1) usa el mismo `composition::refresh`.
    let mut compositions = Vec::new();
    composition::refresh(&mut compositions, scope.project_root().as_deref());

    let cmds = Rc::new(AppCmds {
        app: app.clone(),
        window: window.clone(),
        scope,
        tabs: tabs.clone(),
        editor_pane,
        preview,
        tab_bar,
        external_notice,
        find_bar,
        quickopen,
        command_palette,
        index: RefCell::new(None),
        index_generation: Cell::new(0),
        intelligence_generation: Cell::new(0),
        intelligence_busy: Cell::new(false),
        intelligence_pending: Cell::new(false),
        intelligence_suggest: Cell::new(false),
        completion_source: RefCell::new(None),
        navigation_list,
        navigation_kind: Cell::new(false),
        navigation_positions: RefCell::new(std::collections::BTreeMap::new()),
        recent: RefCell::new(Vec::new()),
        quickopen_dir: RefCell::new(None),
        sidebar,
        watcher,
        compositions: RefCell::new(compositions),
        action_group: RefCell::new(None),
        composition_preview: RefCell::new(None),
        self_ops: Rc::new(SelfOps::default()),
        settings: RefCell::new(settings),
        academic_monitors: RefCell::new(Vec::new()),
        academic_paths: RefCell::new(Vec::new()),
        view: Rc::new(Cell::new(ViewMode::Split)),
        refresh_scheduled: Rc::new(Cell::new(false)),
        slot: slot.clone(),
    });
    *slot.borrow_mut() = Some(Rc::downgrade(&cmds));
    // B1: la ventana conserva una referencia DÉBIL al grupo para reevaluar
    // el `enabled` de las acciones de composiciones cuando la lista cambie
    // (§20) sin cerrar el ciclo de referencias con `cmds`.
    *cmds.action_group.borrow_mut() = Some(install_actions(&window, app, cmds.clone()).downgrade());

    // ISSUE-026 (§46): bucle de eventos del watcher en el hilo de UI. Se detiene
    // con la ventana.
    if cmds.watcher.is_some() {
        glib::timeout_add_local(Duration::from_millis(WATCHER_POLL_MS), {
            let slot = slot.clone();
            let window = window.downgrade();
            move || {
                if window.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    me.pump_watcher();
                }
                glib::ControlFlow::Continue
            }
        });
    }

    // §3.7: cerrar la ventana con cambios sin guardar pide confirmación.
    window.connect_close_request({
        let cmds = cmds.clone();
        move |window| {
            // §14.3: al cerrar se persiste el estado del explorador.
            cmds.persist_explorer_state();
            let any_modified = cmds
                .tabs
                .borrow()
                .tabs()
                .iter()
                .any(|tab| tab.editor.document().is_modified());
            if !any_modified {
                return glib::Propagation::Proceed;
            }
            let window = window.clone();
            glib::spawn_future_local(async move {
                if dialogs::confirm_discard(&window, "los documentos abiertos").await {
                    window.destroy();
                }
            });
            glib::Propagation::Stop
        }
    });

    cmds.reload_active();
    cmds.apply_view();
    // Cambios de configuración global, también con pestañas sin editar.
    // Solo se consulta el stamp del archivo, nunca se recorre el proyecto.
    let mut config_stamp = settings::global_path()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok());
    glib::timeout_add_local(Duration::from_secs(1), {
        let slot = slot.clone();
        let window = window.downgrade();
        move || {
            if window.upgrade().is_none() {
                return glib::ControlFlow::Break;
            }
            let stamp = settings::global_path()
                .and_then(|p| std::fs::metadata(p).ok())
                .and_then(|m| m.modified().ok());
            if stamp != config_stamp {
                config_stamp = stamp;
                if let Some(me) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    me.refresh_now();
                }
            }
            glib::ControlFlow::Continue
        }
    });
    window.present();
    cmds
}

/// Garantiza que el sandbox de WebKit puede funcionar antes de crear la
/// WebView (fix de arranque: crash por `dbus-proxy` en kernels sin user
/// namespaces).
///
/// WebKitGTK lanza sus procesos Web dentro de bubblewrap, que necesita user
/// namespaces sin privilegios. En kernels que los restringen (p. ej. Ubuntu
/// con `kernel.apparmor_restrict_unprivileged_userns=1`) el lanzamiento del
/// `dbus-proxy` dentro de bwrap aborta el proceso entero (`Failed to fully
/// launch dbus-proxy` → SIGTRAP). Si el sandbox no puede arrancar, se
/// desactiva explícitamente con la variable que WebKit define para ello:
/// minmark es una app local sin red (§3.4) y la alternativa es un crash al
/// abrir cualquier ventana con Preview (decisión registrada en CHANGELOG).
fn ensure_sandbox_usable() {
    let bwrap_ok = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "/bin/true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !bwrap_ok {
        std::env::set_var("WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS", "1");
    }
}

fn main() {
    runtime::configure();
    // Antes de tocar ningún widget de WebKit (§4.5: el sandbox es un medio,
    // no el fin; un crash en el arranque no es aceptable).
    ensure_sandbox_usable();
    // Diagnóstico del Preview (ISSUE-57): termina aquí, sin abrir las
    // ventanas de edición.
    if std::env::args().any(|arg| arg == "--check-preview") {
        std::process::exit(check_preview());
    }
    // Argumentos (ISSUE-021): cada carpeta abre una ventana de proyecto y los
    // archivos una ventana standalone con sus pestañas (como Notepad++).
    let mut files: Vec<PathBuf> = Vec::new();
    let mut folders: Vec<PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        let path = PathBuf::from(arg);
        if path.is_dir() {
            folders.push(path);
        } else {
            files.push(path);
        }
    }
    let app = Application::builder().application_id("app.minmark").build();
    app.connect_activate(move |app| {
        if !files.is_empty() {
            build_window(app, WindowScope::Standalone, &files);
        }
        for folder in &folders {
            build_window(app, WindowScope::Project(folder.clone()), &[]);
        }
        if files.is_empty() && folders.is_empty() {
            build_window(app, WindowScope::Standalone, &[]);
        }
    });
    // Los ficheros de la línea de comandos se abren en `connect_activate`; no
    // se pasan como argumentos de GApplication (que exigiría HANDLES_OPEN).
    app.run_with_args::<&str>(&["minmark"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ISSUE-026 criterio 3: los cambios generados por nuestras operaciones
    /// §14.8 no avisan como cambio externo; una entrada por evento.
    #[test]
    fn self_ops_consume_los_cambios_propios() {
        let ops = SelfOps::default();
        ops.register(Path::new("/p/cap"));
        ops.register(Path::new("/p/doc.md"));

        assert!(ops.consume(Path::new("/p/doc.md")), "el propio archivo");
        assert!(
            !ops.consume(Path::new("/p/doc.md")),
            "una sola entrada por registro"
        );
        assert!(
            ops.consume(Path::new("/p/cap/a.md")),
            "los eventos dentro de una carpeta propia también"
        );
        assert!(
            !ops.consume(Path::new("/p/ajeno.md")),
            "lo externo sigue avisando"
        );
    }

    /// §9.1 (petición de usuario): la barra vertical del split editor|preview
    /// está en el CSS de ambos temas con su color y grosor de 4px.
    #[test]
    fn css_del_split_editor_preview_en_ambos_temas() {
        let light = theme_css(&Palette::LIGHT, ResolvedTheme::Light);
        let dark = theme_css(&Palette::DARK, ResolvedTheme::Dark);
        for (css, color) in [(&light, "#767676"), (&dark, "#050505")] {
            assert!(
                css.contains("paned.mm-split > separator"),
                "regla del separador del split"
            );
            assert!(css.contains("min-width: 4px"), "barra de 4px");
            assert!(css.contains(color), "color {color} del tema");
        }
        // Solo el tema claro es gris y solo el oscuro, negro.
        assert!(light.contains("background: #767676"));
        assert!(dark.contains("background: #050505"));
    }

    /// §41 (ISSUE-029): el MRU de Quick Open deja lo más reciente primero,
    /// sin duplicados y acotado a los 8 que bonifica el ranking.
    #[test]
    fn mru_quickopen_reciente_primero_sin_duplicados_y_techo() {
        let mut recent = Vec::new();
        mru_touch(&mut recent, "a.md");
        mru_touch(&mut recent, "docs/b.md");
        mru_touch(&mut recent, "a.md");
        assert_eq!(
            recent,
            ["a.md", "docs/b.md"],
            "lo reciente al frente y sin duplicados"
        );
        for i in 0..MRU_MAX + 2 {
            mru_touch(&mut recent, &format!("f{i}.md"));
        }
        assert_eq!(recent.len(), MRU_MAX, "techo del MRU §41");
        assert_eq!(recent[0], format!("f{}.md", MRU_MAX + 1));
        assert!(
            !recent.iter().any(|opened| opened == "a.md"),
            "lo viejo cae con el techo"
        );
    }

    /// §14.4/§14.5/§14.6 (ISSUE-029): la lista standalone de Quick Open son
    /// los tipos documentales visibles del subárbol, sin ocultos ni
    /// ignorados, en orden estable por `rel_path` §17.
    #[test]
    fn lista_standalone_de_quickopen_respeta_14_4_14_6() {
        let base = std::env::temp_dir().join(format!("minmark-qo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("carpeta")).unwrap();
        std::fs::create_dir_all(base.join(".oculta")).unwrap();
        std::fs::write(base.join("a.md"), "# a\n").unwrap();
        std::fs::write(base.join("carpeta/b.md"), "# b\n").unwrap();
        std::fs::write(base.join("imagen.png"), "").unwrap();
        std::fs::write(base.join("guion.txt"), "").unwrap(); // §14.4: fuera
        std::fs::write(base.join("ignorado.md"), "").unwrap(); // §14.6
        std::fs::write(base.join(".oculto.md"), "").unwrap(); // §14.5
        std::fs::write(base.join(".oculta/c.md"), "").unwrap(); // §14.5
        std::fs::write(base.join(".gitignore"), "ignorado.md\n").unwrap();

        let entries = quickopen_entries(&base);
        let rel_paths: Vec<&str> = entries
            .iter()
            .map(|entry| entry.rel_path.as_str())
            .collect();
        assert_eq!(rel_paths, ["a.md", "carpeta/b.md", "imagen.png"]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Fase 7 #30: cancelar un formulario (botón, Escape o cierre ⇒ `None`)
    /// no produce NINGUNA acción: sin acción no hay edición (cero cambios,
    /// §UI) y, en la importación, tampoco hay plan de copia (el disco solo
    /// se toca tras aceptar).
    #[test]
    fn formularios_30_cancelar_no_producen_accion() {
        assert!(image_edit_action(None).is_none());
        assert!(table_attrs_action(None).is_none());
        assert!(table_insert_action(None).is_none());
        assert!(
            image_import_plan(Path::new("/raiz"), Some(Path::new("/raiz")), None).is_none(),
            "cancelar la importación no planifica copia ni edición"
        );
    }

    /// #30: el formulario de edición de imagen parsea Ruta/Alt/ID/Ancho con
    /// la gramática §7.4.3 y el rango §18.5; campo vacío ⇒ `None` (quita ese
    /// atributo) y valor inválido ⇒ sin acción (cero cambios).
    #[test]
    fn edit_image_action_parsea_campos_30() {
        use minmark_document::format::FormatAction;
        let form = |ruta: &str, alt: &str, id: &str, ancho: &str| {
            Some(vec![
                ruta.to_owned(),
                alt.to_owned(),
                id.to_owned(),
                ancho.to_owned(),
            ])
        };
        assert_eq!(
            image_edit_action(form("img.png", "Diagrama", "fig:d", "40")).unwrap(),
            FormatAction::ImageEdit {
                destination: "img.png".to_owned(),
                alt: "Diagrama".to_owned(),
                id: Some("fig:d".to_owned()),
                width_percent: Some(40),
            }
        );
        // El `#` inicial se admite tecleado; ancho vacío ⇒ sin ancho.
        assert_eq!(
            image_edit_action(form("img.png", "alt", "#fig:d", "")).unwrap(),
            FormatAction::ImageEdit {
                destination: "img.png".to_owned(),
                alt: "alt".to_owned(),
                id: Some("fig:d".to_owned()),
                width_percent: None,
            }
        );
        // Ruta vacía, ID con caracteres ajenos o ancho fuera de §18.5.
        assert!(image_edit_action(form("", "alt", "", "")).is_none());
        assert!(image_edit_action(form("img.png", "alt", "mal id", "")).is_none());
        assert!(image_edit_action(form("img.png", "alt", "", "101")).is_none());
        // El alt es texto libre: se lee tal cual (un valor intacto sigue
        // siendo no-op byte a byte dentro de `ImageEdit`).
        assert_eq!(
            image_edit_action(form("img.png", "  alt  ", "", "")).unwrap(),
            FormatAction::ImageEdit {
                destination: "img.png".to_owned(),
                alt: "  alt  ".to_owned(),
                id: None,
                width_percent: None,
            }
        );
    }

    /// #30 §35: el diálogo de tabla solo genera `TableInsert` dentro de los
    /// límites (filas 1–50, columnas 1–8); cancelar o salirse del rango ⇒
    /// sin acción. Los validadores del diálogo retienen el error (§UI).
    #[test]
    fn insert_table_limites_35() {
        use minmark_document::format::FormatAction;
        let dims = |rows: &str, cols: &str| Some(vec![rows.to_owned(), cols.to_owned()]);
        assert_eq!(
            table_insert_action(dims("2", "3")).unwrap(),
            FormatAction::TableInsert { rows: 2, cols: 3 }
        );
        assert_eq!(
            table_insert_action(dims("1", "1")).unwrap(),
            FormatAction::TableInsert { rows: 1, cols: 1 }
        );
        assert_eq!(
            table_insert_action(dims("50", "8")).unwrap(),
            FormatAction::TableInsert { rows: 50, cols: 8 }
        );
        for (rows, cols) in [("0", "3"), ("51", "3"), ("2", "0"), ("2", "9"), ("x", "3")] {
            assert!(
                table_insert_action(dims(rows, cols)).is_none(),
                "filas={rows} columnas={cols} fuera de §35"
            );
        }
        assert!(validate_table_dims(&["0".to_owned(), "3".to_owned()]).is_some());
        assert!(validate_table_dims(&["2".to_owned(), "9".to_owned()]).is_some());
        assert!(validate_table_dims(&["2".to_owned(), "3".to_owned()]).is_none());
    }

    /// #30: los atributos de tabla llegan como `None` cuando el campo está
    /// vacío (quita id/caption, §26.3); el caption se entrega VERBATIM
    /// (hallazgo 7: un valor precargado intacto debe seguir siendo no-op);
    /// id inválido ⇒ sin acción.
    #[test]
    fn table_attrs_action_parsea_campos_30() {
        use minmark_document::format::FormatAction;
        let form = |id: &str, caption: &str| Some(vec![id.to_owned(), caption.to_owned()]);
        // Caption con espacios de relleno: se conserva byte a byte (solo un
        // cambio real llega al documento).
        assert_eq!(
            table_attrs_action(form("#tbl:x", " Resultados ")).unwrap(),
            FormatAction::TableAttrs {
                id: Some("tbl:x".to_owned()),
                caption: Some(" Resultados ".to_owned()),
            }
        );
        assert_eq!(
            table_attrs_action(form("tbl:x", "Nueva caption")).unwrap(),
            FormatAction::TableAttrs {
                id: Some("tbl:x".to_owned()),
                caption: Some("Nueva caption".to_owned()),
            }
        );
        // Vacío o solo espacios ⇒ `None`: quita el caption (§26.3).
        assert_eq!(
            table_attrs_action(form("", "   ")).unwrap(),
            FormatAction::TableAttrs {
                id: None,
                caption: None,
            }
        );
        assert!(table_attrs_action(form("mal id", "c")).is_none());
    }

    /// #31 §UI: cancelar (o cerrar) cualquier diálogo de ecuaciones o citas
    /// no produce ninguna acción ⇒ cero cambios; tampoco el LaTeX vacío ni
    /// los items inválidos que el diálogo retiene con error visible.
    #[test]
    fn formularios_31_cancelar_no_producen_accion() {
        use minmark_document::CitationItem;
        assert!(equation_edit_action(None).is_none());
        assert!(equation_insert_action(None).is_none());
        assert!(math_attrs_action(None).is_none());
        assert!(citation_insert_action(None).is_none());
        assert!(citation_edit_action(None).is_none());
        // LaTeX vacío o de solo espacios: sin acción (§24; el diálogo lo
        // retiene, el plan núcleo también lo rechaza).
        assert!(equation_edit_action(Some("   \n".to_owned())).is_none());
        assert!(equation_insert_action(Some(String::new())).is_none());
        // Items sin ítems o con clave vacía: sin acción (§25.2).
        assert!(citation_insert_action(Some(Vec::new())).is_none());
        assert!(
            citation_edit_action(Some(vec![CitationItem {
                key: " ".to_owned(),
                locator: None,
            }]))
            .is_none()
        );
    }

    /// #31 §24: `MathEdit` entrega el LaTeX VERBATIM (un valor intacto sigue
    /// siendo no-op byte a byte) y `MathInsert` deriva la forma canónica del
    /// contenido — monolínea ⇒ `$…$`, multilínea ⇒ `$$…$$`.
    #[test]
    fn ecuacion_actions_derivan_la_forma_canonica_31() {
        use minmark_document::format::FormatAction;
        assert_eq!(
            equation_edit_action(Some("E = mc^2 + 1".to_owned())).unwrap(),
            FormatAction::MathEdit {
                latex: "E = mc^2 + 1".to_owned()
            }
        );
        assert_eq!(
            equation_edit_action(Some("E = mc^2".to_owned())).unwrap(),
            FormatAction::MathEdit {
                latex: "E = mc^2".to_owned()
            }
        );
        assert_eq!(
            equation_insert_action(Some("E = mc^2".to_owned())).unwrap(),
            FormatAction::MathInsert {
                block: false,
                latex: "E = mc^2".to_owned()
            }
        );
        assert_eq!(
            equation_insert_action(Some("\\int_0^1 x\\,dx\n+ c".to_owned())).unwrap(),
            FormatAction::MathInsert {
                block: true,
                latex: "\\int_0^1 x\\,dx\n+ c".to_owned()
            }
        );
    }

    /// #31 §26.4: `MathAttrs` reutiliza los campos de Table Attributes (el
    /// MISMO formulario, §7.4.3 y caption verbatim); id inválido ⇒ sin
    /// acción.
    #[test]
    fn math_attrs_action_parsea_campos_31() {
        use minmark_document::format::FormatAction;
        let form = |id: &str, caption: &str| Some(vec![id.to_owned(), caption.to_owned()]);
        assert_eq!(
            math_attrs_action(form("eq:energy", " Equivalencia ")).unwrap(),
            FormatAction::MathAttrs {
                id: Some("eq:energy".to_owned()),
                caption: Some(" Equivalencia ".to_owned()),
            }
        );
        assert_eq!(
            math_attrs_action(form("", "")).unwrap(),
            FormatAction::MathAttrs {
                id: None,
                caption: None,
            }
        );
        assert!(math_attrs_action(form("mal id", "c")).is_none());
    }

    /// #31 §25.2: los items del diálogo (clave + localizador) llegan como
    /// `CitationInsert`/`CitationEdit`; el localizador vacío es `None` (el
    /// diálogo lo recorta al recogerlo) y la clave vacía no genera acción.
    #[test]
    fn cita_actions_parsean_items_31() {
        use minmark_document::format::FormatAction;
        use minmark_document::CitationItem;
        let items = || {
            vec![
                CitationItem {
                    key: "smith2025".to_owned(),
                    locator: Some("p. 42".to_owned()),
                },
                CitationItem {
                    key: "jones2024".to_owned(),
                    locator: None,
                },
            ]
        };
        assert_eq!(
            citation_insert_action(Some(items())).unwrap(),
            FormatAction::CitationInsert { items: items() }
        );
        assert_eq!(
            citation_edit_action(Some(items())).unwrap(),
            FormatAction::CitationEdit { items: items() }
        );
        assert!(
            citation_insert_action(Some(vec![CitationItem {
                key: String::new(),
                locator: None,
            }]))
            .is_none(),
            "clave vacía ⇒ sin acción (§25.2)"
        );

        // Revisión #31 (finding 3): gramática consumidora — espejo de
        // `try_citation`/del núcleo. Clave ingrávida o localizador con
        // separador/cierre ⇒ el diálogo RETIENE (mensaje visible) y las
        // builders NO producen acción: la cita válida del documento nunca
        // se reemplaza por texto que el renderer no pueda parsear.
        let clave_mala = || {
            vec![CitationItem {
                key: "smith 2025".to_owned(),
                locator: None,
            }]
        };
        let localizador_malo = |locator: &str| {
            vec![CitationItem {
                key: "smith2025".to_owned(),
                locator: Some(locator.to_owned()),
            }]
        };
        assert!(
            citation_insert_action(Some(clave_mala())).is_none(),
            "espacio en la clave no es sintaxis (§25.2)"
        );
        assert!(
            citation_edit_action(Some(clave_mala())).is_none(),
            "espacio en la clave no es sintaxis (§25.2)"
        );
        for locator in ["p. 1; fig. 2", "p. 42]", "dos\nlíneas"] {
            assert!(
                citation_edit_action(Some(localizador_malo(locator))).is_none(),
                "localizador {locator:?} no admisible (§25.2)"
            );
        }
        // El validador del diálogo (retención con error, patrón
        // FormValidator) apunta exactamente a esos casos y deja pasar la
        // forma válida `smith2025` + `p. 42`.
        let mensaje = dialogs::validate_citation_items(&clave_mala())
            .expect("clave con espacio: RETIENE el diálogo");
        assert!(mensaje.contains("no válida"), "{mensaje}");
        let mensaje = dialogs::validate_citation_items(&localizador_malo("p. 1; fig. 2"))
            .expect("localizador con «;»: RETIENE el diálogo");
        assert!(mensaje.contains("no válido"), "{mensaje}");
        assert!(
            dialogs::validate_citation_items(&items()).is_none(),
            "smith2025 + p. 42 + jones2024: válida"
        );
        assert!(
            citation_insert_action(Some(items())).is_some(),
            "la forma válida sigue produciendo acción"
        );
    }

    /// #30 §18.2/§18.3/§17: el plan de importación elige el destino libre
    /// (`diagram.png`, `diagram-2.png`, …) bajo `<raíz>/<ruta>`, devuelve la
    /// ruta RELATIVA a la carpeta del documento y NO toca el disco (la copia
    /// es del llamador, que la retira si la edición no aplicara); cancelar o
    /// datos inválidos ⇒ sin plan ⇒ sin copia.
    #[test]
    fn import_imagen_plan_renombrado_18_3_y_relativa_17() {
        use minmark_document::format::FormatAction;
        let base = std::env::temp_dir().join(format!("minmark-import30-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("proyecto");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/diagram.png"), "viejo").unwrap();

        let form = |ruta: &str| {
            Some(vec![
                ruta.to_owned(),
                "Diagrama".to_owned(),
                String::new(),
                String::new(),
            ])
        };

        // Colisión §18.3: `diagram.png` ocupado ⇒ `diagram-2.png`, con la
        // ruta relativa §17 desde la carpeta del documento (aquí, la raíz).
        let (target, action) =
            image_import_plan(&root, Some(&root), form("assets/diagram.png")).expect("plan");
        assert_eq!(target, root.join("assets/diagram-2.png"));
        assert!(
            !target.exists(),
            "el plan NO copia: la copia es del llamador (§UI «cero cambios»)"
        );
        assert_eq!(
            action,
            FormatAction::ImageInsert {
                destination: "assets/diagram-2.png".to_owned(),
                alt: "Diagrama".to_owned(),
                id: None,
                width_percent: None,
            }
        );

        // Documento en subcarpeta: la relativa §17 sale con `..`.
        let (_, action) =
            image_import_plan(&root, Some(&root.join("docs")), form("assets/diagram.png"))
                .expect("plan");
        assert_eq!(
            action,
            FormatAction::ImageInsert {
                destination: "../assets/diagram-2.png".to_owned(),
                alt: "Diagrama".to_owned(),
                id: None,
                width_percent: None,
            }
        );

        // Sin colisión: primer nombre libre §18.3.
        let (target, _) = image_import_plan(&root, Some(&root), form("assets/figura.png"))
            .expect("plan");
        assert_eq!(target, root.join("assets/figura.png"));

        // §48: ruta absoluta, con `..` o sin extensión no se planifican.
        for ruta in ["/abs/figura.png", "../fuera.png", "assets/sin-extension"] {
            assert!(
                image_import_plan(&root, Some(&root), form(ruta)).is_none(),
                "ruta no admisible: {ruta}"
            );
        }
        // ID o ancho inválidos tampoco: no se copia nada que no se va a enlazar.
        let mut id_malo = form("assets/x.png").unwrap();
        id_malo[2] = "mal id".to_owned();
        assert!(image_import_plan(&root, Some(&root), Some(id_malo)).is_none());
        let mut ancho_malo = form("assets/x.png").unwrap();
        ancho_malo[3] = "0".to_owned();
        assert!(image_import_plan(&root, Some(&root), Some(ancho_malo)).is_none());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// #30: los validadores de los formularios retienen el error en abierto
    /// (§UI) — ruta obligatoria en edición, ruta relativa con extensión en
    /// importación (§48/§18.2) e ID/Ancho §7.4.3/§18.5 en ambos.
    #[test]
    fn validadores_de_formularios_30() {
        let valores = |ruta: &str, id: &str, ancho: &str| {
            vec![
                ruta.to_owned(),
                "Alt".to_owned(),
                id.to_owned(),
                ancho.to_owned(),
            ]
        };
        assert!(validate_image_form(&valores("", "", "")).is_some(), "ruta obligatoria");
        assert!(validate_image_form(&valores("img.png", "", "")).is_none());
        // La edición acepta cualquier destino no vacío (externo incluido)…
        assert!(validate_image_form(&valores("https://x/y.png", "", "")).is_none());
        // …la importación exige una ruta relativa con extensión (§48).
        assert!(validate_image_import_form(&valores("assets/x.png", "", "")).is_none());
        assert!(validate_image_import_form(&valores("/abs/x.png", "", "")).is_some());
        assert!(validate_image_import_form(&valores("../x.png", "", "")).is_some());
        assert!(validate_image_import_form(&valores("assets/", "", "")).is_some());
        assert!(validate_image_form(&valores("img.png", "", "101")).is_some());
        assert!(validate_image_form(&valores("img.png", "mal id", "")).is_some());
        assert!(validate_table_attrs_form(&["mal id".to_owned(), "c".to_owned()]).is_some());
        assert!(validate_table_attrs_form(&["tbl:x".to_owned(), String::new()]).is_none());
    }
}

/// Symlink portable para tests (la suite de UI corre también en Windows):
/// `unix` tiene un solo tipo de symlink; Windows distingue directorio y
/// fichero según el destino.
#[cfg(test)]
pub mod test_support {
    use std::path::Path;

    #[cfg(unix)]
    pub fn symlink(origen: impl AsRef<Path>, destino: impl AsRef<Path>) -> std::io::Result<()> {
        std::os::unix::fs::symlink(origen, destino)
    }

    #[cfg(windows)]
    pub fn symlink(origen: impl AsRef<Path>, destino: impl AsRef<Path>) -> std::io::Result<()> {
        let origen = origen.as_ref();
        if origen.is_dir() {
            std::os::windows::fs::symlink_dir(origen, destino)
        } else {
            std::os::windows::fs::symlink_file(origen, destino)
        }
    }
}

/// Espera `condition` drenando el main loop de GTK (`--check-preview`, tests
/// y escenario QA del Preview, ISSUE-57).
fn pump_until(condition: impl Fn() -> bool) {
    // 30 s: los runners de Windows cargados pueden tardar varios segundos en
    // abrir la primera ventana; un cuelgue real también se detecta (y el
    // mensaje de fase del escenario QA dice dónde).
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "timeout GTK");
        // Un backend puede mantener eventos pendientes de forma continua.
        // Ceder tras cada lote permite comprobar la condición y el plazo.
        let context = glib::MainContext::default();
        for _ in 0..16 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Espera a que la inteligencia y el Preview terminen sus refrescos.
fn settle_preview(cmds: &AppCmds) {
    pump_until(|| {
        !cmds.intelligence_busy.get()
            && !cmds.intelligence_pending.get()
            && !cmds.refresh_scheduled.get()
            && !cmds.preview.test_is_loading()
    });
}

/// Evalúa `script` en la página del Preview y devuelve su resultado numérico.
fn preview_number(cmds: &AppCmds, script: &str) -> f64 {
    settle_preview(cmds);
    let result = Rc::new(Cell::new(None));
    cmds.preview.test_eval_number(script, result.clone());
    pump_until(|| result.get().is_some());
    result.get().unwrap()
}

fn wait_preview_allocation(cmds: &AppCmds) -> Result<(), String> {
    let native = cmds.preview.native_widget();
    // El primer tick precede al layout; el siguiente ya observa su allocation.
    let laid_out = Rc::new(Cell::new(false));
    let ready = laid_out.clone();
    let frames = Cell::new(0);
    // El reloj de la ventana sigue activo aunque el host sea una caja vacía
    // sobre la que el sistema dibuja la vista nativa.
    cmds.window.add_tick_callback(move |_, _| {
        frames.set(frames.get() + 1);
        if frames.get() >= 2 {
            ready.set(true);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    pump_until(|| laid_out.get());
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (expected_width, expected_height) = (native.width(), native.height());
        let actual_width = preview_number(cmds, "window.innerWidth");
        let actual_height = preview_number(cmds, "window.innerHeight");
        if expected_width > 0
            && expected_height > 0
            && (actual_width - f64::from(expected_width)).abs() <= 2.0
            && (actual_height - f64::from(expected_height)).abs() <= 2.0
        {
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("Preview desajustada tras redimensionar: GTK {expected_width}x{expected_height}, web {actual_width}x{actual_height}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn check_preview_allocation(cmds: &AppCmds) -> Result<(), String> {
    if let Some(sidebar) = &cmds.sidebar {
        let was_visible = sidebar.visible.get();
        if !was_visible {
            cmds.toggle_explorer();
        }
        let previous = sidebar.paned.position();
        // Regresión Windows: un estado guardado de 160 px desplazaba la
        // cabecera y los nombres 87 px fuera de la ventana.
        sidebar.paned.set_position(explorer::MIN_WIDTH);
        wait_preview_allocation(cmds)?;
        let bounds = sidebar
            .widget
            .compute_bounds(&cmds.window)
            .ok_or("no se pudo medir la barra lateral")?;
        if bounds.x() < 0.0 || bounds.width() > sidebar.paned.position() as f32 + 1.0 {
            return Err(format!(
                "barra lateral recortada al restaurar su ancho mínimo: {bounds:?}"
            ));
        }
        sidebar.paned.set_position(previous);
        wait_preview_allocation(cmds)?;
        if !was_visible {
            cmds.toggle_explorer();
            wait_preview_allocation(cmds)?;
        }
    }
    #[cfg(any(windows, target_os = "macos"))]
    if cmds.preview.native_widget().layout_manager().is_some() {
        return Err("el host nativo no debe interceptar size_allocate con BoxLayout".into());
    }
    let native = cmds.preview.native_widget();
    let mut allocations = std::collections::BTreeSet::new();
    for (width, height) in [(760, 520), (1180, 760), (860, 560)] {
        println!("preview-check: solicitar ventana {width}x{height}");
        cmds.window.set_default_size(width, height);
        wait_preview_allocation(cmds)?;
        let actual = (cmds.window.width(), cmds.window.height());
        println!("preview-check: ventana actual {}x{}", actual.0, actual.1);
        allocations.insert((native.width(), native.height()));
    }
    #[cfg(windows)]
    {
        cmds.window.maximize();
        pump_until(|| cmds.window.is_maximized());
        wait_preview_allocation(cmds)?;
        cmds.window.unmaximize();
        pump_until(|| !cmds.window.is_maximized());
        wait_preview_allocation(cmds)?;
    }
    for mode in [ViewMode::Preview, ViewMode::Split] {
        cmds.set_view(mode);
        wait_preview_allocation(cmds)?;
        allocations.insert((native.width(), native.height()));
    }
    cmds.toggle_explorer();
    wait_preview_allocation(cmds)?;
    allocations.insert((native.width(), native.height()));
    cmds.toggle_explorer();
    wait_preview_allocation(cmds)?;
    allocations.insert((native.width(), native.height()));
    // El gestor de ventanas puede limitar los tamaños exteriores (p. ej.
    // la pantalla de un runner). Exigir cambios REALES del viewport, también
    // al cambiar vistas/barra lateral, mantiene la regresión de size_allocate.
    if allocations.len() < 2 {
        return Err("la Preview no cambió de tamaño durante las pruebas de layout".into());
    }
    Ok(())
}

/// Abre y cancela un selector real: detecta aborts de GSettings que nunca
/// aparecen al limitar la QA al DOM de la Preview. No escribe ningún archivo.
// Cocoa requiere validar la interacción del panel manualmente: su bucle
// modal no siempre devuelve el control al diagnóstico tras cancelar.
#[cfg(not(target_os = "macos"))]
fn check_save_dialog(window: &impl IsA<gtk4::Window>) -> Result<(), String> {
    let dialog = gtk4::FileDialog::builder()
        .title("Autocomprobación: Guardar como (se cierra automáticamente)")
        .initial_name("minmark-qa.md")
        .modal(true)
        .build();
    let cancellable = gtk4::gio::Cancellable::new();
    // Programar la cancelación antes de abrir el selector.
    glib::timeout_add_local_once(Duration::from_millis(300), {
        let cancellable = cancellable.clone();
        move || cancellable.cancel()
    });
    let result = Rc::new(RefCell::new(None));
    let done = result.clone();
    dialog.save(Some(window), Some(&cancellable), move |outcome| {
        *done.borrow_mut() = Some(match outcome {
            Err(error)
                if error.matches(gtk4::DialogError::Cancelled)
                    || error.matches(gtk4::DialogError::Dismissed)
                    || error.matches(gtk4::gio::IOErrorEnum::Cancelled) =>
            {
                Ok(())
            }
            Err(error) => Err(format!("selector Guardar como: {error}")),
            Ok(_) => Err("el selector debía cancelarse sin elegir un destino".into()),
        });
    });
    pump_until(|| result.borrow().is_some());
    let outcome = result.borrow_mut().take().unwrap();
    outcome
}

/// Comprobación del Preview (ISSUE-57): debe pintar el documento, seguir las
/// ediciones, funcionar sobre una «Sin título» recién abierta y renderear el
/// tema oscuro con contraste real. Si la vista nativa no se presenta, el DOM
/// queda vacío (`innerText` exige layout real); si el texto es invisible
/// (color = fondo), también falla.
///
/// Es el motor de `minmark --check-preview` (paquetes empaquetados y
/// máquinas reales), del `#[test]` de Linux y del binario de prueba
/// `tests/qa_preview.rs` (sin harness, hilo principal del proceso: el único
/// camino en macOS, donde GTK exige ese hilo y libtest corre cada test en el
/// suyo).
fn preview_check() -> Result<(), String> {
    // Una API nativa puede bloquear dentro de iteration()/save(), donde el
    // plazo de pump_until no llega a ejecutarse. Acotar también el proceso
    // de diagnóstico para conservar la última fase y evitar una CI colgada.
    struct Completed(std::sync::mpsc::Sender<()>);
    impl Drop for Completed {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let (completed, deadline) = channel();
    let _completed = Completed(completed);
    std::thread::spawn(move || {
        if matches!(
            deadline.recv_timeout(Duration::from_secs(90)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            eprintln!(
                "preview-check: timeout global de 90s; consultar la última fase del diagnóstico"
            );
            std::process::exit(1);
        }
    });
    runtime::configure();
    runtime::check_file_dialog()?;
    gtk4::init().map_err(|error| format!("no hay sesión gráfica: {error}"))?;
    ensure_sandbox_usable();
    let root = std::env::temp_dir().join(format!("minmark-gtk-preview-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("doc.md");
    std::fs::write(&path, "# Original\n").unwrap();
    let app = Application::builder()
        .application_id("app.minmark.test.preview-render")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk4::gio::Cancellable>).unwrap();
    let cmds = build_window(&app, WindowScope::Project(root.clone()), &[path.clone()]);
    println!("preview-check: fase fichero/abrir");
    pump_until(|| cmds.index.borrow().is_some());
    cmds.window.set_default_size(900, 600);
    pump_until(|| cmds.window.is_mapped());
    settle_preview(&cmds);

    // La preview pinta el documento cargado.
    let painted = preview_number(&cmds, "document.body ? document.body.innerText.length : -1");
    if painted <= 0.0 {
        return Err(format!(
            "fase fichero/abrir: la preview no pinta el documento (innerText = {painted})"
        ));
    }

    // Editar actualiza la MISMA página, sin recargarla, con el texto nuevo.
    println!("preview-check: fase fichero/escribir");
    cmds.tabs
        .borrow_mut()
        .insert_active(Position::new(1, 0), "Marcador\n");
    cmds.reload_active();
    settle_preview(&cmds);
    let updated = preview_number(
        &cmds,
        "document.body.innerText.includes('Marcador') ? 1 : 0",
    );
    if updated != 1.0 {
        return Err("fase fichero/escribir: la preview no se actualiza al escribir".into());
    }

    println!("preview-check: fase ventana/redimensionar");
    check_preview_allocation(&cmds)?;
    println!("preview-check: fase fichero/guardar");
    cmds.save();
    if std::fs::read_to_string(&path).unwrap() != "# Original\nMarcador\n"
        || cmds.tabs.borrow().active().editor.document().is_modified()
    {
        return Err("guardar no conserva el contenido o el indicador de cambios".into());
    }
    println!("preview-check: fase unicode/editar-guardar");
    let scroll = cmds
        .editor_pane
        .widget()
        .first_child()
        .and_then(|widget| widget.downcast::<gtk4::ScrolledWindow>().ok())
        .ok_or("no se encontró el contenedor del editor")?;
    let view = scroll
        .child()
        .and_then(|widget| widget.downcast::<gtk4::TextView>().ok())
        .ok_or("no se encontró el editor de texto")?;
    let buffer = view.buffer();
    buffer.insert(&mut buffer.end_iter(), "\n$α_0$.\n");
    let expected_saved = cmds
        .tabs
        .borrow()
        .active()
        .editor
        .document()
        .content()
        .to_owned();
    // Mismo comando que Ctrl+S, justo después de editar el TextBuffer real,
    // sin esperar al refresco diferido que también interpreta las ecuaciones.
    gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.save", None)
        .map_err(|error| format!("no se pudo activar Guardar: {error}"))?;
    if std::fs::read_to_string(&path).unwrap() != expected_saved
        || cmds.tabs.borrow().active().editor.document().is_modified()
    {
        return Err("el guardado tras editar una ecuación Unicode no conserva el buffer".into());
    }
    settle_preview(&cmds);
    if preview_number(&cmds, "document.querySelectorAll('[role=math]').length") < 1.0 {
        return Err("la Preview no muestra la ecuación Unicode guardada".into());
    }
    std::fs::write(&path, "Cambio externo\n").unwrap();
    if !matches!(
        cmds.tabs.borrow_mut().save_active(),
        Err(minmark_fs::FsError::ExternalModification { .. })
    ) {
        return Err("el guardado debe detectar cambios externos".into());
    }
    cmds.save_confirmed();
    if std::fs::read_to_string(&path).unwrap() != expected_saved {
        return Err("la sobrescritura confirmada no guardó el buffer".into());
    }
    println!("preview-check: fase pestaña/cerrar-guardada");
    cmds.close_tab();
    if cmds.tabs.borrow().active().path.is_some() {
        return Err("cerrar la pestaña guardada no activó Sin título".into());
    }
    // Cerrar cambia también el documento de la WebView. Terminar esa carga
    // antes de destruir su ventana evita solapar dos vistas en el diagnóstico.
    settle_preview(&cmds);

    cmds.window.destroy();

    // ISSUE-57: el caso de uso real es arrancar y escribir en una «Sin título»
    // (sin path asignado): su ruta es distinta (`document: None`) y es la que
    // usa todo el que abre la app sin abrir fichero.
    let app = Application::builder()
        .application_id("app.minmark.test.preview-untitled")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk4::gio::Cancellable>).unwrap();
    let cmds = build_window(&app, WindowScope::Standalone, &[]);
    println!("preview-check: fase sin-título/abrir");
    cmds.window.set_default_size(900, 600);
    pump_until(|| cmds.window.is_mapped());
    cmds.tabs
        .borrow_mut()
        .insert_active(Position::new(0, 0), "Desde el arranque\n");
    cmds.reload_active();
    settle_preview(&cmds);
    let painted = preview_number(&cmds, "document.body ? document.body.innerText.length : -1");
    if painted <= 0.0 {
        return Err(format!(
            "fase sin-título/abrir: la preview de la «Sin título» no pinta (innerText = {painted})"
        ));
    }
    println!("preview-check: fase sin-título/escribir");
    cmds.tabs
        .borrow_mut()
        .insert_active(Position::new(1, 0), "Marcador\n");
    cmds.reload_active();
    settle_preview(&cmds);
    let updated = preview_number(
        &cmds,
        "document.body.innerText.includes('Marcador') ? 1 : 0",
    );
    if updated != 1.0 {
        return Err(
            "fase sin-título/escribir: la preview de la «Sin título» no se actualiza al escribir"
                .into(),
        );
    }

    // ISSUE-57: tema oscuro. `innerText` no detecta texto invisible (blanco
    // sobre blanco, p. ej.) y los runners de CI están en claro, así que se
    // exige además contraste real: el color del texto no puede ser el fondo.
    println!("preview-check: fase tema-oscuro");
    cmds.settings.borrow_mut().theme = minmark_config::Theme::Dark;
    cmds.reload_active();
    settle_preview(&cmds);
    let dark = preview_number(
        &cmds,
        "document.documentElement.dataset.theme === 'dark' ? 1 : 0",
    );
    if dark != 1.0 {
        return Err("fase tema-oscuro: el documento no se rindea en tema oscuro".into());
    }
    let contrast = preview_number(
        &cmds,
        "(function(){ const s = getComputedStyle(document.body); \
         return s.color !== s.backgroundColor ? 1 : 0; })()",
    );
    if contrast != 1.0 {
        return Err("fase tema-oscuro: el texto sería invisible (color = fondo)".into());
    }

    #[cfg(not(target_os = "macos"))]
    {
        println!("preview-check: fase sin-título/selector-guardar");
        check_save_dialog(&cmds.window)?;
        // El selector debe devolver una ventana utilizable después de cancelar.
        wait_preview_allocation(&cmds)?;
    }
    #[cfg(target_os = "macos")]
    println!("preview-check: selector nativo de macOS pendiente de comprobación manual");
    cmds.window.destroy();
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}

/// `minmark --check-preview` (ISSUE-57): ejecuta [`preview_check`] sobre la
/// build real —la de los paquetes en CI y la de las máquinas de los
/// usuarios—, imprime cada fase y termina con 0 si la Preview pinta y 1 si
/// no. Es el diagnóstico pedido a los usuarios cuando la Preview falla.
pub fn check_preview() -> i32 {
    match preview_check() {
        Ok(()) => {
            println!("check_preview: ok");
            0
        }
        Err(error) => {
            eprintln!("check_preview: {error}");
            1
        }
    }
}

/// Escenario QA del Preview para las pruebas: paniquea con el fallo.
#[cfg(test)]
pub fn qa_preview_smoke() {
    if let Err(error) = preview_check() {
        panic!("{error}");
    }
}

#[cfg(test)]
mod gtk_integration_tests {
    use super::*;
    /// Captura `window` en el PNG `dest` para la evidencia visual. Espera al
    /// primer frame pintado: según el timing del compositor, `to_node` da
    /// `None` mientras la ventana está mapeada pero aún sin pintar.
    fn capture_window(window: &gtk4::Window, dest: &Path) {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            let snapshot = gtk4::Snapshot::new();
            let paintable = gtk4::WidgetPaintable::new(Some(window));
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            if let Some(node) = snapshot.to_node() {
                let texture = window.renderer().unwrap().render_texture(&node, None);
                texture.save_to_png(dest).unwrap();
                return;
            }
            assert!(std::time::Instant::now() < deadline, "timeout captura GTK");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_external_change_notice_and_reload() {
        fn find_button(widget: &Widget, label: &str) -> Option<gtk4::Button> {
            if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
                if button.label().as_deref() == Some(label) {
                    return Some(button.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(button) = find_button(&widget, label) {
                    return Some(button);
                }
                child = widget.next_sibling();
            }
            None
        }
        fn dialog_button(label: &str) -> Option<gtk4::Button> {
            gtk4::Window::list_toplevels()
                .iter()
                .find_map(|window| find_button(window, label))
        }

        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let root = std::env::temp_dir().join(format!("minmark-gtk-warning-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("mentoring-portainer.md");
        std::fs::write(&path, "# Original\n").unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.external-warning")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Project(root.clone()), &[path.clone()]);
        pump_until(|| cmds.index.borrow().is_some());
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "local ");
        cmds.save();
        cmds.pump_watcher();
        assert!(
            !cmds.external_notice.is_visible(),
            "sin falso aviso al guardar"
        );

        std::fs::write(&path, "# Modificado fuera de minmark\n").unwrap();
        cmds.pump_watcher();
        assert!(cmds.external_notice.is_visible());
        assert_eq!(
            cmds.external_notice.tooltip_text().as_deref(),
            path.to_str()
        );
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "local # Original\n"
        );
        cmds.window.set_default_size(900, 600);
        settle_preview(&cmds);
        pump_until(|| cmds.window.is_mapped());
        capture_window(
            cmds.window.upcast_ref(),
            &std::env::temp_dir().join("minmark-external-warning.png"),
        );

        let reload =
            find_button(cmds.external_notice.upcast_ref(), "Recargar desde disco").unwrap();
        reload.emit_clicked();
        pump_until(|| !cmds.tabs.borrow().active().external_change);
        assert!(!cmds.external_notice.is_visible());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "# Modificado fuera de minmark\n"
        );

        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "sin guardar ");
        std::fs::write(&path, "# Otra version externa\n").unwrap();
        cmds.pump_watcher();
        reload.emit_clicked();
        pump_until(|| dialog_button("Cancelar").is_some());
        dialog_button("Cancelar").unwrap().emit_clicked();
        pump_until(|| dialog_button("Cancelar").is_none());
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .starts_with("sin guardar "));
        assert!(cmds.external_notice.is_visible());
        reload.emit_clicked();
        pump_until(|| dialog_button("Descartar").is_some());
        dialog_button("Descartar").unwrap().emit_clicked();
        pump_until(|| !cmds.tabs.borrow().active().external_change);
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "# Otra version externa\n"
        );
        assert!(!cmds.external_notice.is_visible());

        cmds.window.destroy();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_navigation_outline_completion_and_dirty_buffers() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let root = std::env::temp_dir().join(format!("minmark-gtk-phase4-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.md");
        let b = root.join("b.md");
        std::fs::write(&a, "# Origen\n\n[ir](b.md#nuevo)\n").unwrap();
        std::fs::write(&b, "# Anterior\n").unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.phase4")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(
            &app,
            WindowScope::Project(root.clone()),
            &[a.clone(), b.clone()],
        );
        pump_until(|| cmds.index.borrow().is_some());
        cmds.tabs.borrow_mut().replace_active(
            Range::new(Position::new(0, 0), Position::new(0, 10)),
            "# Nuevo",
        );
        cmds.select_tab(1);
        let before = cmds.tabs.borrow().len();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(2, 2), Position::new(2, 2)));
        assert!(cmds.editor_pane.emit_test_key(
            gtk4::gdk::Key::Return,
            gtk4::gdk::ModifierType::CONTROL_MASK
        ));
        pump_until(|| cmds.tabs.borrow().active().path.as_ref() == Some(&b));
        assert_eq!(cmds.tabs.borrow().len(), before);
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .starts_with("# Nuevo"));
        assert!(cmds.tabs.borrow().active().editor.document().is_modified());
        cmds.select_tab(1);
        cmds.toggle_mode();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(2, 2), Position::new(2, 2)));
        assert!(cmds.editor_pane.emit_test_key(
            gtk4::gdk::Key::Return,
            gtk4::gdk::ModifierType::CONTROL_MASK
        ));
        pump_until(|| cmds.tabs.borrow().active().path.as_ref() == Some(&b));
        cmds.select_tab(1);
        settle_preview(&cmds);
        let clicked = Rc::new(Cell::new(false));
        cmds.preview.test_click_first_link(clicked.clone());
        pump_until(|| clicked.get() && cmds.tabs.borrow().active().path.as_ref() == Some(&b));
        cmds.show_navigation(false);
        assert!(cmds.navigation_list.is_visible());
        cmds.navigation_list.hide();
        cmds.select_tab(1);
        cmds.tabs.borrow_mut().replace_active(
            Range::new(Position::new(2, 0), Position::new(2, 16)),
            "[ir](b",
        );
        cmds.reload_active();
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        let unchanged = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty()));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            unchanged
        );
        // Una pulsación nueva vuelve a habilitar sugerencias tras Escape.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(2, 6), ".");
        cmds.reload_active();
        *cmds.completion_source.borrow_mut() = None;
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Tab, gtk4::gdk::ModifierType::empty()));
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .contains("[ir](b.md"));
        cmds.tabs.borrow_mut().undo_active();
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .contains("[ir](b.\n"));
        // Mantener la posición de lectura durante ediciones sucesivas,
        // sin iniciar una nueva carga de WebKit ni perder el contenido.
        let long_source = (0..100)
            .map(|n| format!("Párrafo {n}.\n\n"))
            .collect::<String>();
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), &long_source);
        cmds.reload_active();
        settle_preview(&cmds);
        let scroll = Rc::new(Cell::new(None));
        cmds.preview
            .test_eval_number("window.scrollTo(0, 900); window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        let before_scroll = scroll.get().unwrap();
        assert!(
            before_scroll > 500.0,
            "Preview must be scrolled for this regression"
        );
        for _ in 0..3 {
            cmds.tabs
                .borrow_mut()
                .insert_active(Position::new(0, 0), "x");
            cmds.refresh_now();
            assert!(
                !cmds.preview.test_is_loading(),
                "editing must not reload Preview"
            );
        }
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview.test_eval_number(
            "document.body.textContent.includes('xxxPárrafo') ? window.scrollY : -1",
            scroll.clone(),
        );
        pump_until(|| scroll.get().is_some());
        assert!((scroll.get().unwrap() - before_scroll).abs() < 2.0);
        // Navegar explícitamente sigue prevaleciendo sobre la posición previa.
        cmds.focus_line(1);
        scroll.set(None);
        cmds.preview
            .test_eval_number("window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        assert!(scroll.get().unwrap() < 50.0);
        // Una edición real sigue el cursor hasta un bloque fuera de pantalla.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(160, 0), "editando ");
        cmds.tabs
            .borrow_mut()
            .set_selection_active(Selection::collapsed(Position::new(160, 9)));
        cmds.schedule_refresh();
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview.test_eval_number(
            "(() => { const e = [...document.querySelectorAll('p')].find(e => e.textContent.startsWith('editando Párrafo 80')); if (!e) return -1; const r = e.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight ? scrollY : -1; })()",
            scroll.clone(),
        );
        pump_until(|| scroll.get().is_some());
        assert!(
            scroll.get().unwrap() > 900.0,
            "Preview must follow the edited block"
        );
        let edit_scroll = scroll.get().unwrap();
        // Seguir escribiendo en un bloque visible no produce saltos.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(160, 9), "x");
        cmds.tabs
            .borrow_mut()
            .set_selection_active(Selection::collapsed(Position::new(160, 10)));
        cmds.schedule_refresh();
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview
            .test_eval_number("window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        assert!((scroll.get().unwrap() - edit_scroll).abs() < 2.0);
        // Otro documento no hereda el scroll de este.
        cmds.select_tab(2);
        settle_preview(&cmds);
        scroll.set(None);
        cmds.preview
            .test_eval_number("window.scrollY", scroll.clone());
        pump_until(|| scroll.get().is_some());
        assert!(scroll.get().unwrap() < 50.0);
        // Flechas más allá de las filas visibles: Quick Open y las listas
        // de documento deben desplazar la selección en ambos sentidos.
        for document_list in [false, true] {
            let palette = QuickOpen::new();
            palette.set_transient_for(&cmds.window.clone().upcast());
            if document_list {
                palette.set_title("Backlinks");
            }
            palette.set_entries(
                (0..1000)
                    .map(|n| minmark_fs::fuzzy::Entry {
                        label: format!("Nota {n:04}"),
                        rel_path: format!("nota-{n:04}.md"),
                        line: None,
                        heading: None,
                        score: 0,
                    })
                    .collect(),
            );
            palette.present();
            pump_until(|| palette.test_selected_visible(0));
            for _ in 0..30 {
                palette.test_key(gtk4::gdk::Key::Down);
            }
            pump_until(|| palette.test_selected_visible(30));
            for _ in 0..30 {
                palette.test_key(gtk4::gdk::Key::Up);
            }
            pump_until(|| palette.test_selected_visible(0));
            palette.hide();
        }
        // El popup de autocompletado también conserva visible su selección
        // mientras el teclado permanece en el editor.
        cmds.editor_pane.show_completions(
            (0..50)
                .map(|n| intelligence::Completion {
                    span: minmark_document::Span { start: 0, end: 0 },
                    text: format!("nota-{n:04}.md"),
                    label: format!("Nota {n:04}"),
                })
                .collect(),
        );
        pump_until(|| cmds.editor_pane.test_completion_selected_visible(0));
        for _ in 0..30 {
            assert!(cmds
                .editor_pane
                .emit_test_key(gtk4::gdk::Key::Down, gtk4::gdk::ModifierType::empty()));
        }
        pump_until(|| cmds.editor_pane.test_completion_selected_visible(30));
        for _ in 0..30 {
            assert!(cmds
                .editor_pane
                .emit_test_key(gtk4::gdk::Key::Up, gtk4::gdk::ModifierType::empty()));
        }
        pump_until(|| cmds.editor_pane.test_completion_selected_visible(0));
        cmds.editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty());
        // Backlinks reales de 1000 documentos: ni la consulta ni la lista
        // deben heredar el límite de 50 sugerencias del autocompletado.
        for n in 0..1000 {
            std::fs::write(root.join(format!("nota-{n:04}.md")), "[Destino](b.md)\n").unwrap();
        }
        spawn_index_refresh(root.clone(), cmds.slot.clone());
        pump_until(|| {
            cmds.index
                .borrow()
                .as_ref()
                .is_some_and(|index| index.len() >= 1002)
        });
        cmds.show_navigation(true);
        pump_until(|| cmds.navigation_positions.borrow().len() == 1000);
        assert_eq!(cmds.navigation_list.test_result_count(), 1000);
        pump_until(|| cmds.navigation_list.test_selected_visible(0));
        for _ in 0..75 {
            cmds.navigation_list.test_key(gtk4::gdk::Key::Down);
        }
        pump_until(|| cmds.navigation_list.test_selected_visible(75));
        cmds.navigation_list.test_filter("nota-0999");
        assert_eq!(cmds.navigation_list.test_result_count(), 1);
        cmds.navigation_list.test_activate();
        assert_eq!(
            cmds.tabs.borrow().active().path.as_ref(),
            Some(&root.join("nota-0999.md"))
        );
        cmds.window.destroy();
        academic_workflow(&app, &root.join("academic"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Evalúa `script` en la WebView y devuelve el número observado.
    fn eval_js(pane: &PreviewPane, script: &str) -> f64 {
        // WebView2 crea el controlador de forma asíncrona. La ventana puede
        // estar visible antes de que llegue el primer HTML de la composición.
        pump_until(|| pane.test_has_loaded_document());
        let result = Rc::new(Cell::new(None));
        pane.test_eval_number(script, result.clone());
        pump_until(|| result.get().is_some());
        result.get().unwrap()
    }

    /// Repite la evaluación hasta observar `esperado`; con plazo, para que un
    /// fallo real acabe en timeout descriptivo y no en un bucle infinito.
    fn espera_js(pane: &PreviewPane, script: &str, esperado: f64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            let valor = eval_js(pane, script);
            if valor == esperado {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timeout GTK esperando {esperado} de: {script} (último {valor})"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn nombres_de(cmds: &AppCmds) -> Vec<String> {
        cmds.compositions
            .borrow()
            .iter()
            .map(|comp| comp.name.clone())
            .collect()
    }

    /// Estado `enabled` de las acciones compositivas del grupo «win»
    /// instalado en la ventana (B1): acción a acción — probar «al menos una»
    /// no acredita que las dos sigan a la lista. Pasa por la referencia
    /// débil de `AppCmds`, que es la vía real de [`AppCmds::refresh_compositions`].
    fn acciones_compositivas(cmds: &AppCmds) -> [bool; 2] {
        let grupo = cmds
            .action_group
            .borrow()
            .as_ref()
            .and_then(|grupo| grupo.upgrade())
            .expect("grupo «win» vivo mientras la ventana existe (retención débil)");
        ["preview-composition", "export-composition-pdf"].map(|id| {
            grupo
                .lookup_action(id)
                .unwrap_or_else(|| panic!("falta la acción {id}"))
                .is_enabled()
        })
    }

    /// B1 (revisión PR #44, #22): el watcher redescubre las composiciones
    /// cuando cambia `.mdproject.toml` en caliente — la ventana no se queda
    /// congelada con la lista de apertura — y `compositions_available`
    /// (origen del `enabled` de las acciones §20) queda acorde: la nueva
    /// composición aparece, quitarlas deshabilita y volver a declararlas
    /// habilita.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn composiciones_se_redescubren_al_cambiar_mdproject_en_caliente() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let root =
            std::env::temp_dir().join(format!("minmark-gtk-redescubre-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("uno.md"), "# Uno\n").unwrap();
        std::fs::write(root.join("dos.md"), "# Dos\n").unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\n",
        )
        .unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.redescubre")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let uno = root.join("uno.md");
        let cmds = build_window(
            &app,
            WindowScope::Project(root.clone()),
            std::slice::from_ref(&uno),
        );
        assert_eq!(nombres_de(&cmds), ["A"], "descubrimiento al abrir");
        assert!(cmds.compositions_available());
        assert_eq!(
            acciones_compositivas(&cmds),
            [true, true],
            "instalación: las dos habilitadas (§20)"
        );

        // Añade una composición en caliente: la ventana la redescubre.
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\n[[composition]]\nname = \"B\"\nfiles = [\"dos.md\"]\n",
        )
        .unwrap();
        pump_until(|| nombres_de(&cmds).iter().any(|n| n == "B"));
        assert_eq!(
            acciones_compositivas(&cmds),
            [true, true],
            "siguen habilitadas con la lista ampliada"
        );

        // Quita todas: `compositions_available` habilita/deshabilita acorde.
        std::fs::write(root.join(".mdproject.toml"), "pagination = false\n").unwrap();
        pump_until(|| !cmds.compositions_available());
        assert_eq!(
            acciones_compositivas(&cmds),
            [false, false],
            "B1: al vaciar la lista, las DOS acciones deshabilitadas"
        );

        // Y al volver a declararlas se habilitan de nuevo.
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\n",
        )
        .unwrap();
        pump_until(|| cmds.compositions_available());
        assert_eq!(nombres_de(&cmds), ["A"]);
        assert_eq!(
            acciones_compositivas(&cmds),
            [true, true],
            "B1: al volver la lista, las DOS acciones habilitadas"
        );

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(root);
    }

    /// S1/ISSUE-024 (revisión PR #44, aceptación de #24): en la ventana del
    /// Preview combinado, pulsar un enlace `#fragmento` (incluidas las
    /// referencias cruzadas resueltas por `compose`) hace scroll in-page
    /// hasta su elemento (§26) sin salir del documento.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn preview_combinado_navega_destinos_internos_con_scroll_in_page() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let root = std::env::temp_dir().join(format!(
            "minmark-gtk-destino-interno-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\", \"dos.md\"]\n",
        )
        .unwrap();
        // El destino queda muy por debajo del pliegue del viewport: sin el
        // scroll in-page no puede entrar en pantalla.
        let mut uno = String::from("[Ir al destino](#destino)\n\n");
        for n in 0..150 {
            uno.push_str(&format!("Párrafo {n}.\n\n"));
        }
        std::fs::write(root.join("uno.md"), uno).unwrap();
        std::fs::write(root.join("dos.md"), "## Destino {#destino}\n").unwrap();
        let app = Application::builder()
            .application_id("app.minmark.test.destino-interno")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let uno_path = root.join("uno.md");
        let cmds = build_window(
            &app,
            WindowScope::Project(root.clone()),
            std::slice::from_ref(&uno_path),
        );
        cmds.preview_composition();
        let pane = {
            let preview = cmds.composition_preview.borrow();
            Rc::clone(
                &preview
                    .as_ref()
                    .expect("ventana del Preview combinado")
                    .pane,
            )
        };
        const EN_VISIBLE: &str = "(() => { const e = document.getElementById('destino'); if (!e) return -1; const r = e.getBoundingClientRect(); return (r.top >= 0 && r.top < innerHeight) ? 1 : 0; })()";
        // El HTML combinado llega con el enlace y con su ancla.
        espera_js(
            &pane,
            "document.querySelector('a[href=\\\"#destino\\\"]') ? 1 : 0",
            1.0,
        );
        assert_eq!(
            eval_js(&pane, "document.getElementById('destino') ? 1 : 0"),
            1.0
        );
        // Antes de pulsar, el destino está fuera del viewport.
        assert_eq!(
            eval_js(&pane, EN_VISIBLE),
            0.0,
            "el destino ya era visible: el test no probaría nada"
        );
        // Pulsar el enlace interno desplaza hasta el elemento.
        assert_eq!(
            eval_js(
                &pane,
                "(() => { const a = document.querySelector('a[href=\\\"#destino\\\"]'); if (!a) return 0; a.click(); return 1; })()"
            ),
            1.0
        );
        espera_js(&pane, EN_VISIBLE, 1.0);

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(root);
    }

    fn academic_workflow(app: &Application, root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        let paper = root.join("academic.md");
        let source = include_str!("../../docs/qa/fixtures/academic.md");
        std::fs::write(&paper, source).unwrap();
        std::fs::write(
            root.join("references.bib"),
            include_str!("../../docs/qa/fixtures/references.bib"),
        )
        .unwrap();
        std::fs::write(
            root.join("extra.bib"),
            include_str!("../../docs/qa/fixtures/extra.bib"),
        )
        .unwrap();
        std::fs::write(
            root.join("author-year.csl"),
            include_str!("../../docs/qa/fixtures/author-year.csl"),
        )
        .unwrap();
        let cmds = build_window(
            app,
            WindowScope::Project(root.to_owned()),
            std::slice::from_ref(&paper),
        );
        pump_until(|| cmds.window.is_mapped());
        wait_preview_allocation(&cmds).unwrap();
        assert_eq!(
            preview_number(&cmds, "document.querySelectorAll('[role=math] svg').length"),
            5.0
        );
        assert_eq!(preview_number(&cmds, "(() => { const ids = [...document.querySelectorAll('svg [id]')].map(e => e.id); return new Set(ids).size === ids.length ? 1 : 0; })()"), 1.0);
        assert_eq!(preview_number(&cmds, "[...document.querySelectorAll('[role=math] svg')].every(e => { const b = e.getBBox(), v = e.viewBox.baseVal; return b.y >= -2 && b.y + b.height <= v.height + 2; }) ? 1 : 0"), 1.0);
        assert_eq!(preview_number(&cmds, "(() => { document.documentElement.dataset.theme = 'dark'; const color = getComputedStyle(document.body).color; return [...document.querySelectorAll('[role=math] svg use')].every(e => getComputedStyle(e).fill === color) && [...document.querySelectorAll('[role=math] svg [stroke=\"#000000\"]')].every(e => getComputedStyle(e).stroke === color) ? 1 : 0; })()"), 1.0);
        preview_number(&cmds, "document.documentElement.dataset.theme = 'light'; 1");
        assert_eq!(
            preview_number(
                &cmds,
                "document.querySelectorAll('.mm-footnotes li').length"
            ),
            2.0
        );
        // WebKit puede aplicar scrollIntoView en el siguiente frame. El
        // resultado debe observarse después del clic, sin exigir sincronía.
        preview_number(
            &cmds,
            "document.querySelector('[role=doc-noteref]').click(); 1",
        );
        espera_js(&cmds.preview, "(() => { const r = document.getElementById('mm-note-1').getBoundingClientRect(); return r.top >= 0 && r.top < innerHeight ? 1 : 0; })()", 1.0);
        preview_number(
            &cmds,
            "document.querySelector('.mm-footnotes a').click(); 1",
        );
        espera_js(&cmds.preview, "(() => { const r = document.getElementById('mm-note-ref-1-1').getBoundingClientRect(); return r.top >= 0 && r.top < innerHeight ? 1 : 0; })()", 1.0);
        assert_eq!(
            preview_number(&cmds, "document.querySelectorAll('.mm-toc a').length"),
            4.0
        );
        assert_eq!(preview_number(&cmds, "document.body.textContent.includes('Ecuación 2.1') && document.body.textContent.includes('Energy Research') ? 1 : 0"), 1.0);
        assert!(!cmds.tabs.borrow().active().editor.document().is_modified());
        let evidence = std::env::var_os("MINMARK_QA_OUTPUT_DIR").map(PathBuf::from);
        if let Some(evidence) = &evidence {
            std::fs::create_dir_all(evidence).unwrap();
            let done = Rc::new(Cell::new(false));
            cmds.preview
                .test_snapshot(&evidence.join("academic-continuous.png"), done.clone());
            pump_until(|| done.get());
            let settings = cmds.settings.borrow().clone();
            let pdf = export::export_document_at(
                source,
                &settings,
                Some(&paper),
                Some(root),
                &minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse(source)),
            )
            .unwrap();
            std::fs::write(evidence.join("academic.pdf"), pdf).unwrap();
            std::fs::write(
                evidence.join("academic.html"),
                preview::render_document(source, &settings),
            )
            .unwrap();
        }
        // Cambiar un .bib actualiza Preview sin editar ni marcar dirty el Markdown.
        let changed = include_str!("../../docs/qa/fixtures/references.bib")
            .replace("Energy Research", "Energy Research Updated");
        std::fs::write(root.join("references.bib"), changed).unwrap();
        pump_until(|| cmds.refresh_scheduled.get() || cmds.intelligence_busy.get());
        assert_eq!(
            preview_number(
                &cmds,
                "document.body.textContent.includes('Energy Research Updated') ? 1 : 0"
            ),
            1.0
        );
        assert!(!cmds.tabs.borrow().active().editor.document().is_modified());
        // Teclado real del popup: aceptar, cancelar y un único undo/redo.
        let completion_source = "---\nbibliography: references.bib\n---\nβ [@Sm, p. 42]\r\n";
        let original_len = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .len();
        let end = cmds.tabs.borrow().active().editor.position_of(original_len);
        cmds.tabs
            .borrow_mut()
            .replace_active(Range::new(Position::new(0, 0), end), completion_source);
        cmds.reload_active();
        let cursor = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .position_of(completion_source.find(", p.").unwrap());
        cmds.editor_pane
            .focus_find_match(Range::new(cursor, cursor));
        *cmds.completion_source.borrow_mut() = None;
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Tab, gtk4::gdk::ModifierType::empty()));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            completion_source.replace("@Sm", "@smith2025")
        );
        cmds.undo();
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            completion_source
        );
        *cmds.completion_source.borrow_mut() = None;
        cmds.refresh_intelligence(true);
        pump_until(|| cmds.completion_source.borrow().is_some());
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty()));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            completion_source
        );
        cmds.redo();
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .contains("@smith2025, p. 42"));
        cmds.toggle_mode();
        cmds.toggle_mode();
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .ends_with("\r\n"));
        // CSL alternativo, notas y TOC comparten la composición paginada/PDF.
        let paginated = source.replace(
            "language: es",
            "language: es\npagination: true\nbibliography_style: author-year.csl",
        );
        let len = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .len();
        let end = cmds.tabs.borrow().active().editor.position_of(len);
        cmds.tabs
            .borrow_mut()
            .replace_active(Range::new(Position::new(0, 0), end), &paginated);
        cmds.reload_active();
        assert!(preview_number(&cmds, "document.querySelectorAll('.mm-paged-page').length") >= 2.0);
        if let Some(evidence) = &evidence {
            let done = Rc::new(Cell::new(false));
            cmds.preview
                .test_snapshot(&evidence.join("academic-paginated.png"), done.clone());
            pump_until(|| done.get());
            let pdf = export::export_document_at(
                &paginated,
                &cmds.settings.borrow(),
                Some(&paper),
                Some(root),
                &minmark_pdf::DocumentMeta::from_ast(&minmark_parser::parse(&paginated)),
            )
            .unwrap();
            std::fs::write(evidence.join("academic-author-year.pdf"), pdf).unwrap();
        }
        assert_eq!(std::fs::read_to_string(&paper).unwrap(), source);
        cmds.window.destroy();
    }

    /// ISSUE-52 (§37, §3.7): la «X» de la barra cierra esa pestaña concreta
    /// sin tocar la activa; con cambios sin guardar pide la misma confirmación
    /// que Close Tab (Cancelar no descarta nada); cerrar la activa pasa a la
    /// siguiente y siempre queda ≥1 pestaña.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_tab_close_buttons_cierran_pestana_concreta() {
        fn tab_close_buttons(window: &Widget) -> Vec<gtk4::Button> {
            fn walk(widget: &Widget, found: &mut Vec<gtk4::Button>) {
                if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
                    if button.has_css_class("mm-tab-close") {
                        found.push(button.clone());
                    }
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    walk(&widget, found);
                    child = widget.next_sibling();
                }
            }
            let mut found = Vec::new();
            walk(window, &mut found);
            found
        }
        fn find_button(widget: &Widget, label: &str) -> Option<gtk4::Button> {
            if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
                if button.label().as_deref() == Some(label) {
                    return Some(button.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(button) = find_button(&widget, label) {
                    return Some(button);
                }
                child = widget.next_sibling();
            }
            None
        }
        fn dialog_button(label: &str) -> Option<gtk4::Button> {
            gtk4::Window::list_toplevels()
                .iter()
                .find_map(|window| find_button(window, label))
        }
        /// Ventana del diálogo que contiene el botón `label` (el diálogo es
        /// otra ventana que [`AppCmds::window`]).
        fn dialog_window(label: &str) -> Option<gtk4::Window> {
            gtk4::Window::list_toplevels()
                .into_iter()
                .find(|window| find_button(window, label).is_some())
                .and_then(|window| window.downcast::<gtk4::Window>().ok())
        }
        /// Captura una ventana real para la evidencia visual (opcional:
        /// `MINMARK_QA_OUTPUT_DIR`).
        fn capture(window: &gtk4::Window, name: &str) {
            let Some(evidence) = std::env::var_os("MINMARK_QA_OUTPUT_DIR").map(PathBuf::from)
            else {
                return;
            };
            std::fs::create_dir_all(&evidence).unwrap();
            capture_window(window, &evidence.join(name));
        }

        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let root = std::env::temp_dir().join(format!("minmark-gtk-close-x-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let paths: Vec<PathBuf> = ["a.md", "b.md", "c.md", "d.md"]
            .iter()
            .map(|name| {
                std::fs::write(root.join(name), format!("# {name}\n")).unwrap();
                root.join(name)
            })
            .collect();
        let app = Application::builder()
            .application_id("app.minmark.test.tab-close-x")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Project(root.clone()), &paths);
        pump_until(|| cmds.index.borrow().is_some());
        cmds.window.set_default_size(900, 600);
        settle_preview(&cmds);
        pump_until(|| cmds.window.is_mapped());

        // La barra muestra una «X» por pestaña (la inicial «Sin título 1» y
        // los cuatro archivos), con tooltip que identifica la pestaña y área
        // de clic fijada por el CSS (`mm-tab-close`).
        assert_eq!(cmds.tabs.borrow().len(), 5);
        let closes = tab_close_buttons(cmds.window.upcast_ref());
        assert_eq!(closes.len(), 5, "una «X» por pestaña (ISSUE-52)");
        assert_eq!(closes[4].tooltip_text().as_deref(), Some("Cerrar «d.md»"));
        assert!(
            closes.iter().all(|close| close.can_focus()),
            "cada «X» es alcanzable con el foco por teclado"
        );
        capture(cmds.window.upcast_ref(), "issue52-00-barra-con-cierre.png");

        // Cerrar una pestaña INACTIVA: el tab activo (d.md) no cambia.
        assert_eq!(cmds.tabs.borrow().active_index(), 4);
        closes[2].emit_clicked(); // b.md
        pump_until(|| cmds.tabs.borrow().len() == 4);
        assert_eq!(cmds.tabs.borrow().active_index(), 3);
        assert_eq!(
            cmds.tabs.borrow().active().path.as_deref(),
            Some(paths[3].as_path()),
            "sigue activo el mismo tab"
        );
        capture(
            cmds.window.upcast_ref(),
            "issue52-01-cerrada-la-inactiva.png",
        );

        // Pestaña inactiva con cambios sin guardar: la misma confirmación que
        // Close Tab (§3.7); Cancelar no descarta nada.
        cmds.tabs.borrow_mut().set_active(1); // a.md
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "sin guardar ");
        cmds.reload_active();
        cmds.tabs.borrow_mut().set_active(3); // d.md
        cmds.reload_active();
        tab_close_buttons(cmds.window.upcast_ref())[1].emit_clicked();
        pump_until(|| dialog_button("Descartar").is_some());
        let dialog = dialog_window("Descartar").expect("diálogo de descarte");
        capture(&dialog, "issue52-02-confirmacion-descarte.png");
        dialog_button("Cancelar").unwrap().emit_clicked();
        pump_until(|| dialog_button("Cancelar").is_none());
        assert_eq!(cmds.tabs.borrow().len(), 4, "Cancelar conserva la pestaña");
        assert_eq!(cmds.tabs.borrow().active_index(), 3, "y el tab activo");
        assert!(
            cmds.tabs.borrow().is_modified(1),
            "y sus cambios sin guardar"
        );

        // «Descartar» confirmado cierra esa pestaña concreta.
        tab_close_buttons(cmds.window.upcast_ref())[1].emit_clicked();
        pump_until(|| dialog_button("Descartar").is_some());
        dialog_button("Descartar").unwrap().emit_clicked();
        pump_until(|| cmds.tabs.borrow().len() == 3);
        assert_eq!(cmds.tabs.borrow().active_index(), 2);
        assert_eq!(
            cmds.tabs.borrow().active().path.as_deref(),
            Some(paths[3].as_path())
        );

        // Cerrar la ACTIVA pasa a la siguiente (§37).
        cmds.tabs.borrow_mut().set_active(1); // c.md
        cmds.reload_active();
        tab_close_buttons(cmds.window.upcast_ref())[1].emit_clicked();
        pump_until(|| cmds.tabs.borrow().len() == 2);
        assert_eq!(cmds.tabs.borrow().active_index(), 1);
        assert_eq!(
            cmds.tabs.borrow().active().path.as_deref(),
            Some(paths[3].as_path()),
            "cerrar la activa pasa a la siguiente"
        );

        // Al quedar una sola pestaña la barra se oculta (§37) y el cierre de
        // la última crea un «Sin título N» nuevo: siempre queda ≥1 (§37).
        tab_close_buttons(cmds.window.upcast_ref())[0].emit_clicked(); // «Sin título 1»
        pump_until(|| cmds.tabs.borrow().len() == 1);
        assert_eq!(cmds.tabs.borrow().active_index(), 0);
        assert_eq!(
            cmds.tabs.borrow().active().path.as_deref(),
            Some(paths[3].as_path())
        );
        assert!(
            !cmds.tab_bar.widget().is_visible(),
            "barra oculta con un solo documento (§37)"
        );
        cmds.close_tab_at(0);
        assert_eq!(cmds.tabs.borrow().len(), 1, "invariante §37: ≥1 pestaña");
        assert_eq!(cmds.tabs.borrow().title(0), "Sin título 2");

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ISSUE-57: la Preview debe pintar el documento y seguir las ediciones en
    /// los tres backends; ver [`qa_preview_smoke`].
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_preview_pinta_y_actualiza_al_escribir() {
        qa_preview_smoke();
    }

    // ── Fase 7 #29: controles editoriales contextuales (§44) ─────────────

    /// #29: `win.format-strong` — el MISMO camino de menú, atajos, paleta y
    /// popover — aplica Bold como UNA transacción (§8.5) y registra la
    /// selección §8.6: undo restaura el texto y la selección previos, redo
    /// los posteriores, y la acción entera es UN solo paso de undo.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_format_bold_via_gaction_conserva_seleccion_y_un_solo_undo() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-bold")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "hola mundo");
        cmds.reload_active();
        // Selección «mundo»: mark_set → EditSink::set_selection → modelo.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 5), Position::new(0, 10)));
        let original = Selection {
            anchor: Position::new(0, 5),
            head: Position::new(0, 10),
        };
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), original);

        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.format-strong", None)
            .expect("win.format-strong registrada (§36)");

        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "hola **mundo**"
        );
        let interior = Selection {
            anchor: Position::new(0, 7),
            head: Position::new(0, 12),
        };
        assert_eq!(
            cmds.tabs.borrow().active().editor.selection(),
            interior,
            "§8.6: la selección posterior es el texto interior"
        );

        // UN solo undo revierte la acción entera y restaura la selección.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "hola mundo"
        );
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), original);
        // El siguiente undo es el insert inicial: Bold fue UN paso (§8.5).
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");
        assert!(!cmds.tabs.borrow().active().editor.can_undo());
        // Redo ×2: primero el insert y después Bold con su selección.
        assert!(cmds.tabs.borrow_mut().redo_active());
        assert!(cmds.tabs.borrow_mut().redo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "hola **mundo**"
        );
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), interior);

        cmds.window.destroy();
    }

    /// #29: `win.insert-link` abre el diálogo de URL (prellenado con la del
    /// enlace si el cursor está dentro); Cancelar deja el buffer byte a byte
    /// idéntico y no añade transacción — «cero cambios» (§UI).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_dialogo_link_cancelar_no_toca_el_buffer() {
        fn find_entry(widget: &Widget) -> Option<gtk4::Entry> {
            if let Some(entry) = widget.downcast_ref::<gtk4::Entry>() {
                return Some(entry.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(entry) = find_entry(&widget) {
                    return Some(entry);
                }
                child = widget.next_sibling();
            }
            None
        }
        fn find_button(widget: &Widget, label: &str) -> Option<gtk4::Button> {
            if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
                if button.label().as_deref() == Some(label) {
                    return Some(button.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(button) = find_button(&widget, label) {
                    return Some(button);
                }
                child = widget.next_sibling();
            }
            None
        }
        /// Solo el diálogo «Link» (la ventana principal también tiene Entries).
        fn link_dialog() -> Option<Widget> {
            gtk4::Window::list_toplevels()
                .into_iter()
                .find(|widget| {
                    widget
                        .downcast_ref::<gtk4::Window>()
                        .is_some_and(|window| window.title().as_deref() == Some("Link"))
                })
        }
        /// Pulsación sintética de Escape sobre los controllers del diálogo
        /// (el controlador de Escape vive en la ventana del prompt).
        fn emit_escape(widget: &Widget) {
            let key = gtk4::gdk::Key::Escape;
            let modifiers = gtk4::gdk::ModifierType::empty();
            let controllers = widget.observe_controllers();
            for i in 0..controllers.n_items() {
                if let Some(controller) = controllers
                    .item(i)
                    .and_then(|c| c.downcast::<gtk4::EventControllerKey>().ok())
                {
                    if controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
                    {
                        break;
                    }
                }
            }
        }

        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-link")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "hola mundo");
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 5), Position::new(0, 10)));
        let contenido = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        let seleccion = cmds.tabs.borrow().active().editor.selection();

        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-link", None)
            .expect("win.insert-link registrada (§36)");
        pump_until(|| link_dialog().is_some());
        // Sin enlace bajo el cursor la precarga queda vacía (§UI).
        assert_eq!(
            find_entry(&link_dialog().unwrap()).expect("entry del diálogo").text(),
            ""
        );

        find_button(&link_dialog().unwrap(), "Cancelar")
            .expect("botón Cancelar")
            .emit_clicked();
        pump_until(|| link_dialog().is_none());

        let despues = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert_eq!(despues.as_bytes(), contenido.as_bytes(), "byte a byte");
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), seleccion);

        // Escape cierra el prompt con el MISMO resultado que Cancelar
        // (#29 revisión): cero cambios, byte a byte.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-link", None)
            .expect("win.insert-link reabre el diálogo");
        pump_until(|| link_dialog().is_some());
        emit_escape(&link_dialog().expect("diálogo Link"));
        pump_until(|| link_dialog().is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            contenido.as_bytes(),
            "Escape deja el buffer byte a byte idéntico"
        );
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), seleccion);

        // Cero cambios: el ÚNICO undo sigue siendo el insert inicial.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            ""
        );
        assert!(
            !cmds.tabs.borrow().active().editor.can_undo(),
            "ni Cancelar ni Escape añadieron transacción"
        );

        cmds.window.destroy();
    }

    /// #29: el popover solo aparece si aporta utilidad — oculto con cursor
    /// plano sin selección, panel de texto visible con selección no vacía,
    /// H1–H6 bajo el cursor en un heading — y Escape lo cierra con cero
    /// cambios en el documento (§UI).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_controles_contextuales_oculto_cursor_plano_y_visible_con_seleccion() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-popover")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "hola mundo\n\n# Título\n");
        cmds.reload_active();

        // Cursor plano en texto llano, sin selección: oculto (§UI).
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 2), Position::new(0, 2)));
        assert!(!cmds.editor_pane.test_context_visible());

        // Selección no vacía: panel de texto con los labels de Command §36.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 0), Position::new(0, 4)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        for label in [
            "Heading 1",
            "Heading 6",
            "Bold",
            "Italic",
            "Strikethrough",
            "Code",
            "Link",
            "Callout",
        ] {
            assert!(
                cmds.editor_pane.test_context_has_button(label),
                "falta el botón «{label}» en el panel de texto"
            );
        }

        // Teclado #29 (revisión): el popover y sus botones admiten foco
        // (Tab los recorre, Enter/Space los activan — GTK nativo) y `Ctrl+.`
        // lleva el foco del editor a los controles.
        assert!(
            cmds.editor_pane.test_context_can_focus(),
            "el popover debe admitir foco"
        );
        let bold = cmds
            .editor_pane
            .test_context_button("Bold")
            .expect("botón Bold");
        assert!(bold.can_focus(), "Bold debe ser operable con teclado");
        assert!(
            !cmds.editor_pane.test_context_focus_inside(),
            "al abrir, el foco inicial se queda en el editor"
        );
        assert!(
            cmds.editor_pane.emit_test_key(
                gtk4::gdk::Key::period,
                gtk4::gdk::ModifierType::CONTROL_MASK
            ),
            "Ctrl+. lo maneja el controller de la vista"
        );
        assert!(
            cmds.editor_pane.test_context_focus_inside(),
            "Ctrl+. mueve el foco a los controles"
        );

        // Escape cierra sin tocar el documento (§UI «cero cambios»).
        let contenido = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert!(cmds
            .editor_pane
            .emit_test_key(gtk4::gdk::Key::Escape, gtk4::gdk::ModifierType::empty()));
        assert!(!cmds.editor_pane.test_context_visible());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            contenido
        );

        // Cursor en un heading (selección vacía): solo H1–H6.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(2, 1), Position::new(2, 1)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(cmds.editor_pane.test_context_has_button("Heading 3"));

        // Volver al cursor plano cierra el popover.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 2), Position::new(0, 2)));
        pump_until(|| !cmds.editor_pane.test_context_visible());

        cmds.window.destroy();
    }

    /// #29 (aceptación): cambiar de pestaña o de modo con el popover abierto
    /// lo cierra, y el handler del popover no aplica nada — ni a la pestaña
    /// activa ni a la que se abandonó.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_cambio_de_pestana_con_popover_abierto_no_aplica_nada() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-tabguard")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "uno seleccionado\n");
        cmds.reload_active();
        // Segunda pestaña con contenido propio (new_tab la activa).
        cmds.tabs.borrow_mut().new_tab();
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "dos\n");
        cmds.reload_active();

        // Pestaña 0 con selección: el popover se abre.
        cmds.select_tab(0);
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 0), Position::new(0, 3)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(cmds.editor_pane.test_context_has_button("Bold"));

        // Cambiar de pestaña cierra el popover (load → hide, §UI).
        cmds.select_tab(1);
        assert!(!cmds.editor_pane.test_context_visible());

        // El click YA no aplica nada: con el cursor colapsado en texto
        // plano Bold insertaría `****` en la pestaña activa, y la 0
        // conservaría su selección.
        assert!(cmds.editor_pane.test_context_click("Bold"));
        assert_eq!(
            cmds.tabs.borrow().tabs()[1].editor.document().content(),
            "dos\n",
            "sin cambios en la pestaña activa"
        );
        assert_eq!(
            cmds.tabs.borrow().tabs()[0].editor.document().content(),
            "uno seleccionado\n",
            "sin cambios en la pestaña abandonada"
        );

        // Abierto de nuevo y cambio de MODO: también se cierra y no aplica.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 0), Position::new(0, 3)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        cmds.toggle_mode();
        assert!(!cmds.editor_pane.test_context_visible(), "cambio de modo cierra");
        assert!(cmds.editor_pane.test_context_click("Heading 1"));
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "dos\n"
        );

        cmds.window.destroy();
    }

    /// #29: el cursor dentro de un callout muestra los controles de tono
    /// (§22) con el tono actual activo, y retonar pasa por la MISMA acción
    /// `win.insert-callout` — el botón fija el estado y activa la acción —
    /// como UNA transacción por acción (§8.5).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_retonado_de_callout_desde_el_popover() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-callout")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "aviso importante\n");
        cmds.reload_active();

        // Insert ▸ Callout (acción sin parámetro): tono inicial «note».
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 0), Position::new(0, 5)));
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-callout", None)
            .expect("win.insert-callout registrada (§36)");
        let envuelto = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert!(
            envuelto.starts_with("::: note\n"),
            "envuelto con el tono inicial: {envuelto:?}"
        );

        // Cursor dentro del callout: fila de tonos con «note» activo.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(1, 3), Position::new(1, 3)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert_eq!(
            cmds.editor_pane.test_context_tone().as_deref(),
            Some("note"),
            "el estado activo refleja el tono real (§22)"
        );

        // Retonar a «warning»: el botón fija el estado de la MISMA acción
        // `win.insert-callout` y la activa.
        assert!(cmds.editor_pane.test_context_click("warning"));
        let retonado = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert!(
            retonado.starts_with("::: warning\n"),
            "retone conservando título y contenido: {retonado:?}"
        );
        assert!(retonado.contains("aviso importante"));
        // Cada acción es UNA transacción: undo vuelve al «note».
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert!(cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .starts_with("::: note\n"));

        cmds.window.destroy();
    }

    /// §38 + revisión #29: la ✕ de la barra de título OCULTA la paleta
    /// (no la destruye, patrón Quick Open §41) y la siguiente reapertura
    /// sigue ejecutando comandos.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_command_palette_sobrevive_close_request_y_sigue_activando() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-palette-close")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "hola mundo");
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 5), Position::new(0, 10)));

        cmds.command_palette.present();
        assert!(cmds.command_palette.is_visible());
        // Close-request (✕ del título): la ventana se OCULTA, no se
        // destruye — el próximo present() no verá un objeto muerto.
        cmds.command_palette.test_close();
        assert!(
            !cmds.command_palette.is_visible(),
            "close-request ⇒ oculta (no destruida)"
        );
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "hola mundo",
            "cerrar la paleta no toca el documento"
        );

        // Reutilizable: reabrir y ejecutar un comando vía win.*.
        cmds.command_palette.present();
        assert!(cmds.command_palette.is_visible());
        cmds.command_palette.test_filter("bold");
        assert_eq!(cmds.command_palette.test_result_count(), 1);
        cmds.command_palette.test_activate();
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "hola **mundo**"
        );

        cmds.window.destroy();
    }

    /// §38: la Command Palette fuzzy lista los comandos y aceptar activa la
    /// MISMA acción `win.*` del camino de menú y atajos.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_command_palette_ejecuta_comando_via_gaction() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase7-palette")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "hola mundo");
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 5), Position::new(0, 10)));

        cmds.command_palette.present();
        assert!(cmds.command_palette.is_visible());
        cmds.command_palette.test_filter("bold");
        assert_eq!(cmds.command_palette.test_result_count(), 1);
        assert_eq!(
            cmds.command_palette.test_selected_label().as_deref(),
            Some("Bold"),
            "el match exacto va primero"
        );

        cmds.command_palette.test_activate();
        assert!(!cmds.command_palette.is_visible(), "aceptar cierra §38");
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "hola **mundo**"
        );

        cmds.window.destroy();
    }

    // ── Fase 7 #30: imágenes y tablas (§18, §26.3, §35) ─────────────────

    /// Diálogo toplevel por título (#30: los formularios se titulan con el
    /// label de su `Command`).
    fn dialog_titled(title: &str) -> Option<Widget> {
        gtk4::Window::list_toplevels().into_iter().find(|widget| {
            widget
                .downcast_ref::<gtk4::Window>()
                .is_some_and(|window| window.title().as_deref() == Some(title))
        })
    }

    /// Entries de `widget` en orden de aparición (campos del formulario).
    fn collect_entries(widget: &Widget, out: &mut Vec<gtk4::Entry>) {
        if let Some(entry) = widget.downcast_ref::<gtk4::Entry>() {
            out.push(entry.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            collect_entries(&widget, out);
            child = widget.next_sibling();
        }
    }

    /// TextViews de `widget` en orden de aparición (campo multilinea #31).
    fn collect_text_views(widget: &Widget, out: &mut Vec<gtk4::TextView>) {
        if let Some(view) = widget.downcast_ref::<gtk4::TextView>() {
            out.push(view.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            collect_text_views(&widget, out);
            child = widget.next_sibling();
        }
    }

    /// Texto completo de un TextView (campo LaTeX del diálogo #31).
    fn text_view_text(view: &gtk4::TextView) -> String {
        let buffer = view.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    /// Etiqueta cuyo texto contiene `needle` (avisos ADR-002 y errores #31).
    fn find_label_containing(widget: &Widget, needle: &str) -> Option<gtk4::Label> {
        if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
            if label.text().contains(needle) {
                return Some(label.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(label) = find_label_containing(&widget, needle) {
                return Some(label);
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Pulsación sintética sobre los controllers de `widget` (patrón del
    /// Escape de #30); devuelve `true` si algún controller la consumió.
    fn emit_key(
        widget: &Widget,
        key: gtk4::gdk::Key,
        modifiers: gtk4::gdk::ModifierType,
    ) -> bool {
        let controllers = widget.observe_controllers();
        for i in 0..controllers.n_items() {
            if let Some(controller) = controllers
                .item(i)
                .and_then(|c| c.downcast::<gtk4::EventControllerKey>().ok())
            {
                if controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]) {
                    return true;
                }
            }
        }
        false
    }

    /// Popover de completado (#31) visible bajo `widget`: GTK mantiene el
    /// popover en la lista de hijos de su entrada.
    fn visible_popover_in(widget: &Widget) -> Option<gtk4::Popover> {
        if let Some(popover) = widget.downcast_ref::<gtk4::Popover>() {
            if popover.is_visible() {
                return Some(popover.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = visible_popover_in(&widget) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Etiquetas (sugerencias §25.3) dentro del popover de completado.
    fn popover_labels(popover: &gtk4::Popover) -> Vec<String> {
        fn walk(widget: &Widget, out: &mut Vec<String>) {
            if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
                out.push(label.text().to_string());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                walk(&widget, out);
                child = widget.next_sibling();
            }
        }
        let mut labels = Vec::new();
        walk(popover.upcast_ref::<gtk4::Widget>(), &mut labels);
        labels
    }

    fn dialog_entries(dialog: &Widget) -> Vec<gtk4::Entry> {
        let mut entries = Vec::new();
        collect_entries(dialog, &mut entries);
        entries
    }

    /// TextViews del diálogo (#31: el campo LaTeX multilinea).
    fn dialog_text_views(dialog: &Widget) -> Vec<gtk4::TextView> {
        let mut views = Vec::new();
        collect_text_views(dialog, &mut views);
        views
    }

    /// Botón por etiqueta dentro del árbol (patrón de los tests #29).
    fn find_button_in(widget: &Widget, label: &str) -> Option<gtk4::Button> {
        if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
            if button.label().as_deref() == Some(label) {
                return Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(button) = find_button_in(&widget, label) {
                return Some(button);
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Etiqueta con texto exacto (errores de validación retenidos, §UI).
    fn find_label_text(widget: &Widget, text: &str) -> Option<gtk4::Label> {
        if let Some(label) = widget.downcast_ref::<gtk4::Label>() {
            if label.text() == text {
                return Some(label.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(label) = find_label_text(&widget, text) {
                return Some(label);
            }
            child = widget.next_sibling();
        }
        None
    }

    /// Pulsación sintética de Escape sobre los controllers de `widget` (el
    /// controlador de Escape vive en la ventana del formulario).
    fn emit_escape_key(widget: &Widget) {
        let key = gtk4::gdk::Key::Escape;
        let modifiers = gtk4::gdk::ModifierType::empty();
        let controllers = widget.observe_controllers();
        for i in 0..controllers.n_items() {
            if let Some(controller) = controllers
                .item(i)
                .and_then(|c| c.downcast::<gtk4::EventControllerKey>().ok())
            {
                if controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]) {
                    break;
                }
            }
        }
    }

    /// #30: `win.edit-image` abre el formulario prellenado desde
    /// `format::context` y aplicar cambia SOLO el campo editado — con
    /// atributos (título, caption e id intactos) y sin ellos (añade `{...}`
    /// tras el constructo sin tocar el resto) —; cada edición es UNA
    /// transacción (§8.5).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_edit_image_conserva_los_bytes_ajenos() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase730-edit-image")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);

        // Caso A: imagen CON atributos — editar SOLO el ancho.
        let fuente =
            "Ver ![Diagrama](diagram.png \"Título\") {#fig:d \"La caption\" width=40%} fin.\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 6), Position::new(0, 6)));

        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-image", None)
            .expect("win.edit-image registrada (§36)");
        pump_until(|| dialog_titled("Edit Image").is_some());
        let dialog = dialog_titled("Edit Image").expect("formulario de imagen");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 4, "Ruta, Alt, ID, Ancho % (§UI)");
        assert_eq!(entries[0].text(), "diagram.png");
        assert_eq!(entries[1].text(), "Diagrama");
        assert_eq!(entries[2].text(), "fig:d");
        assert_eq!(entries[3].text(), "40");

        entries[3].set_text("60");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Edit Image").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "Ver ![Diagrama](diagram.png \"Título\") {#fig:d \"La caption\" width=60%} fin.\n",
            "solo cambia width: ruta, alt, título, id y caption intactos"
        );

        // UNA transacción (§8.5): undo restaura byte a byte la fuente.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            fuente
        );
        assert!(cmds.tabs.borrow_mut().undo_active(), "el insert inicial");
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");

        // Caso B: imagen SIN atributos — pedir ID añade `{...}` al final del
        // constructo y no toca un byte del resto.
        let fuente_b = "sin ![alt](a.png) más\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente_b);
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 6), Position::new(0, 6)));
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-image", None)
            .expect("win.edit-image reabre el formulario");
        pump_until(|| dialog_titled("Edit Image").is_some());
        let dialog = dialog_titled("Edit Image").expect("formulario de imagen");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries[0].text(), "a.png");
        assert_eq!(entries[1].text(), "alt");
        assert_eq!(entries[2].text(), "");
        assert_eq!(entries[3].text(), "");
        entries[2].set_text("fig:x");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Edit Image").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "sin ![alt](a.png) {#fig:x} más\n",
            "sin atributos se añade el bloque {{...}} tras el constructo, el resto intacto"
        );

        cmds.window.destroy();
    }

    /// #30 §UI «cero cambios»: Cancelar y Escape en el formulario de imagen
    /// dejan el buffer byte a byte idéntico, la selección intacta y NO
    /// añaden transacción de undo.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_dialogo_edit_image_cancelar_no_toca_el_buffer() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase730-edit-image-cancel")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let fuente = "![alt](img.png \"T\") {#fig:x width=30%}\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 3), Position::new(0, 3)));
        let seleccion = Selection::collapsed(Position::new(0, 3));

        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-image", None)
            .expect("win.edit-image registrada (§36)");
        pump_until(|| dialog_titled("Edit Image").is_some());
        let dialog = dialog_titled("Edit Image").expect("formulario");
        let entries = dialog_entries(&dialog);
        assert_eq!(
            (
                entries[0].text().to_string(),
                entries[1].text().to_string(),
                entries[2].text().to_string(),
                entries[3].text().to_string()
            ),
            ("img.png".to_owned(), "alt".to_owned(), "fig:x".to_owned(), "30".to_owned()),
            "precarga desde format::context (§UI #30)"
        );

        find_button_in(&dialog, "Cancelar")
            .expect("botón Cancelar")
            .emit_clicked();
        pump_until(|| dialog_titled("Edit Image").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            fuente.as_bytes(),
            "Cancelar deja el buffer byte a byte idéntico"
        );
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), seleccion);

        // Escape ⇒ el MISMO resultado que Cancelar.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-image", None)
            .expect("win.edit-image reabre el formulario");
        pump_until(|| dialog_titled("Edit Image").is_some());
        emit_escape_key(&dialog_titled("Edit Image").expect("formulario"));
        pump_until(|| dialog_titled("Edit Image").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            fuente.as_bytes(),
            "Escape deja el buffer byte a byte idéntico"
        );
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), seleccion);

        // Cero transacciones nuevas: el único undo es el insert inicial.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");
        assert!(
            !cmds.tabs.borrow_mut().undo_active(),
            "ni Cancelar ni Escape añadieron transacción"
        );

        cmds.window.destroy();
    }

    /// #30 §35/§UI: `win.insert-table` abre el diálogo de dimensiones (con
    /// los límites retenidos en abierto y el error visible) y aceptar deja
    /// el esqueleto GFM con la selección en la primera celda de cabecera;
    /// Escape posterior = cero cambios byte a byte.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_insert_table_esqueleto_via_gaction() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase730-insert-table")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);

        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-table", None)
            .expect("win.insert-table registrada (§36)");
        pump_until(|| dialog_titled("Table").is_some());
        let dialog = dialog_titled("Table").expect("diálogo de dimensiones");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2, "Filas, Columnas (§UI)");
        assert_eq!(entries[0].text(), "2");
        assert_eq!(entries[1].text(), "2");

        // §35: fuera de rango el diálogo NO se cierra y el error queda visible.
        entries[0].set_text("99");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        assert!(
            dialog_titled("Table").is_some(),
            "filas fuera de 1–50 retienen el diálogo (§35)"
        );
        let error = find_label_text(&dialog, "Filas: entero entre 1 y 50 (§35)")
            .expect("mensaje de validación visible");
        assert!(error.is_visible());

        // Dentro de rango: aceptar genera el esqueleto §35.
        entries[0].set_text("2");
        entries[1].set_text("3");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Table").is_none());
        let esqueleto = "|     |     |     |\n\
                        | --- | --- | --- |\n\
                        |     |     |     |\n\
                        |     |     |     |";
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), esqueleto);
        assert_eq!(
            cmds.tabs.borrow().active().editor.selection(),
            Selection::collapsed(Position::new(0, 2)),
            "§8.6: la selección posterior es la primera celda de cabecera"
        );

        // Escape tras aceptar: byte idéntico y sin transacción nueva.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-table", None)
            .expect("win.insert-table reabre el diálogo");
        pump_until(|| dialog_titled("Table").is_some());
        emit_escape_key(&dialog_titled("Table").expect("diálogo"));
        pump_until(|| dialog_titled("Table").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            esqueleto.as_bytes(),
            "Escape deja el buffer byte a byte idéntico"
        );

        // UNA transacción por acción (§8.5): el primer undo vacía el buffer.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");
        assert!(!cmds.tabs.borrow_mut().undo_active(), "solo la inserción");

        cmds.window.destroy();
    }

    /// #30 §26.3: «Table Attributes» aparece en el popover dentro de la
    /// tabla y su botón abre el MISMO formulario que menú/paleta §38;
    /// aplicar crea la línea de atributos y después reemplaza SOLO esa
    /// línea (la tabla es byte a byte la misma).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_edit_table_attrs_crea_y_reemplaza_solo_la_linea() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase730-table-attrs")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let tabla = "| A | B |\n| --- | --- |\n| 1 | 2 |\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), tabla);
        cmds.reload_active();

        // Cursor dentro de la tabla: popover con «Table Attributes» (y sin
        // panel de texto, que exige selección no vacía, igual que #29).
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(2, 2), Position::new(2, 2)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        let boton = cmds
            .editor_pane
            .test_context_button("Table Attributes")
            .expect("botón «Table Attributes» (§UI #30)");
        assert!(boton.is_visible(), "la fila de tabla está visible");
        assert!(boton.can_focus(), "operable con teclado (§29 ⇒ #30)");
        assert!(
            !cmds
                .editor_pane
                .test_context_button("Bold")
                .expect("botón Bold")
                .is_visible(),
            "sin selección no se muestra el panel de texto (#29 intacto)"
        );

        // El botón del popover abre el MISMO diálogo precargado (vacío: la
        // tabla todavía no tiene atributos); Escape ⇒ cero cambios.
        assert!(cmds.editor_pane.test_context_click("Table Attributes"));
        pump_until(|| dialog_titled("Table Attributes").is_some());
        let dialog = dialog_titled("Table Attributes").expect("formulario");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2, "ID, Caption (§UI)");
        assert_eq!(entries[0].text(), "");
        assert_eq!(entries[1].text(), "");
        emit_escape_key(&dialog);
        pump_until(|| dialog_titled("Table Attributes").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            tabla,
            "Escape en el diálogo no toca el documento"
        );

        // Crear la línea vía la MISMA acción `win.edit-table-attrs` (§38).
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-table-attrs", None)
            .expect("win.edit-table-attrs registrada (§36)");
        pump_until(|| dialog_titled("Table Attributes").is_some());
        let dialog = dialog_titled("Table Attributes").expect("formulario");
        let entries = dialog_entries(&dialog);
        entries[0].set_text("tbl:x");
        entries[1].set_text("Resultados");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Table Attributes").is_none());
        let con_atributos = "| A | B |\n| --- | --- |\n| 1 | 2 |\n{#tbl:x \"Resultados\"}\n";
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            con_atributos,
            "crea la línea §26.3 tras el bloque sin tocar la tabla"
        );

        // Reemplazar SOLO la línea: precarga vigente, cambia el caption.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-table-attrs", None)
            .expect("win.edit-table-attrs reabre el formulario");
        pump_until(|| dialog_titled("Table Attributes").is_some());
        let dialog = dialog_titled("Table Attributes").expect("formulario");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries[0].text(), "tbl:x", "precarga ID desde el contexto");
        assert_eq!(entries[1].text(), "Resultados", "precarga Caption");
        entries[1].set_text("Nueva caption");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Table Attributes").is_none());
        let reemplazada = "| A | B |\n| --- | --- |\n| 1 | 2 |\n{#tbl:x \"Nueva caption\"}\n";
        let contenido = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert_eq!(contenido, reemplazada);
        assert!(
            contenido.starts_with(tabla),
            "los bytes de la tabla quedan intactos"
        );

        // Cada aplicación es UNA transacción (§8.5).
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            con_atributos
        );
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), tabla);

        cmds.window.destroy();
    }

    /// #30 §18.2/§18.3/§17: aceptar el formulario de importación copia el
    /// fichero a `<raíz>/assets/` con renombrado §18.3 (`diagram.png` ya
    /// ocupado ⇒ `diagram-2.png`, sin pisar nada) e inserta la ruta RELATIVA
    /// a la carpeta del documento; Cancelar no copia ni edita. `<raíz>` es la
    /// raíz del proyecto en ventanas de proyecto y la carpeta del documento
    /// en standalone. El selector nativo §18.1 no es automatizable: el test
    /// entra por el formulario (el camino posterior al selector).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_import_imagen_a_assets_renombrado_y_ruta_relativa() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let base = std::env::temp_dir().join(format!(
            "minmark-gtk-import30-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        // PNG real mínimo (1×1 RGB): la estructura IHDR/IDAT/IEND está
        // verificada (`file` + zlib) y es lo que la importación copia.
        const PNG_1X1: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78,
            0xda, 0x63, 0xf8, 0xcf, 0xc0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xf7, 0x03, 0x41,
            0x43, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let src = base.join("origen");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("diagram.png"), PNG_1X1).unwrap();
        std::fs::write(src.join("extra.png"), PNG_1X1).unwrap();

        let app = Application::builder()
            .application_id("app.minmark.test.fase730-import")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();

        // Ventana de PROYECTO: la copia va a <raíz del proyecto>/assets/ y
        // `diagram.png` ya existe ⇒ renombrado §18.3.
        let root = base.join("proyecto");
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/diagram.png"), "viejo").unwrap();
        let nota = root.join("nota.md");
        std::fs::write(&nota, "Nota\n").unwrap();
        let cmds = build_window(&app, WindowScope::Project(root.clone()), &[nota]);
        pump_until(|| cmds.index.borrow().is_some());
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(1, 0), Position::new(1, 0)));

        cmds.start_image_import(src.join("diagram.png"));
        pump_until(|| dialog_titled("Image").is_some());
        let dialog = dialog_titled("Image").expect("formulario de importación");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 4, "Ruta, Alt, ID, Ancho % (§UI)");
        assert_eq!(entries[0].text(), "assets/diagram.png");
        assert_eq!(
            entries[1].text(),
            "diagram",
            "Alt por defecto = nombre del fichero sin extensión (§UI)"
        );
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Image").is_none());

        assert_eq!(
            std::fs::read(root.join("assets/diagram.png")).unwrap(),
            "viejo".as_bytes(),
            "§18.3/§3.7: nunca sobrescribe el existente"
        );
        assert_eq!(
            std::fs::read(root.join("assets/diagram-2.png")).unwrap(),
            PNG_1X1,
            "§18.3: colisión ⇒ diagram-2.png, con el fichero copiado intacto"
        );
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "Nota\n![diagram](assets/diagram-2.png)",
            "ruta RELATIVA §17 al documento (nunca absoluta)"
        );

        // Cancelar: NI copia NI edición (§UI «cero cambios»).
        let antes = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        cmds.start_image_import(src.join("extra.png"));
        pump_until(|| dialog_titled("Image").is_some());
        find_button_in(&dialog_titled("Image").expect("formulario"), "Cancelar")
            .expect("botón Cancelar")
            .emit_clicked();
        pump_until(|| dialog_titled("Image").is_none());
        assert!(
            !root.join("assets/extra.png").exists(),
            "cancelar NO copia el fichero (§UI)"
        );
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            antes,
            "cancelar NO edita el documento (§UI)"
        );

        // Ventana STANDALONE: <raíz> = carpeta del documento activo (§UI).
        let suelto = base.join("suelto");
        std::fs::create_dir_all(&suelto).unwrap();
        let doc = suelto.join("doc.md");
        std::fs::write(&doc, "Solo\n").unwrap();
        let cmds2 = build_window(&app, WindowScope::Standalone, &[doc]);
        cmds2.reload_active();
        cmds2.editor_pane
            .focus_find_match(Range::new(Position::new(1, 0), Position::new(1, 0)));
        cmds2.start_image_import(src.join("diagram.png"));
        pump_until(|| dialog_titled("Image").is_some());
        find_button_in(&dialog_titled("Image").expect("formulario"), "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Image").is_none());
        assert_eq!(
            std::fs::read(suelto.join("assets/diagram.png")).unwrap(),
            PNG_1X1,
            "standalone: la copia va a la carpeta del documento"
        );
        assert_eq!(
            cmds2.tabs.borrow().active().editor.document().content(),
            "Solo\n![diagram](assets/diagram.png)"
        );

        cmds.window.destroy();
        cmds2.window.destroy();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// #30 (aceptación): dentro de la imagen el popover muestra «Edit Image»
    /// — ortogonal al panel de texto y operable con teclado — y su botón abre
    /// el formulario precargado; cambiar de pestaña o de modo cierra el
    /// popover y el click YA no aplica nada (guardia #29 intacta).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_popover_edit_image_guardia_de_pestana() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase730-popover-guard")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let imagen = "foto ![alt](img.png) fin\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), imagen);
        cmds.reload_active();
        cmds.tabs.borrow_mut().new_tab();
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "otra pestaña\n");
        cmds.reload_active();
        cmds.select_tab(0);

        // Cursor dentro de la imagen (selección vacía): solo «Edit Image».
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 8), Position::new(0, 8)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        let boton = cmds
            .editor_pane
            .test_context_button("Edit Image")
            .expect("botón «Edit Image» (§UI #30)");
        assert!(boton.is_visible(), "la fila de imagen está visible");
        assert!(boton.can_focus(), "operable con teclado (§29 ⇒ #30)");
        assert!(
            !cmds
                .editor_pane
                .test_context_button("Bold")
                .expect("botón Bold")
                .is_visible(),
            "cursor sin selección: el panel de texto sigue oculto (#29 intacto)"
        );

        // El botón abre el formulario precargado; Escape ⇒ cero cambios.
        assert!(cmds.editor_pane.test_context_click("Edit Image"));
        pump_until(|| dialog_titled("Edit Image").is_some());
        let entries = dialog_entries(&dialog_titled("Edit Image").expect("formulario"));
        assert_eq!(entries[0].text(), "img.png");
        assert_eq!(entries[1].text(), "alt");
        emit_escape_key(&dialog_titled("Edit Image").expect("formulario"));
        pump_until(|| dialog_titled("Edit Image").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            imagen
        );

        // Cambio de PESTAÑA: cierra el popover y el click no aplica nada —
        // ni diálogo ni cambios en la activa ni en la abandonada.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 8), Position::new(0, 8)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        cmds.select_tab(1);
        assert!(!cmds.editor_pane.test_context_visible());
        assert!(cmds.editor_pane.test_context_click("Edit Image"));
        assert!(
            dialog_titled("Edit Image").is_none(),
            "guardia de pestaña: sin formulario con el popover cerrado"
        );
        assert_eq!(
            cmds.tabs.borrow().tabs()[1].editor.document().content(),
            "otra pestaña\n",
            "sin cambios en la pestaña activa"
        );
        assert_eq!(
            cmds.tabs.borrow().tabs()[0].editor.document().content(),
            imagen,
            "sin cambios en la pestaña abandonada"
        );

        // Cambio de MODO: también cierra y no aplica.
        cmds.select_tab(0);
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 8), Position::new(0, 8)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        cmds.toggle_mode();
        assert!(!cmds.editor_pane.test_context_visible(), "cambio de modo cierra");
        assert!(cmds.editor_pane.test_context_click("Edit Image"));
        assert!(dialog_titled("Edit Image").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            imagen
        );

        cmds.window.destroy();
    }

    // ── Fase 7 #31: ecuaciones LaTeX (§24, §26.4) y citas (§25.2) ────────

    /// #31 §UI: dentro de una ecuación INLINE el popover muestra «Edit
    /// Equation» (operable con teclado) y NO «Equation Attributes» (§26.4);
    /// el diálogo multilinea prellena el LaTeX del contexto y aplicar cambia
    /// SOLO el contenido (marcadores y bloque intactos) en UNA transacción.
    /// Escape (vía menú/paleta, MISMA acción `win.*`) ⇒ cero cambios byte a
    /// byte, selección intacta y sin transacción nueva.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_edit_equation_valido_y_cancelar_31() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase731-edit-equation")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let fuente = "La energía es $E = mc^2$ hoy.\n\n$$\nE=mc^2\n$$\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();

        // Cursor dentro de la ecuación INLINE con selección vacía.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 16), Position::new(0, 16)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        let boton = cmds
            .editor_pane
            .test_context_button("Edit Equation")
            .expect("botón «Edit Equation» (§UI #31)");
        assert!(boton.is_visible(), "la fila de ecuación está visible");
        assert!(boton.can_focus(), "operable con teclado (§29 ⇒ #31)");
        assert!(
            !cmds
                .editor_pane
                .test_context_button("Equation Attributes")
                .expect("botón «Equation Attributes»")
                .is_visible(),
            "los atributos son solo de bloque (§26.4)"
        );
        assert!(
            !cmds
                .editor_pane
                .test_context_button("Bold")
                .expect("botón Bold")
                .is_visible(),
            "cursor sin selección: el panel de texto sigue oculto (#29 intacto)"
        );

        // Con SELECCIÓN dentro de la ecuación conviven el panel de texto
        // (#29) y la fila de ecuación (#31): el criterio es «cursor o
        // selección dentro del constructo».
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 15), Position::new(0, 22)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(
            cmds.editor_pane
                .test_context_button("Edit Equation")
                .expect("botón «Edit Equation»")
                .is_visible(),
            "selección dentro de la ecuación: la fila #31 acompaña al panel"
        );
        assert!(
            cmds.editor_pane
                .test_context_button("Bold")
                .expect("botón Bold")
                .is_visible(),
            "selección no vacía ⇒ panel de texto (#29 intacto)"
        );

        // El botón del popover abre el MISMO diálogo prellenado.
        assert!(cmds.editor_pane.test_context_click("Edit Equation"));
        pump_until(|| dialog_titled("Edit Equation").is_some());
        let dialog = dialog_titled("Edit Equation").expect("diálogo de ecuación");
        let views = dialog_text_views(&dialog);
        assert_eq!(views.len(), 1, "campo LaTeX multilinea (§UI #31)");
        assert_eq!(text_view_text(&views[0]), "E = mc^2", "precarga del contexto");
        assert!(
            find_label_containing(&dialog, "math no soportado").is_none(),
            "ADR-002: sin aviso con LaTeX soportado"
        );

        // Aplicar: SOLO el contenido LaTeX cambia (marcadores y bloque intactos).
        views[0].buffer().set_text("x^2 + y^2");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Edit Equation").is_none());
        let esperado = "La energía es $x^2 + y^2$ hoy.\n\n$$\nE=mc^2\n$$\n";
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            esperado,
            "solo cambia el LaTeX (§24)"
        );

        // UNA transacción (§8.5): undo restaura la fuente y luego el insert.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            fuente
        );
        assert!(cmds.tabs.borrow_mut().undo_active(), "el insert inicial");
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");

        // Escape ⇒ cero cambios byte a byte, selección intacta y sin
        // transacción nueva — entrando por la MISMA acción `win.*` (menú,
        // paleta §38 y popover comparten este camino).
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 16), Position::new(0, 16)));
        let seleccion = Selection::collapsed(Position::new(0, 16));
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-equation", None)
            .expect("win.edit-equation registrada (§36)");
        pump_until(|| dialog_titled("Edit Equation").is_some());
        let dialog = dialog_titled("Edit Equation").expect("diálogo reabierto");
        let views = dialog_text_views(&dialog);
        views[0].buffer().set_text("TEXTO QUE NO SE GUARDA");
        emit_escape_key(&dialog);
        pump_until(|| dialog_titled("Edit Equation").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            fuente.as_bytes(),
            "Escape deja el buffer byte a byte idéntico"
        );
        assert_eq!(cmds.tabs.borrow().active().editor.selection(), seleccion);
        assert!(
            cmds.tabs.borrow_mut().undo_active(),
            "el insert inicial de la fuente"
        );
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "",
            "el único undo es el insert inicial"
        );
        assert!(
            !cmds.tabs.borrow_mut().undo_active(),
            "Escape no añadió transacción"
        );

        cmds.window.destroy();
    }

    /// #31 ADR-002/§24: con LaTeX FUERA del subconjunto el diálogo muestra
    /// el marcador «⚠ [math no soportado: …]» como aviso NO bloqueante — el
    /// contenido sigue editable y aceptar aplica la edición —; Cancelar y
    /// Escape dejan el documento byte a byte intacto sin transacción nueva.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_edit_equation_no_soportado_avisa_sin_bloquear_31() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase731-eq-unsupported")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let fuente = "sea $\\foo{x}$ hoy\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();

        // Cursor dentro de la ecuación no soportada: diálogo con el marcador
        // ADR-002 visible (el aviso NO bloquea el diálogo).
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 7), Position::new(0, 7)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(cmds.editor_pane.test_context_click("Edit Equation"));
        pump_until(|| dialog_titled("Edit Equation").is_some());
        let dialog = dialog_titled("Edit Equation").expect("diálogo de ecuación");
        let views = dialog_text_views(&dialog);
        assert_eq!(text_view_text(&views[0]), "\\foo{x}", "precarga del contexto");
        let aviso = find_label_containing(&dialog, "⚠ [math no soportado:")
            .expect("marcador ADR-002 visible (§24/ADR-002)");
        assert!(aviso.is_visible());
        assert_eq!(aviso.text(), "⚠ [math no soportado: \\foo]");

        // El contenido sigue EDITABLE: el aviso sigue al LaTeX en vivo.
        views[0].buffer().set_text("x + 1");
        assert!(
            find_label_containing(&dialog, "⚠ [math no soportado:").is_none(),
            "con LaTeX soportado el aviso desaparece"
        );
        views[0].buffer().set_text("\\foo{x} + 1");
        assert!(
            find_label_containing(&dialog, "⚠ [math no soportado:").is_some(),
            "el aviso vuelve con el comando no soportado"
        );

        // Aceptar NO está bloqueado: la edición aplica y el contenido queda
        // visible y editable también en el documento (§24).
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Edit Equation").is_none());
        let editado = "sea $\\foo{x} + 1$ hoy\n";
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            editado,
            "aceptar con el aviso aplica el LaTeX editado"
        );

        // Escape ⇒ cero cambios byte a byte y SIN transacción nueva.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-equation", None)
            .expect("win.edit-equation registrada (§36)");
        pump_until(|| dialog_titled("Edit Equation").is_some());
        let dialog = dialog_titled("Edit Equation").expect("diálogo reabierto");
        let views = dialog_text_views(&dialog);
        views[0].buffer().set_text("otro \\foo");
        emit_escape_key(&dialog);
        pump_until(|| dialog_titled("Edit Equation").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            editado.as_bytes(),
            "Escape deja el buffer byte a byte idéntico"
        );

        // Transacciones: el insert inicial y la edición aceptada — ni una más.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            fuente
        );
        assert!(cmds.tabs.borrow_mut().undo_active(), "el insert inicial");
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");
        assert!(
            !cmds.tabs.borrow_mut().undo_active(),
            "Escape no añadió transacción"
        );

        cmds.window.destroy();
    }

    /// #31 §26.4: «Equation Attributes» aparece en el popover SOLO con
    /// ecuación de bloque y su botón abre el MISMO formulario ID/Caption
    /// que Table Attributes; aplicar crea la línea `{#id "caption"}`,
    /// después reemplaza SOLO esa línea y vaciar ambos campos la elimina
    /// (el bloque queda byte a byte intacto). Menú/paleta usan la MISMA
    /// acción `win.equation-attrs`; cada edición es UNA transacción (§8.5).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_equation_attrs_crea_reemplaza_y_quita_31() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase731-equation-attrs")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let fuente = "Bloque\n\n$$\nE=mc^2\n$$\n\nInline $x$ fin\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();

        // Cursor dentro del BLOQUE: popover con ambas filas #31.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(3, 2), Position::new(3, 2)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(
            cmds.editor_pane
                .test_context_button("Edit Equation")
                .expect("botón «Edit Equation»")
                .is_visible()
        );
        assert!(
            cmds.editor_pane
                .test_context_button("Equation Attributes")
                .expect("botón «Equation Attributes»")
                .is_visible(),
            "bloque: los atributos están disponibles (§26.4)"
        );

        // Crear la línea de atributos (form vacío: el bloque no tiene aún).
        assert!(cmds.editor_pane.test_context_click("Equation Attributes"));
        pump_until(|| dialog_titled("Equation Attributes").is_some());
        let dialog = dialog_titled("Equation Attributes").expect("formulario");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2, "ID, Caption (§UI: mismo formulario que #30)");
        assert_eq!(entries[0].text(), "");
        assert_eq!(entries[1].text(), "");
        entries[0].set_text("eq:energy");
        entries[1].set_text("Equivalencia");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Equation Attributes").is_none());
        let con_atributos =
            "Bloque\n\n$$\nE=mc^2\n$$\n{#eq:energy \"Equivalencia\"}\n\nInline $x$ fin\n";
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            con_atributos,
            "crea la línea tras el bloque sin tocar el bloque"
        );

        // Reemplazar SOLO la línea por la vía menú/paleta (MISMA acción) —
        // el diálogo precarga el estado detectado.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.equation-attrs", None)
            .expect("win.equation-attrs registrada (§36)");
        pump_until(|| dialog_titled("Equation Attributes").is_some());
        let dialog = dialog_titled("Equation Attributes").expect("formulario");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries[0].text(), "eq:energy", "precarga ID del contexto");
        assert_eq!(entries[1].text(), "Equivalencia", "precarga Caption");
        entries[1].set_text("Nueva caption");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Equation Attributes").is_none());
        let reemplazada =
            "Bloque\n\n$$\nE=mc^2\n$$\n{#eq:energy \"Nueva caption\"}\n\nInline $x$ fin\n";
        let contenido = cmds
            .tabs
            .borrow()
            .active()
            .editor
            .document()
            .content()
            .to_owned();
        assert_eq!(contenido, reemplazada);
        assert!(
            contenido.starts_with("Bloque\n\n$$\nE=mc^2\n$$\n"),
            "los bytes del bloque quedan intactos"
        );

        // Vaciar ambos campos ⇒ la línea desaparece entera (§26.4).
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.equation-attrs", None)
            .expect("win.equation-attrs reabre el formulario");
        pump_until(|| dialog_titled("Equation Attributes").is_some());
        let dialog = dialog_titled("Equation Attributes").expect("formulario");
        let entries = dialog_entries(&dialog);
        entries[0].set_text("");
        entries[1].set_text("");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Equation Attributes").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            fuente,
            "sin id ni caption la línea se elimina; la fuente vuelve byte a byte"
        );

        // En ECUACIÓN INLINE el popover NO ofrece Equation Attributes.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(6, 8), Position::new(6, 8)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(
            cmds.editor_pane
                .test_context_button("Edit Equation")
                .expect("botón «Edit Equation»")
                .is_visible()
        );
        assert!(
            !cmds
                .editor_pane
                .test_context_button("Equation Attributes")
                .expect("botón «Equation Attributes»")
                .is_visible(),
            "atributos solo en bloque (§26.4)"
        );

        // Tres ediciones ⇒ tres transacciones (§8.5); los undos vuelven paso
        // a paso hasta la fuente original.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            reemplazada
        );
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            con_atributos
        );
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            fuente
        );

        cmds.window.destroy();
    }

    /// #31 §25.2: `win.insert-citation` abre el diálogo de items (clave +
    /// localizador) con añadir/quitar — y RETIENE el diálogo con el error
    /// visible cuando un campo sale de la gramática consumidora (revisión
    /// #31, finding 3)— y aceptar deja la cita canónica `[@k1, loc; @k2]`
    /// en UNA transacción; `win.edit-citation` (popover) reutiliza el MISMO
    /// diálogo prellenado con los items de la cita, y Escape ⇒ cero cambios
    /// byte a byte.
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_insert_y_edit_citation_items_31() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase731-citations")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "Ver\n");
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 3), Position::new(0, 3)));

        // Diálogo de inserción: una fila inicial + añadir/quitar.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-citation", None)
            .expect("win.insert-citation registrada (§36)");
        pump_until(|| dialog_titled("Citation").is_some());
        let dialog = dialog_titled("Citation").expect("diálogo de citas");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2, "una fila: clave + localizador (§UI)");
        assert_eq!(entries[0].placeholder_text().as_deref(), Some("Clave"));
        assert_eq!(
            entries[1].placeholder_text().as_deref(),
            Some("Localizador (p. 42)")
        );
        for entry in &entries {
            assert!(entry.can_focus(), "operable con teclado (§29 ⇒ #31)");
        }
        let add = find_button_in(&dialog, "Añadir ítem")
            .expect("botón «Añadir ítem»")
            .clone();
        assert!(add.can_focus());
        assert!(find_button_in(&dialog, "Cancelar").is_some());

        // §25.2 (revisión #31, finding 3): claves o localizadores fuera de
        // la gramática consumidora RETIENEN el diálogo con el error visible
        // (patrón FormValidator) — el documento no se toca.
        entries[0].set_text("smith 2025");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        assert!(
            dialog_titled("Citation").is_some(),
            "clave con espacio: el diálogo sigue ABIERTO"
        );
        let error = find_label_containing(&dialog, "no válida")
            .expect("error de clave visible");
        assert!(error.is_visible());
        entries[0].set_text("smith2025");
        entries[1].set_text("p. 1; fig. 2");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        assert!(
            dialog_titled("Citation").is_some(),
            "localizador con «;»: el diálogo sigue ABIERTO"
        );
        let error = find_label_containing(&dialog, "no válido")
            .expect("error de localizador visible");
        assert!(error.is_visible());

        // Con ambos campos válidos el flujo continúa en el MISMO diálogo.
        entries[0].set_text("smith2025");
        entries[1].set_text("p. 42");
        add.emit_clicked();
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 4, "segunda fila añadida");
        entries[2].set_text("jones2024");

        // Quitar la PRIMERA fila (renumeración incluida) y añadir otra:
        // el resultado final son dos items, el segundo con localizador.
        find_button_in(&dialog, "Quitar")
            .expect("botón «Quitar» de la fila 1")
            .emit_clicked();
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2, "fila quitable");
        assert_eq!(entries[0].text(), "jones2024", "sobrevive la fila 2");
        find_button_in(&dialog, "Añadir ítem")
            .expect("«Añadir ítem»")
            .emit_clicked();
        let entries = dialog_entries(&dialog);
        entries[2].set_text("reyes2026");
        entries[3].set_text("p. 9");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Citation").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "Ver[@jones2024; @reyes2026, p. 9]\n",
            "forma canónica §25.2 en el cursor"
        );

        // UNA transacción (§8.5): undo → insert inicial → vacío.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "Ver\n"
        );
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");

        // Editar una cita existente desde el popover (MISMA acción que
        // menú/paleta): diálogo prellenado con los items detectados.
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "notas [@vieja] hoy\n");
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 7), Position::new(0, 7)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(
            cmds.editor_pane
                .test_context_button("Edit Citation")
                .expect("botón «Edit Citation» (§UI #31)")
                .is_visible(),
            "cursor dentro de la cita: la fila de cita se muestra"
        );
        assert!(cmds.editor_pane.test_context_click("Edit Citation"));
        pump_until(|| dialog_titled("Edit Citation").is_some());
        let dialog = dialog_titled("Edit Citation").expect("diálogo de edición");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2, "una fila por item de la cita");
        assert_eq!(entries[0].text(), "vieja", "precarga de la clave (§25.2)");
        assert_eq!(entries[1].text(), "", "sin localizador ⇒ vacío");

        entries[0].set_text("a");
        entries[1].set_text("p. 7");
        find_button_in(&dialog, "Añadir ítem")
            .expect("«Añadir ítem»")
            .emit_clicked();
        let entries = dialog_entries(&dialog);
        entries[2].set_text("b");
        find_button_in(&dialog, "Aceptar")
            .expect("botón Aceptar")
            .emit_clicked();
        pump_until(|| dialog_titled("Edit Citation").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "notas [@a, p. 7; @b] hoy\n",
            "CitationEdit reemplaza la cita entera (§25.2)"
        );

        // Escape ⇒ cero cambios byte a byte y SIN transacción nueva.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.edit-citation", None)
            .expect("win.edit-citation registrada (§36)");
        pump_until(|| dialog_titled("Edit Citation").is_some());
        let dialog = dialog_titled("Edit Citation").expect("diálogo reabierto");
        let entries = dialog_entries(&dialog);
        entries[0].set_text("borrar-esto");
        emit_escape_key(&dialog);
        pump_until(|| dialog_titled("Edit Citation").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content().as_bytes(),
            "notas [@a, p. 7; @b] hoy\n".as_bytes(),
            "Escape deja el buffer byte a byte idéntico"
        );

        // Transacciones: el edit aceptado y el insert de la fuente — ni una más.
        assert!(cmds.tabs.borrow_mut().undo_active());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "notas [@vieja] hoy\n"
        );
        assert!(cmds.tabs.borrow_mut().undo_active(), "el insert inicial");
        assert_eq!(cmds.tabs.borrow().active().editor.document().content(), "");
        assert!(
            !cmds.tabs.borrow_mut().undo_active(),
            "Escape no añadió transacción"
        );

        cmds.window.destroy();
    }

    /// #31 §25.3: la clave del diálogo de citas autocompleta por clave,
    /// AUTOR, título y año con `intelligence::citation_completions` (el MISMO
    /// completion de la fase 5, nunca reimplementado): la lista aparece con
    /// lo tecleado, Enter acepta la fila seleccionada, una clave sin
    /// coincidencias cierra el popup y Escape cierra SOLO el popup (un
    /// segundo Escape cancela el diálogo con cero cambios).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_autocompletado_bibliografico_en_dialogo_citas_31() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let base = std::env::temp_dir().join(format!("minmark-ac31-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let bib = base.join("refs.bib");
        std::fs::write(
            &bib,
            include_str!("../../docs/qa/fixtures/references.bib"),
        )
        .unwrap();

        let app = Application::builder()
            .application_id("app.minmark.test.fase731-citation-ac")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "Ver\n");
        cmds.reload_active();
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 3), Position::new(0, 3)));
        // La MISMA bibliografía que la fase 5 (`academic::load` sobre
        // `settings.bibliography`): se fija DESPUÉS del refresco, porque
        // `refresh_now` recalcula la bibliografía efectiva (§58).
        cmds.settings.borrow_mut().bibliography = vec![bib.to_string_lossy().into_owned()];

        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.insert-citation", None)
            .expect("win.insert-citation registrada (§36)");
        pump_until(|| dialog_titled("Citation").is_some());
        let dialog = dialog_titled("Citation").expect("diálogo de citas");
        let entries = dialog_entries(&dialog);
        assert_eq!(entries.len(), 2);

        // Tecleo de la clave: el popup muestra la sugerencia (§25.3).
        entries[0].set_text("Sm");
        entries[0].set_position(2);
        pump_until(|| visible_popover_in(&dialog).is_some());
        let popover = visible_popover_in(&dialog).expect("popup de completado");
        let labels = popover_labels(&popover);
        assert!(
            labels.iter().any(|label| label.contains("smith2025")),
            "sugerencia visible con «Sm»: {labels:?}"
        );

        // §25.3 también por AUTOR: «Alice» («… · Alice Smith · …»).
        entries[0].set_text("Alice");
        entries[0].set_position(5);
        pump_until(|| visible_popover_in(&dialog).is_some());
        let popover = visible_popover_in(&dialog).expect("popup por autor");
        assert!(
            popover_labels(&popover)
                .iter()
                .any(|label| label.contains("smith2025")),
            "sugerencia visible con «Alice»"
        );

        // Enter (teclado, patrón completion #29) acepta la fila activa.
        assert!(
            emit_key(
                entries[0].upcast_ref::<gtk4::Widget>(),
                gtk4::gdk::Key::Return,
                gtk4::gdk::ModifierType::empty()
            ),
            "el controlador de la entrada consume Enter con el popup abierto"
        );
        assert_eq!(entries[0].text(), "smith2025", "clave aceptada");
        pump_until(|| visible_popover_in(&dialog).is_none());

        // Una clave sin coincidencias cierra el popup (§25.3 filtra).
        entries[0].set_text("zzz-no-existe");
        entries[0].set_position(13);
        pump_until(|| visible_popover_in(&dialog).is_none());

        // Escape cierra SOLO el popup…
        entries[0].set_text("smi");
        entries[0].set_position(3);
        pump_until(|| visible_popover_in(&dialog).is_some());
        assert!(emit_key(
            entries[0].upcast_ref::<gtk4::Widget>(),
            gtk4::gdk::Key::Escape,
            gtk4::gdk::ModifierType::empty()
        ));
        pump_until(|| visible_popover_in(&dialog).is_none());
        assert!(
            dialog_titled("Citation").is_some(),
            "el primer Escape cierra SOLO el popup"
        );

        // …y un segundo Escape cancela el diálogo con cero cambios.
        emit_escape_key(&dialog);
        pump_until(|| dialog_titled("Citation").is_none());
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            "Ver\n",
            "cancelar el diálogo no edita el documento (§UI)"
        );

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(&base);
    }

    /// #31 (aceptación): con el cursor dentro de una ecuación o de una cita
    /// el popover muestra sus controles — «Edit Equation» (y «Equation
    /// Attributes» en bloque) o «Edit Citation» —; cambiar de pestaña o de
    /// modo lo cierra y el click YA no abre ningún diálogo (guardia #29
    /// intacta, sin tocar ninguna pestaña).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_popover_ecuacion_cita_guardia_de_pestana_31() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let app = Application::builder()
            .application_id("app.minmark.test.fase731-popover-guard")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Standalone, &[]);
        let fuente = "Ecuación $x$ y cita [@a] ok\n";
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), fuente);
        cmds.reload_active();
        cmds.tabs.borrow_mut().new_tab();
        cmds.tabs
            .borrow_mut()
            .insert_active(Position::new(0, 0), "otra pestaña\n");
        cmds.reload_active();
        cmds.select_tab(0);

        // Cursor dentro de la ecuación (selección vacía): solo la fila #31.
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 10), Position::new(0, 10)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(
            cmds.editor_pane
                .test_context_button("Edit Equation")
                .expect("botón «Edit Equation»")
                .is_visible()
        );
        assert!(
            !cmds
                .editor_pane
                .test_context_button("Bold")
                .expect("botón Bold")
                .is_visible(),
            "cursor sin selección: sin panel de texto (#29 intacto)"
        );

        // Cambio de PESTAÑA: cierra el popover y el click no abre diálogo.
        cmds.select_tab(1);
        assert!(!cmds.editor_pane.test_context_visible());
        assert!(cmds.editor_pane.test_context_click("Edit Equation"));
        assert!(
            dialog_titled("Edit Equation").is_none(),
            "guardia de pestaña: sin diálogo con el popover cerrado"
        );
        assert_eq!(
            cmds.tabs.borrow().tabs()[1].editor.document().content(),
            "otra pestaña\n",
            "sin cambios en la pestaña activa"
        );
        assert_eq!(
            cmds.tabs.borrow().tabs()[0].editor.document().content(),
            fuente,
            "sin cambios en la pestaña abandonada"
        );

        // Cursor dentro de la cita: la fila «Edit Citation» aparece.
        cmds.select_tab(0);
        cmds.editor_pane
            .focus_find_match(Range::new(Position::new(0, 21), Position::new(0, 21)));
        pump_until(|| cmds.editor_pane.test_context_visible());
        assert!(
            cmds.editor_pane
                .test_context_button("Edit Citation")
                .expect("botón «Edit Citation»")
                .is_visible()
        );

        // Cambio de MODO: cierra el popover y el click no aplica nada.
        cmds.toggle_mode();
        assert!(
            !cmds.editor_pane.test_context_visible(),
            "cambio de modo cierra"
        );
        assert!(cmds.editor_pane.test_context_click("Edit Citation"));
        assert!(
            dialog_titled("Edit Citation").is_none(),
            "guardia de modo: sin diálogo con el popover cerrado"
        );
        assert_eq!(
            cmds.tabs.borrow().active().editor.document().content(),
            fuente,
            "sin cambios en el documento"
        );

        cmds.window.destroy();
    }

    /// #70 (§14.4/§14.5 + §36): los toggles del explorador viven en el menú
    /// View como acciones stateful `win.*` — activarlas voltea el estado
    /// booleano, cambia el filtrado de ESTA ventana (§47.5: sin releer) y el
    /// `Show All Files` alimenta Quick Open (ISSUE-029).
    #[test]
    #[ignore = "requiere sesión gráfica GTK/WebKit"]
    fn gtk_filtros_del_explorador_en_view_via_gaction() {
        gtk4::init().expect("GTK display");
        ensure_sandbox_usable();
        let root = std::env::temp_dir().join(format!("minmark-filtros70-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("documento.md"), "# doc\n").unwrap();
        std::fs::write(root.join(".oculto.md"), "# oculto\n").unwrap();
        std::fs::write(root.join("datos.dat"), "x\n").unwrap();

        let app = Application::builder()
            .application_id("app.minmark.test.filtros70")
            .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gtk4::gio::Cancellable>).unwrap();
        let cmds = build_window(&app, WindowScope::Project(root.clone()), &[]);
        pump_until(|| cmds.index.borrow().is_some());
        let side = cmds
            .sidebar
            .as_ref()
            .expect("la ventana de proyecto tiene barra lateral");
        let explorer = &side.explorer;
        pump_until(|| !explorer.row_names().is_empty());

        /// Estado booleano de una acción `win.*` del grupo de esta ventana.
        fn estado(cmds: &AppCmds, id: &str) -> bool {
            cmds.action_group
                .borrow()
                .as_ref()
                .and_then(|group| group.upgrade())
                .and_then(|group| group.lookup_action(id))
                .and_then(|action| action.state())
                .and_then(|state| state.get::<bool>())
                .unwrap_or_else(|| panic!("win.{id} stateful booleana registrada (#70)"))
        }

        // Estado inicial: ambos toggles sin marcar y sin cambios en el filtro.
        let nombres = explorer.row_names();
        assert!(
            nombres.contains(&"documento.md".to_owned()),
            "el documento del proyecto se lista"
        );
        assert!(
            !nombres.contains(&"datos.dat".to_owned()),
            "§14.4: sin Show All solo tipos documentales"
        );
        assert!(
            !nombres.contains(&".oculto.md".to_owned()),
            "§14.5: los ocultos están fuera por defecto"
        );
        assert!(!explorer.show_all(), "§14.4 arranca apagado");
        assert!(!explorer.show_hidden(), "§14.5 arranca apagado");
        assert!(!estado(&cmds, "show-all-files"), "acción sin marcar");
        assert!(!estado(&cmds, "show-hidden-files"), "acción sin marcar");
        assert!(
            !cmds.quickopen.test_show_all_files(),
            "Quick Open arranca con Show All apagado (ISSUE-029)"
        );

        // Show All Files: voltea el estado, cambia el filtrado y notifica a
        // Quick Open por el MISMO camino `win.*` que menú y paleta §38.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.show-all-files", None)
            .expect("win.show-all-files registrada (§36)");
        assert!(
            estado(&cmds, "show-all-files"),
            "el estado GAction voltea (#70)"
        );
        assert!(explorer.show_all(), "el explorador aplica §14.4");
        assert!(
            explorer.row_names().contains(&"datos.dat".to_owned()),
            "Show All Files revela los no-documentales (§14.4)"
        );
        assert!(
            cmds.quickopen.test_show_all_files(),
            "el notificador §14.4 alimenta Quick Open (ISSUE-029)"
        );

        // Y la MISMA acción lo vuelve a apagar.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.show-all-files", None)
            .expect("win.show-all-files reactivable");
        assert!(
            !estado(&cmds, "show-all-files"),
            "segunda activación ⇒ false"
        );
        assert!(!explorer.show_all(), "§14.4 se apaga");
        assert!(
            !explorer.row_names().contains(&"datos.dat".to_owned()),
            "los no-documentales vuelven a salir"
        );
        assert!(
            !cmds.quickopen.test_show_all_files(),
            "Quick Open vuelve a Show All apagado"
        );

        // Show Hidden Files §14.5: idéntico camino sobre su propio filtro.
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.show-hidden-files", None)
            .expect("win.show-hidden-files registrada (§36)");
        assert!(
            estado(&cmds, "show-hidden-files"),
            "el estado GAction voltea (#70)"
        );
        assert!(explorer.show_hidden(), "el explorador aplica §14.5");
        assert!(
            explorer.row_names().contains(&".oculto.md".to_owned()),
            "Show Hidden Files revela los ocultos (§14.5)"
        );
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.show-hidden-files", None)
            .expect("win.show-hidden-files reactivable");
        assert!(!explorer.show_hidden(), "§14.5 se apaga");
        assert!(
            !explorer.row_names().contains(&".oculto.md".to_owned()),
            "los ocultos vuelven a salir"
        );

        // Ruta programática (#70 revisión): un set directo sobre el panel
        // refleja su valor en la acción `win.*` — antes quedaba desfasada y
        // la siguiente activación re-aplicaba el mismo valor en vez de
        // conmutar—, y el menú marcado/desmarcado sigue al explorador.
        explorer.set_show_all(true);
        assert!(explorer.show_all(), "el set programático aplica §14.4");
        assert!(
            explorer.row_names().contains(&"datos.dat".to_owned()),
            "el set programático también re-renderiza el filtrado"
        );
        assert!(
            estado(&cmds, "show-all-files"),
            "el set programático marca la acción win.* (#70 revisión)"
        );
        assert!(
            cmds.quickopen.test_show_all_files(),
            "el set programático alimenta Quick Open (ISSUE-029)"
        );
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.show-all-files", None)
            .expect("win.show-all-files tras set programático");
        assert!(
            !explorer.show_all(),
            "la siguiente activación APAGA: sin estado desfasado (#70)"
        );
        assert!(
            !estado(&cmds, "show-all-files"),
            "la acción queda desmarcada"
        );
        assert!(
            !cmds.quickopen.test_show_all_files(),
            "Quick Open sigue al explorador"
        );

        explorer.set_show_hidden(true);
        assert!(explorer.show_hidden(), "el set programático aplica §14.5");
        assert!(
            estado(&cmds, "show-hidden-files"),
            "el set programático marca su acción win.* (#70 revisión)"
        );
        gtk4::prelude::WidgetExt::activate_action(&cmds.window, "win.show-hidden-files", None)
            .expect("win.show-hidden-files tras set programático");
        assert!(
            !explorer.show_hidden(),
            "la siguiente activación apaga §14.5 (#70)"
        );
        assert!(
            !estado(&cmds, "show-hidden-files"),
            "su acción queda desmarcada"
        );

        cmds.window.destroy();
        let _ = std::fs::remove_dir_all(&root);
    }
}
