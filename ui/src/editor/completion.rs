//! Popup de destinos; la aceptación delega una única transacción al modelo.
use crate::intelligence::Completion;
use gtk::prelude::*;
use gtk4 as gtk;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub struct CompletionPopup {
    popover: gtk::Popover,
    list: gtk::ListBox,
    items: Rc<RefCell<Vec<Completion>>>,
    dismissed: Rc<Cell<bool>>,
    context: RefCell<String>,
}
impl CompletionPopup {
    pub fn new(view: &gtk::TextView, accept: Rc<dyn Fn(Completion)>) -> Self {
        let popover = gtk::Popover::new();
        popover.set_parent(view);
        popover.set_autohide(false);
        popover.set_has_arrow(false);
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_width(280);
        scroll.set_max_content_height(240);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&list));
        popover.set_child(Some(&scroll));
        let items: Rc<RefCell<Vec<Completion>>> = Rc::new(RefCell::new(Vec::new()));
        let dismissed = Rc::new(Cell::new(false));
        let dismiss = dismissed.clone();
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let popup = popover.clone();
        let rows = list.clone();
        let values = items.clone();
        let apply = accept.clone();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if !popup.is_visible() {
                return gtk::glib::Propagation::Proceed;
            }
            let selected = rows.selected_row().map(|r| r.index()).unwrap_or(0);
            match key {
                gtk::gdk::Key::Escape => {
                    dismiss.set(true);
                    popup.popdown();
                }
                gtk::gdk::Key::Up | gtk::gdk::Key::Down => {
                    let n = if key == gtk::gdk::Key::Up {
                        selected - 1
                    } else {
                        selected + 1
                    };
                    if let Some(row) = rows.row_at_index(n.max(0)) {
                        rows.select_row(Some(&row));
                    }
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::Tab if modifiers.is_empty() => {
                    let item = values.borrow().get(selected as usize).cloned();
                    popup.popdown();
                    if let Some(item) = item {
                        apply(item);
                    }
                }
                _ => {
                    popup.popdown();
                    return gtk::glib::Propagation::Proceed;
                }
            }
            gtk::glib::Propagation::Stop
        });
        view.add_controller(key);
        let values = items.clone();
        let popup = popover.clone();
        list.connect_row_activated(move |_, row| {
            let item = values.borrow().get(row.index() as usize).cloned();
            popup.popdown();
            if let Some(item) = item {
                accept(item);
            }
        });
        Self {
            popover,
            list,
            items,
            dismissed,
            context: RefCell::new(String::new()),
        }
    }
    pub fn show(&self, items: Vec<Completion>, view: &gtk::TextView) {
        let buffer = view.buffer();
        let (a, b) = buffer.bounds();
        let context = format!(
            "{}:{}",
            buffer.iter_at_mark(&buffer.get_insert()).offset(),
            buffer.text(&a, &b, true)
        );
        if *self.context.borrow() != context {
            self.dismissed.set(false);
            *self.context.borrow_mut() = context;
        }
        if self.dismissed.get() {
            return;
        }
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        for item in &items {
            let label = gtk::Label::new(Some(&item.label));
            label.set_xalign(0.0);
            self.list.append(&label);
        }
        let empty = items.is_empty();
        *self.items.borrow_mut() = items;
        if empty {
            self.popover.popdown();
            return;
        }
        self.list.select_row(self.list.row_at_index(0).as_ref());
        let buffer = view.buffer();
        let rect = view.iter_location(&buffer.iter_at_mark(&buffer.get_insert()));
        let (x, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
        self.popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            x,
            y,
            rect.width(),
            rect.height(),
        )));
        self.popover.popup();
        view.grab_focus();
    }
    pub fn hide(&self) {
        self.popover.popdown();
    }
}
impl Drop for CompletionPopup {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}
