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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Contador del proceso: dos tests en paralelo nunca comparten directorio
/// aunque el reloj del sistema repita `nanos` (mismo criterio que
/// `src/test_dir.rs`; los tests de integración no ven el módulo interno).
static CONTADOR_TMP: AtomicUsize = AtomicUsize::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let cuenta = CONTADOR_TMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "minmark-pipe-{}-{nanos}-{cuenta}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
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
    let html = render_preview(&ast, document.content(), &minmark_config::Settings::default());

    assert!(html.contains("<h1 data-mm-span="));
    assert!(html.contains("id=\"hola\""), "el heading recibe su ancla (§26)");
    assert!(html.contains(">Hola</h1>"));
    assert!(html.contains("<em>cuerpo</em>"));
    assert!(html.contains("<code>código</code>"));
}
