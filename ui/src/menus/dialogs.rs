//! Diálogos GTK4 asíncronos de minmark (ISSUE-020).
//!
//! - [`open_file`] / [`save_file_as`]: `gtk::FileDialog` con filtros Markdown
//!   (*.md / *.markdown); el de guardar sugiere nombre (§12.4 Save As).
//! - [`confirm_discard`]: confirmación explícita antes de descartar cambios
//!   sin guardar (§3.7: nada destructivo es silencioso).
//! - [`confirm_overwrite_external`]: sobrescritura solo con consentimiento
//!   explícito cuando el archivo cambió en disco (§12.7).
//!
//! Todos devuelven futuros vía la API real de gtk4 0.9 (`open_future`,
//! `save_future` y `choose_future` de `gtk::FileDialog` / `gtk::AlertDialog`,
//! GTK ≥ 4.10); sin main loop bloqueante y sin fetch de red (§3.4).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

fn file_dialog_available(parent: &impl IsA<gtk::Window>) -> bool {
    if let Err(message) = crate::runtime::check_file_dialog() {
        gtk::AlertDialog::builder()
            .message("No se puede abrir el selector de archivos")
            .detail(&message)
            .build()
            .show(Some(parent));
        false
    } else {
        true
    }
}

/// Filtro Markdown (*.md / *.markdown + MIME `text/markdown`) para los diálogos.
fn markdown_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Markdown (*.md, *.markdown)"));
    filter.add_pattern("*.md");
    filter.add_pattern("*.markdown");
    filter.add_mime_type("text/markdown");
    filter
}

fn pdf_filter() -> gtk::FileFilter {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("PDF (*.pdf)"));
    filter.add_pattern("*.pdf");
    filter.add_mime_type("application/pdf");
    filter
}

/// Diálogo de apertura (§36, File ▸ Open): filtra archivos Markdown
/// (*.md / *.markdown), con «Todos los archivos» como respaldo. Devuelve el
/// path elegido o `None` si se cancela.
pub async fn open_file(parent: &impl IsA<gtk::Window>) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Abrir archivo");
    dialog.set_modal(true);

    let markdown = markdown_filter();
    let all = gtk::FileFilter::new();
    all.set_name(Some("Todos los archivos"));
    all.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&markdown);
    filters.append(&all);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&markdown));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.open_future(Some(parent)).await.ok()?;
    file.path()
}

/// Diálogo de apertura de carpeta (ISSUE-021, File ▸ Open Folder): la
/// carpeta elegida se abre en una ventana de proyecto nueva. Devuelve la
/// carpeta o `None` si se cancela.
pub async fn open_folder(parent: &impl IsA<gtk::Window>) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Abrir carpeta");
    dialog.set_modal(true);
    let parent = parent.upcast_ref::<gtk::Window>();
    let folder = dialog.select_folder_future(Some(parent)).await.ok()?;
    folder.path()
}

/// Diálogo «Guardar como» (§12.4): sugiere `suggested` como nombre inicial.
/// Devuelve el path elegido o `None` si se cancela.
pub async fn save_file_as(parent: &impl IsA<gtk::Window>, suggested: &str) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Guardar como");
    dialog.set_modal(true);
    if !suggested.is_empty() {
        dialog.set_initial_name(Some(suggested));
    }

    let markdown = markdown_filter();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&markdown);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&markdown));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.save_future(Some(parent)).await.ok()?;
    file.path()
}

/// Diálogo «Exportar PDF» (§32.1, ISSUE-035): sugiere `suggested` como nombre
/// inicial y filtra `*.pdf`. Devuelve el path elegido o `None` si se cancela.
pub async fn save_pdf(parent: &impl IsA<gtk::Window>, suggested: &str) -> Option<PathBuf> {
    if !file_dialog_available(parent) {
        return None;
    }
    let dialog = gtk::FileDialog::new();
    dialog.set_title("Exportar PDF");
    dialog.set_modal(true);
    if !suggested.is_empty() {
        dialog.set_initial_name(Some(suggested));
    }

    let pdf = pdf_filter();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&pdf);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&pdf));

    let parent = parent.upcast_ref::<gtk::Window>();
    let file = dialog.save_future(Some(parent)).await.ok()?;
    file.path()
}

/// Confirmación de descarte de cambios sin guardar (§3.7).
///
/// `title` identifica el documento afectado. Solo el botón «Descartar»
/// devuelve `true`; Cancelar, Escape o cerrar el diálogo devuelven `false`
/// (nunca se descarta en silencio).
pub async fn confirm_discard(parent: &impl IsA<gtk::Window>, title: &str) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .message("¿Descartar cambios sin guardar?")
        .detail(format!("Se perderán los cambios de {title}."))
        .buttons(["Descartar", "Cancelar"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let parent = parent.upcast_ref::<gtk::Window>();
    matches!(dialog.choose_future(Some(parent)).await, Ok(0))
}

/// §12.7: el archivo cambió en disco; sobrescribir solo con consentimiento explícito.
///
/// Solo el botón «Sobrescribir» devuelve `true`; en cualquier otro caso se
/// conserva el archivo externo y el usuario decide cómo continuar.
pub async fn confirm_overwrite_external(parent: &impl IsA<gtk::Window>, path: &Path) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .message("El archivo cambió en disco")
        .detail(format!(
            "«{}» fue modificado fuera de minmark. ¿Sobrescribirlo con tu versión?",
            path.display()
        ))
        .buttons(["Sobrescribir", "Cancelar"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let parent = parent.upcast_ref::<gtk::Window>();
    matches!(dialog.choose_future(Some(parent)).await, Ok(0))
}

/// Confirmación de envío a la papelera (§14.9, §3.7: nada destructivo en
/// silencio). Solo «Enviar a la papelera» devuelve `true`.
pub async fn confirm_trash(parent: &impl IsA<gtk::Window>, name: &str) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .message("¿Enviar a la papelera?")
        .detail(format!(
            "«{name}» se enviará a la papelera del sistema (§14.9); no se \
             elimina permanentemente."
        ))
        .buttons(["Enviar a la papelera", "Cancelar"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let parent = parent.upcast_ref::<gtk::Window>();
    matches!(dialog.choose_future(Some(parent)).await, Ok(0))
}

/// Prompt de texto modal para operaciones del explorador (§14.8): nombre de
/// Markdown/carpeta, nuevo nombre en `Rename` o destino en `Move`.
///
/// `on_submit(Some(text))` al aceptar (botón o Enter), `on_submit(None)` al
/// cancelar. Callback y no futuro: las operaciones del explorador son
/// cadenas de diálogos con estado propio (ISSUE-023).
pub fn prompt_text(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    initial: &str,
    on_submit: Rc<dyn Fn(Option<String>)>,
) {
    let window = gtk::Window::builder()
        .title(title)
        .modal(true)
        .transient_for(parent)
        .default_width(360)
        .resizable(false)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(true);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&entry);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    {
        let window = window.clone();
        let entry = entry.clone();
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let text = entry.text().to_string();
            window.destroy();
            on_submit(Some(text));
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}

/// Selección de composición (§20, ISSUE-022/023): lista modal con las
/// composiciones del proyecto, en el patrón de [`prompt_text`].
///
/// `on_submit(Some(nombre))` al aceptar la fila seleccionada (doble click o
/// Aceptar), `on_submit(None)` al cancelar (botón, Escape o cierre). El
/// diálogo es callback y no futuro para encadenarse con los futuros del
/// flujo de exportación.
pub fn choose_composition(
    parent: &impl IsA<gtk::Window>,
    names: &[String],
    on_submit: Rc<dyn Fn(Option<String>)>,
) {
    let window = gtk::Window::builder()
        .title("Composición")
        .modal(true)
        .transient_for(parent)
        .default_width(360)
        .resizable(false)
        .build();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    for name in names {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        list.append(&row);
    }
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_min_content_height(180);
    scroll.set_vexpand(true);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancelar");
    let accept = gtk::Button::with_label("Aceptar");
    accept.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&accept);

    vbox.append(&scroll);
    vbox.append(&buttons);
    window.set_default_widget(Some(&accept));
    window.set_child(Some(&vbox));

    // Nombre de la fila seleccionada; `None` si no hay fila. La lista es
    // propia del diálogo (los callbacks GTK son `'static`).
    let nombres = names.to_vec();
    let seleccionada = {
        let list = list.clone();
        Rc::new(move || -> Option<String> {
            let row = list.selected_row()?;
            let row = row.index().max(0) as usize;
            nombres.get(row).cloned()
        })
    };
    {
        let window = window.clone();
        let seleccionada = seleccionada.clone();
        let on_submit = Rc::clone(&on_submit);
        accept.connect_clicked(move |_| {
            let nombre = seleccionada();
            window.destroy();
            on_submit(nombre);
        });
    }
    // Doble click sobre una fila: acepta ESA fila (la selecciona primero).
    {
        let para_seleccionar = list.clone();
        let seleccionada = seleccionada.clone();
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        list.connect_row_activated(move |_, row| {
            para_seleccionar.select_row(Some(row));
            let nombre = seleccionada();
            window.destroy();
            on_submit(nombre);
        });
    }
    {
        let window = window.clone();
        let on_submit = Rc::clone(&on_submit);
        cancel.connect_clicked(move |_| {
            window.destroy();
            on_submit(None);
        });
    }
    // Escape y el cierre de la ventana cancelan (§3.7: nunca en silencio
    // seleccionan una por defecto).
    {
        let on_escape = window.clone();
        let on_submit = Rc::clone(&on_submit);
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed(move |_, key, _, _| {
            if key == gtk4::gdk::Key::Escape {
                on_escape.destroy();
                on_submit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }
    window.connect_close_request({
        let on_submit = Rc::clone(&on_submit);
        move |_| {
            on_submit(None);
            glib::Propagation::Proceed
        }
    });

    window.present();
}
