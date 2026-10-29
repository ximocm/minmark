//! Panel de edición GTK4: Markdown Mode (ISSUE-015) + modo Visual (§8.2,
//! ISSUE-030) sobre el MISMO `TextBuffer`.
//!
//! Puente TextView ↔ modelo (core `Editor`): el documento Markdown es la
//! fuente de verdad y el buffer único (§6.1) tiene DOS espejos — el
//! `gtk::TextBuffer` de la vista y el `Editor` del modelo — no dos versiones:
//! cada señal de edición del TextView replica en el modelo a través de
//! [`EditSink`] y viceversa [`EditorPane::load`] recarga la vista.
//!
//! Los dos modos son DOS PLANES DE TAGS excluyentes sobre el mismo buffer
//! (decisión ISSUE-030): Markdown aplica el highlighting de
//! [`minmark_parser::highlight`]; Visual oculta la sintaxis (tag invisible de
//! ancho cero, §8.3) y estiliza el contenido semántico (§8.2). Editar en
//! Visual es editar el buffer: §6.2 (sin reformatear), §8.5 (undo único) y
//! §8.6 (cursor/selección) salen gratis.
//!
//! - §8.3: `visual_syntax` = ocultar siempre, o revelar temporalmente la
//!   sintaxis del elemento (bloque) actualmente editado.
//! - §8.4: lo no representado por el plan visual queda como bloque Markdown
//!   editable (su sintaxis no se oculta).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib::translate::IntoGlib;
use gtk::pango;
use gtk::prelude::*;
use gtk::{ScrolledWindow, TextBuffer, TextIter, TextTag, TextTagTable, TextView, Widget};
use gtk4 as gtk;
use minmark_config::VisualSyntax;
use minmark_document::{Editor, Position, Range, Selection};
use minmark_parser::VisualStyle;
use minmark_renderer::Palette;

use super::mode::EditMode;
use super::{tags, visual};

/// Receptor de ediciones del TextView hacia el modelo (core `Editor`).
///
/// El TextView sigue aplicando los cambios a su `TextBuffer`; el sink replica
/// la misma operación en el modelo (dos espejos del mismo buffer §6.1).
pub trait EditSink {
    fn insert_at(&self, at: Position, text: &str);
    fn delete_range(&self, from: Position, to: Position);
    fn set_selection(&self, selection: Selection);
}

/// Estado §8.3 «reveal-current» del modo Visual, compartido con el handler
/// de `mark-set`: qué rangos de sintaxis están ocultos y qué elemento tiene
/// la sintaxis revelada mientras se edita.
struct RevealState {
    buffer: TextBuffer,
    syntax_tag: TextTag,
    mode: Cell<EditMode>,
    visual_syntax: Cell<VisualSyntax>,
    /// Rangos ocultables (chars) del plan visual actual.
    hide: RefCell<Vec<(usize, usize)>>,
    /// Elementos (bloques) en chars, en orden de aparición.
    elements: RefCell<Vec<(usize, usize)>>,
    /// Elemento con la sintaxis revelada actualmente.
    revealed: Cell<Option<(usize, usize)>>,
}

impl RevealState {
    /// Revela la sintaxis del elemento más interno bajo el cursor y vuelve a
    /// ocultar el que estaba revelado (§8.3). En modo `Hidden` o Markdown no
    /// hace nada: la sintaxis permanece oculta (o no hay tag de sintaxis).
    fn update(&self) {
        if self.mode.get() != EditMode::Visual
            || self.visual_syntax.get() != VisualSyntax::RevealCurrent
        {
            return;
        }
        let cursor = self.buffer.iter_at_mark(&self.buffer.get_insert()).offset() as usize;
        let elements = self.elements.borrow();
        let new = innermost(&elements, cursor);
        let old = self.revealed.get();
        if new == old {
            return;
        }
        let hide = self.hide.borrow();
        if let Some(b) = new {
            self.remove(b);
        }
        if let Some(a) = old {
            if Some(a) != new {
                self.remove(a);
                for range in rehide_ranges(&hide, a, new) {
                    self.apply(range);
                }
            }
        }
        self.revealed.set(new);
    }

    fn apply(&self, range: (usize, usize)) {
        let (from, to) = self.iters(range);
        self.buffer.apply_tag(&self.syntax_tag, &from, &to);
    }

    fn remove(&self, range: (usize, usize)) {
        let (from, to) = self.iters(range);
        self.buffer.remove_tag(&self.syntax_tag, &from, &to);
    }

    fn iters(&self, range: (usize, usize)) -> (TextIter, TextIter) {
        (
            self.buffer.iter_at_offset(range.0 as i32),
            self.buffer.iter_at_offset(range.1 as i32),
        )
    }
}

/// Elemento más interno (span mínimo) que contiene `cursor` (§8.3).
fn innermost(elements: &[(usize, usize)], cursor: usize) -> Option<(usize, usize)> {
    elements
        .iter()
        .copied()
        .filter(|(start, end)| *start <= cursor && cursor < *end)
        .min_by_key(|(start, end)| end - start)
}

/// Rangos ocultables dentro de `element` que hay que volver a ocultar al
/// salir de él: los que no caen dentro del elemento `excluded` (el nuevo
/// elemento revelado, que permanece visible).
fn rehide_ranges(
    hide: &[(usize, usize)],
    element: (usize, usize),
    excluded: Option<(usize, usize)>,
) -> Vec<(usize, usize)> {
    hide.iter()
        .copied()
        .filter(|range| {
            range.0 >= element.0
                && range.1 <= element.1
                && !excluded.is_some_and(|(s, e)| range.0 >= s && range.1 <= e)
        })
        .collect()
}

/// Panel de edición: un `TextView` monoespaciado sobre el buffer Markdown
/// único (§6.1) con los dos modos §8 (Markdown y Visual) como planes de tags.
pub struct EditorPane {
    root: gtk::Box,
    buffer: TextBuffer,
    text_view: TextView,
    /// Un `gtk::TextTag` por [`minmark_parser::HighlightKind`] ya estilizado
    /// (Markdown Mode).
    tags: Vec<TextTag>,
    /// Tags del modo Visual (ISSUE-030): un `gtk::TextTag` por estilo
    /// semántico, más el de sintaxis ocultable (§8.3).
    vis_tags: Vec<(VisualStyle, TextTag)>,
    vis_syntax: TextTag,
    /// Marcado discreto de coincidencias y resaltado de la activa (§40.1).
    find_match: TextTag,
    find_active: TextTag,
    broken_link: TextTag,
    wiki_links: Cell<bool>,
    completion: RefCell<Option<super::completion::CompletionPopup>>,
    /// Controles contextuales §44 (fase 7 #29): popover anclado al TextView
    /// que sigue a cursor/selección vía [`EditorPane::update_context`].
    context: RefCell<Option<super::context_controls::ContextControls>>,
    /// Guardia de reentrancia: las reescrituras programáticas del buffer
    /// (`load`) no invocan al [`EditSink`] ni a `on_edited`.
    guard: Rc<Cell<bool>>,
    /// Estado §8.3 (reveal-current), compartido con `mark-set`.
    reveal: Rc<RevealState>,
}

impl EditorPane {
    pub fn scroll_to_cursor(&self) {
        self.text_view
            .scroll_to_mark(&self.buffer.get_insert(), 0.15, false, 0.0, 0.0);
    }
    #[cfg(test)]
    pub fn emit_test_key(&self, key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
        let controllers = self.text_view.observe_controllers();
        for i in 0..controllers.n_items() {
            if let Some(controller) = controllers
                .item(i)
                .and_then(|c| c.downcast::<gtk::EventControllerKey>().ok())
            {
                if controller.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers]) {
                    return true;
                }
            }
        }
        false
    }
    pub fn connect_completion(&self, accept: Rc<dyn Fn(crate::intelligence::Completion)>) {
        *self.completion.borrow_mut() = Some(super::completion::CompletionPopup::new(
            &self.text_view,
            accept,
        ));
    }
    pub fn show_completions(&self, items: Vec<crate::intelligence::Completion>) {
        if let Some(popup) = self.completion.borrow().as_ref() {
            popup.show(items, &self.text_view);
        }
    }
    #[cfg(test)]
    pub fn test_completion_selected_visible(&self, index: i32) -> bool {
        self.completion
            .borrow()
            .as_ref()
            .is_some_and(|p| p.test_selected_visible(index))
    }

    /// Actualiza los controles contextuales §44 (fase 7 #29) con el estado
    /// vigente `(tab, mode, source, selection)` leído del modelo: muestra el
    /// popover solo si aporta utilidad, lo actualiza o lo cierra.
    pub fn update_context(
        &self,
        source: &str,
        selection: Selection,
        tab: usize,
        mode: EditMode,
    ) {
        if let Some(controls) = self.context.borrow().as_ref() {
            controls.update(&self.text_view, source, selection, tab, mode);
        }
    }

    /// Enlaza el fijado del tono de `win.insert-callout` del popover (la
    /// resuelve el integrador sobre el grupo «win», main.rs).
    pub fn connect_context_tone(&self, setter: Rc<dyn Fn(String)>) {
        if let Some(controls) = self.context.borrow().as_ref() {
            controls.set_tone_setter(setter);
        }
    }

    /// Cierra los controles contextuales (recarga de pestaña/modo §37/§8).
    fn hide_context(&self) {
        if let Some(controls) = self.context.borrow().as_ref() {
            controls.hide();
        }
    }

    #[cfg(test)]
    pub fn test_context_visible(&self) -> bool {
        self.context
            .borrow()
            .as_ref()
            .is_some_and(|controls| controls.is_visible())
    }

    #[cfg(test)]
    pub fn test_context_has_button(&self, name: &str) -> bool {
        self.context
            .borrow()
            .as_ref()
            .and_then(|controls| controls.test_button(name))
            .is_some()
    }

    /// Botón del popover por nombre accesible (teclado/pruebas §29).
    #[cfg(test)]
    pub fn test_context_button(&self, name: &str) -> Option<gtk::Button> {
        self.context
            .borrow()
            .as_ref()
            .and_then(|controls| controls.test_button(name))
    }

    /// ¿El popover admite foco? (§29: Tab/Enter/Space nativos.)
    #[cfg(test)]
    pub fn test_context_can_focus(&self) -> bool {
        self.context
            .borrow()
            .as_ref()
            .is_some_and(|controls| controls.test_can_focus())
    }

    /// ¿Algún control tiene el foco? (tras `Ctrl+.`.)
    #[cfg(test)]
    pub fn test_context_focus_inside(&self) -> bool {
        self.context
            .borrow()
            .as_ref()
            .is_some_and(|controls| controls.test_focus_inside())
    }

    /// Emite `clicked` sobre un botón del popover; `false` si no existe.
    /// La guardia de pestaña vive en el handler del popover.
    #[cfg(test)]
    pub fn test_context_click(&self, name: &str) -> bool {
        self.context
            .borrow()
            .as_ref()
            .is_some_and(|controls| controls.test_click(name))
    }

    #[cfg(test)]
    pub fn test_context_tone(&self) -> Option<String> {
        self.context
            .borrow()
            .as_ref()
            .and_then(|controls| controls.test_tone_state())
    }
    pub fn set_wiki_links(&self, enabled: bool) {
        self.wiki_links.set(enabled);
    }

    pub fn set_link_diagnostics(&self, source: &str, links: &[crate::intelligence::Link]) {
        let (start, end) = self.buffer.bounds();
        if self.buffer.text(&start, &end, true).as_str() != source {
            return;
        }
        self.buffer.remove_tag(&self.broken_link, &start, &end);
        for link in links {
            if crate::intelligence::problem(&link.target).is_none() {
                continue;
            }
            let a = source[..link.span.start as usize].chars().count() as i32;
            let b = source[..link.span.end as usize].chars().count() as i32;
            self.buffer.apply_tag(
                &self.broken_link,
                &self.buffer.iter_at_offset(a),
                &self.buffer.iter_at_offset(b),
            );
        }
    }
    /// Ctrl+Click conserva la edición normal; Ctrl+Enter ofrece el mismo
    /// recorrido desde el teclado en ambos modos del buffer único.
    pub fn connect_link_activation(&self, activate: Rc<dyn Fn(Position)>) {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let view = self.text_view.clone();
        let callback = activate.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            if !gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return;
            }
            let (x, y) =
                view.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
            if let Some(iter) = view.iter_at_location(x, y) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                callback(Position::new(iter.line() as u32, iter.line_offset() as u32));
            }
        });
        self.text_view.add_controller(click);
        let key = gtk::EventControllerKey::new();
        let buffer = self.buffer.clone();
        key.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::Return
                && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                let iter = buffer.iter_at_mark(&buffer.get_insert());
                activate(Position::new(iter.line() as u32, iter.line_offset() as u32));
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        self.text_view.add_controller(key);
    }
    /// Crea el pane con su `TextTagTable` completa (un tag por tipo de token
    /// de Markdown y por estilo de Visual, estilizados con [`tags::tag_style`]
    /// / [`visual::tag_style`] sobre la [`Palette`] del tema resuelto por el
    /// integrador (§11, ISSUE-017)).
    ///
    /// `on_edited` se dispara solo tras ediciones reales del usuario.
    pub fn new(sink: Rc<dyn EditSink>, on_edited: Rc<dyn Fn()>, palette: &Palette) -> Self {
        let table = TextTagTable::new();
        let mut tag_handles = Vec::with_capacity(tags::all_kinds().len());
        for &kind in tags::all_kinds() {
            let tag = TextTag::new(Some(tags::tag_name(kind)));
            apply_tag_style(&tag, &tags::tag_style(kind, palette));
            table.add(&tag);
            tag_handles.push(tag);
        }

        // Tags del modo Visual (ISSUE-030): estilos semánticos §8.2 + tag de
        // sintaxis ocultable §8.3 (invisible = no pintado y de ancho cero).
        let mut vis_tags = Vec::with_capacity(visual::all_styles().len());
        for style in visual::all_styles() {
            let tag = TextTag::new(Some(visual::tag_name(style)));
            apply_tag_style(&tag, &visual::tag_style(style, palette));
            table.add(&tag);
            vis_tags.push((style, tag));
        }
        let vis_syntax = TextTag::new(Some(visual::SYNTAX_TAG));
        vis_syntax.set_invisible(true);
        table.add(&vis_syntax);

        let buffer = TextBuffer::new(Some(&table));
        let text_view = TextView::builder().buffer(&buffer).monospace(true).build();
        // Ajuste visual de líneas largas: `WordChar` parte la línea solo en
        // pantalla —el buffer sigue siendo el documento §6.1, sin saltos
        // insertados— para ver todo el texto escrito sin scroll horizontal
        // (sustituye al `wrap-mode` `GTK_WRAP_NONE` de §34.2 en el editor).
        text_view.set_wrap_mode(gtk::WrapMode::WordChar);
        text_view.set_left_margin(12);
        text_view.set_right_margin(12);
        text_view.set_top_margin(8);
        text_view.set_bottom_margin(8);

        // Marcado de búsqueda (§40.1): discreto para todas las coincidencias
        // y distinto para la activa; colores de la [`Palette`] (§23).
        let find_match = TextTag::new(Some("mm-find-match"));
        find_match.set_background(Some(palette.color("highlight").unwrap_or(palette.code_bg)));
        let find_active = TextTag::new(Some("mm-find-active"));
        find_active.set_background(Some(palette.color("orange").unwrap_or("#ef6c00")));
        find_active.set_foreground(Some(palette.bg));
        find_active.set_weight(pango::Weight::Bold.into_glib());
        table.add(&find_match);
        table.add(&find_active);
        let broken_link = TextTag::new(Some("mm-broken-link"));
        broken_link.set_underline(pango::Underline::Error);
        table.add(&broken_link);

        // Estado §8.3 del modo Visual.
        let reveal = Rc::new(RevealState {
            buffer: buffer.clone(),
            syntax_tag: vis_syntax.clone(),
            mode: Cell::new(EditMode::Markdown),
            visual_syntax: Cell::new(VisualSyntax::default()),
            hide: RefCell::new(Vec::new()),
            elements: RefCell::new(Vec::new()),
            revealed: Cell::new(None),
        });

        // Puente TextView → modelo (§6.1): `insert-text` se emite ANTES de
        // insertar, con el iter de destino; `delete-range` con ambos iters.
        let guard = Rc::new(Cell::new(false));
        {
            let sink = Rc::clone(&sink);
            let on_edited = Rc::clone(&on_edited);
            let guard = Rc::clone(&guard);
            buffer.connect_insert_text(move |_buffer, iter, text| {
                if guard.get() {
                    return;
                }
                let at = Position::new(iter.line() as u32, iter.line_offset() as u32);
                sink.insert_at(at, text);
                on_edited();
            });
        }
        {
            let sink = Rc::clone(&sink);
            let on_edited = Rc::clone(&on_edited);
            let guard = Rc::clone(&guard);
            buffer.connect_delete_range(move |_buffer, start, end| {
                if guard.get() {
                    return;
                }
                let from = Position::new(start.line() as u32, start.line_offset() as u32);
                let to = Position::new(end.line() as u32, end.line_offset() as u32);
                sink.delete_range(from, to);
                on_edited();
            });
        }
        {
            let sink = Rc::clone(&sink);
            let guard = Rc::clone(&guard);
            let reveal = Rc::clone(&reveal);
            buffer.connect_mark_set(move |buffer, _iter, mark| {
                if guard.get() || !is_selection_mark(mark.name().as_deref()) {
                    return;
                }
                let head = buffer.iter_at_mark(&buffer.get_insert());
                let anchor = buffer.iter_at_mark(&buffer.selection_bound());
                sink.set_selection(Selection {
                    anchor: Position::new(anchor.line() as u32, anchor.line_offset() as u32),
                    head: Position::new(head.line() as u32, head.line_offset() as u32),
                });
                // §8.3: al editar, el elemento bajo el cursor revela su
                // sintaxis (solo en modo Visual y `reveal-current`).
                reveal.update();
            });
        }

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroll = ScrolledWindow::new();
        scroll.set_child(Some(&text_view));
        scroll.set_hexpand(true);
        scroll.set_vexpand(true);
        root.append(&scroll);

        // Controles contextuales §44 (fase 7 #29): popover anclado al
        // TextView; la actualización llega con cada cambio de cursor o
        // selección (mark_set → EditSink::set_selection → main.rs).
        let context =
            RefCell::new(Some(super::context_controls::ContextControls::new(&text_view)));

        Self {
            root,
            buffer,
            text_view,
            tags: tag_handles,
            vis_tags,
            vis_syntax,
            find_match,
            find_active,
            broken_link,
            wiki_links: Cell::new(false),
            completion: RefCell::new(None),
            context,
            guard,
            reveal,
        }
    }

    /// Widget raíz (caja con la vista).
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast::<Widget>()
    }

    /// Sincroniza la vista con el documento activo (cambio de tab o de modo,
    /// open, undo/redo): carga el texto en el TextBuffer y repinta el plan
    /// del modo activo.
    ///
    /// Carga programática: no invoca al [`EditSink`] ni a `on_edited`. El
    /// historial Undo/Redo vive en el modelo y no se toca (§8.5); el cursor se
    /// restaura como posición semántica (§8.6), idéntica en ambos modos
    /// porque ambos son el mismo buffer.
    pub fn load(&self, editor: &Editor, mode: EditMode) {
        if let Some(popup) = self.completion.borrow().as_ref() {
            popup.hide();
        }
        // Cambio de tab/modo/recarga: el popover de controles contextuales
        // se cierra y su registro de pestaña queda invalidado (§UI #29).
        self.hide_context();
        let text = editor.document().content();
        self.guard.set(true);
        self.buffer.set_text(text);
        self.restore_cursor(editor.selection());
        self.reveal.mode.set(mode);
        self.guard.set(false);
        self.refresh(text);
    }

    /// Modo de edición activo del panel (§8).
    pub fn mode(&self) -> EditMode {
        self.reveal.mode.get()
    }

    /// Visibilidad de la sintaxis en Visual (§8.3). El llamador repinta
    /// después de cambiarla ([`EditorPane::refresh`]).
    pub fn set_visual_syntax(&self, syntax: VisualSyntax) {
        self.reveal.visual_syntax.set(syntax);
    }

    /// Repinta el plan del modo activo (§8): highlighting en Markdown,
    /// representación visual en Visual (ISSUE-030).
    pub fn refresh(&self, text: &str) {
        match self.mode() {
            EditMode::Markdown => self.refresh_highlight(text),
            EditMode::Visual => self.refresh_visual(text),
        }
    }

    /// Repinta el highlighting de Markdown Mode con `minmark_parser::highlight`.
    ///
    /// Quita antes todos los tags de estilo de todo buffer (Markdown y Visual:
    /// los modos son planes excluyentes) y aplica el plan de
    /// [`tags::tag_plan`]. Tolera ediciones concurrentes: si el texto del
    /// buffer cambió entre parseo y aplicación, se descarta el plan entero.
    pub fn refresh_highlight(&self, text: &str) {
        let plan = tags::tag_plan(
            text,
            &minmark_parser::highlight_with(
                text,
                &minmark_parser::ParseOptions {
                    wiki_links: self.wiki_links.get(),
                    ..Default::default()
                },
            ),
        );
        let styled = plan
            .into_iter()
            .map(|range| (range.start_char, range.end_char, range.kind))
            .collect();
        self.repaint(text, styled);
    }

    /// Repinta la representación visual del modo Visual (ISSUE-030):
    /// `minmark_parser::visual` + [`visual::visual_tag_plan`]. La sintaxis se
    /// oculta con el tag invisible (§8.3) y el contenido recibe su estilo
    /// semántico (§8.2).
    pub fn refresh_visual(&self, text: &str) {
        let plan = minmark_parser::visual_with(
            text,
            &minmark_parser::ParseOptions {
                wiki_links: self.wiki_links.get(),
                ..Default::default()
            },
        );
        let ranges = visual::visual_tag_plan(text, &plan);
        let elements = visual::element_ranges(text, &plan);
        let styled: Vec<StyledRange> = ranges
            .into_iter()
            .map(|range| (range.start_char, range.end_char, range.style))
            .collect();
        self.repaint_visual(text, styled, elements);
    }

    /// Limpia todos los tags de estilo y aplica el plan de Markdown.
    fn repaint(&self, text: &str, plan: Vec<(usize, usize, minmark_parser::HighlightKind)>) {
        let (start, end) = self.buffer.bounds();
        let current = self.buffer.text(&start, &end, false);
        if current.as_str() != text {
            return;
        }
        self.clear_style_tags();
        for (start_char, end_char, kind) in plan {
            let from = self.buffer.iter_at_offset(start_char as i32);
            let to = self.buffer.iter_at_offset(end_char as i32);
            self.buffer
                .apply_tag_by_name(tags::tag_name(kind), &from, &to);
        }
    }

    /// Limpia todos los tags de estilo y aplica el plan visual (ISSUE-030),
    /// con su estado §8.3 recalculado.
    fn repaint_visual(&self, text: &str, plan: Vec<StyledRange>, elements: Vec<(usize, usize)>) {
        let (start, end) = self.buffer.bounds();
        let current = self.buffer.text(&start, &end, false);
        if current.as_str() != text {
            return;
        }
        self.clear_style_tags();
        let mut hide = Vec::new();
        for (start_char, end_char, style) in plan {
            let from = self.buffer.iter_at_offset(start_char as i32);
            let to = self.buffer.iter_at_offset(end_char as i32);
            match style {
                None => {
                    self.buffer.apply_tag(&self.vis_syntax, &from, &to);
                    hide.push((start_char, end_char));
                }
                Some(style) => {
                    if let Some((_, tag)) = self.vis_tags.iter().find(|(s, _)| *s == style) {
                        self.buffer.apply_tag(tag, &from, &to);
                    }
                }
            }
        }
        *self.reveal.hide.borrow_mut() = hide;
        *self.reveal.elements.borrow_mut() = elements;
        self.reveal.revealed.set(None);
        self.reveal.update();
    }

    /// Quita todos los tags de estilo (Markdown y Visual) de todo el buffer.
    /// Los tags de búsqueda (§40.1) no se tocan.
    fn clear_style_tags(&self) {
        let (start, end) = self.buffer.bounds();
        for tag in &self.tags {
            self.buffer.remove_tag(tag, &start, &end);
        }
        for (_, tag) in &self.vis_tags {
            self.buffer.remove_tag(tag, &start, &end);
        }
        self.buffer.remove_tag(&self.vis_syntax, &start, &end);
    }

    /// Marcado discreto de las coincidencias y resaltado de la activa (§40.1).
    ///
    /// Usa tags propios («mm-find-match»/«mm-find-active»), independientes de
    /// los de highlighting: los refrescos de estilo no los tocan.
    pub fn set_find_marks(&self, matches: &[Range], active: Option<usize>) {
        self.clear_find_marks();
        for (index, range) in matches.iter().enumerate() {
            let from = self.iter_at(range.start);
            let to = self.iter_at(range.end);
            let tag = if Some(index) == active {
                &self.find_active
            } else {
                &self.find_match
            };
            self.buffer.apply_tag(tag, &from, &to);
        }
    }

    /// Quita el marcado de búsqueda (§40.1: cerrar la barra limpia el marcado).
    pub fn clear_find_marks(&self) {
        let (start, end) = self.buffer.bounds();
        self.buffer.remove_tag(&self.find_match, &start, &end);
        self.buffer.remove_tag(&self.find_active, &start, &end);
    }

    /// Lleva la coincidencia activa a la vista: cursor/selección sobre ella
    /// (§8.6: las marcas `insert`/`selection_bound` propagan al modelo) y
    /// scroll hasta hacerla visible (§40.1: foco en la activa).
    ///
    /// No roba el foco de teclado: el usuario sigue tecleando en la barra.
    pub fn focus_find_match(&self, range: Range) {
        let from = self.iter_at(range.start);
        let mut to = self.iter_at(range.end);
        self.buffer.select_range(&to, &from);
        self.text_view.scroll_to_iter(&mut to, 0.2, false, 0.0, 0.0);
    }

    /// Reubica las marcas `insert` y `selection_bound` a la selección del
    /// modelo (§8.6: posición semántica). Fuera de la guardia dispararía
    /// `mark-set` con eco al sink: el llamador debe fijarla.
    fn restore_cursor(&self, selection: Selection) {
        let anchor = self.iter_at(selection.anchor);
        let head = self.iter_at(selection.head);
        self.buffer.move_mark_by_name("selection_bound", &anchor);
        self.buffer.move_mark_by_name("insert", &head);
    }

    /// Iter del buffer para una [`Position`] semántica (línea + columna en
    /// chars unicode, como `TextIter::line()`/`line_offset()`).
    fn iter_at(&self, at: Position) -> TextIter {
        self.buffer
            .iter_at_line_offset(at.line as i32, at.column as i32)
            .unwrap_or_else(|| self.buffer.start_iter())
    }
}

/// Rango estilizado del plan visual en chars: `(inicio, fin, estilo)`;
/// `None` = sintaxis ocultable (§8.3).
type StyledRange = (usize, usize, Option<VisualStyle>);

/// Aplica un [`tags::TagStyle`] a un tag fijando también las propiedades
/// ausentes a su valor neutro, de modo que reestilizar con otra paleta deja
/// los estilos anteriores limpios (§11).
fn apply_tag_style(tag: &TextTag, style: &tags::TagStyle) {
    tag.set_foreground(style.fg);
    tag.set_background(style.bg);
    let weight = if style.bold {
        pango::Weight::Bold
    } else {
        pango::Weight::Normal
    };
    tag.set_weight(weight.into_glib());
    tag.set_style(if style.italic {
        pango::Style::Italic
    } else {
        pango::Style::Normal
    });
    tag.set_family(style.monospace.then_some("monospace"));
    tag.set_scale(style.scale.map(|scale| scale as f64 / 100.0).unwrap_or(1.0));
    tag.set_underline(if style.underline {
        pango::Underline::Single
    } else {
        pango::Underline::None
    });
    tag.set_left_margin(style.left_margin.unwrap_or(0) as i32 + 12);
    // Los tags sin sangría no deben anular el margen del TextView.
    tag.set_property("left-margin-set", style.left_margin.is_some());
}

/// `mark-set` solo propaga las marcas de cursor/selección (`insert` y
/// `selection_bound`).
fn is_selection_mark(name: Option<&str>) -> bool {
    matches!(name, Some("insert") | Some("selection_bound"))
}

#[cfg(test)]
mod tests {
    use super::{innermost, is_selection_mark, rehide_ranges};

    #[test]
    fn solo_las_marcas_de_seleccion_propagan() {
        assert!(is_selection_mark(Some("insert")));
        assert!(is_selection_mark(Some("selection_bound")));
        assert!(!is_selection_mark(Some("gtk-multiline")));
        assert!(!is_selection_mark(None));
    }

    #[test]
    fn elemento_mas_interno_bajo_el_cursor() {
        // Cita (0..30) con párrafo interno (5..20).
        let elements = [(0usize, 30usize), (5, 20)];
        assert_eq!(innermost(&elements, 10), Some((5, 20)));
        assert_eq!(innermost(&elements, 25), Some((0, 30)));
        assert_eq!(innermost(&elements, 40), None);
    }

    #[test]
    fn reoculta_fuera_del_elemento_revelado() {
        let hide = [(1usize, 3usize), (6, 8), (24, 26)];
        // Salimos de (0,30) con (5,20) revelado: solo se reocultan (1,3) y (24,26).
        assert_eq!(
            rehide_ranges(&hide, (0, 30), Some((5, 20))),
            vec![(1, 3), (24, 26)]
        );
        // Sin elemento nuevo, todo lo del antiguo vuelve a ocultarse.
        assert_eq!(
            rehide_ranges(&hide, (0, 30), None),
            vec![(1, 3), (6, 8), (24, 26)]
        );
    }
}
