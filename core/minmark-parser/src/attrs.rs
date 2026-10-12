//! Gramática de atributos (§7.4.3).
//!
//! Forma: `{#id "caption" clave=valor ...}` — `#id` (`[A-Za-z0-9:._-]+`),
//! caption entrecomillado y pares `clave=valor`. Único valor con significado
//! en v1: `width=N%` con N en 1–100 (§18.5). Cualquier otra cosa no encaja
//! con la gramática y la construcción completa es texto literal (§7.4.1).

use minmark_document::Attributes;

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-')
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

fn is_value_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '%')
}

/// Parsea el interior de `{...}` (sin las llaves) como atributos.
/// `None` = no encaja con la gramática (la construcción será texto literal).
pub(crate) fn parse_attributes(inner: &str) -> Option<Attributes> {
    let mut attrs = Attributes::default();
    let mut cur = inner.chars().peekable();
    let mut seen_id = false;
    let mut seen_caption = false;

    loop {
        while matches!(cur.peek(), Some(c) if c.is_whitespace()) {
            cur.next();
        }
        match cur.peek() {
            None => break,
            Some('#') => {
                if seen_id {
                    return None;
                }
                cur.next();
                let id: String = take_while(&mut cur, is_id_char);
                if id.is_empty() {
                    return None;
                }
                attrs.id = Some(id);
                seen_id = true;
            }
            Some('"') => {
                if seen_caption {
                    return None;
                }
                cur.next();
                let mut caption = String::new();
                loop {
                    match cur.next() {
                        Some('\\') => match cur.next() {
                            Some(c) => caption.push(c),
                            None => return None,
                        },
                        Some('"') => break,
                        Some(c) => caption.push(c),
                        None => return None,
                    }
                }
                attrs.caption = Some(caption);
                seen_caption = true;
            }
            Some(_) => {
                let key: String = take_while(&mut cur, is_key_char);
                if key.is_empty() || cur.peek() != Some(&'=') {
                    return None;
                }
                cur.next();
                let value: String = take_while(&mut cur, is_value_char);
                if value.is_empty() {
                    return None;
                }
                if key == "width" {
                    // §18.5: solo porcentaje, 1%–100%.
                    let pct = value.strip_suffix('%')?;
                    let n: u8 = pct.parse().ok()?;
                    if !(1..=100).contains(&n) || attrs.width_percent.is_some() {
                        return None;
                    }
                    attrs.width_percent = Some(n);
                } else {
                    attrs.extra.push((key, value));
                }
            }
        }
    }
    Some(attrs)
}

fn take_while(cur: &mut std::iter::Peekable<std::str::Chars<'_>>, f: fn(char) -> bool) -> String {
    let mut out = String::new();
    while matches!(cur.peek(), Some(c) if f(*c)) {
        out.push(cur.next().unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formas_validas() {
        assert_eq!(parse_attributes("#architecture"), Some(Attributes { id: Some("architecture".into()), ..Default::default() }));

        let attrs = parse_attributes("#tbl:results \"Resultados del experimento\"").unwrap();
        assert_eq!(attrs.id.as_deref(), Some("tbl:results"));
        assert_eq!(attrs.caption.as_deref(), Some("Resultados del experimento"));

        let attrs = parse_attributes("width=50%").unwrap();
        assert_eq!(attrs.width_percent, Some(50));

        let attrs = parse_attributes("#fig:arq \"Diagrama\" width=30% other=x").unwrap();
        assert_eq!(attrs.id.as_deref(), Some("fig:arq"));
        assert_eq!(attrs.caption.as_deref(), Some("Diagrama"));
        assert_eq!(attrs.width_percent, Some(30));
        assert_eq!(attrs.extra, vec![("other".to_string(), "x".to_string())]);
    }

    #[test]
    fn formas_invalidas() {
        // §18.5: fuera de 1–100 no es gramática de atributos.
        assert_eq!(parse_attributes("width=150%"), None);
        assert_eq!(parse_attributes("width=0%"), None);
        assert_eq!(parse_attributes("width=50"), None);
        // Colores no son atributos (§7.4.4): hay dos puntos, sin `=`.
        assert_eq!(parse_attributes("danger: texto"), None);
        // Caption sin cerrar, id duplicado, clave sin valor.
        assert_eq!(parse_attributes("\"sin cerrar"), None);
        assert_eq!(parse_attributes("#a #b"), None);
        assert_eq!(parse_attributes("solo-clave"), None);
    }

    #[test]
    fn escapes_en_caption() {
        let attrs = parse_attributes("#x \"dijo: \\\"hola\\\"\"").unwrap();
        assert_eq!(attrs.caption.as_deref(), Some("dijo: \"hola\""));
    }
}
