//! Motor del Preview sobre WebKitGTK 6.0 (ADR-001): la única implementación
//! de [`super::PreviewEngine`] de momento (ADR-005). Windows y macOS
//! añadirán WebView2 y WKWebView en sus propios módulos.
//!
//! Este módulo es el único punto del `ui` que puede nombrar `webkit6`: el
//! resto de la UI habla con el motor a través del trait.

use gtk4 as gtk;
use gtk4::prelude::*;
use webkit6::prelude::*;

use super::PreviewEngine;

/// `WebView` del Preview. Los objetos GLib son apuntadores refcount, así que
/// un handle de este motor es barato de clonar y siempre apunta al mismo
/// widget (el callback de carga clona el suyo propio para poder evaluar
/// scripts en él sin capturar referencias con lifetime).
pub struct WebkitEngine {
    view: webkit6::WebView,
}

impl WebkitEngine {
    /// Crea la `WebView`. Requiere GTK inicializado en el hilo principal
    /// (lo hace `main`).
    pub(super) fn new() -> Self {
        Self {
            view: webkit6::WebView::new(),
        }
    }
}

impl PreviewEngine for WebkitEngine {
    fn widget(&self) -> gtk::Widget {
        self.view.clone().upcast::<gtk::Widget>()
    }

    fn load_html(&self, html: &str, base_uri: &str) {
        // La página viaja en memoria (`load_html` de WebKitGTK), como hasta
        // ahora: el archivo temporal local es el transporte del backend
        // WebView2 (nunca `NavigateToString`, límite de 2 MiB, ADR-005).
        self.view.load_html(html, Some(base_uri));
    }

    fn run_script(&self, script: &str) {
        self.view
            .evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }

    fn is_loading(&self) -> bool {
        self.view.is_loading()
    }

    fn connect_load_finished(&self, callback: std::rc::Rc<dyn Fn(&dyn PreviewEngine)>) {
        // El emisor sigue siendo la misma WebView: el callback recibe un
        // handle nuevo del mismo motor para evaluar sus scripts pendientes.
        let runner = self.view.clone();
        self.view.connect_load_changed(move |_, event| {
            if event == webkit6::LoadEvent::Finished {
                callback(&WebkitEngine {
                    view: runner.clone(),
                });
            }
        });
    }

    fn connect_link_activation(&self, activate: std::rc::Rc<dyn Fn(String)>) {
        let manager = self
            .view
            .user_content_manager()
            .expect("WebView content manager");
        manager.register_script_message_handler("minmarkLink", None);
        manager.connect_script_message_received(Some("minmarkLink"), move |_, value| {
            activate(value.to_str().to_string());
        });
        manager.add_script(&webkit6::UserScript::new(
            "document.addEventListener('click', e => { const a = e.target.closest('a[href]'); if (a) { e.preventDefault(); const href = a.getAttribute('href'); if (href.startsWith('#mm-note-')) { document.getElementById(href.slice(1))?.scrollIntoView({block: 'center'}); } else { window.webkit.messageHandlers.minmarkLink.postMessage(href); } } });",
            webkit6::UserContentInjectedFrames::TopFrame,
            webkit6::UserScriptInjectionTime::End, &[], &[],
        ));
    }

    fn block_navigation(&self) {
        // También bloquea navegación de teclado, ventanas nuevas y enlaces
        // que no pasen por el puente: la WebView conserva el documento.
        self.view.connect_decide_policy(|_, decision, _| {
            if let Some(nav) = decision.downcast_ref::<webkit6::NavigationPolicyDecision>() {
                if nav.navigation_action().is_some_and(|mut a| {
                    a.navigation_type() == webkit6::NavigationType::LinkClicked
                }) {
                    decision.ignore();
                    return true;
                }
            }
            false
        });
    }

    #[cfg(test)]
    fn snapshot(&self, path: &std::path::Path, done: std::rc::Rc<std::cell::Cell<bool>>) {
        let path = path.to_owned();
        self.view.snapshot(
            webkit6::SnapshotRegion::FullDocument,
            webkit6::SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                result
                    .expect("WebKit snapshot")
                    .save_to_png(&path)
                    .expect("snapshot PNG");
                done.set(true);
            },
        );
    }

    #[cfg(test)]
    fn eval_ok(&self, script: &str, done: std::rc::Rc<std::cell::Cell<bool>>) {
        self.view.evaluate_javascript(
            script,
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |result| {
                done.set(result.is_ok());
            },
        );
    }

    #[cfg(test)]
    fn eval_number(&self, script: &str, result: std::rc::Rc<std::cell::Cell<Option<f64>>>) {
        self.view.evaluate_javascript(
            script,
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |value| {
                result.set(Some(value.expect("Preview JavaScript").to_double()));
            },
        );
    }
}
