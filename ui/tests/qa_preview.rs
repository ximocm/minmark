//! QA gráfica del Preview sin harness de libtest (ISSUE-57).
//!
//! GTK en macOS exige el hilo principal del proceso y libtest ejecuta cada
//! test en su propio hilo, así que el escenario [`app::qa_preview_smoke`] se
//! ejecuta en el `main` de este binario. Con `MINMARK_QA=1` corre de verdad;
//! sin esa variable se omite, para no romper `cargo test` en máquinas sin
//! sesión gráfica.

#[path = "../src/main.rs"]
#[allow(dead_code, unused_imports)]
mod app;

// `main.rs` referencia a sus módulos como `crate::…`; al incluirlo como
// módulo `app`, esta reexportación mantiene esos caminos resolubles.
pub use app::*;

fn main() {
    if std::env::var_os("MINMARK_QA").is_none() {
        println!("qa_preview: omitido (fija MINMARK_QA=1 para ejecutarlo)");
        return;
    }
    app::qa_preview_smoke();
    println!("qa_preview: ok");
}
