//! Traductor LaTeX → Typst para matemáticas (ADR-002).
//!
//! El usuario escribe LaTeX (§24) y Typst es el único motor de composición
//! (§5.3): la traducción es explícita y cubre el **subconjunto estándar** de
//! ADR-002 (fracciones, raíces, super/índices, `\left...\right`, griegas,
//! símbolos y operadores grandes, matrices, funciones y acentos).
//!
//! NFR-026: fuera del subconjunto no se silencia ni se deforma nada — el
//! resultado marca `unsupported` y el llamador emite el marcador visible
//! `⚠ [math no soportado: <comando>]` (ADR-002).

/// Resultado de la traducción de una expresión LaTeX.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathTranslation {
    /// Código Typst math (vacío si hay comandos no soportados).
    pub typst: String,
    /// Comandos fuera del subconjunto, en orden de aparición.
    pub unsupported: Vec<String>,
}

impl MathTranslation {
    pub fn is_supported(&self) -> bool {
        self.unsupported.is_empty()
    }

    /// Marcador visible en Preview y PDF para sintaxis no soportada
    /// (ADR-002: nunca silenciar, omitir ni deformar).
    pub fn marker(&self) -> String {
        format!("⚠ [math no soportado: {}]", self.unsupported.join(", "))
    }
}

/// Traduce una expresión LaTeX math a Typst math.
pub fn translate(latex: &str) -> MathTranslation {
    let mut parser = Parser {
        toks: tokenize(latex),
        pos: 0,
        unsupported: Vec::new(),
    };
    let exprs = parser.parse_seq(&Stop::group());
    let mut typst = String::new();
    emit(&exprs, &mut typst);
    let typst = typst.trim().to_string();
    if parser.unsupported.is_empty() {
        MathTranslation {
            typst,
            unsupported: Vec::new(),
        }
    } else {
        // NFR-026: con cualquier comando no soportado la ecuación completa se
        // sustituye por el marcador (no se deforma).
        MathTranslation {
            typst: String::new(),
            unsupported: parser.unsupported,
        }
    }
}

// --- Tokens -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Cmd(String),
    LBrace,
    RBrace,
    Sup,
    Sub,
    Amp,
    /// `\\` de LaTeX (fin de fila en matrices).
    RowSep,
    Char(char),
}

fn tokenize(latex: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut chars = latex.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some('\\') => {
                    chars.next();
                    toks.push(Tok::RowSep);
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    let mut name = String::new();
                    while let Some(c) = chars.peek() {
                        if c.is_ascii_alphabetic() {
                            name.push(*c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    toks.push(Tok::Cmd(name));
                }
                Some(c) => {
                    // Escapes como `\_`, `\{`, `\%`: carácter literal.
                    let c = *c;
                    chars.next();
                    toks.push(Tok::Char(c));
                }
                None => toks.push(Tok::Char('\\')),
            },
            '%' => {
                // Comentario LaTeX: hasta fin de línea.
                while let Some(c) = chars.peek() {
                    if *c == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '{' => toks.push(Tok::LBrace),
            '}' => toks.push(Tok::RBrace),
            '^' => toks.push(Tok::Sup),
            '_' => toks.push(Tok::Sub),
            '&' => toks.push(Tok::Amp),
            _ => toks.push(Tok::Char(c)),
        }
    }
    toks
}

// --- Parser -----------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Stop {
    at_rbrace: bool,
    at_rbracket: bool,
    at_amp: bool,
    at_rowsep: bool,
    at_end: bool,
}

impl Stop {
    fn group() -> Self {
        Self {
            at_rbrace: true,
            ..Self::none()
        }
    }
    fn none() -> Self {
        Self {
            at_rbrace: false,
            at_rbracket: false,
            at_amp: false,
            at_rowsep: false,
            at_end: false,
        }
    }
    fn cell() -> Self {
        Self {
            at_rbrace: true,
            at_amp: true,
            at_rowsep: true,
            at_end: true,
            at_rbracket: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Lit(String),
    Sym(&'static str),
    Sup(Box<Expr>),
    Sub(Box<Expr>),
    Frac(Box<Expr>, Box<Expr>),
    Sqrt {
        index: Option<Box<Expr>>,
        radicand: Box<Expr>,
    },
    Func(&'static str, Box<Expr>),
    Delim(char),
    Matrix {
        delim: char,
        rows: Vec<Vec<Expr>>,
    },
    Seq(Vec<Expr>),
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    unsupported: Vec<String>,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let tok = self.toks.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn stops(&self, stop: &Stop) -> bool {
        match self.peek() {
            Some(Tok::RBrace) => stop.at_rbrace,
            Some(Tok::Amp) => stop.at_amp,
            Some(Tok::RowSep) => stop.at_rowsep,
            Some(Tok::Cmd(name)) if name == "end" => stop.at_end,
            Some(Tok::Char(']')) => stop.at_rbracket,
            _ => false,
        }
    }

    fn parse_seq(&mut self, stop: &Stop) -> Vec<Expr> {
        let mut out = Vec::new();
        let mut literal = String::new();
        while !self.stops(stop) {
            match self.next() {
                None => break,
                Some(Tok::Char(c)) => literal.push(c),
                Some(Tok::LBrace) => {
                    flush_literal(&mut literal, &mut out);
                    let inner = self.parse_seq(&Stop::group());
                    if let Some(Tok::RBrace) = self.next() {
                        out.push(Expr::Seq(inner));
                    } else {
                        self.unsupported.push("{".to_string());
                    }
                }
                Some(Tok::Sup) => {
                    flush_literal(&mut literal, &mut out);
                    let atom = self.parse_atom();
                    out.push(Expr::Sup(Box::new(atom)));
                }
                Some(Tok::Sub) => {
                    flush_literal(&mut literal, &mut out);
                    let atom = self.parse_atom();
                    out.push(Expr::Sub(Box::new(atom)));
                }
                Some(Tok::Cmd(name)) => {
                    flush_literal(&mut literal, &mut out);
                    let nodes = self.parse_cmd(&name);
                    out.extend(nodes);
                }
                Some(Tok::Amp) | Some(Tok::RowSep) | Some(Tok::RBrace) => {
                    // Fuera de contexto de matriz/estructura: separador.
                    literal.push(' ');
                }
            }
        }
        flush_literal(&mut literal, &mut out);
        out
    }

    /// Un átomo: grupo `{...}`, un carácter o un comando simple.
    fn parse_atom(&mut self) -> Expr {
        match self.next() {
            Some(Tok::LBrace) => {
                let inner = self.parse_seq(&Stop::group());
                self.next(); // RBrace (o EOF)
                Expr::Seq(inner)
            }
            Some(Tok::Char(c)) => Expr::Lit(c.to_string()),
            Some(Tok::Cmd(name)) => {
                let nodes = self.parse_cmd(&name);
                Expr::Seq(nodes)
            }
            Some(other) => Expr::Lit(format!("{other:?}")),
            None => Expr::Lit(String::new()),
        }
    }

    /// Grupo `{...}` obligatorio como argumento.
    fn parse_group(&mut self) -> Expr {
        match self.next() {
            Some(Tok::LBrace) => {
                let inner = self.parse_seq(&Stop::group());
                self.next(); // RBrace
                Expr::Seq(inner)
            }
            _ => Expr::Seq(Vec::new()),
        }
    }

    fn parse_cmd(&mut self, name: &str) -> Vec<Expr> {
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.parse_group();
                let den = self.parse_group();
                vec![Expr::Frac(Box::new(num), Box::new(den))]
            }
            "sqrt" => {
                // `\sqrt[n]{x}` → `root(n, x)`; `\sqrt{x}` → `sqrt(x)`.
                let mut index = None;
                if let Some(Tok::Char('[')) = self.peek().cloned() {
                    self.next();
                    let inner = self.parse_seq(&Stop {
                        at_rbracket: true,
                        ..Stop::none()
                    });
                    self.next(); // ]
                    index = Some(Box::new(Expr::Seq(inner)));
                }
                let radicand = self.parse_group();
                vec![Expr::Sqrt {
                    index,
                    radicand: Box::new(radicand),
                }]
            }
            "left" | "right" => vec![self.parse_delim()],
            "begin" => vec![self.parse_env()],
            "end" => {
                // `end` sin `begin` correspondiente: no soportado.
                self.unsupported.push("\\end".to_string());
                Vec::new()
            }
            "hat" | "tilde" | "vec" | "dot" | "ddot" | "overline" | "underline" | "bar"
            | "widehat" | "widetilde" | "overrightarrow" => {
                let arg = self.parse_atom();
                let func = match name {
                    "bar" => "overline",
                    "widehat" => "hat",
                    "widetilde" => "tilde",
                    "overrightarrow" => "vec",
                    other => leak_func(other),
                };
                vec![Expr::Func(func, Box::new(arg))]
            }
            // Espaciado LaTeX: se colapsa a espacio (sin semántica visual).
            "," | ";" | ":" | "!" | " " | "quad" | "qquad" => vec![Expr::Lit(" ".to_string())],
            _ => {
                if let Some(sym) = symbol(name) {
                    vec![Expr::Sym(sym)]
                } else {
                    self.unsupported.push(format!("\\{name}"));
                    Vec::new()
                }
            }
        }
    }

    fn parse_delim(&mut self) -> Expr {
        match self.next() {
            Some(Tok::Char(c)) => Expr::Delim(c),
            Some(Tok::Cmd(name)) if name == "." => Expr::Delim(' '),
            Some(Tok::LBrace) => Expr::Delim('{'),
            Some(Tok::RBrace) => Expr::Delim('}'),
            _ => Expr::Delim(' '),
        }
    }

    /// Entornos de matriz (ADR-002): `pmatrix`, `bmatrix`, `vmatrix`, etc.
    fn parse_env(&mut self) -> Expr {
        let mut name = String::new();
        if let Some(Tok::LBrace) = self.next() {
            loop {
                match self.next() {
                    Some(Tok::Char(c)) => name.push(c),
                    _ => break,
                }
            }
        }
        let delim = match name.as_str() {
            "pmatrix" | "matrix" | "smallmatrix" => '(',
            "bmatrix" => '[',
            "vmatrix" => '|',
            "Vmatrix" => '‖',
            "cases" => '{',
            _ => {
                self.unsupported
                    .push(format!("\\begin{{{name}}}"));
                '('
            }
        };
        let mut rows = Vec::new();
        let mut row = Vec::new();
        loop {
            let cell = self.parse_seq(&Stop::cell());
            if !cell.is_empty() {
                row.push(Expr::Seq(cell));
            }
            match self.next() {
                Some(Tok::Amp) => {}
                Some(Tok::RowSep) => {
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                    }
                }
                Some(Tok::Cmd(name)) if name == "end" => {
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                    }
                    // Consume `{nombre}` del `\end{...}`.
                    if let Some(Tok::LBrace) = self.next() {
                        while !matches!(self.next(), Some(Tok::RBrace) | None) {}
                    }
                    break;
                }
                _ => break,
            }
        }
        Expr::Matrix { delim, rows }
    }
}

fn flush_literal(literal: &mut String, out: &mut Vec<Expr>) {
    if !literal.is_empty() {
        out.push(Expr::Lit(std::mem::take(literal)));
    }
}

fn leak_func(name: &str) -> &'static str {
    match name {
        "hat" => "hat",
        "tilde" => "tilde",
        "vec" => "vec",
        "dot" => "dot",
        "ddot" => "dot.double",
        "overline" => "overline",
        "underline" => "underline",
        _ => "upright",
    }
}

/// Símbolos y operadores del subconjunto estándar (ADR-002).
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Griegas.
        "alpha" => "alpha",
        "beta" => "beta",
        "gamma" => "gamma",
        "delta" => "delta",
        "epsilon" | "varepsilon" => "epsilon",
        "zeta" => "zeta",
        "eta" => "eta",
        "theta" | "vartheta" => "theta",
        "iota" => "iota",
        "kappa" => "kappa",
        "lambda" => "lambda",
        "mu" => "mu",
        "nu" => "nu",
        "xi" => "xi",
        "pi" => "pi",
        "rho" => "rho",
        "sigma" => "sigma",
        "tau" => "tau",
        "upsilon" => "upsilon",
        "phi" | "varphi" => "phi",
        "chi" => "chi",
        "psi" => "psi",
        "omega" => "omega",
        "Gamma" => "Gamma",
        "Delta" => "Delta",
        "Theta" => "Theta",
        "Lambda" => "Lambda",
        "Xi" => "Xi",
        "Pi" => "Pi",
        "Sigma" => "Sigma",
        "Phi" => "Phi",
        "Psi" => "Psi",
        "Omega" => "Omega",
        // Operadores grandes y símbolos.
        "sum" => "sum",
        "prod" => "product",
        "int" => "int",
        "iint" => "integral.double",
        "infty" => "infinity",
        "partial" => "diff",
        "nabla" => "nabla",
        "to" | "rightarrow" => "->",
        "leftarrow" => "<-",
        "Rightarrow" => "=>",
        "mapsto" => "|->",
        "cdot" => "dot",
        "times" => "times",
        "div" => "div",
        "pm" => "plus.minus",
        "mp" => "minus.plus",
        "leq" | "le" => "<=",
        "geq" | "ge" => ">=",
        "neq" | "ne" => "!=",
        "approx" => "approx",
        "equiv" => "equiv",
        "sim" => "tilde",
        "in" => "in",
        "notin" => "in.not",
        "subset" => "subset",
        "subseteq" => "subset.eq",
        "cup" => "union",
        "cap" => "sect",
        "forall" => "forall",
        "exists" => "exists",
        "neg" | "lnot" => "not",
        "ldots" | "dots" => "dots",
        "cdots" => "dots.c",
        "vdots" => "dots.v",
        "prime" => "prime",
        "angle" => "angle",
        "degree" => "degree",
        "star" => "star",
        "ast" => "ast",
        "circ" => "circle",
        "bullet" => "bullet",
        "propto" => "propto",
        "perp" => "perp",
        "parallel" => "parallel",
        "emptyset" | "varnothing" => "nothing",
        "Re" => "Re",
        "Im" => "Im",
        "aleph" => "aleph",
        // Funciones.
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "cot" => "cot",
        "sec" => "sec",
        "csc" => "csc",
        "arcsin" => "arcsin",
        "arccos" => "arccos",
        "arctan" => "arctan",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "tanh" => "tanh",
        "log" => "log",
        "ln" => "ln",
        "lg" => "lg",
        "exp" => "exp",
        "lim" => "lim",
        "max" => "max",
        "min" => "min",
        "sup" => "sup",
        "inf" => "inf",
        "det" => "det",
        "gcd" => "gcd",
        "deg" => "deg",
        "mod" => "mod",
        _ => return None,
    })
}

// --- Emisión ----------------------------------------------------------------

fn emit(exprs: &[Expr], out: &mut String) {
    for expr in exprs {
        match expr {
            Expr::Lit(text) => out.push_str(&escape_literal(text)),
            Expr::Sym(name) => out.push_str(name),
            Expr::Sup(arg) => {
                out.push('^');
                out.push_str(&compact(arg));
            }
            Expr::Sub(arg) => {
                out.push('_');
                out.push_str(&compact(arg));
            }
            Expr::Frac(num, den) => {
                out.push_str(&compact(num));
                out.push('/');
                out.push_str(&compact(den));
            }
            Expr::Sqrt { index, radicand } => match index {
                Some(index) => {
                    out.push_str(&format!("root({}, {})", compact(index), compact(radicand)));
                }
                None => out.push_str(&format!("sqrt({})", compact(radicand))),
            },
            Expr::Func(name, arg) => out.push_str(&format!("{name}({})", compact(arg))),
            Expr::Delim(c) => {
                if *c != ' ' {
                    out.push(*c);
                }
            }
            Expr::Matrix { delim, rows } => {
                out.push_str("mat(delim: \"");
                out.push(*delim);
                out.push_str("\", ");
                let rendered: Vec<String> = rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| {
                                let mut s = String::new();
                                emit(std::slice::from_ref(cell), &mut s);
                                s.trim().to_string()
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .collect();
                out.push_str(&rendered.join("; "));
                out.push(')');
            }
            Expr::Seq(inner) => emit(inner, out),
        }
    }
}

/// Átomo compacto para super/índices y operandos: sin paréntesis si es un
/// único símbolo (ADR-002: `x^{2}` → `x^2`, `\sum_{i=1}^{n}` → `sum_(i=1)^n`).
fn compact(expr: &Expr) -> String {
    match expr {
        Expr::Lit(text) if text.chars().count() == 1 && !text.contains(' ') => text.clone(),
        Expr::Sym(name) => name.to_string(),
        Expr::Delim(c) => c.to_string(),
        Expr::Seq(inner) if inner.len() == 1 => compact(&inner[0]),
        Expr::Seq(inner) => {
            let mut s = String::new();
            emit(inner, &mut s);
            let s = s.trim();
            if s.contains(' ') || s.len() > 1 {
                format!("({s})")
            } else {
                s.to_string()
            }
        }
        other => {
            let mut s = String::new();
            emit(std::slice::from_ref(other), &mut s);
            format!("({})", s.trim())
        }
    }
}

/// Literales: se escapan los caracteres especiales del modo math de Typst.
fn escape_literal(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '#' | '$' | '_' | '^' | '&' | '%' | '\\' | '{' | '}' | '`' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_typst(latex: &str) -> String {
        let result = translate(latex);
        assert!(result.is_supported(), "no soportado: {:?}", result.unsupported);
        result.typst
    }

    #[test]
    fn fracciones_raices_y_scripts() {
        // ADR-002: operadores básicos.
        assert_eq!(to_typst(r"\frac{a}{b}"), "a/b");
        assert_eq!(to_typst(r"\frac{a+b}{c}"), "(a+b)/c");
        assert_eq!(to_typst(r"\sqrt{x}"), "sqrt(x)");
        assert_eq!(to_typst(r"\sqrt[3]{x}"), "root(3, x)");
        assert_eq!(to_typst(r"x^{2}"), "x^2");
        assert_eq!(to_typst(r"x_{i}"), "x_i");
        assert_eq!(to_typst(r"\left(x\right)"), "(x)");
    }

    #[test]
    fn operadores_grandes_y_griegas() {
        // ADR-002: `\sum_{i=1}^{n}` → `sum_(i=1)^n`.
        assert_eq!(to_typst(r"\sum_{i=1}^{n}"), "sum_(i=1)^n");
        assert_eq!(to_typst(r"\int_{a}^{b}"), "int_a^b");
        assert_eq!(to_typst(r"\prod_{i=1}^{n}"), "product_(i=1)^n");
        assert_eq!(to_typst(r"\alpha + \beta"), "alpha + beta");
        assert_eq!(to_typst(r"\infty"), "infinity");
    }

    #[test]
    fn matrices() {
        assert_eq!(
            to_typst(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}"),
            "mat(delim: \"(\", a, b; c, d)"
        );
        assert_eq!(
            to_typst(r"\begin{bmatrix} 1 \\ 2 \end{bmatrix}"),
            "mat(delim: \"[\", 1; 2)"
        );
    }

    #[test]
    fn funciones_limite_y_acentos() {
        assert_eq!(to_typst(r"\lim_{x \to 0}"), "lim_(x -> 0)");
        assert_eq!(to_typst(r"\sin x"), "sin x");
        assert_eq!(to_typst(r"\hat{x}"), "hat(x)");
        assert_eq!(to_typst(r"\bar{x}"), "overline(x)");
        assert_eq!(to_typst(r"\vec{x}"), "vec(x)");
    }

    #[test]
    fn comentarios_y_escapes() {
        assert_eq!(to_typst("x % comentario"), "x");
        // `\_` es un `_` literal: en Typst math se preserva escapado.
        assert_eq!(to_typst(r"a \_ b"), r"a \_ b");
    }

    #[test]
    fn no_soportado_produce_marcador() {
        // NFR-026 / ADR-002: nunca silenciar ni deformar.
        let result = translate(r"\foo{x} + 1");
        assert!(!result.is_supported());
        assert_eq!(result.unsupported, vec![r"\foo".to_string()]);
        assert_eq!(result.typst, "");
        assert_eq!(result.marker(), "⚠ [math no soportado: \\foo]");
    }
}
