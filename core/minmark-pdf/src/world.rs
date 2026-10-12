//! World mínimo de Typst para componer en memoria (ISSUE-018).
//!
//! Un solo documento (`main.typ`) y las fuentes embebidas de `typst-assets`
//! (§33: la aplicación no depende de fuentes del sistema). Sin acceso a disco
//! ni red (NFR-016: todo el core funciona offline).

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
}

impl MiniWorld {
    pub fn new(text: String) -> Self {
        let fonts: Vec<Font> = typst_assets::fonts()
            .filter_map(|data| Font::new(Bytes::new(data), 0))
            .collect();
        let book = FontBook::from_fonts(fonts.iter());
        let main = Source::new(FileId::new(None, VirtualPath::new("/main.typ")), text);
        Self {
            main,
            library: LazyHash::new(Library::builder().build()),
            book: LazyHash::new(book),
            fonts,
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
