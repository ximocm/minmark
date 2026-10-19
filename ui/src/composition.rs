//! Composiciones multiarchivo (§20, ISSUE-022/023): descubrimiento, fuente
//! combinada y render/export del documento lógico.
//!
//! - [`discover`] lee las tablas `[[composition]]` del `.mdproject.toml` de
//!   la raíz del proyecto activo (§20); [`refresh`] redescubre sobre la
//!   caché de la ventana al abrir el proyecto y con el watcher cuando ese
//!   archivo cambia (B1, revisión PR #44), de modo que `files` y `overrides`
//!   salen siempre de la misma lectura que la cadena §58. Sin proyecto no
//!   hay composiciones y las acciones quedan deshabilitadas.
//! - [`build`] carga cada archivo en orden declarado (§20.1): el buffer
//!   abierto manda sobre el disco (§6.1, ISSUE-023), una ruta fuera de la
//!   raíz no se lee nunca y produce «Archivo fuera del ámbito» (§48, B2) y
//!   un archivo ausente produce un bloque de diagnóstico visible **en su
//!   posición** sin abortar el resto. Consultar o renderizar nunca escribe
//!   los fuentes (§20).
//! - [`render`] produce el Preview combinado (§20.4): continuo sobre la AST
//!   compuesta o paginado con el camino Typst existente, según el ajuste
//!   `pagination` efectivo (cadena §58).
//!
//! Módulo sin widgets: todo su comportamiento es verificable sin display
//! (estilo `ui/src/export.rs`).

use minmark_config::{Composition, Settings};
use minmark_document::{compose, Block, BlockKind, ComposePart, Composed, Inline, Span};
use minmark_pdf::DocumentMeta;
use std::path::{Component, Path, PathBuf};
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

/// Redescubrimiento de la ventana (B1, revisión PR #44, #22): relée el
/// `.mdproject.toml` de `root` y deja en `cache` las composiciones actuales,
/// devolviendo si la lista quedó no vacía — el estado `enabled` de las
/// acciones de composición (§20, ISSUE-022). Se usa al abrir el proyecto
/// (`build_window`) y con el watcher cuando cambia `.mdproject.toml`, de
/// modo que la caché de la ventana sigue al archivo en cuanto cambia. La
/// cadena §58 (`load_composition_at`) vuelve a leerlo al construir: `files`
/// y `overrides` son de lecturas próximas, eventualmente consistentes con
/// el watcher, no de una misma lectura atómica. Sin proyecto, la cache
/// queda vacía.
pub fn refresh(cache: &mut Vec<Composition>, root: Option<&Path>) -> bool {
    *cache = root.map(discover).unwrap_or_default();
    !cache.is_empty()
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
        // §48 (B2): una ruta fuera de la raíz NO se lee nunca (ni del disco
        // ni del buffer abierto); su hueco conserva la posición con el
        // diagnóstico visible, sin abortar el resto (mismo patrón que
        // «Archivo ausente», §20.1).
        let fuente = if fuera_del_ambito(root, Path::new(rel)) {
            format!("Archivo fuera del ámbito: {rel}\n")
        } else {
            let abierto = buffers
                .iter()
                .find(|(abierto, _)| comparable(abierto) == comparable(&path));
            match abierto {
                Some((_, fuente)) => fuente.clone(),
                None => std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| format!("Archivo ausente: {rel}\n")),
            }
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

/// Frontera §48 (B2, revisión PR #44): ¿`rel` — declarada en
/// `[[composition]].files` — se sale de `root`? Chequeo léxico (relativa,
/// sin `..` que salga de la raíz, no absoluta) y canónico con la misma
/// política que [`minmark_fs::resolve_in_project`]: la ruta resuelta debe
/// seguir dentro de la raíz canónica, lo que caza el `..` intermedio y los
/// symlinks que escapan. Si el destino no existe se resuelve el ancestro
/// existente más cercano, de modo que un padre que escapa también está
/// fuera de ámbito aunque la canónica del fichero completo no resuelva
/// (revisión PR #44). Un error de E/S no etiqueta la ruta como fuera de
/// ámbito: su hueco lo diagnostica después la lectura («Archivo ausente»,
/// §20.1).
fn fuera_del_ambito(root: &Path, rel: &Path) -> bool {
    if rel.is_absolute() {
        return true;
    }
    let mut profundidad = 0usize;
    for componente in rel.components() {
        match componente {
            Component::CurDir => {}
            Component::Normal(_) => profundidad += 1,
            Component::ParentDir => {
                if profundidad == 0 {
                    return true;
                }
                profundidad -= 1;
            }
            // Prefijo (`C:\`) u otra grafía que no es una relativa válida.
            Component::Prefix(_) | Component::RootDir => return true,
        }
    }
    // Reutiliza la política §48 del core en lugar de repetirla: resuelve el
    // ancestro existente más cercano y solo un escape real está fuera de
    // ámbito (un error de E/S no etiqueta la ruta).
    matches!(
        minmark_fs::resolve_in_project(root, rel),
        Err(minmark_fs::FsError::PathOutsideProject { .. })
    )
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

    /// §48 (B2, revisión PR #44): `files` con `../` que sale de la raíz NO
    /// se lee nunca: el hueco conserva su posición con el diagnóstico
    /// «Archivo fuera del ámbito» (§20.1) y el contenido de fuera jamás
    /// aparece en la fuente virtual ni en el Preview.
    #[test]
    fn files_con_parent_dir_fuera_del_ambito_diagnostican_y_no_se_leen() {
        let base = std::env::temp_dir().join(format!(
            "minmark-composition-ambito-parent-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("proyecto");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"dentro.md\", \"../fuera.md\", \"ultimo.md\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("dentro.md"), "Contenido dentro.\n").unwrap();
        std::fs::write(root.join("ultimo.md"), "Contenido último.\n").unwrap();
        std::fs::write(base.join("fuera.md"), "SECRETO-FUERA-DEL-AMBITO\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        let fuente = &combined.composed.source;
        let diagnostico = fuente
            .find("Archivo fuera del ámbito: ../fuera.md")
            .expect("diagnóstico visible en su posición");
        let dentro = fuente.find("Contenido dentro.").expect("primera parte");
        let ultimo = fuente.find("Contenido último.").expect("tercera parte");
        assert!(
            dentro < diagnostico && diagnostico < ultimo,
            "el hueco va en su posición"
        );
        assert!(
            !fuente.contains("SECRETO-FUERA-DEL-AMBITO"),
            "el contenido de fuera de la raíz se filtró en la fuente virtual"
        );

        let html = render(build(&root, &composition, &[]));
        assert!(html.contains("Archivo fuera del ámbito: ../fuera.md"));
        assert!(!html.contains("SECRETO-FUERA-DEL-AMBITO"));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// §48 (B2): una ruta absoluta en `files` no se lee nunca y produce el
    /// mismo bloque de diagnóstico en su posición, sin abortar el resto.
    #[test]
    fn files_absolutos_fuera_del_ambito_no_se_leen() {
        let root = directorio("absoluta");
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"/etc/passwd\", \"uno.md\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("uno.md"), "Contenido uno.\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        let fuente = &combined.composed.source;
        assert!(
            fuente.contains("Archivo fuera del ámbito: /etc/passwd"),
            "diagnóstico ausente: {fuente}"
        );
        assert!(
            !fuente.contains("root:"),
            "el contenido de /etc/passwd se filtró en la fuente virtual"
        );
        assert!(fuente.contains("Contenido uno."), "el resto no se aborta");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// §48/§20.1 (B2): una ruta relativa DENTRO de la raíz, en
    /// subdirectorio, se lee normalmente (la frontera no la bloquea).
    #[test]
    fn files_en_subdirectorio_dentro_del_ambito_se_leen() {
        let root = directorio("subdirectorio");
        std::fs::create_dir_all(root.join("caps")).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"caps/uno.md\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("caps/uno.md"), "Contenido en subdirectorio.\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let combined = build(&root, &composition, &[]);
        assert!(combined
            .composed
            .source
            .contains("Contenido en subdirectorio."));
        assert!(!combined.composed.source.contains("fuera del ámbito"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// §48 (B2): el chequeo canónico (como `resource_paths`) caza lo que una
    /// comprobación léxica ingenua no ve: el `..` intermedio que escapa
    /// (`docs/../../fuera.md`) y el symlink dentro de la raíz que apunta
    /// fuera (`enlace/fuera.md`).
    #[test]
    fn files_que_escapan_canonicamente_no_se_leen() {
        let base = std::env::temp_dir().join(format!(
            "minmark-composition-ambito-canonico-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("proyecto");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(base.join("fuera.md"), "SECRETO-CANONICO\n").unwrap();
        std::os::unix::fs::symlink(&base, root.join("enlace")).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"docs/../../fuera.md\"]\n[[composition]]\nname = \"B\"\nfiles = [\"enlace/fuera.md\"]\n",
        )
        .unwrap();

        for nombre in ["A", "B"] {
            let composition = discover(&root)
                .into_iter()
                .find(|c| c.name == nombre)
                .expect("composición");
            let combined = build(&root, &composition, &[]);
            let fuente = &combined.composed.source;
            assert!(
                fuente.contains("Archivo fuera del ámbito"),
                "{nombre}: diagnóstico ausente: {fuente}"
            );
            assert!(
                !fuente.contains("SECRETO-CANONICO"),
                "{nombre}: el contenido fuera de la raíz se filtró"
            );
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    /// §48 (B2, revisión PR #44): un destino cuyo PADRE es un symlink que
    /// escapa está fuera de ámbito aunque el fichero completo no exista (la
    /// canónica falla por NotFound): se resuelve el ancestro existente más
    /// cercano y no se lee ni del disco ni del buffer abierto. Un fichero
    /// ausente DENTRO de la raíz sigue siendo «Archivo ausente» (§20.1).
    #[test]
    fn files_bajo_symlink_escapado_sin_fichero_diagnostican_y_no_usan_el_buffer() {
        let base = std::env::temp_dir().join(format!(
            "minmark-composition-ambito-symlink-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("proyecto");
        std::fs::create_dir_all(root.join("caps")).unwrap();
        std::fs::create_dir_all(base.join("exterior")).unwrap();
        std::os::unix::fs::symlink(base.join("exterior"), root.join("enlace")).unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"enlace/falta.md\"]\n[[composition]]\nname = \"B\"\nfiles = [\"caps/falta.md\"]\n",
        )
        .unwrap();

        // El buffer abierto de la ruta que escapa tampoco se cuela.
        let buffers = [(
            root.join("enlace/falta.md"),
            "BUFFER-FUERA-DEL-AMBITO\n".to_string(),
        )];
        let composition = discover(&root)
            .into_iter()
            .find(|c| c.name == "A")
            .expect("composición A");
        let fuente = &build(&root, &composition, &buffers).composed.source;
        assert!(
            fuente.contains("Archivo fuera del ámbito: enlace/falta.md"),
            "diagnóstico ausente: {fuente}"
        );
        assert!(
            !fuente.contains("BUFFER-FUERA-DEL-AMBITO"),
            "el buffer de la ruta que escapa se coló en la fuente virtual"
        );

        // Contraprueba: la misma ausencia dentro de la raíz no es un escape.
        let dentro = discover(&root)
            .into_iter()
            .find(|c| c.name == "B")
            .expect("composición B");
        let fuente = &build(&root, &dentro, &[]).composed.source;
        assert!(
            fuente.contains("Archivo ausente: caps/falta.md"),
            "dentro de la raíz el hueco es «Archivo ausente»: {fuente}"
        );
        assert!(!fuente.contains("fuera del ámbito"), "{fuente}");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// S1/ISSUE-024 (revisión PR #44): el Preview combinado conserva los
    /// destinos internos `#fragmento` (§20.4) y las anclas `id` de los
    /// headings (§26) — la navegación por scroll in-page se apoya en ambos.
    #[test]
    fn preview_combinado_conserva_las_anclas_y_los_destinos_internos() {
        let root = directorio("destinos-internos");
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"Libro\"\nfiles = [\"uno.md\", \"dos.md\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("uno.md"), "[Ir al destino](#destino)\n").unwrap();
        std::fs::write(root.join("dos.md"), "## Destino {#destino}\n").unwrap();

        let composition = discover(&root).pop().expect("composición");
        let html = render(build(&root, &composition, &[]));
        assert!(
            html.contains("href=\"#destino\""),
            "el destino interno se reescribió: falta href"
        );
        assert!(
            html.contains("id=\"destino\""),
            "falta la ancla en el combinado"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// B1 (revisión PR #44, #22): el redescubrimiento tras cambiar
    /// `.mdproject.toml` devuelve la cache de la ventana a la MISMA lectura
    /// que la cadena §58: la nueva composición aparece, `files` y `overrides`
    /// salen de la tabla nueva y el estado de las acciones (no vacía ⇒
    /// habilitadas, vacía ⇒ deshabilitadas) queda acorde.
    #[test]
    fn redescubre_las_composiciones_tras_cambiar_el_mdproject() {
        let root = directorio("redescubrir");
        std::fs::create_dir_all(root.join("caps")).unwrap();
        std::fs::write(root.join("uno.md"), "Contenido uno.\n").unwrap();
        std::fs::write(root.join("caps/dos.md"), "Contenido dos.\n").unwrap();
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"A\"\nfiles = [\"uno.md\"]\npdf_orientation = \"landscape\"\n",
        )
        .unwrap();

        let mut cache = Vec::new();
        assert!(
            refresh(&mut cache, Some(&root)),
            "con composiciones las acciones quedan habilitadas (§20)"
        );
        assert_eq!(cache.len(), 1);

        // Edición en caliente: A cambia de `files` y de `overrides` y nace B.
        std::fs::write(
            root.join(".mdproject.toml"),
            "pdf_page_size = \"Letter\"\n[[composition]]\nname = \"A\"\nfiles = [\"caps/dos.md\"]\npdf_orientation = \"portrait\"\n[[composition]]\nname = \"B\"\nfiles = [\"uno.md\"]\n",
        )
        .unwrap();
        assert!(refresh(&mut cache, Some(&root)));
        let nombres: Vec<String> = cache.iter().map(|c| c.name.clone()).collect();
        assert_eq!(nombres, ["A", "B"], "la nueva composición aparece");

        // Coherencia §58: `files`/`overrides` de la cache y el ajuste de
        // proyecto que relee `load_composition_at` salen de la MISMA lectura.
        let combined = build(&root, &cache[0], &[]);
        assert!(
            combined.composed.source.contains("Contenido dos."),
            "files de la tabla nueva"
        );
        assert!(!combined.composed.source.contains("Contenido uno."));
        assert_eq!(
            combined.settings.pdf_orientation,
            Orientation::Portrait,
            "overrides de la tabla nueva (§58.1)"
        );
        assert_eq!(
            combined.settings.pdf_page_size,
            minmark_config::PageSize::Letter,
            "proyecto de la misma lectura que files/overrides (§58)"
        );

        // Sin composiciones ⇒ acciones deshabilitadas; al volver ⇒ habilitadas.
        std::fs::write(root.join(".mdproject.toml"), "pdf_page_size = \"Letter\"\n").unwrap();
        assert!(
            !refresh(&mut cache, Some(&root)),
            "sin composiciones las acciones quedan deshabilitadas (§20)"
        );
        assert!(cache.is_empty());
        std::fs::write(
            root.join(".mdproject.toml"),
            "[[composition]]\nname = \"B\"\nfiles = [\"uno.md\"]\n",
        )
        .unwrap();
        assert!(refresh(&mut cache, Some(&root)));
        std::fs::remove_dir_all(root).unwrap();
    }
}
