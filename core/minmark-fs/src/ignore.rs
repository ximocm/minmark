//! Filtro `.gitignore` mínimo (ISSUE-023, §14.6).
//!
//! La exclusión de `.gitignore` aplica al análisis de enlaces, búsqueda e
//! indexado (§14.6); **nunca** al explorador. Este módulo implementa el
//! subconjunto aburrido y suficiente de la sintaxis git sobre el
//! `.gitignore` de la raíz del proyecto:
//!
//! - líneas vacías y comentarios `#`;
//! - negación `!`;
//! - sufijo `/` (solo directorios);
//! - patrón con `/` → anclado a la raíz; sin `/` → coincide con el nombre de
//!   cualquier nivel;
//! - comodines `*` (sin cruzar `/`) y `?` por segmento.
//!
//! Sin anidamiento de `.gitignore` ni `**` (frontera documentada en
//! ISSUE-023; ISSUE-025 lo generalizará con el índice).

use std::path::Path;

/// Reglas de un `.gitignore`, en orden (la última coincidencia decide).
#[derive(Debug, Clone, Default)]
pub struct IgnorePatterns {
    rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
struct Rule {
    negated: bool,
    dir_only: bool,
    /// El patrón contiene `/`: se ancla a la raíz y compara todos los
    /// segmentos.
    anchored: bool,
    segments: Vec<String>,
}

impl IgnorePatterns {
    /// Parsea el contenido de un `.gitignore` (reglas vacías = nada ignorado).
    pub fn parse(source: &str) -> Self {
        let mut rules = Vec::new();
        for line in source.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, line) = match line.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            let (dir_only, line) = match line.strip_suffix('/') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            if line.is_empty() {
                continue;
            }
            let anchored = line.starts_with('/') || line.trim_end_matches('/') .contains('/');
            let line = line.trim_start_matches('/');
            rules.push(Rule {
                negated,
                dir_only,
                anchored,
                segments: line.split('/').map(str::to_owned).collect(),
            });
        }
        Self { rules }
    }

    /// Carga `<root>/.gitignore` si existe; sin él, nada está ignorado.
    pub fn from_root(root: &Path) -> Self {
        std::fs::read_to_string(root.join(".gitignore"))
            .map(|source| Self::parse(&source))
            .unwrap_or_default()
    }

    /// ¿La ruta `rel` (relativa a la raíz del `.gitignore`) está ignorada?
    ///
    /// Un directorio ignorado arrastra todo su contenido (git).
    pub fn is_ignored(&self, rel: &Path, is_dir: bool) -> bool {
        let comps: Vec<&str> = rel
            .components()
            .map(|c| c.as_os_str().to_str().unwrap_or(""))
            .filter(|c| !c.is_empty())
            .collect();
        if comps.is_empty() {
            return false;
        }
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches(&comps, is_dir) {
                ignored = !rule.negated;
            }
        }
        ignored
    }
}

impl Rule {
    /// La regla coincide con la ruta o con alguno de sus ancestros (un
    /// directorio ignorado ignora su contenido).
    fn matches(&self, comps: &[&str], is_dir: bool) -> bool {
        for len in 1..=comps.len() {
            let target_is_dir = len < comps.len() || is_dir;
            if self.dir_only && !target_is_dir {
                continue;
            }
            if self.matches_target(&comps[..len]) {
                return true;
            }
        }
        false
    }

    fn matches_target(&self, target: &[&str]) -> bool {
        if self.anchored {
            target.len() == self.segments.len()
                && self
                    .segments
                    .iter()
                    .zip(target)
                    .all(|(pattern, name)| glob_match(pattern, name))
        } else {
            self.segments.len() == 1
                && target
                    .last()
                    .is_some_and(|name| glob_match(&self.segments[0], name))
        }
    }
}

/// Glob por segmento: `*` (sin cruzar `/`) y `?`; el resto literal.
fn glob_match(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let nam: Vec<char> = name.chars().collect();
    fn go(pat: &[char], nam: &[char]) -> bool {
        match (pat.first(), nam.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                go(&pat[1..], nam)
                    || (nam.first().is_some_and(|c| *c != '/') && go(pat, &nam[1..]))
            }
            (Some('?'), Some(c)) if *c != '/' => go(&pat[1..], &nam[1..]),
            (Some(p), Some(n)) if p == n => go(&pat[1..], &nam[1..]),
            _ => false,
        }
    }
    go(&pat, &nam)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ignored(source: &str, rel: &str, is_dir: bool) -> bool {
        IgnorePatterns::parse(source).is_ignored(Path::new(rel), is_dir)
    }

    #[test]
    fn basicos_comentarios_y_negacion() {
        let source = "# comentario\n\nsalida.md\n!conservar.md\n";
        assert!(ignored(source, "salida.md", false));
        assert!(ignored(source, "sub/salida.md", false), "sin /: cualquier nivel");
        assert!(ignored(source, "conservar.md", false) == false);
        assert!(!ignored(source, "otro.md", false));
    }

    #[test]
    fn solo_directorios_con_barra_final() {
        let source = "cache/\n";
        assert!(ignored(source, "cache", true));
        assert!(!ignored(source, "cache", false), "archivo llamado cache no");
        assert!(ignored(source, "a/cache/dentro.md", false), "contenido ignorado");
    }

    #[test]
    fn anclado_con_barra_interna() {
        let source = "docs/*.md\n/tmp2/\n";
        assert!(ignored(source, "docs/x.md", false));
        assert!(!ignored(source, "sub/docs/x.md", false), "anclado a la raíz");
        assert!(ignored(source, "tmp2/x.png", false));
        assert!(!ignored(source, "a/tmp2", true), "tmp2/ sin / interno no anclado");
    }

    #[test]
    fn comodines_por_segmento() {
        let source = "build-?/*.o\n*.tmp\n";
        assert!(ignored(source, "build-1/a.o", false));
        assert!(!ignored(source, "build-11/a.o", false), "? = un solo carácter");
        assert!(ignored(source, "x/y/z.tmp", false));
    }

    #[test]
    fn la_ultima_regla_decide() {
        let source = "*.log\n!debug.log\n";
        assert!(!ignored(source, "debug.log", false));
        assert!(ignored(source, "app.log", false));
    }

    #[test]
    fn glob_conoce_los_limites() {
        assert!(glob_match("a*b", "a-xxx-b"));
        assert!(glob_match("?.md", "x.md"));
        assert!(!glob_match("?.md", "xx.md"));
        assert!(!glob_match("a*b", "a/b"), "* no cruza /");
        assert!(glob_match("", ""));
    }

    #[test]
    fn sin_gitignore_no_se_ignora_nada() {
        let patterns = IgnorePatterns::from_root(Path::new("/no-existe-mm"));
        assert!(!patterns.is_ignored(Path::new("x.md"), false));
    }
}
