//! Composiciones multiarchivo (§20, ISSUE-022/023): descubrimiento, fuente
//! combinada y render/export del documento lógico.
//!
//! - [`discover`] lee las tablas `[[composition]]` del `.mdproject.toml` de
//!   la raíz del proyecto activo (§20). Sin proyecto no hay composiciones y
//!   las acciones quedan deshabilitadas.
//! - [`build`] carga cada archivo en orden declarado (§20.1): el buffer
//!   abierto manda sobre el disco (§6.1, ISSUE-023) y un archivo ausente
//!   produce un bloque de diagnóstico visible **en su posición** sin abortar
//!   el resto. Consultar o renderizar nunca escribe los fuentes (§20).
//! - [`render`] produce el Preview combinado (§20.4): continuo sobre la AST
//!   compuesta o paginado con el camino Typst existente, según el ajuste
//!   `pagination` efectivo (cadena §58).
//!
//! Módulo sin widgets: todo su comportamiento es verificable sin display
//! (estilo `ui/src/export.rs`).

use minmark_config::{Composition, Settings};
use minmark_document::{compose, Block, BlockKind, ComposePart, Composed, Inline, Span};
use minmark_pdf::DocumentMeta;
use std::path::{Path, PathBuf};
// Preludio de GTK para `FileExt` (`gio::File::uri`, base URI del Preview).
use gtk4::prelude::*;

/// Composiciones declaradas en el `.mdproject.toml` de la raíz del proyecto
/// (ISSUE-022), en orden de declaración. Los avisos del parser son no
/// bloqueantes (§58.1) y se registran en stderr.
pub fn discover(root: &Path) -> Vec<Composition> {
    let Ok(text) = std::fs::read_to_string(root.join(".mdproject.toml")) else {
        return Vec::new();
    };
    let (compositions, warnings) = minmark_config::parse_compositions(&text);
    for warning in warnings {
        eprintln!(
            "minmark: composición {:?} clave {:?}: {:?}",
            warning.name, warning.key, warning.reason
        );
    }
    compositions
}

/// Documento lógico combinado, listo para Preview y para la exportación
/// PDF única (§20.4, ISSUE-023).
pub struct Combined {
    /// Nombre de la composición: título del PDF y de la ventana (ISSUE-023).
    pub name: String,
    /// Fuente virtual + AST combinados (§20), con los diagnósticos de
    /// `compose` ya añadidos como bloques visibles.
    pub composed: Composed,
    /// Ajustes efectivos: cadena §58 (documento > composición > proyecto >
    /// global > defecto) con la bibliografía unida de los archivos (ISSUE-024).
    pub settings: Settings,
    /// Metadatos de exportación (§32.9).
    pub meta: DocumentMeta,
    /// Raíz del proyecto: los destinos de la composición se resuelven
    /// respecto a ella (§20.4).
    pub root: PathBuf,
}

/// Construye el documento combinado de `composition` (ISSUE-022/023).
///
/// `buffers` es el contenido de los ficheros abiertos en la ventana como
/// `(ruta, fuente)`: un buffer abierto se usa aunque no se haya guardado.
/// Un archivo que ni existe ni está abierto se sustituye por un párrafo
/// `Archivo ausente: <ruta>` que conserva su hueco en el orden (§20.1) sin
/// abortar la composición. Nunca escribe los fuentes.
pub fn build(root: &Path, composition: &Composition, buffers: &[(PathBuf, String)]) -> Combined {
    let mut fuentes: Vec<(PathBuf, String)> = Vec::with_capacity(composition.files.len());
    for rel in &composition.files {
        let path = root.join(rel);
        let abierto = buffers
            .iter()
            .find(|(abierto, _)| comparable(abierto) == comparable(&path));
        let fuente = match abierto {
            Some((_, fuente)) => fuente.clone(),
            None => std::fs::read_to_string(&path)
                .unwrap_or_else(|_| format!("Archivo ausente: {rel}\n")),
        };
        fuentes.push((PathBuf::from(rel), fuente));
    }

    // §58.1: la cadena efectiva de la composición gobierna render y PDF.
    let settings = crate::settings::load_composition_at(root, composition, &fuentes);
    let parts: Vec<ComposePart> = fuentes
        .iter()
        .map(|(rel, source)| ComposePart {
            path: rel.clone(),
            ast: minmark_parser::parse_with(
                source,
                &minmark_parser::ParseOptions {
                    wiki_links: settings.wiki_links,
                    ..Default::default()
                },
            ),
            source: source.clone(),
        })
        .collect();

    // §20.3: `page_break_between_files` efectivo (nivel composición, por
    // defecto false).
    let mut composed = compose(&parts, settings.page_break_between_files);
    // §20.4/ISSUE-023: los diagnósticos de `compose` se muestran como
    // bloques en el Preview y en la exportación (patrón de mensajes de
    // `academic::prepare_pdf`; el renderer los escapa §7.2).
    for mensaje in composed.diagnostics.clone() {
        composed.ast.blocks.push(Block {
            span: Span::default(),
            kind: BlockKind::Paragraph {
                content: vec![Inline::Text(mensaje)],
            },
        });
    }

    let meta = metadatos(&composed, composition, root);
    Combined {
        name: composition.name.clone(),
        composed,
        settings,
        meta,
        root: root.to_path_buf(),
    }
}

/// Render del Preview combinado (§20.4, ISSUE-023): continuo sobre la AST
/// compuesta con la bibliografía efectiva (ISSUE-024) o paginado con el
/// camino Typst existente, según el ajuste `pagination` efectivo. Consume
/// el documento porque el render paginado deriva de su AST.
pub fn render(combined: Combined) -> String {
    let Combined {
        composed,
        settings,
        meta,
        root,
        ..
    } = combined;
    if settings.pagination_enabled {
        match crate::export::paginated_preview_composed(&composed.ast, &settings, &root, &meta) {
            Ok(html) => return html,
            Err(error) => {
                // Mismo patrón que `preview::render_document_at`: el
                // diagnóstico es visible y el Preview continuo sigue vivo.
                eprintln!("minmark: Preview paginado: {error}");
                let diagnostico = escapa_html(&error.to_string());
                let html = continuo(&composed, &settings);
                return html.replace(
                    "</body>",
                    &format!("<p role=\"status\">Preview paginado: {diagnostico}</p></body>"),
                );
            }
        }
    }
    continuo(&composed, &settings)
}

/// Preview continuo del documento compuesto: la AST y el fuente virtual
/// viajan juntos para que anclas (§26) y `data-mm-span` (§9.1) sean los del
/// documento combinado; la bibliografía (ISSUE-024) resuelve con los
/// ajustes efectivos de la composición.
fn continuo(composed: &Composed, settings: &Settings) -> String {
    minmark_renderer::render_preview_with_bibliography(
        &composed.ast,
        &composed.source,
        settings,
        &crate::academic::load(settings),
    )
}

/// Metadatos de exportación de la composición (ISSUE-023, §32.9):
/// `title` = nombre de la composición; `author` = front matter del primer
/// archivo; `project` = nombre del directorio raíz; `date` = fecha local
/// del momento en ISO `YYYY-MM-DD` (el core no depende del reloj).
fn metadatos(composed: &Composed, composition: &Composition, root: &Path) -> DocumentMeta {
    let mut meta = DocumentMeta::from_ast(&composed.ast);
    meta.title = Some(composition.name.clone());
    meta.project = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    meta.date = Some(fecha_local_iso());
    meta
}

/// Fecha local del momento en ISO `YYYY-MM-DD`; GLib aporta la zona
/// horaria local sin dependencias nuevas (§4).
fn fecha_local_iso() -> String {
    gtk4::glib::DateTime::now_local()
        .and_then(|date| date.format("%Y-%m-%d"))
        .map(|text| text.to_string())
        .unwrap_or_default()
}

/// Base URI de la carga en la WebView (§10.2, §3.4: sin red): la raíz del
/// proyecto, porque `compose` rebasa los destinos a rutas relativas a ella
/// (§20.4).
pub fn base_uri(root: &Path) -> String {
    let canonica = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    format!(
        "{}/",
        gtk4::gio::File::for_path(canonica)
            .uri()
            .trim_end_matches('/')
    )
}

/// Ruta comparable: canónica si se resuelve y la propia si no (dos grafías
/// de un mismo fichero son el mismo fichero, §12.7).
fn comparable(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Escapado HTML mínimo para mensajes (§7.2: nada sale crudo).
fn escapa_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use minmark_config::Orientation;

    /// Directorio temporal propio del test (los tests corren en paralelo).
    fn directorio(nombre: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "minmark-composition-{nombre}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn svg() -> &'static str {
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20" fill="red"/></svg>"#
    }

    /// §20.4/ISSUE-023: la composición de dos archivos en dirs distintos se
    /// exporta a un único PDF y AMBAS imágenes se resuelven respecto a su
    /// archivo de origen: si la resolución falla (p. ej. la del segundo
    /// archivo), la exportación devuelve error y el test falla.
    #[test]
    fn exporta_pdf_combinado_con_las_imagenes_de_los_dos_archivos() {
        let root = directorio("pdf-imagenes");
        std::fs::create_dir_all(root.join("caps")).unwrap();
        std::fs::create_dir_all(root.join("otro")).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"caps/uno.md\", \"otro/dos.md\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("caps/uno.md"),
            "# Capítulo Uno\n\n![Uno](img-uno.svg)\n",
        )
        .unwrap();
        std::fs::write(
            root.join("otro/dos.md"),
            "# Capítulo Dos\n\n![Dos](img-dos.svg)\n",
        )
        .unwrap();
        std::fs::write(root.join("caps/img-uno.svg"), svg()).unwrap();
        std::fs::write(root.join("otro/img-dos.svg"), svg()).unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        let Combined {
            composed,
            settings,
            meta,
            root: raiz,
            ..
        } = combined;
        let pdf = crate::export::export_composed(composed.ast, &settings, &raiz, &meta)
            .expect("exportación con las dos imágenes");
        assert!(pdf.starts_with(b"%PDF"), "cabecera PDF ausente");

        // Contraprueba: sin la imagen del SEGUNDO archivo (otro dir, otra
        // ruta) la resolución falla con un error que la nombra.
        std::fs::remove_file(root.join("otro/img-dos.svg")).unwrap();
        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        let Combined {
            composed,
            settings,
            meta,
            root: raiz,
            ..
        } = combined;
        let error = crate::export::export_composed(composed.ast, &settings, &raiz, &meta)
            .expect_err("la imagen ausente debe producir error visible");
        assert!(error.to_string().contains("img-dos.svg"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// ISSUE-023: el buffer abierto sin guardar de un archivo incluido
    /// aparece en el resultado (fuente virtual y Preview) y consultar o
    /// renderizar no escribe el fuente (§20).
    #[test]
    fn buffer_sin_guardado_de_archivo_incluido_aparece_en_el_resultado() {
        let root = directorio("buffer");
        std::fs::create_dir_all(root.join("caps")).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"caps/uno.md\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("caps/uno.md"), "# En disco\n").unwrap();

        let buffers = [(
            root.join("caps/uno.md"),
            "# Buffer sin guardar\n".to_string(),
        )];
        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &buffers);
        assert!(combined.composed.source.contains("Buffer sin guardar"));
        assert!(!combined.composed.source.contains("En disco"));
        let html = render(build(&root, &composition, &buffers));
        assert!(html.contains("Buffer sin guardar"));

        assert_eq!(
            std::fs::read_to_string(root.join("caps/uno.md")).unwrap(),
            "# En disco\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// ISSUE-022: un archivo ausente produce el diagnóstico visible en SU
    /// posición del orden (§20.1) sin abortar el resto de la composición.
    #[test]
    fn archivo_ausente_diagnostica_en_su_posicion_sin_abortar() {
        let root = directorio("ausente");
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\", \"falta.md\", \"tres.md\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("uno.md"), "Contenido uno.\n").unwrap();
        std::fs::write(root.join("tres.md"), "Contenido tres.\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        let fuente = &combined.composed.source;
        let uno = fuente.find("Contenido uno.").expect("primera parte");
        let hueco = fuente
            .find("Archivo ausente: falta.md")
            .expect("diagnóstico visible");
        let tres = fuente.find("Contenido tres.").expect("tercera parte");
        assert!(uno < hueco && hueco < tres, "el hueco va en su posición");

        let html = render(build(&root, &composition, &[]));
        assert!(html.contains("Archivo ausente: falta.md"));
        assert!(html.contains("Contenido uno."));
        assert!(html.contains("Contenido tres."));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// §20.2/§20.4 + ISSUE-023: los niveles de heading se conservan
    /// exactamente (sin reajuste por archivo), los dos archivos aparecen en
    /// orden declarado y `page_break_between_files` por defecto es false.
    #[test]
    fn preview_continuo_conserva_los_niveles_y_el_orden() {
        let root = directorio("continuo");
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"primero.md\", \"segundo.md\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("primero.md"),
            "# Capítulo Uno\n\n## Subnivel uno\n\nCuerpo primero.\n",
        )
        .unwrap();
        std::fs::write(root.join("segundo.md"), "# Capítulo Dos\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        assert!(
            !combined.composed.source.contains("{{pagebreak}}"),
            "page_break_between_files es false por defecto (§20.3)"
        );
        let html = render(build(&root, &composition, &[]));
        let primero = html.find("Capítulo Uno").expect("primer archivo");
        let segundo = html.find("Capítulo Dos").expect("segundo archivo");
        assert!(primero < segundo, "orden declarado §20.1");
        assert!(html.contains(">Capítulo Uno</h1>"), "H1 intacto §20.2");
        assert!(html.contains(">Subnivel uno</h2>"), "H2 intacto §20.2");
        assert!(html.contains(">Capítulo Dos</h1>"), "sin reajuste §20.2");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// §20.3/§20.4 + ISSUE-023: con `pagination = true` y
    /// `page_break_between_files = true` el Preview paginado de la
    /// composición tiene varias páginas y muestra los dos archivos.
    #[test]
    fn preview_paginado_de_la_composicion_tiene_varias_paginas() {
        let root = directorio("paginado");
        std::fs::write(
            root.join(".mdproject.toml"),
            "pagination = true\n[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\", \"dos.md\"]\npage_break_between_files = true\n",
        )
        .unwrap();
        std::fs::write(root.join("uno.md"), "# Capítulo Uno\n").unwrap();
        std::fs::write(root.join("dos.md"), "# Capítulo Dos\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        assert!(composition.page_break_between_files);
        let combined = build(&root, &composition, &[]);
        assert!(combined.composed.source.contains("{{pagebreak}}"));
        let uno = combined
            .composed
            .source
            .find("Capítulo Uno")
            .expect("primer archivo en la fuente virtual");
        let dos = combined
            .composed
            .source
            .find("Capítulo Dos")
            .expect("segundo archivo en la fuente virtual");
        assert!(uno < dos, "orden declarado §20.1");
        // El HTML paginado compone cada página como SVG (§29: mismas glyphas
        // que el PDF), así que el texto no es literal; lo verificable es la
        // estructura de páginas y sus etiquetas aria.
        let html = render(combined);
        let paginas = html.matches("mm-paged-page").count();
        assert!(paginas >= 2, "composición paginada con {paginas} páginas");
        assert!(html.contains("aria-label=\"Página 1\""));
        assert!(html.contains("aria-label=\"Página 2\""));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// §58.1/ISSUE-023: los overrides de la composición (p. ej.
    /// `pdf_orientation = "landscape"`) aplican sobre el proyecto/global.
    #[test]
    fn overrides_de_la_composicion_aplican_sobre_el_proyecto() {
        let root = directorio("overrides");
        std::fs::write(
            root.join(".mdproject.toml"),
            "pdf_orientation = \"portrait\"\npdf_page_size = \"Letter\"\n[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\"]\npdf_orientation = \"landscape\"\n",
        )
        .unwrap();
        std::fs::write(root.join("uno.md"), "# Uno\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        assert_eq!(
            combined.settings.pdf_orientation,
            Orientation::Landscape,
            "la composición prevalece sobre el proyecto (§58.1)"
        );
        assert_eq!(
            combined.settings.pdf_page_size,
            minmark_config::PageSize::Letter,
            "el proyecto prevalece sobre el defecto/global (§58.1)"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// ISSUE-023 (§32.9): `title` = nombre de la composición (no el del
    /// front matter), `author` = front matter del primer archivo,
    /// `project` = directorio raíz y `date` = fecha local ISO `YYYY-MM-DD`.
    #[test]
    fn metadatos_de_la_composicion() {
        let root = directorio("meta");
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\", \"dos.md\"]\n",
        )
        .unwrap();
        std::fs::write(
            root.join("uno.md"),
            "---\ntitle: \"Título del front matter\"\nauthor: \"Autora X\"\n---\n# Uno\n",
        )
        .unwrap();
        std::fs::write(root.join("dos.md"), "# Dos\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        let meta = &combined.meta;
        assert_eq!(meta.title.as_deref(), Some("Libro"));
        assert_eq!(meta.author.as_deref(), Some("Autora X"));
        assert_eq!(
            meta.project,
            Some(root.file_name().unwrap().to_string_lossy().into_owned())
        );
        let fecha = meta.date.as_deref().expect("fecha ISO");
        let partes: Vec<&str> = fecha.split('-').collect();
        assert_eq!(partes.len(), 3, "fecha ISO YYYY-MM-DD: {fecha}");
        assert_eq!(partes[0].len(), 4);
        assert_eq!(partes[1].len(), 2);
        assert_eq!(partes[2].len(), 2);
        assert!(fecha.chars().all(|c| c.is_ascii_digit() || c == '-'));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// §36/ISSUE-022: descubrimiento desde la raíz del proyecto — en orden
    /// de declaración, con sus archivos y su `page_break_between_files`.
    #[test]
    fn descubre_las_composiciones_del_proyecto_en_orden() {
        let root = directorio("descubrimiento");
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"a1.md\", \"a2.md\"]\n[[composition]]\nname = \"B\"\nfiles = [\"b.md\"]\npage_break_between_files = true\n",
        )
        .unwrap();
        let compositions = discover(&root);
        let nombres: Vec<&str> = compositions.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(nombres, ["A", "B"]);
        assert_eq!(compositions[0].files, ["a1.md", "a2.md"]);
        assert!(!compositions[0].page_break_between_files);
        assert!(compositions[1].page_break_between_files);

        // Sin `.mdproject.toml` no hay composiciones (acciones deshabilitadas).
        let vacio = root.join("sin-config");
        std::fs::create_dir_all(&vacio).unwrap();
        assert!(discover(&vacio).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
