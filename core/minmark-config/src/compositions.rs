//! Composiciones multiarchivo (§20): modelo y parser de las tablas
//! `[[composition]]` de `.mdproject.toml`.
//!
//! Parser de líneas propio, sin dependencias (§4), consciente de comentarios
//! `#` y cadenas entrecomilladas, en el mismo estilo minimalista que
//! [`crate::parse_scalar_layer`] y `find_toml_references` (§16).

use crate::{ConfigLayer, Level, Value, WarningReason, KEYS};

/// Composición multiarchivo (§20): documento lógico construido a partir de
/// varios `.md` sin concatenerlos físicamente.
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    /// Nombre de la composición dentro del archivo (obligatorio, §20).
    pub name: String,
    /// Rutas `.md` en el orden declarado (§20.1).
    pub files: Vec<String>,
    /// Salto de página entre archivos (§20.3). Por defecto `false`.
    pub page_break_between_files: bool,
    /// Ajustes declarados en la tabla como capa de nivel composición (§58.1).
    pub overrides: ConfigLayer,
}

/// Aviso no bloqueante emitido al parsear composiciones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionWarning {
    /// Nombre de la tabla afectada, si se conoce.
    pub name: Option<String>,
    /// Clave implicada, si procede.
    pub key: Option<String>,
    /// Motivo por el que se ignora la tabla o la clave.
    pub reason: WarningReason,
}

/// Parsea las tablas `[[composition]]` de un `.mdproject.toml` (§20).
///
/// Cada tabla empieza en una cabecera `[[composition]]` a columna 0 y su
/// cuerpo llega hasta la siguiente cabecera `[...]`/`[[...]]` o hasta el fin
/// del archivo; tablas ajenas se ignoran con su cuerpo. Las tablas pueden
/// intercalarse con escalares de proyecto (esos no forman parte del
/// resultado; para la capa de proyecto/global véase
/// [`crate::parse_scalar_layer`]).
///
/// Reglas:
/// - `name` (cadena obligatoria): vacío, ausente o no entrecomillado → aviso
///   [`WarningReason::InvalidValue`] y se descarta la tabla entera.
/// - `files` (lista obligatoria, orden declarado §20.1): vacía o ausente →
///   aviso [`WarningReason::InvalidValue`] y se descarta la tabla entera.
/// - `page_break_between_files` (§20.3): booleano, por defecto `false`; el
///   valor se guarda en [`Composition::page_break_between_files`] y también en
///   [`Composition::overrides`] para que [`crate::resolve`] lo aplique como
///   ajuste de nivel composición.
/// - El resto de claves de [`KEYS`] pasan a [`Composition::overrides`] con la
///   misma conversión de valores que [`crate::parse_scalar_layer`]; las que no
///   están en `KEYS` → aviso [`WarningReason::UnknownKey`]; valores inválidos
///   → aviso [`WarningReason::InvalidValue`]. En todos los casos la clave se
///   ignora sin bloquear (§58.1).
/// - Nombres duplicados en el mismo archivo: se conserva el primero y los
///   siguientes se descartan con aviso [`WarningReason::InvalidValue`].
pub fn parse_compositions(source: &str) -> (Vec<Composition>, Vec<CompositionWarning>) {
    let mut compositions: Vec<Composition> = Vec::new();
    let mut warnings = Vec::new();

    for body in composition_bodies(source) {
        let entries = table_entries(&body);

        // name obligatorio: cadena no vacía; sin él se descarta la tabla
        // entera sin leer el resto de sus claves.
        let name = entries
            .iter()
            .find(|(key, _)| key == "name")
            .and_then(|(_, raw)| parse_text(raw))
            .filter(|name| !name.trim().is_empty());
        let Some(name) = name else {
            warnings.push(CompositionWarning {
                name: None,
                key: Some("name".to_string()),
                reason: WarningReason::InvalidValue,
            });
            continue;
        };
        if compositions.iter().any(|c| c.name == name) {
            // Duplicado: se conserva la primera tabla con ese nombre (§20).
            warnings.push(CompositionWarning {
                name: Some(name),
                key: Some("name".to_string()),
                reason: WarningReason::InvalidValue,
            });
            continue;
        }

        // files obligatoria y no vacía; sin ella se descarta la tabla entera.
        let files = entries
            .iter()
            .find(|(key, _)| key == "files")
            .map(|(_, raw)| parse_file_list(raw))
            .unwrap_or_default();
        if files.is_empty() {
            warnings.push(CompositionWarning {
                name: Some(name),
                key: Some("files".to_string()),
                reason: WarningReason::InvalidValue,
            });
            continue;
        }

        let mut composition = Composition {
            name,
            files,
            page_break_between_files: false,
            overrides: ConfigLayer::new(Level::Composition),
        };

        for (key, raw) in &entries {
            match key.as_str() {
                "name" | "files" => {}
                "page_break_between_files" => match parse_scalar(raw) {
                    // §20.3: el valor vive en el campo y en overrides para
                    // que resolve() lo lea como ajuste composición.
                    Some(Value::Bool(value)) => {
                        composition.page_break_between_files = value;
                        composition.overrides.set(key, Value::Bool(value));
                    }
                    _ => warnings.push(CompositionWarning {
                        name: Some(composition.name.clone()),
                        key: Some(key.clone()),
                        reason: WarningReason::InvalidValue,
                    }),
                },
                _ if !KEYS.iter().any(|(known, _)| *known == key) => {
                    warnings.push(CompositionWarning {
                        name: Some(composition.name.clone()),
                        key: Some(key.clone()),
                        reason: WarningReason::UnknownKey,
                    });
                }
                _ => match parse_scalar(raw) {
                    Some(value) => composition.overrides.set(key, value),
                    None => warnings.push(CompositionWarning {
                        name: Some(composition.name.clone()),
                        key: Some(key.clone()),
                        reason: WarningReason::InvalidValue,
                    }),
                },
            }
        }

        compositions.push(composition);
    }

    (compositions, warnings)
}

// --- Auxiliares de lectura ---------------------------------------------------

/// Cuerpos de las tablas `[[composition]]`, línea a línea. Una cabecera es
/// cualquier línea que empieza por `[` en la columna 0; las tablas ajenas se
/// descartan con su cuerpo.
fn composition_bodies(source: &str) -> Vec<Vec<&str>> {
    let mut bodies = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in source.lines() {
        if line.starts_with('[') {
            if let Some(body) = current.take() {
                bodies.push(body);
            }
            let header = line.split('#').next().unwrap_or("").trim();
            if header == "[[composition]]" {
                current = Some(Vec::new());
            }
        } else if let Some(body) = current.as_mut() {
            body.push(line);
        }
    }
    if let Some(body) = current {
        bodies.push(body);
    }
    bodies
}

/// Pares `clave = valor` del cuerpo de una tabla. Una lista multilínea (`[`
/// sin cerrar) se acumula en su valor hasta cerrar los corchetes; líneas en
/// blanco, comentarios y líneas sin `=` se ignoran.
fn table_entries(body: &[&str]) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let line = body[index];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            index += 1;
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            index += 1;
            continue;
        };
        let mut raw = raw.trim().to_string();
        while unclosed_brackets(&raw) > 0 && index + 1 < body.len() {
            index += 1;
            raw.push('\n');
            raw.push_str(body[index]);
        }
        entries.push((key.trim().to_string(), raw));
        index += 1;
    }
    entries
}

/// Corchetes sin cerrar en `text`, ignorando comentarios `#` y cadenas
/// entrecomilladas (incluidas las multilíneas `"""` / `'''`).
fn unclosed_brackets(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            quote @ (b'"' | b'\'') => index = skip_string(bytes, index, quote),
            b'[' => {
                depth += 1;
                index += 1;
            }
            b']' => {
                depth -= 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    depth
}

/// Índice justo después de la cadena que empieza en `start` (con `\"` dentro
/// de las dobles y `"""`/`'''` multilínea); sin cierre, el final del texto.
fn skip_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    if bytes[start..].starts_with(&[quote, quote, quote]) {
        return bytes[start + 3..]
            .windows(3)
            .position(|window| window == [quote, quote, quote])
            .map(|pos| start + 3 + pos + 3)
            .unwrap_or(bytes.len());
    }
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' && quote == b'"' {
            index += 2;
            continue;
        }
        if bytes[index] == quote {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

/// Contenido entre el primer `[` y su `]` de cierre, fuera de comentarios y
/// cadenas; `None` si no hay lista cerrada.
fn bracket_contents(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    let mut opened: Option<usize> = None;
    let mut depth = 0usize;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            quote @ (b'"' | b'\'') => index = skip_string(bytes, index, quote),
            b'[' => {
                opened.get_or_insert(index);
                depth += 1;
                index += 1;
            }
            b']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[opened? + 1..index]);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Elementos de una lista TOML: cadenas entrecomilladas (simples o dobles,
/// con `\"` escapado en las dobles) o tokens sin comillas; comentarios `#` y
/// comas se ignoran. Se conserva el orden declarado (§20.1).
fn list_items(inner: &str) -> Vec<String> {
    let bytes = inner.as_bytes();
    let mut items = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'#' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            quote @ (b'"' | b'\'') => {
                let lead = if bytes[index..].starts_with(&[quote, quote, quote]) {
                    3
                } else {
                    1
                };
                let end = skip_string(bytes, index, quote);
                items.push(inner[index + lead..end.saturating_sub(lead)].to_owned());
                index = end;
            }
            b',' | b'[' | b']' => index += 1,
            byte if byte.is_ascii_whitespace() => index += 1,
            _ => {
                let start = index;
                while index < bytes.len()
                    && !bytes[index].is_ascii_whitespace()
                    && !matches!(bytes[index], b',' | b'#' | b'[' | b']')
                {
                    index += 1;
                }
                items.push(inner[start..index].to_owned());
            }
        }
    }
    items
}

/// Lista de archivos `files = [...]`, con elementos vacíos descartados; vacía
/// si la lista no está cerrada o no contiene rutas.
fn parse_file_list(raw: &str) -> Vec<String> {
    bracket_contents(raw)
        .map(list_items)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| !item.is_empty())
        .collect()
}

/// Convierte un valor literal como hace [`crate::parse_scalar_layer`]: bool,
/// entero, decimal o cadena entrecomillada con comentario `#` opcional al
/// final; el resto no es un valor válido.
fn parse_scalar(raw: &str) -> Option<Value> {
    let raw = raw.trim();
    if let Some(quote) = raw.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = raw[1..].find(quote)? + 1;
        let suffix = raw[end + 1..].trim();
        if !suffix.is_empty() && !suffix.starts_with('#') {
            return None;
        }
        return Some(Value::Text(raw[1..end].to_owned()));
    }
    let raw = raw.split('#').next().unwrap_or("").trim();
    match raw {
        "true" => Some(Value::Bool(true)),
        "false" => Some(Value::Bool(false)),
        _ => {
            if let Ok(value) = raw.parse::<i64>() {
                Some(Value::Integer(value))
            } else if let Ok(value) = raw.parse::<f64>() {
                Some(Value::Number(value))
            } else {
                None
            }
        }
    }
}

/// Cadena entrecomillada del valor, si lo es.
fn parse_text(raw: &str) -> Option<String> {
    match parse_scalar(raw) {
        Some(Value::Text(text)) => Some(text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_scalar_layer, resolve, Settings};

    fn warning(warning: &CompositionWarning) -> (Option<&str>, Option<&str>, WarningReason) {
        (
            warning.name.as_deref(),
            warning.key.as_deref(),
            warning.reason,
        )
    }

    #[test]
    fn composicion_completa_con_archivos_multilinea_y_overrides_20() {
        // §20: name + files multilínea con comentarios y comillas simples y
        // dobles; §20.3 page_break_between_files; override escalar ajeno a la
        // tabla de composiciones (pdf_orientation).
        let source = "\
[[composition]]
name = \"Libro\"
files = [
    \"title.md\",   # portada
    'intro.md',    # comillas simples
    \"chapter-01.md\" # sin coma final
]
page_break_between_files = true
pdf_orientation = \"landscape\"
";
        let (compositions, warnings) = parse_compositions(source);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(compositions.len(), 1);
        let composition = &compositions[0];
        assert_eq!(composition.name, "Libro");
        assert_eq!(composition.files, ["title.md", "intro.md", "chapter-01.md"]);
        assert!(composition.page_break_between_files);
        assert_eq!(composition.overrides.level, Level::Composition);
        assert_eq!(
            composition
                .overrides
                .entries
                .get("page_break_between_files"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            composition.overrides.entries.get("pdf_orientation"),
            Some(&Value::Text("landscape".to_string()))
        );

        // resolve() obtiene el salto de página vía Settings (§20.3, §58.1).
        let resolved = resolve(&[composition.overrides.clone()]);
        assert!(resolved.settings.page_break_between_files);
        assert_eq!(
            resolved.settings.pdf_orientation,
            crate::Orientation::Landscape
        );
        assert!(resolved.warnings.is_empty(), "{:?}", resolved.warnings);
    }

    #[test]
    fn defaults_page_break_falso_y_overrides_vacios_20_3() {
        let (compositions, warnings) =
            parse_compositions("[[composition]]\nname = \"A\"\nfiles = [\"a.md\"]\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        let composition = &compositions[0];
        assert!(!composition.page_break_between_files);
        assert!(composition.overrides.entries.is_empty());
        assert!(!Settings::default().page_break_between_files);
    }

    #[test]
    fn name_vacio_o_ausente_descarta_la_tabla() {
        // Vacío: aviso sin nombre conocido y tabla entera descartada.
        let (compositions, warnings) =
            parse_compositions("[[composition]]\nname = \"\"\nfiles = [\"a.md\"]\n");
        assert!(compositions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warning(&warnings[0]),
            (None, Some("name"), WarningReason::InvalidValue)
        );

        // Ausente: mismo aviso; el resto de la tabla no se llega a leer.
        let (compositions, warnings) = parse_compositions("[[composition]]\nfiles = [\"a.md\"]\n");
        assert!(compositions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warning(&warnings[0]),
            (None, Some("name"), WarningReason::InvalidValue)
        );
    }

    #[test]
    fn files_vacia_o_ausente_descarta_la_tabla() {
        let (compositions, warnings) =
            parse_compositions("[[composition]]\nname = \"A\"\nfiles = []\n");
        assert!(compositions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warning(&warnings[0]),
            (Some("A"), Some("files"), WarningReason::InvalidValue)
        );

        let (compositions, warnings) = parse_compositions("[[composition]]\nname = \"A\"\n");
        assert!(compositions.is_empty());
        assert_eq!(
            warning(&warnings[0]),
            (Some("A"), Some("files"), WarningReason::InvalidValue)
        );
    }

    #[test]
    fn clave_desconocida_con_aviso_y_tabla_conservada() {
        let (compositions, warnings) = parse_compositions(
            "[[composition]]\nname = \"A\"\nfiles = [\"a.md\"]\npreferred_language = \"es\"\n",
        );
        assert_eq!(compositions.len(), 1);
        assert!(!compositions[0]
            .overrides
            .entries
            .contains_key("preferred_language"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warning(&warnings[0]),
            (
                Some("A"),
                Some("preferred_language"),
                WarningReason::UnknownKey
            )
        );
    }

    #[test]
    fn valor_invalido_con_aviso_y_clave_ignorada() {
        // Valor no convertible (no es bool/entero/decimal/cadena): aviso y la
        // clave no llega a overrides; la tabla se conserva.
        let (compositions, warnings) = parse_compositions(
            "[[composition]]\nname = \"A\"\nfiles = [\"a.md\"]\npdf_orientation = landscape\n",
        );
        assert_eq!(compositions.len(), 1);
        assert!(!compositions[0]
            .overrides
            .entries
            .contains_key("pdf_orientation"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warning(&warnings[0]),
            (
                Some("A"),
                Some("pdf_orientation"),
                WarningReason::InvalidValue
            )
        );

        // page_break_between_files inválido: campo y overrides en default.
        let (compositions, warnings) = parse_compositions(
            "[[composition]]\nname = \"A\"\nfiles = [\"a.md\"]\npage_break_between_files = maybe\n",
        );
        assert_eq!(compositions.len(), 1);
        assert!(!compositions[0].page_break_between_files);
        assert!(compositions[0].overrides.entries.is_empty());
        assert_eq!(
            warning(&warnings[0]),
            (
                Some("A"),
                Some("page_break_between_files"),
                WarningReason::InvalidValue
            )
        );
    }

    #[test]
    fn nombre_duplicado_conserva_el_primero() {
        let source = "\
[[composition]]
name = \"Libro\"
files = [\"primero.md\"]
[[composition]]
name = \"Libro\"
files = [\"segundo.md\"]
page_break_between_files = true
";
        let (compositions, warnings) = parse_compositions(source);
        assert_eq!(compositions.len(), 1);
        assert_eq!(compositions[0].files, ["primero.md"]);
        assert!(!compositions[0].page_break_between_files);
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            warning(&warnings[0]),
            (Some("Libro"), Some("name"), WarningReason::InvalidValue)
        );
    }

    #[test]
    fn orden_de_files_preservado_20_1() {
        let files = ["zeta.md", "alfa.md", "01-capitulo.md", "beta.md"];
        let source = format!(
            "[[composition]]\nname = \"Orden\"\nfiles = [\"{}\"]\n",
            files.join("\", \"")
        );
        let (compositions, _) = parse_compositions(&source);
        assert_eq!(compositions[0].files, files);
    }

    #[test]
    fn varias_composiciones_en_el_mismo_archivo() {
        let source = "\
[[composition]]
name = \"A\"
files = [\"a1.md\", \"a2.md\"]
[[composition]]
name = \"B\"
files = [\"b1.md\"]
page_break_between_files = true
[[composition]]
name = \"C\"
files = [
    \"c1.md\",
    \"c2.md\",
    \"c3.md\"
]
";
        let (compositions, warnings) = parse_compositions(source);
        assert!(warnings.is_empty(), "{warnings:?}");
        let names: Vec<&str> = compositions.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "C"]);
        assert_eq!(compositions[0].files, ["a1.md", "a2.md"]);
        assert_eq!(compositions[1].files, ["b1.md"]);
        assert!(compositions[1].page_break_between_files);
        assert_eq!(compositions[2].files, ["c1.md", "c2.md", "c3.md"]);
        assert!(!compositions[2].page_break_between_files);
    }

    #[test]
    fn escalares_de_proyecto_antes_de_las_tablas_sin_ingerir_claves_internas() {
        // Escalares raíz antes de las tablas [[composition]] (§58.1 nivel
        // proyecto) leídos como capa de proyecto; las claves dentro de las
        // tablas (incluida la última, hasta EOF) no llegan a la capa.
        let source = "\
wiki_links = true
toc_max_level = 4
[[composition]]
name = \"A\"
files = [\"a.md\"]
language = 'fr'
pdf_orientation = \"landscape\"
[[composition]]
name = \"B\"
files = [\"b.md\"]
language = 'fr'
heading_numbering = false
";
        let layer = parse_scalar_layer(source, Level::Project);
        assert_eq!(
            layer.entries.get("wiki_links"),
            Some(&Value::Bool(true)),
            "escalar raíz antes de las tablas"
        );
        assert_eq!(
            layer.entries.get("toc_max_level"),
            Some(&Value::Integer(4)),
            "escalar raíz antes de las tablas"
        );
        assert!(!layer.entries.contains_key("language"));
        assert!(!layer.entries.contains_key("pdf_orientation"));
        assert!(!layer.entries.contains_key("heading_numbering"));

        // Las mismas tablas se parsean como composiciones completas.
        let (compositions, warnings) = parse_compositions(source);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(compositions.len(), 2);
        assert_eq!(compositions[0].files, ["a.md"]);
        assert_eq!(compositions[1].files, ["b.md"]);
    }
}
