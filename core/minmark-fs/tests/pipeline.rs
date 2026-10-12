//! Pipeline end-to-end del criterio 1 de ISSUE-013: abrir un `.md` real,
//! parsearlo con minmark-parser y renderizarlo para Preview.
//!
//! La AST es derivada del buffer (§6.1: el `.md` es la fuente de verdad), por
//! lo que `open` devuelve el buffer y el parseo se hace sobre él.

use minmark_fs::{FileStore, LocalFs};
use minmark_parser::parse;
use minmark_renderer::render_preview;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("minmark-pipe-{}-{}", std::process::id(), nanos));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn abrir_parsear_y_renderizar() {
    let dir = TestDir::new();
    let path = dir.0.join("entrada.md");
    fs::write(&path, "# Hola\n\n*cuerpo* con `código`\n").unwrap();

    let mut store = LocalFs::new();
    let document = store.open(&path).unwrap();
    let ast = parse(document.content());
    let html = render_preview(&ast, &minmark_config::Settings::default());

    assert!(html.contains("<h1 data-mm-span="));
    assert!(html.contains(">Hola</h1>"));
    assert!(html.contains("<em>cuerpo</em>"));
    assert!(html.contains("<code>código</code>"));
}
