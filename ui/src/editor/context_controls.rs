//! Popover de controles contextuales §44 (fase 7 #29).
//!
//! `gtk::Popover` anclado al `TextView` con el patrón de [`super::completion`]
//! (`set_parent(view)`, `has_arrow(false)`), que se muestra SOLO si aporta
//! utilidad según [`minmark_document::format::context`]:
//!
//! - selección no vacía → panel de texto (Heading 1–6, Bold, Italic,
//!   Strikethrough, Code, Link, Callout);
//! - cursor dentro de un enlace → «Edit link» (el diálogo de `win.insert-link`
//!   sale prellenado; Cancelar no toca el documento);
//! - cursor dentro de un callout → controles de tono (note, info, tip,
//!   warning, danger, success) para retonar;
//! - cursor en un heading → H1–H6;
//! - cursor plano sin selección → oculto.
//!
//! Todo botón activa la MISMA acción `win.*` que menú, atajos y paleta §38
//! (`view.activate_action`), con destino único [`TabManager::apply_format_active`]
//! (crate::tabs::TabManager::apply_format_active). El tono del callout viaja
//! en el estado de `win.insert-callout` (fija la acción antes de activarla).
//!
//! Integridad de pestaña: al abrir registra `(tab, modo)`; si la vista
//! recarga otro tab o modo ([`super::pane::EditorPane::load`] cierra el
//! popover) o el registro no concuerda, los handlers no aplican nada.
//! Escape cierra (vista o popover), Tab recorre los botones y Enter/Space
//! los activan (GTK nativo); `autohide(true)` cierra con un clic fuera.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;

use minmark_document::format::{self, EditContext};
use minmark_document::Selection;

use super::mode::EditMode;
use crate::menus::actions::Command;

/// Tono de callout §22 en el orden del popover.
const CALLOUT_TONES: [&str; 6] = ["note", "info", "tip", "warning", "danger", "success"];

/// Panel visible del popover según el contexto.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Panel {
    Text,
    Link,
    Callout,
    Heading,
}

/// Setter del tono de `win.insert-callout`: fija el estado de la acción en
/// el grupo «win» (enlazado por main.rs, referencia débil B1).
type ToneSetter = Rc<RefCell<Option<Rc<dyn Fn(String)>>>>;

pub struct ContextControls {
    popover: gtk::Popover,
    heading_row: gtk::Box,
    inline_row: gtk::Box,
    action_row: gtk::Box,
    link_row: gtk::Box,
    tone_row: gtk::Box,
    headings: [gtk::ToggleButton; 6],
    strong: gtk::ToggleButton,
    emphasis: gtk::ToggleButton,
    strikethrough: gtk::ToggleButton,
    code: gtk::ToggleButton,
    tones: [gtk::ToggleButton; 6],
    /// `(tab, modo)` con los que se abrió; `None` = cerrado. Compartido con
    /// los handlers, que no aplican nada si no está abierto.
    opened: Rc<Cell<Option<(usize, EditMode)>>>,
    /// Fija el estado de tono de `win.insert-callout` antes de activarla
    /// (enlazado por main.rs; ver [`Self::set_tone_setter`]).
    set_tone: ToneSetter,
    /// Botones por nombre accesible (labels de [`Command`] y tonos §22).
    #[cfg(test)]
    buttons: Vec<(&'static str, gtk::Button)>,
}

impl ContextControls {
    /// Crea el popover anclado a `view` (oculto hasta [`Self::update`]).
    pub fn new(view: &gtk::TextView) -> Self {
        let popover = gtk::Popover::new();
        popover.set_parent(view);
        popover.set_autohide(true);
        popover.set_has_arrow(false);
        // Teclado §29/#29: el popover y sus botones deben poder recibir
        // foco (Tab los recorre, Enter/Space los activa, Escape cierra y
        // devuelve el foco al editor); el foco inicial se queda en la vista
        // y `Ctrl+.` entra en los controles.
        popover.set_can_focus(true);

        let opened: Rc<Cell<Option<(usize, EditMode)>>> = Rc::new(Cell::new(None));
        // Setter del tono (main.rs → grupo «win»): el popover solo activa
        // `win.insert-callout`; fijar el estado de la acción es del
        // integrador, que conserva el grupo (B1, referencia débil).
        let set_tone: ToneSetter = Rc::new(RefCell::new(None));

        // Cerrar (clic fuera, autohide) invalida el registro de pestaña.
        popover.connect_closed({
            let opened = Rc::clone(&opened);
            move |_| opened.set(None)
        });

        // Fila de headings (H1–H6): visible también con el cursor en un
        // heading. Etiqueta visible corta; nombre accesible = label §36.
        let heading_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut headings: [gtk::ToggleButton; 6] = [
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
        ];
        let heading_commands = [
            Command::FormatHeading1,
            Command::FormatHeading2,
            Command::FormatHeading3,
            Command::FormatHeading4,
            Command::FormatHeading5,
            Command::FormatHeading6,
        ];
        for (index, button) in headings.iter_mut().enumerate() {
            let name = heading_commands[index].label();
            button.set_label(&format!("H{}", index + 1));
            button.set_tooltip_text(Some(name));
            button.update_property(&[gtk::accessible::Property::Label(name)]);
            heading_row.append(button);
        }

        // Fila inline: Bold, Italic, Strikethrough, Code (los toggles
        // exponen su estado activo a AT, no solo color/hover).
        let inline_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let toggles = [
            ("B", Command::FormatStrong),
            ("I", Command::FormatEmphasis),
            ("S", Command::FormatStrikethrough),
            ("C", Command::FormatCode),
        ]
        .map(|(glyph, cmd)| {
            let button = gtk::ToggleButton::with_label(glyph);
            button.set_tooltip_text(Some(cmd.label()));
            button.update_property(&[gtk::accessible::Property::Label(cmd.label())]);
            inline_row.append(&button);
            button
        });
        let [strong, emphasis, strikethrough, code] = toggles;

        // Fila de acciones del panel de texto: Link y Callout.
        let action_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let link = gtk::Button::with_label(Command::InsertLink.label());
        link.update_property(&[gtk::accessible::Property::Label(Command::InsertLink.label())]);
        let callout = gtk::Button::with_label(Command::InsertCallout.label());
        callout.update_property(&[gtk::accessible::Property::Label(
            Command::InsertCallout.label(),
        )]);
        action_row.append(&link);
        action_row.append(&callout);

        // Fila «Edit link» (cursor dentro de un enlace).
        let link_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let edit_link = gtk::Button::with_label("Edit link");
        edit_link.update_property(&[gtk::accessible::Property::Label("Edit link")]);
        link_row.append(&edit_link);

        // Fila de tonos (cursor dentro de un callout, §22).
        let tone_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut tones: [gtk::ToggleButton; 6] = [
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
            gtk::ToggleButton::new(),
        ];
        for (index, button) in tones.iter_mut().enumerate() {
            let tone = CALLOUT_TONES[index];
            button.set_label(tone);
            button.update_property(&[gtk::accessible::Property::Label(tone)]);
            tone_row.append(button);
        }

        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.append(&heading_row);
        root.append(&inline_row);
        root.append(&action_row);
        root.append(&link_row);
        root.append(&tone_row);
        popover.set_child(Some(&root));

        // Handlers: guardia de pestaña y activación de la acción win.*
        // compartida (menú/atajos/paleta/popover → apply_format_active).
        for (index, button) in headings.iter().enumerate() {
            activate_action_on_click(
                button,
                &popover,
                &opened,
                view,
                heading_commands[index].action_id(),
            );
        }
        for (button, action_id) in [
            (&strong, Command::FormatStrong.action_id()),
            (&emphasis, Command::FormatEmphasis.action_id()),
            (&strikethrough, Command::FormatStrikethrough.action_id()),
            (&code, Command::FormatCode.action_id()),
        ] {
            activate_action_on_click(button, &popover, &opened, view, action_id);
        }
        for (button, action_id) in [
            (&link, Command::InsertLink.action_id()),
            (&callout, Command::InsertCallout.action_id()),
            // «Edit link» reutiliza el MISMO `win.insert-link`: el diálogo se
            // precarga con la URL del contexto (main.rs).
            (&edit_link, Command::InsertLink.action_id()),
        ] {
            activate_action_on_click(button, &popover, &opened, view, action_id);
        }

        // Tono de callout: fija el estado de `win.insert-callout` (vía el
        // setter que main.rs enlaza al grupo «win») y la activa — la MISMA
        // acción stateful sin parámetro de menú y paleta (§38).
        for (index, button) in tones.iter().enumerate() {
            let tone = CALLOUT_TONES[index];
            let popover = popover.clone();
            let opened = Rc::clone(&opened);
            let set_tone = Rc::clone(&set_tone);
            let view = view.downgrade();
            button.connect_clicked(move |_| {
                if !guard_abierto(&popover, &opened) {
                    popover.popdown();
                    return;
                }
                if let Some(setter) = set_tone.borrow().as_ref() {
                    setter(tone.to_owned());
                }
                if let Some(view) = view.upgrade() {
                    let _ = view.activate_action(Command::InsertCallout.action_id(), None);
                }
            });
        }

        // Escape desde un botón del popover: cierra y devuelve el foco al
        // editor sin tocar el documento (§UI «cero cambios»).
        {
            let esc_popover = popover.clone();
            let esc_view = view.downgrade();
            let opened = Rc::clone(&opened);
            let key = gtk::EventControllerKey::new();
            key.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape && esc_popover.is_visible() {
                    esc_popover.popdown();
                    opened.set(None);
                    if let Some(view) = esc_view.upgrade() {
                        view.grab_focus();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            popover.add_controller(key);
        }
        // Escape desde la vista cierra; `Ctrl+.` (§29, revisión #29) lleva
        // el foco del editor al primer control visible cuando el popover
        // está abierto — la única puerta de teclado a los controles, ya que
        // son una superficie aparte.
        {
            let rows = [
                heading_row.clone(),
                inline_row.clone(),
                action_row.clone(),
                link_row.clone(),
                tone_row.clone(),
            ];
            let popover = popover.clone();
            let opened = Rc::clone(&opened);
            let key = gtk::EventControllerKey::new();
            key.set_propagation_phase(gtk::PropagationPhase::Capture);
            key.connect_key_pressed(move |_, key, _, modifiers| {
                if key == gtk::gdk::Key::Escape && popover.is_visible() {
                    popover.popdown();
                    opened.set(None);
                    return glib::Propagation::Stop;
                }
                if key == gtk::gdk::Key::period
                    && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                {
                    if popover.is_visible() {
                        focus_first_control(&rows);
                        return glib::Propagation::Stop;
                    }
                    return glib::Propagation::Proceed;
                }
                glib::Propagation::Proceed
            });
            view.add_controller(key);
        }

        #[cfg(test)]
        let buttons = {
            let mut buttons: Vec<(&'static str, gtk::Button)> = heading_commands
                .iter()
                .enumerate()
                .map(|(index, cmd)| (cmd.label(), headings[index].clone().upcast::<gtk::Button>()))
                .collect();
            buttons.extend(
                [
                    (Command::FormatStrong, &strong),
                    (Command::FormatEmphasis, &emphasis),
                    (Command::FormatStrikethrough, &strikethrough),
                    (Command::FormatCode, &code),
                ]
                .map(|(cmd, button)| (cmd.label(), button.clone().upcast::<gtk::Button>())),
            );
            buttons.push((Command::InsertLink.label(), link.upcast::<gtk::Button>()));
            buttons.push((
                Command::InsertCallout.label(),
                callout.upcast::<gtk::Button>(),
            ));
            buttons.push(("Edit link", edit_link.upcast::<gtk::Button>()));
            for (index, button) in tones.iter().enumerate() {
                buttons.push((
                    CALLOUT_TONES[index],
                    button.clone().upcast::<gtk::Button>(),
                ));
            }
            buttons
        };

        Self {
            popover,
            heading_row,
            inline_row,
            action_row,
            link_row,
            tone_row,
            headings,
            strong,
            emphasis,
            strikethrough,
            code,
            tones,
            opened,
            set_tone,
            #[cfg(test)]
            buttons,
        }
    }

    /// Enlaza el fijado del estado de tono de `win.insert-callout`
    /// (main.rs lo resuelve sobre el grupo «win» que instaló con
    /// [`crate::menus::install_actions`]).
    pub fn set_tone_setter(&self, setter: Rc<dyn Fn(String)>) {
        *self.set_tone.borrow_mut() = Some(setter);
    }

    /// Recalcula [`format::context`] y muestra/actualiza/cierra el popover
    /// para `(tab, mode, source, selection)` — el estado vigente que el
    /// integrador (main.rs) leyó del modelo. Solo muestra si aporta
    /// utilidad; un `(tab, mode)` distinto al registrado cierra sin reabrir.
    pub fn update(
        &self,
        view: &gtk::TextView,
        source: &str,
        selection: Selection,
        tab: usize,
        mode: EditMode,
    ) {
        if self
            .opened
            .get()
            .is_some_and(|opened| opened != (tab, mode))
        {
            // Cambió la pestaña o el modo con el popover abierto: se cierra
            // y no se reabre con contexto de otra pestaña (§UI).
            self.hide();
            return;
        }
        let ctx = format::context(source, selection);
        let panel = if !selection.is_empty() {
            Panel::Text
        } else if ctx.link.is_some() {
            Panel::Link
        } else if ctx.callout.is_some() {
            Panel::Callout
        } else if ctx.heading_level.is_some() {
            Panel::Heading
        } else {
            // Cursor plano sin selección: sin utilidad (§UI), oculto.
            self.hide();
            return;
        };
        self.render(&ctx, panel);
        if !self.popover.is_visible() {
            self.opened.set(Some((tab, mode)));
            self.point(view);
            self.popover.popup();
            // El foco vuelve al editor: escribir tras seleccionar sigue
            // escribiendo en el documento (patrón del completion).
            view.grab_focus();
        }
    }

    /// Cierra el popover (Escape, clic fuera o recarga de pestaña/modo).
    pub fn hide(&self) {
        self.popover.popdown();
    }

    #[cfg(test)]
    pub fn is_visible(&self) -> bool {
        self.popover.is_visible()
    }

    /// Renderiza el panel `ctx` según el contexto; los toggles reflejan el
    /// estado activo (`format::context`), expuesto a AT como estado pulsado.
    fn render(&self, ctx: &EditContext, panel: Panel) {
        self.heading_row
            .set_visible(matches!(panel, Panel::Text | Panel::Heading));
        self.inline_row.set_visible(panel == Panel::Text);
        self.action_row.set_visible(panel == Panel::Text);
        self.link_row.set_visible(panel == Panel::Link);
        self.tone_row.set_visible(panel == Panel::Callout);

        for (index, button) in self.headings.iter().enumerate() {
            button.set_active(ctx.heading_level == Some(index as u8 + 1));
        }
        self.strong.set_active(ctx.strong);
        self.emphasis.set_active(ctx.emphasis);
        self.strikethrough.set_active(ctx.strikethrough);
        self.code.set_active(ctx.code);
        let actual = ctx
            .callout
            .as_ref()
            .map(|callout| callout.callout_type.as_str());
        for (index, button) in self.tones.iter().enumerate() {
            button.set_active(actual == Some(CALLOUT_TONES[index]));
        }
    }

    /// Ancla el popover a la línea del cursor/selección (patrón del
    /// completion, sin flecha).
    fn point(&self, view: &gtk::TextView) {
        let buffer = view.buffer();
        let rect = view.iter_location(&buffer.iter_at_mark(&buffer.get_insert()));
        let (x, y) = view.buffer_to_window_coords(gtk::TextWindowType::Widget, rect.x(), rect.y());
        self.popover
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x,
                y,
                rect.width(),
                rect.height(),
            )));
    }

    #[cfg(test)]
    pub fn test_button(&self, name: &str) -> Option<gtk::Button> {
        self.buttons
            .iter()
            .find(|(button_name, _)| *button_name == name)
            .map(|(_, button)| button.clone())
    }

    /// ¿El popover admite foco? (§29: operable con teclado.)
    #[cfg(test)]
    pub fn test_can_focus(&self) -> bool {
        self.popover.can_focus()
    }

    /// ¿Algún control es el foco de su superficie? (`Ctrl+.` debe moverlo
    /// aquí; `is_focus` es el foco raíz de la superficie del popover, el
    /// observable sin window manager.)
    #[cfg(test)]
    pub fn test_focus_inside(&self) -> bool {
        self.buttons.iter().any(|(_, button)| button.is_focus())
    }

    /// Emite `clicked` sobre el botón `name`; `false` si no existe (la
    /// guardia de pestaña vive en el handler, no aquí).
    #[cfg(test)]
    pub fn test_click(&self, name: &str) -> bool {
        match self.test_button(name) {
            Some(button) => {
                button.emit_clicked();
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub fn test_tone_state(&self) -> Option<String> {
        self.tones
            .iter()
            .find(|button| button.is_active())
            .and_then(|button| button.label())
            .map(|label| label.to_string())
    }
}

impl Drop for ContextControls {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}

/// Guardia de integridad: el popover está abierto con una pestaña registrada.
fn guard_abierto(popover: &gtk::Popover, opened: &Cell<Option<(usize, EditMode)>>) -> bool {
    popover.is_visible() && opened.get().is_some()
}

/// Foco de teclado dentro de los controles: `Ctrl+.` (§29) lleva el foco al
/// primer control FOCOABLE de la fila visible de arriba abajo (las filas
/// ocultas no tienen botones operables).
fn focus_first_control(rows: &[gtk::Box]) {
    for row in rows {
        if !row.is_visible() {
            continue;
        }
        let mut child = row.first_child();
        while let Some(widget) = child {
            if widget.is_visible() && widget.can_focus() {
                widget.grab_focus();
                return;
            }
            child = widget.next_sibling();
        }
    }
}

/// Conecta un botón a la acción `win.*` `action_id`: mismo camino que menú,
/// atajos y paleta §38, con guardia de pestaña (sin registro ⇒ nada que
/// aplicar, §UI «cambio de pestaña sin aplicar una acción al documento
/// equivocado»).
fn activate_action_on_click(
    button: &impl IsA<gtk::Button>,
    popover: &gtk::Popover,
    opened: &Rc<Cell<Option<(usize, EditMode)>>>,
    view: &gtk::TextView,
    action_id: &'static str,
) {
    let popover = popover.clone();
    let opened = Rc::clone(opened);
    let view = view.downgrade();
    button.connect_clicked(move |_| {
        if !guard_abierto(&popover, &opened) {
            popover.popdown();
            return;
        }
        if let Some(view) = view.upgrade() {
            let _ = view.activate_action(action_id, None);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::actions::DEFAULT_CALLOUT_TONE;

    #[test]
    fn panel_de_texto_solo_con_seleccion_no_vacia() {
        // Lógica de visibilidad sin GTK: la decisión es pura sobre
        // `selection` + [`format::context`] (el resto vive en `update`).
        let source = "hola mundo\n";
        let seleccion = Selection {
            anchor: minmark_document::Position::new(0, 0),
            head: minmark_document::Position::new(0, 4),
        };
        let ctx = format::context(source, seleccion);
        assert!(!seleccion.is_empty());
        assert!(ctx.link.is_none() && ctx.callout.is_none());

        let cursor = minmark_document::Position::new(1, 0);
        let plano = format::context(source, minmark_document::Selection::collapsed(cursor));
        assert!(plano.link.is_none());
        assert!(plano.callout.is_none());
        assert!(plano.heading_level.is_none(), "cursor en línea vacía: nada");
    }

    #[test]
    fn context_detecta_link_callout_y_heading() {
        let source = "# Título\n\n[ir](https://ejemplo.com)\n\n::: note\naviso\n:::\n";
        // Cursor en el heading (fila 0).
        let heading = format::context(
            source,
            Selection::collapsed(minmark_document::Position::new(0, 2)),
        );
        assert_eq!(heading.heading_level, Some(1));
        // Cursor dentro del enlace (fila 2).
        let link = format::context(
            source,
            Selection::collapsed(minmark_document::Position::new(2, 5)),
        );
        let link = link.link.expect("enlace bajo el cursor");
        assert_eq!(link.url, "https://ejemplo.com");
        // Cursor dentro del callout (fila 5).
        let callout = format::context(
            source,
            Selection::collapsed(minmark_document::Position::new(5, 2)),
        );
        assert_eq!(
            callout.callout.expect("callout bajo el cursor").callout_type,
            DEFAULT_CALLOUT_TONE
        );
    }
}
