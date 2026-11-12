//! Function graphs y = f(x) on a coordinate system, and their variation table (bảng biến thiên).
//!
//! The formula can be typed (`x^3 - 3x + 1`, `2sin(x)`, `|x|`) or pasted from MathType,
//! either as LaTeX (`\frac{2x+1}{x-1}`) or as MathML (MathType's default copy format).

use crate::strokes::textstroke::TextStyle;
use crate::strokes::{ShapeStroke, Stroke, TextStroke};
use p2d::math::Vector2;
use rnote_compose::shapes::{Arrow, Line, Polyline};
use rnote_compose::{Color, Shape, Style};

// ==========================================
// Expression
// ==========================================

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    X,
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(&'static str, Box<Expr>),
}

const FUNCS: [&str; 13] = [
    "arcsin", "arccos", "arctan", "sqrt", "sin", "cos", "tan", "cot", "abs", "exp", "log", "ln",
    "lg",
];

/// A parsed real function of `x`.
#[derive(Debug, Clone)]
pub struct Function {
    expr: Expr,
    /// The formula as it was normalized, e.g. `x^3-3*x+1`
    pub text: String,
}

impl Function {
    /// Parse a typed formula, a MathType LaTeX copy or a MathType MathML copy.
    /// A leading `y =` / `f(x) =` is ignored.
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        let text =
            if input.contains("<math") || input.contains("<m:math") || input.contains("<mrow") {
                mathml_to_text(input)?
            } else if input.contains('\\') || input.contains('{') {
                latex_to_text(input)
            } else {
                input.to_string()
            };
        // Keep only the right side of "y = ...", "f(x) = ..."
        let text = text.rsplit('=').next().unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Err("Chưa nhập hàm số".to_string());
        }
        let tokens = tokenize(&text)?;
        let mut p = Parser {
            tokens,
            pos: 0,
            abs_depth: 0,
        };
        let expr = p.expr()?;
        if p.pos != p.tokens.len() {
            return Err(format!("Không hiểu phần \"{}\"", p.rest()));
        }
        Ok(Self { expr, text })
    }

    /// f(x); NaN outside the domain.
    pub fn eval(&self, x: f64) -> f64 {
        eval(&self.expr, x)
    }
}

fn eval(e: &Expr, x: f64) -> f64 {
    match e {
        Expr::Num(v) => *v,
        Expr::X => x,
        Expr::Neg(a) => -eval(a, x),
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval(a, x), eval(b, x));
            match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => {
                    if b == 0.0 {
                        f64::NAN
                    } else {
                        a / b
                    }
                }
                _ => {
                    // Odd roots of negatives: x^(1/3) is real
                    if a < 0.0 && b.fract() != 0.0 {
                        let inv = 1.0 / b;
                        if (inv.round() - inv).abs() < 1e-9 && inv.round() as i64 % 2 != 0 {
                            -(-a).powf(b)
                        } else {
                            f64::NAN
                        }
                    } else {
                        a.powf(b)
                    }
                }
            }
        }
        Expr::Call(f, a) => {
            let a = eval(a, x);
            match *f {
                "sin" => a.sin(),
                "cos" => a.cos(),
                "tan" => a.tan(),
                "cot" => 1.0 / a.tan(),
                "sqrt" => a.sqrt(),
                "abs" => a.abs(),
                "exp" => a.exp(),
                "ln" => a.ln(),
                "log" | "lg" => a.log10(),
                "arcsin" => a.asin(),
                "arccos" => a.acos(),
                _ => a.atan(),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(&'static str),
    Op(char),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            // A comma between digits is a decimal comma (Vietnamese notation: 1,5)
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || chars[i] == '.'
                    || (chars[i] == ',' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())))
            {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            out.push(Tok::Num(
                num.replace(',', ".")
                    .parse()
                    .map_err(|_| format!("Số không hợp lệ: {num}"))?,
            ));
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphabetic() {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
            // Split run-together names: "xsinx" -> x sin x
            let mut rest = word.as_str();
            while !rest.is_empty() {
                if let Some(f) = FUNCS.iter().find(|f| rest.starts_with(**f)) {
                    out.push(Tok::Ident(f));
                    rest = &rest[f.len()..];
                } else if let Some(r) = rest.strip_prefix("pi") {
                    out.push(Tok::Num(std::f64::consts::PI));
                    rest = r;
                } else if let Some(r) = rest.strip_prefix('x') {
                    out.push(Tok::Ident("x"));
                    rest = r;
                } else if let Some(r) = rest.strip_prefix('e') {
                    out.push(Tok::Num(std::f64::consts::E));
                    rest = r;
                } else {
                    return Err(format!("Không hiểu ký hiệu \"{rest}\""));
                }
            }
        } else {
            let op = match c {
                '+' | '-' | '*' | '/' | '^' | '(' | ')' | '|' => c,
                '−' | '–' => '-',
                '·' | '×' | '⋅' => '*',
                '÷' | ':' => '/',
                'π' => {
                    out.push(Tok::Num(std::f64::consts::PI));
                    i += 1;
                    continue;
                }
                '²' | '³' => {
                    out.push(Tok::Op('^'));
                    out.push(Tok::Num(if c == '²' { 2.0 } else { 3.0 }));
                    i += 1;
                    continue;
                }
                '[' => '(',
                ']' => ')',
                _ => return Err(format!("Không hiểu ký tự \"{c}\"")),
            };
            out.push(Tok::Op(op));
            i += 1;
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
    abs_depth: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn rest(&self) -> String {
        self.tokens[self.pos..]
            .iter()
            .map(|t| match t {
                Tok::Num(v) => v.to_string(),
                Tok::Ident(s) => s.to_string(),
                Tok::Op(c) => c.to_string(),
            })
            .collect()
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.term()?;
        loop {
            if self.eat('+') {
                lhs = Expr::Bin('+', lhs.into(), self.term()?.into());
            } else if self.eat('-') {
                lhs = Expr::Bin('-', lhs.into(), self.term()?.into());
            } else {
                return Ok(lhs);
            }
        }
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut lhs = self.unary()?;
        loop {
            if self.eat('*') {
                lhs = Expr::Bin('*', lhs.into(), self.unary()?.into());
            } else if self.eat('/') {
                lhs = Expr::Bin('/', lhs.into(), self.unary()?.into());
            } else if match self.peek() {
                // Implicit product: 2x, 3(x+1), x sin x; '|' only opens a new |..| at top level
                Some(Tok::Num(_) | Tok::Ident(_) | Tok::Op('(')) => true,
                Some(Tok::Op('|')) => self.abs_depth == 0,
                _ => false,
            } {
                lhs = Expr::Bin('*', lhs.into(), self.power()?.into());
            } else {
                return Ok(lhs);
            }
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.eat('-') {
            Ok(Expr::Neg(self.unary()?.into()))
        } else if self.eat('+') {
            self.unary()
        } else {
            self.power()
        }
    }

    fn power(&mut self) -> Result<Expr, String> {
        let base = self.primary()?;
        if self.eat('^') {
            Ok(Expr::Bin('^', base.into(), self.unary()?.into()))
        } else {
            Ok(base)
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let tok = self.peek().cloned();
        self.pos += 1;
        match tok {
            Some(Tok::Num(v)) => Ok(Expr::Num(v)),
            Some(Tok::Ident("x")) => Ok(Expr::X),
            Some(Tok::Ident(f)) => Ok(Expr::Call(f, self.power()?.into())),
            Some(Tok::Op('(')) => {
                let depth = std::mem::take(&mut self.abs_depth);
                let e = self.expr()?;
                self.abs_depth = depth;
                if !self.eat(')') {
                    return Err("Thiếu dấu \")\"".to_string());
                }
                Ok(e)
            }
            Some(Tok::Op('|')) => {
                self.abs_depth += 1;
                let e = self.expr()?;
                self.abs_depth -= 1;
                if !self.eat('|') {
                    return Err("Thiếu dấu \"|\"".to_string());
                }
                Ok(Expr::Call("abs", e.into()))
            }
            Some(t) => {
                self.pos -= 1;
                Err(format!(
                    "Không hiểu phần \"{}\"",
                    match t {
                        Tok::Op(c) => c.to_string(),
                        _ => self.rest(),
                    }
                ))
            }
            None => Err("Biểu thức chưa hết".to_string()),
        }
    }
}

/// LaTeX (MathType "Copy as LaTeX", or typed) to the plain formula syntax.
fn latex_to_text(s: &str) -> String {
    fn group(chars: &[char], i: &mut usize) -> String {
        while *i < chars.len() && chars[*i].is_whitespace() {
            *i += 1;
        }
        if chars.get(*i) != Some(&'{') {
            // A single token argument: \frac12
            let c = chars.get(*i).copied().unwrap_or(' ');
            *i += 1;
            return c.to_string();
        }
        *i += 1;
        let start = *i;
        let mut depth = 1;
        while *i < chars.len() {
            match chars[*i] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            *i += 1;
        }
        let inner: String = chars[start..(*i).min(chars.len())].iter().collect();
        *i += 1;
        convert(&inner)
    }
    fn convert(s: &str) -> String {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                i += 1;
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i == start {
                    i += 1; // "\," "\;" "\ " "\{"
                    if let Some(&b) = chars.get(start)
                        && (b == '{' || b == '}')
                    {
                        out.push(if b == '{' { '(' } else { ')' });
                    }
                    continue;
                }
                let cmd: String = chars[start..i].iter().collect();
                match cmd.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let a = group(&chars, &mut i);
                        let b = group(&chars, &mut i);
                        out.push_str(&format!("(({a})/({b}))"));
                    }
                    "sqrt" => {
                        if chars.get(i) == Some(&'[') {
                            let end = chars[i..].iter().position(|&c| c == ']').map(|p| p + i);
                            let n: String = chars[i + 1..end.unwrap_or(i + 1)].iter().collect();
                            i = end.map_or(i + 1, |e| e + 1);
                            let a = group(&chars, &mut i);
                            out.push_str(&format!("(({a})^(1/({n})))"));
                        } else {
                            let a = group(&chars, &mut i);
                            out.push_str(&format!("sqrt({a})"));
                        }
                    }
                    "cdot" | "times" => out.push('*'),
                    "div" => out.push('/'),
                    "pi" => out.push_str("pi"),
                    "left" | "right" | "displaystyle" | "mathrm" | "mathit" | "text" | "big"
                    | "Big" => {}
                    // \sin, \ln, ... are already plain names
                    other => {
                        out.push(' ');
                        out.push_str(other);
                        out.push(' ');
                    }
                }
            } else {
                match c {
                    '{' => out.push('('),
                    '}' => out.push(')'),
                    '$' => {}
                    _ => out.push(c),
                }
                i += 1;
            }
        }
        out
    }
    let s = s
        .trim()
        .trim_start_matches("\\[")
        .trim_end_matches("\\]")
        .trim_start_matches("\\(")
        .trim_end_matches("\\)");
    convert(s)
}

/// MathML (MathType's default clipboard format) to the plain formula syntax.
fn mathml_to_text(s: &str) -> Result<String, String> {
    fn walk(n: roxmltree::Node) -> String {
        let kids: Vec<String> = n.children().filter(|c| c.is_element()).map(walk).collect();
        let arg = |i: usize| kids.get(i).cloned().unwrap_or_default();
        match n.tag_name().name() {
            "mi" | "mn" | "mo" | "mtext" => {
                let t = n.text().unwrap_or("").trim().to_string();
                if t == "\u{2061}" || t == "\u{2062}" {
                    String::new()
                } else {
                    format!(" {t} ")
                }
            }
            "mfrac" => format!("(({})/({}))", arg(0), arg(1)),
            "msup" => format!("({})^({})", arg(0), arg(1)),
            "msqrt" => format!("sqrt({})", kids.concat()),
            "mroot" => format!("(({})^(1/({})))", arg(0), arg(1)),
            "mfenced" => format!("({})", kids.concat()),
            "annotation" | "annotation-xml" => String::new(),
            _ => kids.concat(),
        }
    }
    // MathType may add a namespace prefix (m:math) or a <?xml ...?> header; roxmltree handles both
    let doc = roxmltree::Document::parse(s.trim()).map_err(|e| format!("MathML lỗi: {e}"))?;
    Ok(walk(doc.root_element()))
}

// ==========================================
// Study: what a variation table shows
// ==========================================

/// A value in the table: a number or an infinite limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Val {
    NegInf,
    Finite(f64),
    PosInf,
}

/// One column of the variation table.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub x: Val,
    /// f just left and just right of x (equal for an ordinary point)
    pub left: Val,
    pub right: Val,
    /// Extremum: f'(x) = 0. A break (vertical asymptote) has `left != right` and shows "||".
    pub stationary: bool,
    pub is_break: bool,
}

/// Nodes in increasing x and the sign of f' on each interval between consecutive nodes.
#[derive(Debug, Clone)]
pub struct VariationTable {
    pub nodes: Vec<Node>,
    pub signs: Vec<i8>,
}

const STUDY_RANGE: f64 = 30.0;
const SAMPLES: usize = 6000;
const BIG: f64 = 1e7;

fn to_val(v: f64) -> Val {
    if v.is_nan() {
        Val::Finite(f64::NAN)
    } else if v >= BIG {
        Val::PosInf
    } else if v <= -BIG {
        Val::NegInf
    } else {
        Val::Finite(v)
    }
}

/// Limit of f towards `x` from the side of `towards` (numeric, by approaching geometrically).
fn side_limit(f: &Function, x: f64, dir: f64) -> Val {
    let mut last = f64::NAN;
    for k in 3..12 {
        let v = f.eval(x + dir * 10f64.powi(-k));
        if v.is_finite() {
            last = v;
        }
    }
    if last.abs() > 1e5 {
        if last > 0.0 { Val::PosInf } else { Val::NegInf }
    } else {
        Val::Finite(round(last))
    }
}

fn limit_at_infinity(f: &Function, dir: f64) -> Val {
    let (a, b) = (f.eval(dir * 1e4), f.eval(dir * 1e6));
    if !b.is_finite() && !a.is_finite() {
        return Val::Finite(f64::NAN);
    }
    if b.abs() > 1e5 && b.abs() > a.abs() {
        if b > 0.0 { Val::PosInf } else { Val::NegInf }
    } else {
        Val::Finite(round(b))
    }
}

fn round(v: f64) -> f64 {
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 { 0.0 } else { r }
}

fn deriv(f: &Function, x: f64) -> f64 {
    let h = 1e-6 * x.abs().max(1.0);
    (f.eval(x + h) - f.eval(x - h)) / (2.0 * h)
}

fn sign(v: f64) -> i8 {
    if v > 1e-9 {
        1
    } else if v < -1e-9 {
        -1
    } else {
        0
    }
}

impl VariationTable {
    /// Study f numerically on [-30, 30] and towards ±∞.
    pub fn study(f: &Function) -> Result<Self, String> {
        let step = 2.0 * STUDY_RANGE / SAMPLES as f64;
        let xs: Vec<f64> = (0..=SAMPLES)
            .map(|i| -STUDY_RANGE + i as f64 * step + step * 0.123) // off-grid: avoids hitting x = 0 exactly
            .collect();
        let defined = |x: f64| f.eval(x).is_finite();

        // Domain runs [start, end] of consecutive defined samples
        let mut runs: Vec<(f64, f64)> = Vec::new();
        let mut run_start: Option<f64> = None;
        for w in xs.windows(2) {
            let (a, b) = (w[0], w[1]);
            match (run_start, defined(a)) {
                (None, true) => run_start = Some(a),
                (Some(s), false) => {
                    runs.push((s, a));
                    run_start = None;
                }
                _ => {}
            }
            let _ = b;
        }
        if let Some(s) = run_start {
            runs.push((s, *xs.last().unwrap()));
        }
        if runs.is_empty() {
            return Err("Hàm số không xác định trên đoạn khảo sát".to_string());
        }

        let refine_edge = |inside: f64, outside: f64| {
            let (mut a, mut b) = (inside, outside);
            for _ in 0..60 {
                let m = 0.5 * (a + b);
                if defined(m) { a = m } else { b = m }
            }
            round(a)
        };

        let mut nodes: Vec<Node> = Vec::new();
        for (ri, &(s, e)) in runs.iter().enumerate() {
            // Left end of this run: -∞, a domain edge, or (between runs) a gap
            let left_open = ri == 0 && (s - xs[0]).abs() < 1e-9;
            let start = if left_open {
                let lim = limit_at_infinity(f, -1.0);
                Node {
                    x: Val::NegInf,
                    left: lim,
                    right: lim,
                    stationary: false,
                    is_break: false,
                }
            } else {
                let x0 = refine_edge(s, s - step);
                let v = to_val(f.eval(x0));
                let v = if matches!(v, Val::Finite(n) if n.is_nan()) {
                    side_limit(f, x0, 1.0)
                } else {
                    v
                };
                Node {
                    x: Val::Finite(x0),
                    left: v,
                    right: v,
                    stationary: false,
                    is_break: false,
                }
            };
            // Merge a gap of a single excluded point (1/x at 0) into one break node
            if let Some(prev) = nodes.last_mut()
                && let (Val::Finite(px), Val::Finite(sx)) = (prev.x, start.x)
                && (px - sx).abs() < 1e-3
            {
                prev.right = side_limit(f, sx, 1.0);
                prev.is_break = true;
            } else {
                nodes.push(start);
            }

            // Inside the run: extrema (f' changes sign) and jumps (f moves against f')
            let inner: Vec<f64> = xs.iter().copied().filter(|&x| x >= s && x <= e).collect();
            for w in inner.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (da, db) = (deriv(f, a), deriv(f, b));
                let jump = f.eval(b) - f.eval(a);
                if sign(da) != 0
                    && sign(da) == sign(db)
                    && sign(jump) == -sign(da)
                    && jump.abs() > 1.0
                {
                    // Jump: f(b) - f(a) against the monotony on both sides
                    let (mut l, mut r) = (a, b);
                    for _ in 0..60 {
                        let m = 0.5 * (l + r);
                        if sign(f.eval(m) - f.eval(l)) == -sign(da) {
                            r = m
                        } else {
                            l = m
                        }
                    }
                    let x0 = round(0.5 * (l + r));
                    nodes.push(Node {
                        x: Val::Finite(x0),
                        left: side_limit(f, x0, -1.0),
                        right: side_limit(f, x0, 1.0),
                        stationary: false,
                        is_break: true,
                    });
                } else if sign(da) * sign(db) < 0 || (sign(da) == 0 && sign(db) != 0 && a > s) {
                    let (mut l, mut r) = (a, b);
                    for _ in 0..60 {
                        let m = 0.5 * (l + r);
                        if sign(deriv(f, m)) == sign(da) {
                            l = m
                        } else {
                            r = m
                        }
                    }
                    let x0 = round(0.5 * (l + r));
                    let y = f.eval(x0);
                    if !y.is_finite() || y.abs() > 1e5 {
                        // 1/x^2 at 0: f' flips through a pole
                        nodes.push(Node {
                            x: Val::Finite(x0),
                            left: side_limit(f, x0, -1.0),
                            right: side_limit(f, x0, 1.0),
                            stationary: false,
                            is_break: true,
                        });
                    } else if nodes.last().is_none_or(|n| n.x != Val::Finite(x0)) {
                        let v = Val::Finite(round(y));
                        nodes.push(Node {
                            x: Val::Finite(x0),
                            left: v,
                            right: v,
                            stationary: true,
                            is_break: false,
                        });
                    }
                }
            }

            let right_open = ri == runs.len() - 1 && (e - xs[SAMPLES]).abs() < 1e-9;
            if right_open {
                let lim = limit_at_infinity(f, 1.0);
                nodes.push(Node {
                    x: Val::PosInf,
                    left: lim,
                    right: lim,
                    stationary: false,
                    is_break: false,
                });
            } else {
                let x1 = refine_edge(e, e + step);
                let v = to_val(f.eval(x1));
                let v = if matches!(v, Val::Finite(n) if n.is_nan()) {
                    side_limit(f, x1, -1.0)
                } else {
                    v
                };
                nodes.push(Node {
                    x: Val::Finite(x1),
                    left: v,
                    right: v,
                    stationary: false,
                    is_break: false,
                });
            }
        }

        // Sign of f' on each interval, from a sample inside it
        let pick = |a: Val, b: Val| -> f64 {
            match (a, b) {
                (Val::NegInf, Val::Finite(b)) => b - 1.0,
                (Val::Finite(a), Val::PosInf) => a + 1.0,
                (Val::Finite(a), Val::Finite(b)) => 0.5 * (a + b),
                _ => 0.0,
            }
        };
        let signs = nodes
            .windows(2)
            .map(|w| sign(deriv(f, pick(w[0].x, w[1].x))))
            .collect();
        Ok(Self { nodes, signs })
    }
}

/// "1.5" as "1,5"; integers without decimals; ±∞.
pub fn fmt_val(v: Val) -> String {
    match v {
        Val::NegInf => "−∞".to_string(),
        Val::PosInf => "+∞".to_string(),
        Val::Finite(n) if n.is_nan() => "".to_string(),
        Val::Finite(n) => {
            let s = format!("{:.2}", n);
            let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
            let s = if s == "-0" { "0".to_string() } else { s };
            s.replace('.', ",").replace('-', "−")
        }
    }
}

// ==========================================
// Strokes
// ==========================================

/// Where and how big a graph is drawn.
#[derive(Debug, Clone, Copy)]
pub struct GraphFrame {
    /// Canvas position of the origin O
    pub origin: Vector2,
    /// Pixels per unit
    pub unit: f64,
    pub x_range: (f64, f64),
    pub y_range: (f64, f64),
}

fn text(s: &str, pos: Vector2, color: Color, size: f64) -> Stroke {
    let mut style = TextStyle::default();
    style.font_size = size;
    style.color = color;
    Stroke::TextStroke(TextStroke::new(s.to_string(), pos, style))
}

fn shape(shape: Shape, style: &Style) -> Stroke {
    Stroke::ShapeStroke(ShapeStroke::new(shape, style.clone()))
}

/// Axes and the curve of f.
pub fn graph_strokes(f: &Function, frame: GraphFrame, style: &Style, color: Color) -> Vec<Stroke> {
    let mut out = axes_strokes(frame, style, color);
    out.extend(curve_strokes(f, frame, style));
    out
}

impl GraphFrame {
    /// Canvas position of the point (x; y) in axis units (y up).
    pub fn to_canvas(&self, x: f64, y: f64) -> Vector2 {
        self.origin + Vector2::new(x * self.unit, -y * self.unit)
    }
}

/// Oxy axes with unit ticks and numbers.
pub fn axes_strokes(frame: GraphFrame, style: &Style, color: Color) -> Vec<Stroke> {
    let GraphFrame {
        origin,
        unit,
        x_range: (x0, x1),
        y_range: (y0, y1),
    } = frame;
    let to_canvas = |x: f64, y: f64| origin + Vector2::new(x * unit, -y * unit);
    let mut out = Vec::new();
    let mut thin = style.clone();
    thin.set_stroke_width((style.stroke_width() * 0.6).max(1.0));

    // Axes, half a unit past the range, with x / y / O
    out.push(shape(
        Shape::Arrow(Arrow::new(
            to_canvas(x0 - 0.5, 0.0),
            to_canvas(x1 + 0.7, 0.0),
        )),
        &thin,
    ));
    out.push(shape(
        Shape::Arrow(Arrow::new(
            to_canvas(0.0, y0 - 0.5),
            to_canvas(0.0, y1 + 0.7),
        )),
        &thin,
    ));
    out.push(text(
        "x",
        to_canvas(x1 + 0.7, 0.0) + Vector2::new(-4.0, 6.0),
        color,
        16.0,
    ));
    out.push(text(
        "y",
        to_canvas(0.0, y1 + 0.7) + Vector2::new(8.0, -8.0),
        color,
        16.0,
    ));
    out.push(text("O", origin + Vector2::new(-16.0, 2.0), color, 14.0));
    // One label every `every` units keeps numbers readable at small units
    let every = (28.0 / unit).ceil().max(1.0) as i64;
    let tick = 4.0;
    for i in x0.ceil() as i64..=x1.floor() as i64 {
        if i == 0 {
            continue;
        }
        let p = to_canvas(i as f64, 0.0);
        out.push(shape(
            Shape::Line(Line::new(
                p - Vector2::new(0.0, tick),
                p + Vector2::new(0.0, tick),
            )),
            &thin,
        ));
        if i % every == 0 {
            out.push(text(
                &fmt_val(Val::Finite(i as f64)),
                p + Vector2::new(-5.0, 6.0),
                color,
                12.0,
            ));
        }
    }
    for i in y0.ceil() as i64..=y1.floor() as i64 {
        if i == 0 {
            continue;
        }
        let p = to_canvas(0.0, i as f64);
        out.push(shape(
            Shape::Line(Line::new(
                p - Vector2::new(tick, 0.0),
                p + Vector2::new(tick, 0.0),
            )),
            &thin,
        ));
        if i % every == 0 {
            out.push(text(
                &fmt_val(Val::Finite(i as f64)),
                p + Vector2::new(-22.0, -9.0),
                color,
                12.0,
            ));
        }
    }

    out
}

/// The curve of f, split where it leaves the window or jumps (asymptotes).
pub fn curve_strokes(f: &Function, frame: GraphFrame, style: &Style) -> Vec<Stroke> {
    let GraphFrame {
        unit,
        x_range: (x0, x1),
        y_range: (y0, y1),
        ..
    } = frame;
    let to_canvas = |x: f64, y: f64| frame.to_canvas(x, y);
    let mut out = Vec::new();
    // Curve: about one sample per pixel
    let n = (((x1 - x0) * unit).ceil() as usize).clamp(50, 20000);
    let mut piece: Vec<Vector2> = Vec::new();
    let flush = |piece: &mut Vec<Vector2>, out: &mut Vec<Stroke>| {
        if piece.len() >= 2 {
            let mut pl = Polyline::new(piece[0]);
            pl.path = piece[1..].to_vec();
            out.push(shape(Shape::Polyline(pl), style));
        }
        piece.clear();
    };
    let mut last_y: Option<f64> = None;
    for i in 0..=n {
        let x = x0 + (x1 - x0) * i as f64 / n as f64;
        let y = f.eval(x);
        let inside = y.is_finite() && y >= y0 - 0.05 && y <= y1 + 0.05;
        // A jump of more than the window height between neighbours is an asymptote, not a steep part
        let jump = last_y.is_some_and(|ly| (y - ly).abs() > (y1 - y0));
        if !inside || jump {
            flush(&mut piece, &mut out);
        }
        if inside {
            piece.push(to_canvas(x, y));
        }
        last_y = y.is_finite().then_some(y);
    }
    flush(&mut piece, &mut out);
    out
}

/// The variation table drawn with `top_left` as its upper left corner.
pub fn table_strokes(
    t: &VariationTable,
    top_left: Vector2,
    style: &Style,
    color: Color,
) -> Vec<Stroke> {
    let mut out = Vec::new();
    let mut thin = style.clone();
    thin.set_stroke_width((style.stroke_width() * 0.6).max(1.0));
    let head_w = 70.0;
    let col_w = 90.0;
    let row_h = [40.0, 40.0, 110.0]; // x, f'(x), f(x)
    let n = t.nodes.len();
    let width = head_w + col_w * (n.max(2) - 1) as f64 + 40.0;
    let height: f64 = row_h.iter().sum();
    let at = |x: f64, y: f64| top_left + Vector2::new(x, y);
    let line = |a: Vector2, b: Vector2| shape(Shape::Line(Line::new(a, b)), &thin);

    // Frame: outer box, row separators, header column
    out.push(line(at(0.0, 0.0), at(width, 0.0)));
    out.push(line(at(0.0, height), at(width, height)));
    out.push(line(at(0.0, 0.0), at(0.0, height)));
    out.push(line(at(width, 0.0), at(width, height)));
    out.push(line(at(0.0, row_h[0]), at(width, row_h[0])));
    out.push(line(
        at(0.0, row_h[0] + row_h[1]),
        at(width, row_h[0] + row_h[1]),
    ));
    out.push(line(at(head_w, 0.0), at(head_w, height)));
    out.push(text("x", at(28.0, 10.0), color, 16.0));
    out.push(text("f'(x)", at(16.0, row_h[0] + 10.0), color, 16.0));
    out.push(text(
        "f(x)",
        at(18.0, row_h[0] + row_h[1] + row_h[2] * 0.5 - 10.0),
        color,
        16.0,
    ));

    let col_x = |i: usize| head_w + 30.0 + col_w * i as f64;
    let (y_x, y_d, f_top, f_bot) = (
        10.0,
        row_h[0] + 10.0,
        row_h[0] + row_h[1] + 6.0,
        height - 28.0,
    );
    // A value sits high when the arrow next to it goes down from it or comes up to it
    let high = |i: usize, left_side: bool| -> bool {
        let s_left = if i > 0 {
            t.signs.get(i - 1).copied()
        } else {
            None
        };
        let s_right = t.signs.get(i).copied();
        let s = if left_side {
            s_left.or(s_right.map(|s| -s))
        } else {
            s_right.map(|s| -s).or(s_left)
        };
        match s {
            Some(s) => s > 0,
            None => false,
        }
    };
    let mut f_pos: Vec<(Vector2, Vector2)> = Vec::new(); // arrow anchor left side / right side of each node
    for (i, node) in t.nodes.iter().enumerate() {
        let cx = col_x(i);
        out.push(text(&fmt_val(node.x), at(cx - 8.0, y_x), color, 15.0));
        if node.stationary {
            out.push(text("0", at(cx - 4.0, y_d), color, 15.0));
        }
        if node.is_break {
            // "||": double bar through f' and f rows
            for dx in [-2.5, 2.5] {
                out.push(line(at(cx + dx, row_h[0]), at(cx + dx, height)));
            }
        }
        let (lh, rh) = (high(i, true), high(i, false));
        let ly = if lh { f_top } else { f_bot };
        let ry = if rh { f_top } else { f_bot };
        if node.is_break {
            out.push(text(&fmt_val(node.left), at(cx - 34.0, ly), color, 15.0));
            out.push(text(&fmt_val(node.right), at(cx + 8.0, ry), color, 15.0));
            f_pos.push((at(cx - 12.0, ly + 10.0), at(cx + 14.0, ry + 10.0)));
        } else {
            let y = if i == 0 { ry } else { ly };
            out.push(text(&fmt_val(node.left), at(cx - 10.0, y), color, 15.0));
            f_pos.push((at(cx - 6.0, y + 10.0), at(cx + 22.0, y + 10.0)));
        }
    }
    for (i, s) in t.signs.iter().enumerate() {
        let mid = 0.5 * (col_x(i) + col_x(i + 1));
        let sym = match s {
            1 => "+",
            -1 => "−",
            _ => "0",
        };
        out.push(text(sym, at(mid - 4.0, y_d), color, 16.0));
        // Arrow from this node's right value to the next node's left value
        let (a, b) = (f_pos[i].1, f_pos[i + 1].0);
        let (a, b) = (a + (b - a) * 0.12, b - (b - a) * 0.12);
        if (b - a).length() > 8.0 {
            out.push(shape(Shape::Arrow(Arrow::new(a, b)), &thin));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Function {
        Function::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn parses_typed_latex_and_mathml() {
        assert_eq!(f("x^3-3x+1").eval(2.0), 3.0);
        assert_eq!(f("y = 2(x+1)^2").eval(1.0), 8.0);
        assert_eq!(f("-x^2").eval(3.0), -9.0);
        assert_eq!(f("|x-2| + 1").eval(0.0), 3.0);
        assert_eq!(f("2|x|").eval(-3.0), 6.0);
        assert_eq!(f("x^(1/3)").eval(-8.0), -2.0);
        assert!((f("2sin(x) + cos x").eval(0.0) - 1.0).abs() < 1e-12);
        assert_eq!(f("1,5x").eval(2.0), 3.0);
        assert!(f("1/x").eval(0.0).is_nan());
        assert!(f("sqrt(x)").eval(-1.0).is_nan());

        // MathType "Copy as LaTeX"
        assert_eq!(f(r"$y=\frac{2x+1}{x-1}$").eval(2.0), 5.0);
        assert_eq!(f(r"\[y={{x}^{3}}-3x+1\]").eval(2.0), 3.0);
        assert_eq!(f(r"f\left( x \right)=\sqrt{x+5}").eval(4.0), 3.0);
        assert_eq!(f(r"\sqrt[3]{x}").eval(27.0), 3.0);
        assert!(
            (f(r"x\cdot \sin x").eval(std::f64::consts::FRAC_PI_2) - std::f64::consts::FRAC_PI_2)
                .abs()
                < 1e-12
        );

        // MathType MathML copy
        let mathml = r#"<math xmlns="http://www.w3.org/1998/Math/MathML"><mi>y</mi><mo>=</mo>
            <mfrac><mrow><mn>2</mn><mi>x</mi><mo>+</mo><mn>1</mn></mrow><mrow><mi>x</mi><mo>−</mo><mn>1</mn></mrow></mfrac></math>"#;
        assert_eq!(f(mathml).eval(2.0), 5.0);
        let sup =
            r#"<math><msup><mi>x</mi><mn>2</mn></msup><mo>+</mo><msqrt><mi>x</mi></msqrt></math>"#;
        assert_eq!(f(sup).eval(4.0), 18.0);

        assert!(Function::parse("x +* ").is_err());
        assert!(Function::parse("foo(x)").is_err());
        assert!(Function::parse("(x+1").is_err());
    }

    #[test]
    fn studies_cubic() {
        let t = VariationTable::study(&f("x^3-3x+1")).unwrap();
        let xs: Vec<Val> = t.nodes.iter().map(|n| n.x).collect();
        assert_eq!(
            xs,
            vec![
                Val::NegInf,
                Val::Finite(-1.0),
                Val::Finite(1.0),
                Val::PosInf
            ]
        );
        assert_eq!(t.signs, vec![1, -1, 1]);
        assert_eq!(t.nodes[1].left, Val::Finite(3.0));
        assert_eq!(t.nodes[2].left, Val::Finite(-1.0));
        assert_eq!(t.nodes[0].left, Val::NegInf);
        assert_eq!(t.nodes[3].left, Val::PosInf);
    }

    #[test]
    fn studies_rational_with_asymptote() {
        // y = (2x+1)/(x-1): decreasing on both sides of x = 1, horizontal asymptote y = 2
        let t = VariationTable::study(&f("(2x+1)/(x-1)")).unwrap();
        assert_eq!(t.nodes.len(), 3, "{:?}", t.nodes);
        assert_eq!(t.nodes[1].x, Val::Finite(1.0));
        assert!(t.nodes[1].is_break);
        assert_eq!(
            (t.nodes[1].left, t.nodes[1].right),
            (Val::NegInf, Val::PosInf)
        );
        assert_eq!(t.nodes[0].left, Val::Finite(2.0));
        assert_eq!(t.nodes[2].left, Val::Finite(2.0));
        assert_eq!(t.signs, vec![-1, -1]);

        // y = 1/x^2: f' flips through the pole
        let t = VariationTable::study(&f("1/x^2")).unwrap();
        assert!(t.nodes[1].is_break, "{:?}", t.nodes);
        assert_eq!(
            (t.nodes[1].left, t.nodes[1].right),
            (Val::PosInf, Val::PosInf)
        );
        assert_eq!(t.signs, vec![1, -1]);
    }

    #[test]
    fn studies_domain_edge_and_quadratic() {
        let t = VariationTable::study(&f("sqrt(x)")).unwrap();
        assert_eq!(t.nodes[0].x, Val::Finite(0.0));
        assert_eq!(t.nodes[0].left, Val::Finite(0.0));
        assert_eq!(t.nodes.last().unwrap().x, Val::PosInf);
        assert_eq!(t.signs, vec![1]);

        let t = VariationTable::study(&f("x^2-4x+3")).unwrap();
        assert_eq!(t.nodes[1].x, Val::Finite(2.0));
        assert_eq!(t.nodes[1].left, Val::Finite(-1.0));
        assert_eq!(t.signs, vec![-1, 1]);
        assert_eq!(fmt_val(Val::Finite(-1.5)), "−1,5");
        assert_eq!(fmt_val(Val::Finite(3.0)), "3");
    }

    #[test]
    fn graph_breaks_at_asymptote() {
        let frame = GraphFrame {
            origin: Vector2::new(0.0, 0.0),
            unit: 40.0,
            x_range: (-5.0, 5.0),
            y_range: (-5.0, 5.0),
        };
        let strokes = graph_strokes(&f("1/x"), frame, &Style::default(), Color::BLACK);
        let curves = strokes
            .iter()
            .filter(
                |s| matches!(s, Stroke::ShapeStroke(ss) if matches!(ss.shape, Shape::Polyline(_))),
            )
            .count();
        assert_eq!(curves, 2);
    }
}
