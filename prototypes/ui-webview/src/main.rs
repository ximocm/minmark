// UI Webview prototype — Phase 0 benchmark (ISSUE-008)
// Minimal: open file, edit buffer, render preview.

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, ScrolledWindow, TextView, TextBuffer};
use webkit6::WebView;

fn main() {
    let app = Application::builder().build();

    app.connect_activate(|app| {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("ui-webview prototype")
            .default_width(1200)
            .default_height(800)
            .build();

        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

        // Editor pane
        let buffer = TextBuffer::new(None);
        let editor = TextView::with_buffer(&buffer);
        let editor_scroll = ScrolledWindow::new();
        editor_scroll.set_child(Some(&editor));

        // Preview pane (Webview)
        let webview = WebView::new();
        let preview_scroll = ScrolledWindow::new();
        preview_scroll.set_child(Some(&webview));

        hbox.append(&editor_scroll);
        hbox.append(&preview_scroll);

        window.set_child(Some(&hbox));
        window.present();

        // Benchmark: input latency test
        let start = std::time::Instant::now();
        for i in 0..1000 {
            buffer.insert(&mut buffer.start_iter(), &format!("test {}\n", i));
        }
        let duration = start.elapsed();
        eprintln!("Input latency (1000 inserts): {:?}", duration / 1000);
    });

    app.run();
}
