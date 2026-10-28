//! World mínimo de Typst para componer en memoria (ISSUE-018).
//!
//! Un solo documento (`main.typ`), las fuentes embebidas de `typst-assets` y
//! Liberation Sans embebida como assets del crate: la aplicación no depende
//! de fuentes del sistema (§33) y el default sans-serif del cuerpo (§33.1)
//! resuelve a una familia sans-serif real. Sin acceso a disco ni red
//! (NFR-016: todo el core funciona offline).

use std::path::PathBuf;
use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime};
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, World};

pub struct MiniWorld {
    main: Source,
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    files: std::collections::BTreeMap<PathBuf, Vec<u8>>,
}

/// Liberation Sans (SIL OFL 1.1, `assets/fonts/OFL.txt`): familia sans-serif
/// del world (§33) y fuente determinista del render de diagramas (ADR-007,
/// [`crate::diagram`]).
pub(crate) const SANS_FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/LiberationSans-Regular.ttf"),
    include_bytes!("../assets/fonts/LiberationSans-Italic.ttf"),
    include_bytes!("../assets/fonts/LiberationSans-Bold.ttf"),
    include_bytes!("../assets/fonts/LiberationSans-BoldItalic.ttf"),
];

impl MiniWorld {
    pub fn new(text: String, files: std::collections::BTreeMap<PathBuf, Vec<u8>>) -> Self {
        let fonts: Vec<Font> = typst_assets::fonts()
            .filter_map(|data| Font::new(Bytes::new(data), 0))
            // §33: Liberation Sans (SIL OFL 1.1, `assets/fonts/OFL.txt`) como
            // familia sans-serif del world; sin ella el default §33.1 caía a
            // una serif de `typst-assets`.
            .chain(
                SANS_FONTS
                    .iter()
                    .filter_map(|data| Font::new(Bytes::new(*data), 0)),
            )
            .collect();
        let book = FontBook::from_fonts(fonts.iter());
        let main = Source::new(FileId::new(None, VirtualPath::new("/main.typ")), text);
        Self {
            main,
            library: LazyHash::new(Library::builder().build()),
            book: LazyHash::new(book),
            fonts,
            files,
        }
    }
}

impl World for MiniWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::NotFound(PathBuf::new()))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id.package().is_none() {
            if let Some(bytes) = self.files.get(id.vpath().as_rootless_path()) {
                return Ok(Bytes::new(bytes.clone()));
            }
        }
        Err(FileError::NotFound(
            id.vpath().as_rootless_path().to_owned(),
        ))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        // `{date}` (§32.9) se resuelve en la capa de composición; el core no
        // depende del reloj para resultados reproducibles.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_con_sans_serif_y_mono_embebidas_33() {
        // §33: el world lleva sans (Liberation Sans, asset del crate), serif
        // (Libertinus Serif) y mono (DejaVu Sans Mono) sin tocar el sistema.
        let world = MiniWorld::new(String::new(), Default::default());
        let mut familias = std::collections::BTreeSet::new();
        for index in 0.. {
            let Some(font) = world.font(index) else {
                break;
            };
            familias.insert(font.info().family.to_string());
        }
        for esperada in ["Liberation Sans", "Libertinus Serif", "DejaVu Sans Mono"] {
            assert!(familias.contains(esperada), "falta {esperada:?}");
        }
    }
}
