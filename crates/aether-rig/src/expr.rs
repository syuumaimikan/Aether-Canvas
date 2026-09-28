//! A small, sandboxed expression language.
//!
//! Drivers let one parameter be computed from others —
//! `BodyAngleX = AngleX * 0.3`, `Breath = sin(time * 2) * 0.5 + 0.5`,
//! `EyeROpen = EyeLOpen` — so relationships an artist would otherwise have to
//! re-key in every motion are stated once.
//!
//! The language is deliberately tiny: numbers, parameter names, arithmetic,
//! comparisons, `&& || !`, the `? :` conditional and a fixed library of pure
//! functions. There are no loops, assignments, strings or I/O, and parsing
//! enforces limits on length, node count and nesting, so a project file can
//! never smuggle in anything that hangs or escapes. Evaluation never fails:
//! non-finite results become zero.
//!
//! Names with spaces can be written in double quotes: `"Hair Front" * 2`.

use aether_core::{AetherError, Result};

/// Longest accepted source text.
pub const MAX_SOURCE_LEN: usize = 4096;
/// Most nodes a program may contain.
pub const MAX_NODES: usize = 1024;
/// Deepest nesting accepted.
pub const MAX_DEPTH: usize = 64;

/// A compiled expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    root: Node,
    /// Variable slots referenced, in first-use order.
    pub variables: Vec<usize>,
    /// Whether the program reads `time` (directly or through `wiggle`).
    pub uses_time: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Num(f64),
    Var(usize),
    Time,
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(BinOp, Box<Node>, Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

/// Library functions: name, arity range.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sqrt,
    Abs,
    Floor,
    Ceil,
    Round,
    Fract,
    Sign,
    Min,
    Max,
    Clamp,
    Lerp,
    Smoothstep,
    Step,
    Pow,
    Exp,
    Ln,
    Log10,
    Mod,
    Remap,
    Noise,
    Wiggle,
    PingPong,
    Deg,
    Rad,
}

impl Func {
    fn lookup(name: &str) -> Option<(Func, usize, usize)> {
        Some(match name {
            "sin" => (Func::Sin, 1, 1),
            "cos" => (Func::Cos, 1, 1),
            "tan" => (Func::Tan, 1, 1),
            "asin" => (Func::Asin, 1, 1),
            "acos" => (Func::Acos, 1, 1),
            "atan" => (Func::Atan, 1, 1),
            "atan2" => (Func::Atan2, 2, 2),
            "sqrt" => (Func::Sqrt, 1, 1),
            "abs" => (Func::Abs, 1, 1),
            "floor" => (Func::Floor, 1, 1),
            "ceil" => (Func::Ceil, 1, 1),
            "round" => (Func::Round, 1, 1),
            "fract" => (Func::Fract, 1, 1),
            "sign" => (Func::Sign, 1, 1),
            "min" => (Func::Min, 2, 8),
            "max" => (Func::Max, 2, 8),
            "clamp" => (Func::Clamp, 3, 3),
            "lerp" | "mix" => (Func::Lerp, 3, 3),
            "smoothstep" => (Func::Smoothstep, 3, 3),
            "step" => (Func::Step, 2, 2),
            "pow" => (Func::Pow, 2, 2),
            "exp" => (Func::Exp, 1, 1),
            "ln" | "log" => (Func::Ln, 1, 1),
            "log10" => (Func::Log10, 1, 1),
            "mod" => (Func::Mod, 2, 2),
            "remap" => (Func::Remap, 5, 5),
            "noise" => (Func::Noise, 1, 2),
            "wiggle" => (Func::Wiggle, 2, 3),
            "pingpong" => (Func::PingPong, 2, 2),
            "deg" => (Func::Deg, 1, 1),
            "rad" => (Func::Rad, 1, 1),
            _ => return None,
        })
    }

    /// Every built-in name, for documentation and autocompletion.
    const NAMES: [&'static str; 37] = [
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "sqrt",
        "abs",
        "floor",
        "ceil",
        "round",
        "fract",
        "sign",
        "min",
        "max",
        "clamp",
        "lerp",
        "mix",
        "smoothstep",
        "step",
        "pow",
        "exp",
        "ln",
        "log",
        "log10",
        "mod",
        "remap",
        "noise",
        "wiggle",
        "pingpong",
        "deg",
        "rad",
        "time",
        "pi",
        "tau",
        "e",
    ];
}

/// Names of the functions and constants the language provides.
pub fn builtin_names() -> &'static [&'static str] {
    &Func::NAMES
}

/// True when `name` is taken by the language and cannot name a parameter
/// usable in expressions.
pub fn is_reserved(name: &str) -> bool {
    Func::NAMES.contains(&name) || matches!(name, "t" | "PI" | "TAU" | "E" | "true" | "false")
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Question,
    Colon,
}

fn tokenize(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let save = i;
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                if i < chars.len() && chars[i].is_ascii_digit() {
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let value: f64 = text
                .parse()
                .map_err(|_| AetherError::rig(format!("'{text}' is not a number")))?;
            out.push(Token::Num(value));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
            continue;
        }
        if c == '"' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            if i >= chars.len() {
                return Err(AetherError::rig("a quoted name is missing its closing quote"));
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
            i += 1;
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let op2 = match two.as_str() {
            "<=" => Some("<="),
            ">=" => Some(">="),
            "==" => Some("=="),
            "!=" => Some("!="),
            "&&" => Some("&&"),
            "||" => Some("||"),
            "**" => Some("^"),
            _ => None,
        };
        if let Some(op) = op2 {
            out.push(Token::Op(op));
            i += 2;
            continue;
        }
        let token = match c {
            '+' => Token::Op("+"),
            '-' => Token::Op("-"),
            '*' => Token::Op("*"),
            '/' => Token::Op("/"),
            '%' => Token::Op("%"),
            '^' => Token::Op("^"),
            '<' => Token::Op("<"),
            '>' => Token::Op(">"),
            '!' => Token::Op("!"),
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            '?' => Token::Question,
            ':' => Token::Colon,
            other => return Err(AetherError::rig(format!("unexpected character '{other}'"))),
        };
        out.push(token);
        i += 1;
    }
    Ok(out)
}

struct Parser<'r> {
    tokens: Vec<Token>,
    pos: usize,
    nodes: usize,
    depth: usize,
    resolve: &'r dyn Fn(&str) -> Option<usize>,
    variables: Vec<usize>,
    uses_time: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn node(&mut self, n: Node) -> Result<Node> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(AetherError::rig("the expression is too long"));
        }
        Ok(n)
    }

    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(AetherError::rig("the expression is nested too deeply"));
        }
        Ok(())
    }

    fn expect_op(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Token::Op(o)) if *o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self) -> Result<Node> {
        self.enter()?;
        let condition = self.or()?;
        let result = if matches!(self.peek(), Some(Token::Question)) {
            self.pos += 1;
            let yes = self.expression()?;
            if !matches!(self.next(), Some(Token::Colon)) {
                return Err(AetherError::rig("expected ':' in a conditional"));
            }
            let no = self.expression()?;
            self.node(Node::Cond(Box::new(condition), Box::new(yes), Box::new(no)))?
        } else {
            condition
        };
        self.depth -= 1;
        Ok(result)
    }

    fn binary_level(&mut self, ops: &[(&str, BinOp)], next: fn(&mut Self) -> Result<Node>) -> Result<Node> {
        let mut left = next(self)?;
        'outer: loop {
            for (text, op) in ops {
                if self.expect_op(text) {
                    let right = next(self)?;
                    left = self.node(Node::Bin(*op, Box::new(left), Box::new(right)))?;
                    continue 'outer;
                }
            }
            return Ok(left);
        }
    }

    fn or(&mut self) -> Result<Node> {
        let mut left = self.and()?;
        while self.expect_op("||") {
            let right = self.and()?;
            left = self.node(Node::Or(Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node> {
        let mut left = self.comparison()?;
        while self.expect_op("&&") {
            let right = self.comparison()?;
            left = self.node(Node::And(Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }

    fn comparison(&mut self) -> Result<Node> {
        self.binary_level(
            &[
                ("<=", BinOp::Le),
                (">=", BinOp::Ge),
                ("<", BinOp::Lt),
                (">", BinOp::Gt),
                ("==", BinOp::Eq),
                ("!=", BinOp::Ne),
            ],
            Self::additive,
        )
    }

    fn additive(&mut self) -> Result<Node> {
        self.binary_level(&[("+", BinOp::Add), ("-", BinOp::Sub)], Self::multiplicative)
    }

    fn multiplicative(&mut self) -> Result<Node> {
        self.binary_level(
            &[("*", BinOp::Mul), ("/", BinOp::Div), ("%", BinOp::Rem)],
            Self::unary,
        )
    }

    fn unary(&mut self) -> Result<Node> {
        self.enter()?;
        let result = if self.expect_op("-") {
            let inner = self.unary()?;
            self.node(Node::Neg(Box::new(inner)))?
        } else if self.expect_op("+") {
            self.unary()?
        } else if self.expect_op("!") {
            let inner = self.unary()?;
            self.node(Node::Not(Box::new(inner)))?
        } else {
            self.power()?
        };
        self.depth -= 1;
        Ok(result)
    }

    fn power(&mut self) -> Result<Node> {
        let base = self.primary()?;
        if self.expect_op("^") {
            // Right-associative, and binds tighter than unary minus on the
            // left: -2^2 is -(2^2).
            let exponent = self.unary()?;
            return self.node(Node::Bin(BinOp::Pow, Box::new(base), Box::new(exponent)));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<Node> {
        match self.next() {
            Some(Token::Num(v)) => self.node(Node::Num(v)),
            Some(Token::LParen) => {
                let inner = self.expression()?;
                if !matches!(self.next(), Some(Token::RParen)) {
                    return Err(AetherError::rig("missing ')'"));
                }
                Ok(inner)
            }
            Some(Token::Ident(name)) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.pos += 1;
                    let (func, min_args, max_args) = Func::lookup(&name)
                        .ok_or_else(|| AetherError::rig(format!("unknown function '{name}'")))?;
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            args.push(self.expression()?);
                            match self.next() {
                                Some(Token::Comma) => continue,
                                Some(Token::RParen) => break,
                                _ => {
                                    return Err(AetherError::rig(format!(
                                        "expected ',' or ')' in {name}(...)"
                                    )))
                                }
                            }
                        }
                    } else {
                        self.pos += 1;
                    }
                    if args.len() < min_args || args.len() > max_args {
                        return Err(AetherError::rig(format!(
                            "{name}() takes {} argument{}, not {}",
                            if min_args == max_args {
                                min_args.to_string()
                            } else {
                                format!("{min_args}–{max_args}")
                            },
                            if max_args == 1 { "" } else { "s" },
                            args.len()
                        )));
                    }
                    if func == Func::Wiggle {
                        self.uses_time = true;
                    }
                    return self.node(Node::Call(func, args));
                }
                match name.as_str() {
                    "pi" | "PI" => self.node(Node::Num(std::f64::consts::PI)),
                    "tau" | "TAU" => self.node(Node::Num(std::f64::consts::TAU)),
                    "e" | "E" => self.node(Node::Num(std::f64::consts::E)),
                    "true" => self.node(Node::Num(1.0)),
                    "false" => self.node(Node::Num(0.0)),
                    "time" | "t" => {
                        self.uses_time = true;
                        self.node(Node::Time)
                    }
                    _ => {
                        let slot = (self.resolve)(&name)
                            .ok_or_else(|| AetherError::rig(format!("unknown name '{name}'")))?;
                        if !self.variables.contains(&slot) {
                            self.variables.push(slot);
                        }
                        self.node(Node::Var(slot))
                    }
                }
            }
            Some(other) => Err(AetherError::rig(format!("unexpected {}", describe(&other)))),
            None => Err(AetherError::rig("the expression ended unexpectedly")),
        }
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Num(v) => format!("number {v}"),
        Token::Ident(n) => format!("name '{n}'"),
        Token::Op(o) => format!("'{o}'"),
        Token::LParen => "'('".into(),
        Token::RParen => "')'".into(),
        Token::Comma => "','".into(),
        Token::Question => "'?'".into(),
        Token::Colon => "':'".into(),
    }
}

impl Program {
    /// Compile `source`, resolving names to variable slots with `resolve`.
    pub fn compile(source: &str, resolve: &dyn Fn(&str) -> Option<usize>) -> Result<Self> {
        if source.len() > MAX_SOURCE_LEN {
            return Err(AetherError::rig("the expression is too long"));
        }
        let tokens = tokenize(source)?;
        if tokens.is_empty() {
            return Err(AetherError::rig("the expression is empty"));
        }
        let mut parser = Parser {
            tokens,
            pos: 0,
            nodes: 0,
            depth: 0,
            resolve,
            variables: Vec::new(),
            uses_time: false,
        };
        let root = parser.expression()?;
        if let Some(extra) = parser.peek() {
            return Err(AetherError::rig(format!(
                "unexpected {} after the expression",
                describe(extra)
            )));
        }
        Ok(Self {
            root,
            variables: parser.variables,
            uses_time: parser.uses_time,
        })
    }

    /// Evaluate with variable slots read from `vars` and the given time.
    pub fn eval(&self, vars: &dyn Fn(usize) -> f32, time: f32) -> f32 {
        let v = eval(&self.root, vars, time as f64) as f32;
        if v.is_finite() {
            v
        } else {
            0.0
        }
    }
}

fn truthy(v: f64) -> bool {
    v != 0.0 && !v.is_nan()
}

fn eval(node: &Node, vars: &dyn Fn(usize) -> f32, time: f64) -> f64 {
    match node {
        Node::Num(v) => *v,
        Node::Var(slot) => vars(*slot) as f64,
        Node::Time => time,
        Node::Neg(inner) => -eval(inner, vars, time),
        Node::Not(inner) => (!truthy(eval(inner, vars, time))) as i32 as f64,
        Node::Cond(c, a, b) => {
            if truthy(eval(c, vars, time)) {
                eval(a, vars, time)
            } else {
                eval(b, vars, time)
            }
        }
        Node::And(a, b) => (truthy(eval(a, vars, time)) && truthy(eval(b, vars, time))) as i32 as f64,
        Node::Or(a, b) => (truthy(eval(a, vars, time)) || truthy(eval(b, vars, time))) as i32 as f64,
        Node::Bin(op, a, b) => {
            let x = eval(a, vars, time);
            let y = eval(b, vars, time);
            match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x / y
                    }
                }
                BinOp::Rem => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x.rem_euclid(y)
                    }
                }
                BinOp::Pow => x.powf(y),
                BinOp::Lt => (x < y) as i32 as f64,
                BinOp::Le => (x <= y) as i32 as f64,
                BinOp::Gt => (x > y) as i32 as f64,
                BinOp::Ge => (x >= y) as i32 as f64,
                BinOp::Eq => ((x - y).abs() < 1e-9) as i32 as f64,
                BinOp::Ne => ((x - y).abs() >= 1e-9) as i32 as f64,
            }
        }
        Node::Call(func, args) => {
            let a: Vec<f64> = args.iter().map(|n| eval(n, vars, time)).collect();
            let arg = |i: usize| a.get(i).copied().unwrap_or(0.0);
            match func {
                Func::Sin => arg(0).sin(),
                Func::Cos => arg(0).cos(),
                Func::Tan => arg(0).tan(),
                Func::Asin => arg(0).clamp(-1.0, 1.0).asin(),
                Func::Acos => arg(0).clamp(-1.0, 1.0).acos(),
                Func::Atan => arg(0).atan(),
                Func::Atan2 => arg(0).atan2(arg(1)),
                Func::Sqrt => arg(0).max(0.0).sqrt(),
                Func::Abs => arg(0).abs(),
                Func::Floor => arg(0).floor(),
                Func::Ceil => arg(0).ceil(),
                Func::Round => arg(0).round(),
                Func::Fract => arg(0) - arg(0).floor(),
                Func::Sign => {
                    let v = arg(0);
                    if v > 0.0 {
                        1.0
                    } else if v < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                Func::Min => a.iter().copied().fold(f64::INFINITY, f64::min),
                Func::Max => a.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                Func::Clamp => {
                    let (lo, hi) = (arg(1).min(arg(2)), arg(1).max(arg(2)));
                    arg(0).clamp(lo, hi)
                }
                Func::Lerp => arg(0) + (arg(1) - arg(0)) * arg(2),
                Func::Smoothstep => {
                    let (e0, e1, x) = (arg(0), arg(1), arg(2));
                    if (e1 - e0).abs() < 1e-12 {
                        return if x < e0 { 0.0 } else { 1.0 };
                    }
                    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                }
                Func::Step => (arg(1) >= arg(0)) as i32 as f64,
                Func::Pow => arg(0).powf(arg(1)),
                Func::Exp => arg(0).min(700.0).exp(),
                Func::Ln => {
                    if arg(0) > 0.0 {
                        arg(0).ln()
                    } else {
                        0.0
                    }
                }
                Func::Log10 => {
                    if arg(0) > 0.0 {
                        arg(0).log10()
                    } else {
                        0.0
                    }
                }
                Func::Mod => {
                    if arg(1) == 0.0 {
                        0.0
                    } else {
                        arg(0).rem_euclid(arg(1))
                    }
                }
                Func::Remap => {
                    let (x, a0, a1, b0, b1) = (arg(0), arg(1), arg(2), arg(3), arg(4));
                    if (a1 - a0).abs() < 1e-12 {
                        b0
                    } else {
                        b0 + (x - a0) / (a1 - a0) * (b1 - b0)
                    }
                }
                Func::Noise => value_noise(arg(0), arg(1) as i64),
                Func::Wiggle => {
                    // wiggle(frequency, amplitude[, seed]): smooth random
                    // motion over time, two octaves.
                    let (freq, amp, seed) = (arg(0), arg(1), arg(2) as i64);
                    let t = time * freq;
                    amp * (value_noise(t, seed) * 0.7 + value_noise(t * 2.1, seed + 17) * 0.3)
                }
                Func::PingPong => {
                    let length = arg(1).abs();
                    if length < 1e-12 {
                        0.0
                    } else {
                        let m = arg(0).rem_euclid(length * 2.0);
                        length - (m - length).abs()
                    }
                }
                Func::Deg => arg(0).to_degrees(),
                Func::Rad => arg(0).to_radians(),
            }
        }
    }
}

/// Smooth 1D value noise in `-1..=1`.
pub fn value_noise(x: f64, seed: i64) -> f64 {
    fn hash(i: i64, seed: i64) -> f64 {
        let mut h = (i as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((seed as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
        h ^= h >> 33;
        h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        h ^= h >> 33;
        (h as f64 / u64::MAX as f64) * 2.0 - 1.0
    }
    let i = x.floor();
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(i as i64, seed);
    let b = hash(i as i64 + 1, seed);
    a + (b - a) * u
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> f32 {
        let resolve = |name: &str| match name {
            "AngleX" => Some(0),
            "Hair Front" => Some(1),
            _ => None,
        };
        let program = Program::compile(src, &resolve).unwrap_or_else(|e| panic!("{src}: {e}"));
        program.eval(&|slot| [20.0, 0.5][slot], 2.0)
    }

    #[test]
    fn arithmetic_follows_precedence() {
        assert_eq!(run("1 + 2 * 3"), 7.0);
        assert_eq!(run("(1 + 2) * 3"), 9.0);
        assert_eq!(run("2 ^ 3 ^ 2"), 512.0, "power is right-associative");
        assert_eq!(run("-2 ^ 2"), -4.0);
        assert_eq!(run("7 % 3"), 1.0);
        assert_eq!(run("1e2 + .5"), 100.5);
    }

    #[test]
    fn names_time_and_constants_resolve() {
        assert_eq!(run("AngleX * 0.5"), 10.0);
        assert_eq!(run("\"Hair Front\" * 4"), 2.0);
        assert_eq!(run("time"), 2.0);
        assert!((run("sin(pi / 2)") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn logic_and_conditionals_work() {
        assert_eq!(run("AngleX > 10 ? 1 : -1"), 1.0);
        assert_eq!(run("AngleX > 10 && AngleX < 15"), 0.0);
        assert_eq!(run("!(AngleX < 10) || 0"), 1.0);
        assert_eq!(run("1 < 2 ? 3 < 4 ? 5 : 6 : 7"), 5.0);
    }

    #[test]
    fn library_functions_behave() {
        assert_eq!(run("clamp(AngleX, -10, 10)"), 10.0);
        assert_eq!(run("max(1, 5, 3)"), 5.0);
        assert_eq!(run("lerp(0, 10, 0.25)"), 2.5);
        assert_eq!(run("smoothstep(0, 1, 0.5)"), 0.5);
        assert_eq!(run("remap(AngleX, -30, 30, 0, 1)"), (50.0f32 / 60.0));
        assert_eq!(run("pingpong(3, 2)"), 1.0);
        let w = run("wiggle(2, 5)");
        assert!(w.abs() <= 5.0);
        assert_eq!(run("wiggle(2, 5)"), w, "wiggle is deterministic for a given time");
    }

    #[test]
    fn division_by_zero_and_domain_errors_are_harmless() {
        assert_eq!(run("1 / 0"), 0.0);
        assert_eq!(run("sqrt(-4)"), 0.0);
        assert_eq!(run("ln(0)"), 0.0);
        assert!(run("exp(10000)").is_finite());
    }

    #[test]
    fn errors_explain_themselves() {
        let resolve = |_: &str| None;
        for (src, needle) in [
            ("", "empty"),
            ("1 +", "ended"),
            ("Nope * 2", "unknown name 'Nope'"),
            ("frobnicate(1)", "unknown function"),
            ("clamp(1, 2)", "takes 3 arguments"),
            ("(1 + 2", "missing ')'"),
            ("1 2", "after the expression"),
            ("1 $ 2", "unexpected character"),
            ("\"open", "closing quote"),
        ] {
            let err = Program::compile(src, &resolve).expect_err(src);
            assert!(err.to_string().contains(needle), "{src}: {err}");
        }
    }

    #[test]
    fn resource_limits_are_enforced() {
        let resolve = |_: &str| None;
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(Program::compile(&deep, &resolve).is_err());
        let long = vec!["1"; 3000].join("+");
        assert!(Program::compile(&long, &resolve).is_err());
    }

    #[test]
    fn programs_report_what_they_read() {
        let resolve = |name: &str| (name == "AngleX").then_some(3);
        let p = Program::compile("AngleX + AngleX * wiggle(1, 1)", &resolve).expect("compile");
        assert_eq!(p.variables, vec![3]);
        assert!(p.uses_time);
    }

    #[test]
    fn noise_is_smooth_and_bounded() {
        let mut previous = value_noise(0.0, 1);
        for i in 1..1000 {
            let v = value_noise(i as f64 * 0.01, 1);
            assert!((-1.0..=1.0).contains(&v));
            assert!((v - previous).abs() < 0.05, "noise jumped at {i}");
            previous = v;
        }
    }
}
