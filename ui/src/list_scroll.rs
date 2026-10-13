//! Mantiene visible la selección de listas navegadas desde otro widget.
use gtk::prelude::*;
use gtk4 as gtk;

pub fn follow_selection(list: &gtk::ListBox) {
    list.connect_row_selected(|list, row| {
        if row.is_none() {
            return;
        }
        reveal_selected(list);
        // Tras repintar, la fila puede no tener aún su tamaño definitivo.
        list.add_tick_callback(|list, _| {
            reveal_selected(list);
            gtk::glib::ControlFlow::Break
        });
    });
}

fn reveal_selected(list: &gtk::ListBox) {
    let Some(row) = list.selected_row() else {
        return;
    };
    let Some(scroll) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
    else {
        return;
    };
    let Some(bounds) = row.compute_bounds(list) else {
        return;
    };
    if bounds.height() > 0.0 {
        scroll
            .vadjustment()
            .clamp_page(bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
    }
}

#[cfg(test)]
pub fn selected_visible(list: &gtk::ListBox, index: i32) -> bool {
    let Some(row) = list.selected_row().filter(|r| r.index() == index) else {
        return false;
    };
    let Some(scroll) = list
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_then(|w| w.downcast::<gtk::ScrolledWindow>().ok())
    else {
        return false;
    };
    let Some(bounds) = row.compute_bounds(list) else {
        return false;
    };
    let adjustment = scroll.vadjustment();
    bounds.height() > 0.0
        && adjustment.page_size() > 0.0
        && bounds.y() as f64 >= adjustment.value() - 1.0
        && (bounds.y() + bounds.height()) as f64
            <= adjustment.value() + adjustment.page_size() + 1.0
}
