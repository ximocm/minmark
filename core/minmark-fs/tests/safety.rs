//! Escenarios de seguridad end-to-end de ISSUE-019 sobre el filesystem real:
//! detección de cambios externos (§12.7, §46) integrada con el buffer.

use minmark_fs::{ChangeKind, FileStore, FsError, InotifyWatcher, LocalFs, Watcher};
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
        let path = std::env::temp_dir().join(format!("minmark-safety-{}-{}", std::process::id(), nanos));
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
fn cambios_externos_se_notifican_sin_recargar_el_buffer() {
    // §12.7: se notifica, nunca se recarga automáticamente y nunca se
    // sobrescribe silenciosamente.
    let dir = TestDir::new();
    let path = dir.0.join("doc.md");
    fs::write(&path, "original\n").unwrap();

    let mut store = LocalFs::new();
    let mut doc = store.open(&path).unwrap();
    doc.replace_content("edición local\n");

    let mut watcher = InotifyWatcher::new().unwrap();
    watcher.watch(&path).unwrap();
    fs::write(&path, "otro programa\n").unwrap();

    // Notificación nativa al watcher…
    let events = watcher.poll().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.kind == ChangeKind::Modified && e.path.ends_with("doc.md")),
        "eventos: {events:?}"
    );

    // …pero el buffer NO se recarga solo: conserva la edición local.
    assert_eq!(doc.content(), "edición local\n");
    // Y el guardado queda bloqueado hasta confirmación explícita (§3.7).
    assert!(matches!(
        store.save(&mut doc),
        Err(FsError::ExternalModification { .. })
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "otro programa\n");

    store.save_confirmed(&mut doc).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "edición local\n");
}
