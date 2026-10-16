//! Comandos, barra de menú §36 y atajos §39.
//!
//! El modelo de menú vive en datos puros ([`MENUBAR_SPEC`]) para poder
//! verificar la estructura §36 sin display ni main loop de GTK;
//! [`build_menubar`] lo materializa en `gio::Menu` (GMenu es GObject puro de
//! GIO y no necesita `gtk_init`). [`install_actions`] registra las acciones
//! `win.*` (deshabilitando las de composición sin proyecto, ISSUE-022) y
//! fija los aceleradores §39. [`ALL_COMMANDS`] es la lista completa §38,
//! pública para que la Command Palette se alimente de ella.

use std::rc::Rc;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::prelude::*;

use crate::preview::ViewMode;

/// Comandos de la aplicación (§36). Insert/Format/Project llegan con sus fases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    New,
    /// Nueva ventana standalone (ISSUE-021): archivo nuevo fuera de proyecto.
    NewWindow,
    Open,
    /// Quick Open (§41, §39: `Ctrl+P`): paleta fuzzy de archivos del proyecto.
    QuickOpen,
    Outline,
    FollowLink,
    Backlinks,
    /// Abrir carpeta como ventana de proyecto nueva (ISSUE-021).
    OpenFolder,
    /// Import Markdown (§15, ISSUE-024): copia un `.md` externo al proyecto.
    ImportMarkdown,
    Save,
    SaveAs,
    /// Exporta el documento activo a PDF (§32.1, ISSUE-035, §39: `Ctrl+E`).
    ExportPdf,
    /// Exporta la composición activa como un único PDF (§20.4, ISSUE-023).
    ExportCompositionPdf,
    CloseTab,
    Undo,
    Redo,
    /// Búsqueda en el archivo actual (§40.1, §39: `Ctrl+F`).
    Find,
    /// Reemplazo en el archivo actual (§40.1, §39: `Ctrl+H`).
    Replace,
    /// Búsqueda en el proyecto (§40.2, §39: `Ctrl+Shift+F`).
    ProjectSearch,
    ToggleMode,
    /// Visibilidad de la sintaxis en Visual (§8.3, ISSUE-030): ocultar
    /// siempre / revelar la del elemento editado.
    VisualSyntax,
    /// Muestra/oculta el explorador de archivos (§14.3, §39: `Ctrl+B`).
    ToggleExplorer,
    /// Cambio de modo de vista (§36, menú View).
    View(ViewMode), // ViewMode de crate::preview
    /// Preview combinado de una composición (§20.4, ISSUE-023, menú Project).
    PreviewComposition,
    About,
    Shortcuts,
}

impl Command {
    /// Identificador de acción GTK («win.…») con el que se registra el
    /// comando y al que apuntan menús y aceleradores (§36).
    pub fn action_id(&self) -> &'static str {
        match self {
            Command::New => "win.new",
            Command::NewWindow => "win.new-window",
            Command::Open => "win.open",
            Command::QuickOpen => "win.quick-open",
            Command::Outline => "win.outline",
            Command::FollowLink => "win.follow-link",
            Command::Backlinks => "win.backlinks",
            Command::OpenFolder => "win.open-folder",
            Command::ImportMarkdown => "win.import-markdown",
            Command::Save => "win.save",
            Command::SaveAs => "win.save-as",
            Command::ExportPdf => "win.export-pdf",
            Command::ExportCompositionPdf => "win.export-composition-pdf",
            Command::CloseTab => "win.close-tab",
            Command::Undo => "win.undo",
            Command::Redo => "win.redo",
            Command::Find => "win.find",
            Command::Replace => "win.replace",
            Command::ProjectSearch => "win.project-search",
            Command::ToggleMode => "win.toggle-mode",
            Command::VisualSyntax => "win.visual-syntax",
            Command::ToggleExplorer => "win.toggle-explorer",
            Command::View(ViewMode::Editor) => "win.view-editor",
            Command::View(ViewMode::Split) => "win.view-split",
            Command::View(ViewMode::Preview) => "win.view-preview",
            Command::PreviewComposition => "win.preview-composition",
            Command::About => "win.about",
            Command::Shortcuts => "win.shortcuts",
        }
    }

    /// Etiqueta de menú en inglés (§36: File, Edit, …). El ítem de atajos
    /// conserva «Atajos», el nombre con el que §36 lo lista en Help.
    pub fn label(&self) -> &'static str {
        match self {
            Command::New => "New",
            Command::NewWindow => "New Window",
            Command::Open => "Open",
            Command::QuickOpen => "Quick Open",
            Command::Outline => "Outline",
            Command::FollowLink => "Open Link",
            Command::Backlinks => "Backlinks",
            Command::OpenFolder => "Open Folder",
            Command::ImportMarkdown => "Import Markdown",
            Command::Save => "Save",
            Command::SaveAs => "Save As",
            Command::ExportPdf => "Export PDF",
            Command::ExportCompositionPdf => "Export Composition PDF",
            Command::CloseTab => "Close Tab",
            Command::Undo => "Undo",
            Command::Redo => "Redo",
            Command::Find => "Find",
            Command::Replace => "Replace",
            Command::ProjectSearch => "Project Search",
            Command::ToggleMode => "Markdown/Visual",
            Command::VisualSyntax => "Toggle Visual Syntax",
            Command::ToggleExplorer => "Toggle Explorer",
            Command::View(mode) => mode.label(),
            Command::PreviewComposition => "Preview Composition",
            Command::About => "About",
            Command::Shortcuts => "Atajos",
        }
    }

    /// Acción que exige un proyecto con composiciones (§20, ISSUE-022): sin
    /// ellas el ítem queda deshabilitado en el menú y en la paleta (§38)
    /// vía [`Commands::compositions_available`].
    pub fn requires_composition(&self) -> bool {
        matches!(
            self,
            Command::PreviewComposition | Command::ExportCompositionPdf
        )
    }
}

/// Receptor de comandos: lo implementa el integrador (main.rs).
pub trait Commands {
    fn new(&self);
    /// Nueva ventana standalone con un archivo nuevo (ISSUE-021).
    fn new_window(&self);
    fn open(&self);
    /// Abre la paleta Quick Open (§41, §39: `Ctrl+P`).
    fn quick_open(&self);
    fn outline(&self) {}
    fn follow_link(&self) {}
    fn backlinks(&self) {}
    /// Abre una carpeta en una ventana de proyecto nueva (ISSUE-021).
    fn open_folder(&self);
    /// Import Markdown (§15): copia un `.md` externo dentro del proyecto.
    fn import_markdown(&self);
    fn save(&self);
    fn save_as(&self);
    /// Exporta el documento activo a PDF (§32.1, ISSUE-035): diálogo de
    /// destino y motor [`minmark_pdf::TypstEngine`].
    fn export_pdf(&self);
    /// Exporta la composición elegida como un único PDF (§20.4, ISSUE-023):
    /// el mismo flujo de diálogo que [`Commands::export_pdf`].
    fn export_composition_pdf(&self) {}
    /// Preview combinado de la composición elegida (§20.4, ISSUE-023) en
    /// una ventana temporal con el patrón Outline/Backlinks.
    fn preview_composition(&self) {}
    /// ¿Hay composiciones que gestionar (§20, ISSUE-022)? Sin proyecto
    /// abierto las acciones de composición quedan deshabilitadas.
    fn compositions_available(&self) -> bool {
        false
    }
    fn close_tab(&self);
    fn undo(&self);
    fn redo(&self);
    /// Abre la barra de búsqueda del archivo actual (§40.1).
    fn find(&self);
    /// Abre la barra de búsqueda con la fila de reemplazo (§40.1).
    fn replace(&self);
    /// Abre la vista de búsqueda en el proyecto de la barra lateral
    /// (§40.2); sin-op en ventanas standalone.
    fn project_search(&self);
    fn toggle_mode(&self);
    /// Alterna la visibilidad de la sintaxis en Visual (§8.3).
    fn toggle_visual_syntax(&self);
    /// Muestra/oculta el explorador (§14.3); sin-op en ventanas standalone.
    fn toggle_explorer(&self);
    fn set_view(&self, mode: ViewMode);
    fn about(&self);
    fn shortcuts(&self);
}

/// Atajos §39 (identificador de acción → acelerador GTK).
pub const ACCELERATORS: &[(&str, &str)] = &[
    ("win.new", "<Control>n"),
    ("win.new-window", "<Control><Shift>n"),
    ("win.open", "<Control>o"),
    ("win.quick-open", "<Control>p"),
    ("win.outline", "<Control><Alt>o"),
    ("win.follow-link", "<Control>Return"),
    ("win.backlinks", "<Control><Alt>b"),
    ("win.open-folder", "<Control><Shift>o"),
    ("win.save", "<Control>s"),
    ("win.save-as", "<Control><Shift>s"),
    ("win.export-pdf", "<Control>e"),
    ("win.close-tab", "<Control>w"),
    ("win.undo", "<Control>z"),
    ("win.redo", "<Control><Shift>z"),
    ("win.find", "<Control>f"),
    ("win.replace", "<Control>h"),
    ("win.project-search", "<Control><Shift>f"),
    ("win.toggle-mode", "<Control><Alt>m"),
    ("win.toggle-explorer", "<Control>b"),
];

/// Todos los comandos, en orden de aparición en los menús §36. §38: es la
/// lista completa que la Command Palette ejecuta — pública para que la
/// paleta se alimente de ella — y todo comando es alcanzable desde el menú.
pub const ALL_COMMANDS: &[Command] = &[
    Command::New,
    Command::NewWindow,
    Command::Open,
    Command::QuickOpen,
    Command::Outline,
    Command::FollowLink,
    Command::Backlinks,
    Command::OpenFolder,
    Command::ImportMarkdown,
    Command::Save,
    Command::SaveAs,
    Command::ExportPdf,
    Command::ExportCompositionPdf,
    Command::CloseTab,
    Command::Undo,
    Command::Redo,
    Command::Find,
    Command::Replace,
    Command::ProjectSearch,
    Command::ToggleMode,
    Command::VisualSyntax,
    Command::ToggleExplorer,
    Command::View(ViewMode::Editor),
    Command::View(ViewMode::Split),
    Command::View(ViewMode::Preview),
    Command::PreviewComposition,
    Command::About,
    Command::Shortcuts,
];

/// Ítem puro del modelo de menú §36: `(etiqueta, action_id)`.
type EntrySpec = (&'static str, &'static str);
/// Sección pura del modelo de menú §36: ítems contiguos con separador.
type SectionSpec = &'static [EntrySpec];
/// Submenú puro §36: `(título, secciones)`.
type SubmenuSpec = (&'static str, &'static [SectionSpec]);

/// Modelo de menú §36 en datos puros: File, Edit, View, Project y Help
/// (Insert/Format llegan con sus fases). View agrupa «Markdown/Visual»
/// (ToggleMode) y la sección radio implícita Editor/Split/Preview (§36);
/// Project nace con las composiciones (§20, ISSUE-022).
const MENUBAR_SPEC: &[SubmenuSpec] = &[
    (
        "File",
        &[&[
            ("New", "win.new"),
            ("New Window", "win.new-window"),
            ("Open", "win.open"),
            ("Quick Open", "win.quick-open"),
            ("Open Folder", "win.open-folder"),
            ("Import Markdown", "win.import-markdown"),
            ("Save", "win.save"),
            ("Save As", "win.save-as"),
            ("Export PDF", "win.export-pdf"),
            ("Export Composition PDF", "win.export-composition-pdf"),
            ("Close Tab", "win.close-tab"),
        ]],
    ),
    (
        "Edit",
        &[&[
            ("Undo", "win.undo"),
            ("Redo", "win.redo"),
            ("Find", "win.find"),
            ("Replace", "win.replace"),
            ("Project Search", "win.project-search"),
            ("Open Link", "win.follow-link"),
        ]],
    ),
    (
        "View",
        &[
            &[
                ("Markdown/Visual", "win.toggle-mode"),
                ("Toggle Visual Syntax", "win.visual-syntax"),
                ("Toggle Explorer", "win.toggle-explorer"),
                ("Outline", "win.outline"),
                ("Backlinks", "win.backlinks"),
            ],
            &[
                ("Editor", "win.view-editor"),
                ("Split", "win.view-split"),
                ("Preview", "win.view-preview"),
            ],
        ],
    ),
    (
        "Project",
        &[&[("Preview Composition", "win.preview-composition")]],
    ),
    (
        "Help",
        &[&[("Atajos", "win.shortcuts"), ("About", "win.about")]],
    ),
];

/// Modelo de menú §36: File, Edit, View, Project, Help (Insert/Format llegan
/// con sus fases).
pub fn build_menubar() -> gio::Menu {
    let menubar = gio::Menu::new();
    for &(title, sections) in MENUBAR_SPEC {
        let submenu = gio::Menu::new();
        for entries in sections {
            let section = gio::Menu::new();
            for &(label, action_id) in *entries {
                section.append_item(&gio::MenuItem::new(Some(label), Some(action_id)));
            }
            submenu.append_section(None::<&str>, &section);
        }
        menubar.append_submenu(Some(title), &submenu);
    }
    menubar
}

/// Enruta un comando activado al receptor [`Commands`] (§3.7: toda acción de
/// la UI es explícita y observable en el host).
fn dispatch(cmd: Command, host: &dyn Commands) {
    match cmd {
        Command::New => host.new(),
        Command::NewWindow => host.new_window(),
        Command::Open => host.open(),
        Command::QuickOpen => host.quick_open(),
        Command::Outline => host.outline(),
        Command::FollowLink => host.follow_link(),
        Command::Backlinks => host.backlinks(),
        Command::OpenFolder => host.open_folder(),
        Command::ImportMarkdown => host.import_markdown(),
        Command::Save => host.save(),
        Command::SaveAs => host.save_as(),
        Command::ExportPdf => host.export_pdf(),
        Command::ExportCompositionPdf => host.export_composition_pdf(),
        Command::CloseTab => host.close_tab(),
        Command::Undo => host.undo(),
        Command::Redo => host.redo(),
        Command::Find => host.find(),
        Command::Replace => host.replace(),
        Command::ProjectSearch => host.project_search(),
        Command::ToggleMode => host.toggle_mode(),
        Command::VisualSyntax => host.toggle_visual_syntax(),
        Command::ToggleExplorer => host.toggle_explorer(),
        Command::View(mode) => host.set_view(mode),
        Command::PreviewComposition => host.preview_composition(),
        Command::About => host.about(),
        Command::Shortcuts => host.shortcuts(),
    }
}

/// Conecta `gio::SimpleAction`s de la ventana con `host` y fija los aceleradores.
///
/// Cada comando se registra sin parámetro en un `gio::SimpleActionGroup`
/// insertado con el prefijo «win» (los `action_id` §36 son nombres de acción
/// detallados `win.*`). `View(ViewMode)` usa tres acciones simples
/// `win.view-editor|split|preview` (forma simple verificada en gtk4 0.9; la
/// variante con target sobre `glib::Variant` no aporta nada aquí). Los
/// aceleradores §39 se fijan con `gtk::Application::set_accels_for_action`.
pub fn install_actions<W: IsA<gtk::Window>>(
    window: &W,
    app: &gtk::Application,
    host: Rc<dyn Commands>,
) {
    let group = gio::SimpleActionGroup::new();
    for &cmd in ALL_COMMANDS {
        let name = cmd
            .action_id()
            .strip_prefix("win.")
            .expect("los action_id usan el prefijo «win.» (§36)");
        let action = gio::SimpleAction::new(name, None);
        // §20/ISSUE-022: sin proyecto con composiciones el ítem queda
        // deshabilitado (también en la paleta §38: la misma acción `win.*`).
        if cmd.requires_composition() {
            action.set_enabled(host.compositions_available());
        }
        let host = Rc::clone(&host);
        action.connect_activate(move |_, _| dispatch(cmd, &*host));
        group.add_action(&action);
    }
    window
        .upcast_ref::<gtk::Window>()
        .insert_action_group("win", Some(&group));
    for &(action_id, accel) in ACCELERATORS {
        app.set_accels_for_action(action_id, &[accel]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashSet;

    /// §39: los atajos exigidos (incluido `Ctrl+P` Quick Open y `Ctrl+Shift+F`
    /// Project Search) están registrados y no hay aceleradores duplicados ni
    /// acciones sin registrar.
    #[test]
    fn aceleradores_39_sin_duplicados() {
        for (id, accel) in [
            ("win.new", "<Control>n"),
            ("win.open", "<Control>o"),
            ("win.quick-open", "<Control>p"),
            ("win.outline", "<Control><Alt>o"),
            ("win.follow-link", "<Control>Return"),
            ("win.backlinks", "<Control><Alt>b"),
            ("win.save", "<Control>s"),
            ("win.save-as", "<Control><Shift>s"),
            ("win.export-pdf", "<Control>e"),
            ("win.find", "<Control>f"),
            ("win.replace", "<Control>h"),
            ("win.project-search", "<Control><Shift>f"),
            ("win.toggle-explorer", "<Control>b"),
        ] {
            assert!(
                ACCELERATORS.contains(&(id, accel)),
                "falta el atajo §39 {id} → {accel}"
            );
        }

        let registrados: HashSet<&str> = ALL_COMMANDS.iter().map(|c| c.action_id()).collect();
        let mut acciones = HashSet::new();
        let mut aceleradores = HashSet::new();
        for &(id, accel) in ACCELERATORS {
            assert!(registrados.contains(id), "acción sin registrar: {id}");
            assert!(
                acciones.insert(id),
                "acción duplicada en ACCELERATORS: {id}"
            );
            assert!(
                aceleradores.insert(accel),
                "acelerador duplicado en ACCELERATORS: {accel}"
            );
        }
    }

    /// §36: cada comando tiene un `action_id` único con el prefijo «win.».
    #[test]
    fn action_ids_unicos_con_prefijo_win() {
        let mut vistos = HashSet::new();
        for &cmd in ALL_COMMANDS {
            let id = cmd.action_id();
            assert!(id.starts_with("win."), "{id} sin prefijo «win.»");
            assert!(vistos.insert(id), "action_id duplicado: {id}");
        }
        assert_eq!(vistos.len(), ALL_COMMANDS.len());
    }

    /// §36: estructura exacta del menú — File (New/New Window/Open/Quick
    /// Open/Open Folder/Import Markdown/Save/Save As/Export PDF/Export
    /// Composition PDF/Close Tab, ISSUE-021/029/035/023), Edit (Undo/Redo/
    /// Find/Replace/Project Search, §40.1/§40.2), View (Markdown/Visual +
    /// Editor/Split/Preview), Project (Preview Composition, §20/ISSUE-022)
    /// y Help (Atajos/About).
    #[test]
    fn estructura_menu_36() {
        let esperado: &[(&str, &[&[(&str, &str)]])] = &[
            (
                "File",
                &[&[
                    ("New", "win.new"),
                    ("New Window", "win.new-window"),
                    ("Open", "win.open"),
                    ("Quick Open", "win.quick-open"),
                    ("Open Folder", "win.open-folder"),
                    ("Import Markdown", "win.import-markdown"),
                    ("Save", "win.save"),
                    ("Save As", "win.save-as"),
                    ("Export PDF", "win.export-pdf"),
                    ("Export Composition PDF", "win.export-composition-pdf"),
                    ("Close Tab", "win.close-tab"),
                ]],
            ),
            (
                "Edit",
                &[&[
                    ("Undo", "win.undo"),
                    ("Redo", "win.redo"),
                    ("Find", "win.find"),
                    ("Replace", "win.replace"),
                    ("Project Search", "win.project-search"),
                    ("Open Link", "win.follow-link"),
                ]],
            ),
            (
                "View",
                &[
                    &[
                        ("Markdown/Visual", "win.toggle-mode"),
                        ("Toggle Visual Syntax", "win.visual-syntax"),
                        ("Toggle Explorer", "win.toggle-explorer"),
                        ("Outline", "win.outline"),
                        ("Backlinks", "win.backlinks"),
                    ],
                    &[
                        ("Editor", "win.view-editor"),
                        ("Split", "win.view-split"),
                        ("Preview", "win.view-preview"),
                    ],
                ],
            ),
            (
                "Project",
                &[&[("Preview Composition", "win.preview-composition")]],
            ),
            (
                "Help",
                &[&[("Atajos", "win.shortcuts"), ("About", "win.about")]],
            ),
        ];
        assert_eq!(MENUBAR_SPEC, esperado);
    }

    /// §20/ISSUE-022: las acciones de composición (Project ▸ Preview
    /// Composition y File ▸ Export Composition PDF) son las que exigen
    /// composiciones y por eso se deshabilitan sin proyecto.
    #[test]
    fn comandos_de_composicion_marcados_como_requirientes() {
        for cmd in ALL_COMMANDS {
            let esperado = matches!(
                cmd.action_id(),
                "win.preview-composition" | "win.export-composition-pdf"
            );
            assert_eq!(
                cmd.requires_composition(),
                esperado,
                "requires_composition desalineado en {}",
                cmd.action_id()
            );
        }
    }

    /// `build_menubar()` materializa [`MENUBAR_SPEC`] en `gio::Menu` con los
    /// mismos títulos, secciones, ítems y `action_id` (GMenu no necesita
    /// display; la introspección de `gio::MenuModel` tampoco).
    #[test]
    fn build_menubar_materializa_estructura_36() {
        let menubar = build_menubar();
        assert_eq!(menubar.n_items(), MENUBAR_SPEC.len() as i32);
        for (i, &(title, sections)) in MENUBAR_SPEC.iter().enumerate() {
            let i = i as i32;
            let titulo = menubar
                .item_attribute_value(i, "label", None)
                .expect("título del submenú");
            assert_eq!(titulo.str(), Some(title));
            let submenu = menubar.item_link(i, "submenu").expect("submenú");
            assert_eq!(submenu.n_items(), sections.len() as i32);
            for (j, entries) in sections.iter().enumerate() {
                let j = j as i32;
                let section = submenu.item_link(j, "section").expect("sección");
                assert_eq!(section.n_items(), entries.len() as i32);
                for (k, &(label, action_id)) in entries.iter().enumerate() {
                    let k = k as i32;
                    let etiqueta = section
                        .item_attribute_value(k, "label", None)
                        .expect("etiqueta del ítem");
                    let accion = section
                        .item_attribute_value(k, "action", None)
                        .expect("acción del ítem");
                    assert_eq!(etiqueta.str(), Some(label));
                    assert_eq!(accion.str(), Some(action_id));
                }
            }
        }
    }

    /// Coherencia §36/§37: todo ítem del menú corresponde a un comando
    /// registrado con su etiqueta y todo comando es alcanzable desde el menú.
    #[test]
    fn menu_y_comandos_coherentes() {
        let mut en_menu = HashSet::new();
        for &(_, sections) in MENUBAR_SPEC {
            for entries in sections {
                for &(label, action_id) in *entries {
                    let cmd = ALL_COMMANDS
                        .iter()
                        .find(|c| c.action_id() == action_id)
                        .unwrap_or_else(|| panic!("ítem sin comando: {action_id}"));
                    assert_eq!(cmd.label(), label, "etiqueta desalineada en {action_id}");
                    en_menu.insert(action_id);
                }
            }
        }
        for &cmd in ALL_COMMANDS {
            assert!(
                en_menu.contains(cmd.action_id()),
                "comando sin ítem de menú §36: {}",
                cmd.action_id()
            );
        }
    }

    /// Gramática de aceleradores de GTK: grupos `<Modificador>` seguidos de
    /// una tecla. Un literal inválido lo ignora GTK en silencio (el atajo
    /// queda muerto), así que la tabla §39 se valida contra la gramática.
    fn accel_es_valido(accel: &str) -> bool {
        let mut rest = accel;
        while let Some(start) = rest.find('<') {
            if start != 0 {
                return false; // texto suelto antes de un grupo
            }
            let Some(end) = rest.find('>') else {
                return false; // grupo sin cerrar
            };
            let modifier = &rest[1..end];
            if !matches!(
                modifier,
                "Control" | "Shift" | "Alt" | "Meta" | "Super" | "Primary"
            ) {
                return false;
            }
            rest = &rest[end + 1..];
        }
        // Tecla final: sin restos de grupo ni espacios.
        !rest.is_empty() && !rest.contains('>') && !rest.contains('<') && !rest.contains(' ')
    }

    #[test]
    fn aceleradores_siguen_la_gramatica_de_gtk() {
        for (action, accel) in ACCELERATORS {
            assert!(
                accel_es_valido(accel),
                "acelerador {accel:?} inválido para {action} (§39)"
            );
        }
        // El validador no es vacío: conoce los fallos reales.
        for malo in [
            "<Control>Alt>m",
            "<Controll>n",
            "<Control>",
            "Control>n",
            "",
        ] {
            assert!(!accel_es_valido(malo), "{malo:?} no debería validar");
        }
    }
}
