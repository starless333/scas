//! Function plotting shared by the terminal REPL and the desktop GUI.
//!
//! A plot samples one or more SCAS expressions at the session's own
//! definitions: `:plot f(x)` looks up `f` in the live interpreter, binds the
//! independent variable to each sample point, and evaluates the expression
//! with the normal evaluator. Sampling temporarily lowers the decimal
//! precision and restores both the precision and any displaced variable
//! binding afterwards, so plotting never disturbs session state.

use crate::ast::statement::FunctionBody;
use crate::ast::{BinaryOp, Expr, ExprKind, StatementKind, UnaryOp};
use crate::interpreter::value::{SymbolicExpr, Value};
use crate::interpreter::{Env, Environment, Interpreter};
use crate::lexer::{Scanner, Symbol, SymbolTable};
use crate::number::{MathConstant, Number};
use crate::parser::Parser;
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{ToPrimitive, Zero};
use std::fs;
use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

/// Default horizontal window when no `from A to B` range is given.
const DEFAULT_X_MIN: f64 = -10.0;
const DEFAULT_X_MAX: f64 = 10.0;

/// Significant digits used while sampling. Plotting needs a dozen digits at
/// most, and transcendental evaluation at this precision is fast enough for
/// live redrawing in the GUI.
const SAMPLE_PRECISION: u64 = 12;

/// Sample count for plots drawn through the `:plot` command. Higher than the
/// ASCII resolution so SVG exports stay smooth; the ASCII renderer aggregates
/// samples per column.
const COMMAND_SAMPLE_COUNT: usize = 401;

const SVG_WIDTH: u32 = 800;
const SVG_HEIGHT: u32 = 520;

const SERIES_COLORS: &[&str] = &[
    "#1f6feb", "#d73a49", "#0a8f4d", "#8250df", "#d97706", "#0e7490", "#db2777", "#4d7c0f",
];

const ASCII_GLYPHS: &[char] = &['*', 'o', '#', '@', '%', '&', 'X', 'W'];

const PLOT_USAGE: &str =
    "usage: :plot [terminal|svg] EXPRESSION[, EXPRESSION…] [from A to B] [> FILE]

  The independent variable is x. The default is terminal output. The svg
  mode writes SVG to the terminal, or to FILE when redirected. A FILE ending
  in .svg also receives SVG in the default mode.

examples:
  :plot sin(x)
  :plot svg sin(x)
  :plot x^2 - 2x from -5 to 5
  :plot sin(x), cos(x) from -6 to 6 > graph.svg";

/// One sampled curve. `samples` pairs each x with `Some(y)` for finite real
/// values and `None` where the expression is undefined, complex, or errored.
#[derive(Debug, Clone)]
pub struct SampledSeries {
    pub label: String,
    pub variable: String,
    pub samples: Vec<(f64, Option<f64>)>,
}

/// The visible window of a plot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotBounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl PlotBounds {
    pub fn new(x_min: f64, x_max: f64, y_min: f64, y_max: f64) -> Self {
        Self {
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }

    pub fn x_span(&self) -> f64 {
        self.x_max - self.x_min
    }

    pub fn y_span(&self) -> f64 {
        self.y_max - self.y_min
    }

    /// Zoom around a point in viewport coordinates (0 = left/top, 1 = right/bottom).
    pub fn zoomed_at(&self, factor: f64, x_ratio: f64, y_ratio: f64) -> Option<Self> {
        let x_span = self.x_span() * factor;
        let y_span = self.y_span() * factor;
        if !factor.is_finite() || !(1e-6..=1e9).contains(&x_span) || !(1e-6..=1e9).contains(&y_span)
        {
            return None;
        }
        let x_ratio = x_ratio.clamp(0.0, 1.0);
        let y_ratio = y_ratio.clamp(0.0, 1.0);
        let anchor_x = self.x_min + x_ratio * self.x_span();
        let anchor_y = self.y_max - y_ratio * self.y_span();
        Some(Self::new(
            anchor_x - x_ratio * x_span,
            anchor_x + (1.0 - x_ratio) * x_span,
            anchor_y - (1.0 - y_ratio) * y_span,
            anchor_y + y_ratio * y_span,
        ))
    }

    pub fn panned_by_pixels(&self, dx: f32, dy: f32, width: f32, height: f32) -> Option<Self> {
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let x_shift = -f64::from(dx) * self.x_span() / f64::from(width);
        let y_shift = f64::from(dy) * self.y_span() / f64::from(height);
        let next = Self::new(
            self.x_min + x_shift,
            self.x_max + x_shift,
            self.y_min + y_shift,
            self.y_max + y_shift,
        );
        [next.x_min, next.x_max, next.y_min, next.y_max]
            .iter()
            .all(|bound| bound.is_finite())
            .then_some(next)
    }

    /// Whether an x coordinate falls inside the window (inclusive).
    pub fn contains_x(&self, x: f64) -> bool {
        x >= self.x_min && x <= self.x_max
    }

    /// Whether a y coordinate falls inside the window (inclusive).
    pub fn contains_y(&self, y: f64) -> bool {
        y >= self.y_min && y <= self.y_max
    }
}

/// What one `:plot` invocation produced.
pub struct PlotOutcome {
    pub series: Vec<SampledSeries>,
    pub bounds: PlotBounds,
    /// The terminal-resolution ASCII drawing (also used for text files).
    pub ascii: String,
    pub svg: Option<String>,
    pub wrote_file: Option<PathBuf>,
}

struct PlotCommand {
    series: Vec<String>,
    from: Option<String>,
    to: Option<String>,
    redirect: Option<String>,
}

// ---------------------------------------------------------------------------
// Numeric conversion helpers
// ---------------------------------------------------------------------------

/// The exact binary rational equal to `value`, so sampled x values keep full
/// `f64` fidelity while remaining exact `Number`s for the evaluator.
pub fn number_from_f64(value: f64) -> Option<Number> {
    if !value.is_finite() {
        return None;
    }
    if value == 0.0 {
        return Some(Number::Rational(BigRational::zero()));
    }
    let bits = value.to_bits();
    let sign_magnitude = if bits >> 63 == 1 {
        -(i128::from(bits & 0x000f_ffff_ffff_ffff | 0x0010_0000_0000_0000))
    } else {
        i128::from(bits & 0x000f_ffff_ffff_ffff | 0x0010_0000_0000_0000)
    };
    let exponent = i32::try_from((bits >> 52) & 0x7ff).ok()? - 1075;
    let mantissa = BigInt::from(sign_magnitude);
    let number = if exponent >= 0 {
        BigRational::from_integer(mantissa << exponent)
    } else {
        BigRational::new(mantissa, BigInt::from(1) << (-exponent))
    };
    Some(Number::Rational(number))
}

/// Real part of a numeric value; complex numbers with a nonzero imaginary
/// part map to `None` so they become gaps in the drawn curve.
pub fn number_to_f64(value: &Number) -> Option<f64> {
    match value {
        Number::Integer(value) => value.to_f64(),
        Number::Rational(value) => value.to_f64(),
        Number::Decimal(value) => value.to_f64(),
        Number::Complex(value) => {
            if value.imaginary.is_zero() {
                value.real.to_f64()
            } else {
                None
            }
        }
    }
}

pub fn value_to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number_to_f64(number),
        Value::Derivation { result, .. } => value_to_f64(result),
        _ => None,
    }
}

fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

// ---------------------------------------------------------------------------
// Compiled numeric sampling (fast path)
// ---------------------------------------------------------------------------

/// A compiled f64 evaluation of a plotted expression. Interactive graphers
/// compile each expression once and evaluate the compiled form for every
/// sample point instead of walking a general-purpose evaluator thousands of
/// times per redraw. The exact evaluator remains the fallback whenever an
/// expression leaves the modeled numeric subset (matrices, comparisons,
/// ...), so plotting stays semantically identical there.
#[derive(Clone)]
enum NumericExpr {
    Constant(f64),
    Variable,
    Negate(Box<NumericExpr>),
    Add(Box<NumericExpr>, Box<NumericExpr>),
    Sub(Box<NumericExpr>, Box<NumericExpr>),
    Mul(Box<NumericExpr>, Box<NumericExpr>),
    Div(Box<NumericExpr>, Box<NumericExpr>),
    Pow(Box<NumericExpr>, Box<NumericExpr>),
    Call(NumFn, Box<NumericExpr>),
    Call2(NumFn2, Box<NumericExpr>, Box<NumericExpr>),
}

#[derive(Clone, Copy)]
enum NumFn {
    Abs,
    Sqrt,
    Cbrt,
    Exp,
    Ln,
    Log10,
    Log2,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Floor,
    Ceil,
    Trunc,
    Sign,
    Degrees,
    Radians,
}

#[derive(Clone, Copy)]
enum NumFn2 {
    Atan2,
    Hypot,
    Min,
    Max,
    LogBase,
}

impl NumericExpr {
    fn evaluate(&self, x: f64) -> f64 {
        match self {
            Self::Constant(value) => *value,
            Self::Variable => x,
            Self::Negate(inner) => -inner.evaluate(x),
            Self::Add(left, right) => left.evaluate(x) + right.evaluate(x),
            Self::Sub(left, right) => left.evaluate(x) - right.evaluate(x),
            Self::Mul(left, right) => left.evaluate(x) * right.evaluate(x),
            Self::Div(left, right) => left.evaluate(x) / right.evaluate(x),
            Self::Pow(left, right) => left.evaluate(x).powf(right.evaluate(x)),
            Self::Call(function, inner) => {
                let y = inner.evaluate(x);
                match function {
                    NumFn::Abs => y.abs(),
                    NumFn::Sqrt => y.sqrt(),
                    NumFn::Cbrt => y.cbrt(),
                    NumFn::Exp => y.exp(),
                    NumFn::Ln => y.ln(),
                    NumFn::Log10 => y.log10(),
                    NumFn::Log2 => y.log2(),
                    NumFn::Sin => y.sin(),
                    NumFn::Cos => y.cos(),
                    NumFn::Tan => y.tan(),
                    NumFn::Asin => y.asin(),
                    NumFn::Acos => y.acos(),
                    NumFn::Atan => y.atan(),
                    NumFn::Sinh => y.sinh(),
                    NumFn::Cosh => y.cosh(),
                    NumFn::Tanh => y.tanh(),
                    NumFn::Asinh => y.asinh(),
                    NumFn::Acosh => y.acosh(),
                    NumFn::Atanh => y.atanh(),
                    NumFn::Floor => y.floor(),
                    NumFn::Ceil => y.ceil(),
                    NumFn::Trunc => y.trunc(),
                    NumFn::Sign => {
                        if y == 0.0 {
                            0.0
                        } else {
                            y.signum()
                        }
                    }
                    NumFn::Degrees => y.to_degrees(),
                    NumFn::Radians => y.to_radians(),
                }
            }
            Self::Call2(function, left, right) => {
                let a = left.evaluate(x);
                let b = right.evaluate(x);
                match function {
                    NumFn2::Atan2 => a.atan2(b),
                    NumFn2::Hypot => a.hypot(b),
                    NumFn2::Min => a.min(b),
                    NumFn2::Max => a.max(b),
                    NumFn2::LogBase => a.ln() / b.ln(),
                }
            }
        }
    }
}

/// Compiles `expression` for f64 sampling in `variable`, resolving free
/// names through the live interpreter exactly as the evaluator would.
/// Returns `None` the moment any node falls outside the modeled numeric
/// subset; callers then keep using the exact evaluator.
fn compile_numeric<W: Write>(
    expression: &Expr,
    symbols: &SymbolTable,
    variable: &str,
    interpreter: &Interpreter<W>,
) -> Option<NumericExpr> {
    Compiler {
        symbols,
        variable,
        interpreter,
        closure: None,
        parameters: Vec::new(),
        depth: 0,
    }
    .compile(expression)
}

/// One level of compiled-expression construction. At the top level free
/// names resolve through the live globals; inside an inlined user function
/// they resolve through that function's closure, mirroring the evaluator's
/// scoping rules exactly (including shadowed builtins and captured state).
struct Compiler<'a, W: Write> {
    symbols: &'a SymbolTable,
    variable: &'a str,
    interpreter: &'a Interpreter<W>,
    closure: Option<&'a Env>,
    parameters: Vec<(Symbol, NumericExpr)>,
    depth: u8,
}

/// Maximum user-function inlining depth before giving up on compilation.
const MAX_INLINE_DEPTH: u8 = 8;

impl<'a, W: Write> Compiler<'a, W> {
    fn compile(&self, expression: &Expr) -> Option<NumericExpr> {
        match &expression.kind {
            ExprKind::Number(text) => parse_number_literal(text).map(NumericExpr::Constant),
            ExprKind::Group(inner) => self.compile(inner),
            ExprKind::Variable(symbol) => self.compile_variable(*symbol),
            ExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(NumericExpr::Negate(Box::new(self.compile(expr)?))),
            ExprKind::Binary {
                left, op, right, ..
            } => {
                let left = self.compile(left)?;
                let right = self.compile(right)?;
                let combine = |make: fn(Box<NumericExpr>, Box<NumericExpr>) -> NumericExpr| {
                    Some(make(Box::new(left), Box::new(right)))
                };
                match op {
                    BinaryOp::Add => combine(NumericExpr::Add),
                    BinaryOp::Sub => combine(NumericExpr::Sub),
                    BinaryOp::Mul => combine(NumericExpr::Mul),
                    BinaryOp::Div => combine(NumericExpr::Div),
                    BinaryOp::Pow => combine(NumericExpr::Pow),
                    _ => None,
                }
            }
            ExprKind::Call { callee, args } => self.compile_call(callee, args),
            _ => None,
        }
    }

    fn compile_variable(&self, symbol: Symbol) -> Option<NumericExpr> {
        if let Some((_, compiled)) = self
            .parameters
            .iter()
            .rev()
            .find(|(name, _)| *name == symbol)
        {
            return Some(compiled.clone());
        }
        if self.symbols.resolve(symbol) == self.variable {
            return Some(NumericExpr::Variable);
        }
        match self.lookup_value(symbol) {
            Some(Value::Number(number)) => number_to_f64(&number)
                .filter(|value| value.is_finite())
                .map(NumericExpr::Constant),
            Some(Value::Symbolic(expression)) => match expression.as_ref() {
                SymbolicExpr::Constant(constant) => {
                    numeric_constant_value(*constant).map(NumericExpr::Constant)
                }
                _ => None,
            },
            // Constants the interpreter has not interned yet still compile
            // from their well-known names.
            None => numeric_constant(self.symbols.resolve(symbol)).map(NumericExpr::Constant),
            _ => None,
        }
    }

    /// Resolves a free name exactly where the evaluator would: the inlined
    /// function's closure when compiling a body, the live globals otherwise.
    fn lookup_value(&self, symbol: Symbol) -> Option<Value> {
        match self.closure {
            Some(closure) => Environment::get(closure, symbol),
            None => self.interpreter.global_value(self.symbols.resolve(symbol)),
        }
    }

    fn compile_call(&self, callee: &Expr, args: &[Expr]) -> Option<NumericExpr> {
        let ExprKind::Variable(symbol) = callee.kind else {
            return None;
        };
        match self.lookup_value(symbol) {
            // A user function with an expression body is inlined so its
            // arguments evaluate on the fast path too. Block bodies and
            // runaway recursion fall back to the exact evaluator.
            Some(Value::Function(function)) => {
                if self.depth >= MAX_INLINE_DEPTH || function.params.len() != args.len() {
                    return None;
                }
                let FunctionBody::Expression(body) = &function.body else {
                    return None;
                };
                let mut parameters = self.parameters.clone();
                for (param, argument) in function.params.iter().zip(args.iter()) {
                    parameters.push((*param, self.compile(argument)?));
                }
                Compiler {
                    symbols: self.symbols,
                    variable: self.variable,
                    interpreter: self.interpreter,
                    closure: Some(&function.closure),
                    parameters,
                    depth: self.depth + 1,
                }
                .compile(body)
            }
            Some(Value::Builtin(builtin)) => self.compile_builtin(builtin.name(), args),
            _ => None,
        }
    }

    fn compile_builtin(&self, name: &str, args: &[Expr]) -> Option<NumericExpr> {
        // `evalf` only forces numeric evaluation; f64 sampling already is one.
        if name == "evalf" && args.len() == 1 {
            return self.compile(&args[0]);
        }
        if args.len() == 1 {
            let function = match name {
                "abs" => NumFn::Abs,
                "sqrt" => NumFn::Sqrt,
                "cbrt" => NumFn::Cbrt,
                "exp" => NumFn::Exp,
                "ln" | "log" => NumFn::Ln,
                "log10" => NumFn::Log10,
                "log2" => NumFn::Log2,
                "sin" => NumFn::Sin,
                "cos" => NumFn::Cos,
                "tan" => NumFn::Tan,
                "asin" => NumFn::Asin,
                "acos" => NumFn::Acos,
                "atan" => NumFn::Atan,
                "sinh" => NumFn::Sinh,
                "cosh" => NumFn::Cosh,
                "tanh" => NumFn::Tanh,
                "asinh" => NumFn::Asinh,
                "acosh" => NumFn::Acosh,
                "atanh" => NumFn::Atanh,
                "floor" => NumFn::Floor,
                "ceil" => NumFn::Ceil,
                "trunc" => NumFn::Trunc,
                "sign" => NumFn::Sign,
                "degrees" => NumFn::Degrees,
                "radians" => NumFn::Radians,
                _ => return None,
            };
            let inner = self.compile(&args[0])?;
            return Some(NumericExpr::Call(function, Box::new(inner)));
        }
        if args.len() == 2 {
            let function = match name {
                "atan2" => NumFn2::Atan2,
                "hypot" => NumFn2::Hypot,
                "min" => NumFn2::Min,
                "max" => NumFn2::Max,
                "log" => NumFn2::LogBase,
                _ => return None,
            };
            let left = self.compile(&args[0])?;
            let right = self.compile(&args[1])?;
            return Some(NumericExpr::Call2(
                function,
                Box::new(left),
                Box::new(right),
            ));
        }
        None
    }
}

fn parse_number_literal(text: &str) -> Option<f64> {
    let cleaned = text.replace('_', "");
    let value = match cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
    {
        Some(hex) => u64::from_str_radix(hex, 16).ok()? as f64,
        None => cleaned.parse::<f64>().ok()?,
    };
    value.is_finite().then_some(value)
}

fn numeric_constant(name: &str) -> Option<f64> {
    match name {
        "pi" | "π" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        "tau" | "τ" => Some(std::f64::consts::TAU),
        "phi" | "φ" => Some((1.0 + 5.0f64.sqrt()) / 2.0),
        _ => None,
    }
}

fn numeric_constant_value(constant: MathConstant) -> Option<f64> {
    numeric_constant(constant.name())
}

/// Samples a compiled expression adaptively. A uniform base grid anchors the
/// curve; segments are then recursively subdivided wherever the midpoint
/// sags away from the chord by more than a sub-pixel tolerance, keeping the
/// drawn error roughly constant across the curve. This adaptive subdivision
/// is the standard technique behind responsive graphing tools: points land
/// where the curvature is instead of uniformly, so peaks, valleys, and
/// domain edges stay smooth without oversampling flat stretches.
fn sample_numeric_adaptive(
    expression: &NumericExpr,
    x_min: f64,
    x_max: f64,
    count: usize,
) -> Vec<(f64, Option<f64>)> {
    const MAX_DEPTH: u8 = 5;
    let count = count.clamp(2, 4_000);
    let span = x_max - x_min;
    let evaluate = |x: f64| Some(expression.evaluate(x)).filter(|y| y.is_finite());

    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        let x = if index == 0 {
            x_min
        } else if index + 1 == count {
            x_max
        } else {
            x_min + span * index as f64 / (count - 1) as f64
        };
        samples.push((x, evaluate(x)));
    }

    // Sub-pixel tolerance from a robust vertical spread, so asymptote tails
    // do not force runaway refinement.
    let mut values = samples.iter().filter_map(|(_, y)| *y).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let trim = values.len() / 20;
    let low = values.get(trim).copied().unwrap_or(0.0);
    let high = values
        .get(values.len().saturating_sub(1 + trim))
        .copied()
        .unwrap_or(0.0);
    let y_tolerance = ((high - low) / 250.0).max(1.0e-12);
    let mut budget = count.saturating_mul(2);

    let mut output = Vec::with_capacity(count * 2);
    let mut previous = samples[0];
    output.push(previous);
    for &current in &samples[1..] {
        refine_segment(
            expression,
            previous,
            current,
            y_tolerance,
            0,
            MAX_DEPTH,
            &mut budget,
            &mut output,
        );
        output.push(current);
        previous = current;
    }
    output
}

#[allow(clippy::too_many_arguments)]
fn refine_segment(
    expression: &NumericExpr,
    left: (f64, Option<f64>),
    right: (f64, Option<f64>),
    y_tolerance: f64,
    depth: u8,
    max_depth: u8,
    budget: &mut usize,
    output: &mut Vec<(f64, Option<f64>)>,
) {
    if depth >= max_depth || *budget == 0 {
        return;
    }
    let mid_x = (left.0 + right.0) * 0.5;
    if !(mid_x > left.0 && mid_x < right.0) {
        return;
    }
    let mid_y = Some(expression.evaluate(mid_x)).filter(|y| y.is_finite());
    let needed = match (left.1, mid_y, right.1) {
        (Some(y0), Some(y1), Some(y2)) => (y1 - (y0 + y2) * 0.5).abs() > y_tolerance,
        // Undefined land needs no points, but a mix marks a domain edge.
        (left_y, mid_y, right_y) => left_y.is_some() || mid_y.is_some() || right_y.is_some(),
    };
    if !needed {
        return;
    }
    *budget -= 1;
    let mid = (mid_x, mid_y);
    refine_segment(
        expression,
        left,
        mid,
        y_tolerance,
        depth + 1,
        max_depth,
        budget,
        output,
    );
    output.push(mid);
    refine_segment(
        expression,
        mid,
        right,
        y_tolerance,
        depth + 1,
        max_depth,
        budget,
        output,
    );
}

// ---------------------------------------------------------------------------
// Command parsing
// ---------------------------------------------------------------------------

/// Marks which bytes of `source` sit outside strings and delimiters, so
/// `from`, `to`, `,`, and `>` are only recognized at the top level.
fn top_level_mask(source: &str) -> Vec<bool> {
    let mut mask = vec![false; source.len()];
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in source.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => mask[index] = character.is_ascii(),
        }
    }
    mask
}

fn find_top_level(source: &str, needle: &str, from_end: bool) -> Option<usize> {
    let mask = top_level_mask(source);
    let bytes = source.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.len() > bytes.len() {
        return None;
    }
    let mut found = None;
    for start in 0..=bytes.len() - needle_bytes.len() {
        let matches = needle_bytes
            .iter()
            .enumerate()
            .all(|(offset, &byte)| mask[start + offset] && bytes[start + offset] == byte);
        if matches {
            found = Some(start);
            if !from_end {
                return found;
            }
        }
    }
    found
}

/// Detects a trailing `> PATH` redirect. Only a top-level `>` followed by a
/// whitespace and a path-like token (containing `.`, `/`, or `\`) counts, so
/// comparisons such as `x > 3` stay expressions.
fn split_redirect(source: &str) -> (String, Option<String>) {
    let bytes = source.as_bytes();
    let mask = top_level_mask(source);
    for start in (0..bytes.len()).rev() {
        if bytes[start] != b'>' || !mask[start] {
            continue;
        }
        if start + 1 < bytes.len() && bytes[start + 1] != b' ' {
            continue;
        }
        if start > 0 && bytes[start - 1] == b'>' {
            continue;
        }
        let target = source[start + 1..].trim();
        if target.is_empty()
            || target.contains(char::is_whitespace)
            || !target.contains(['.', '/', '\\'])
        {
            continue;
        }
        return (
            source[..start].trim_end().to_owned(),
            Some(target.to_owned()),
        );
    }
    (source.trim().to_owned(), None)
}

fn parse_plot_command(rest: &str) -> Result<PlotCommand, String> {
    let (series_part, redirect) = split_redirect(rest);
    let (series_part, range_part) = match find_top_level(&series_part, " from ", true) {
        Some(position) => (
            series_part[..position].to_owned(),
            Some(series_part[position + " from ".len()..].to_owned()),
        ),
        None => (series_part.clone(), None),
    };
    let (from, to) = match range_part {
        Some(range) => {
            let (low, high) = match find_top_level(&range, " to ", false) {
                Some(position) => (
                    range[..position].trim().to_owned(),
                    range[position + " to ".len()..].trim().to_owned(),
                ),
                None => {
                    return Err("plot ranges need both bounds: from A to B".to_owned());
                }
            };
            if low.is_empty() || high.is_empty() {
                return Err("plot ranges need both bounds: from A to B".to_owned());
            }
            (Some(low), Some(high))
        }
        None => (None, None),
    };
    let series = series_part
        .split_top_level_commas()
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if series.is_empty() {
        return Err(PLOT_USAGE.to_owned());
    }
    Ok(PlotCommand {
        series,
        from,
        to,
        redirect,
    })
}

trait SplitTopLevel {
    fn split_top_level_commas(&self) -> Vec<String>;
}

impl SplitTopLevel for str {
    fn split_top_level_commas(&self) -> Vec<String> {
        let mask = top_level_mask(self);
        let mut parts = Vec::new();
        let mut start = 0;
        for (index, character) in self.char_indices() {
            if character == ',' && mask[index] {
                parts.push(self[start..index].to_owned());
                start = index + 1;
            }
        }
        parts.push(self[start..].to_owned());
        parts
    }
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

fn render_scan_errors(submission: &str, errors: &[crate::lexer::ScanError]) -> String {
    errors
        .iter()
        .map(|error| error.render(submission, "<plot>"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn terminate(source: &str) -> String {
    let trimmed = source.trim_end();
    if trimmed.ends_with(';') || trimmed.ends_with('}') {
        source.to_owned()
    } else {
        format!("{source};")
    }
}

/// Parses `source` into a program plus, when the final statement is a bare
/// expression, that expression. Executes nothing.
fn parse_submission(
    symbols: &mut SymbolTable,
    source: &str,
) -> Result<(Vec<crate::ast::Statement>, Option<Expr>, String), String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("nothing to plot: the expression is empty".to_owned());
    }
    let submission = terminate(trimmed);
    let mut scanner = Scanner::with_symbols(&submission, symbols.clone());
    let tokens = scanner.scan_tokens().clone();
    if scanner.had_error() {
        return Err(render_scan_errors(&submission, scanner.errors()));
    }
    *symbols = scanner.into_symbols();
    let program = Parser::new(tokens)
        .parse()
        .map_err(|error| error.render(&submission, "<plot>"))?;
    if program.is_empty() {
        return Err("nothing to plot: the expression is empty".to_owned());
    }
    let expression = program.last().and_then(|statement| match &statement.kind {
        StatementKind::Expr(expression) => Some(expression.clone()),
        _ => None,
    });
    Ok((program, expression, submission))
}

fn require_expression(
    parsed: (Vec<crate::ast::Statement>, Option<Expr>, String),
) -> Result<(Vec<crate::ast::Statement>, Expr, String), String> {
    let (program, expression, submission) = parsed;
    let expression = expression.ok_or_else(|| {
        "plot the value of an expression or one-variable function, for example f(x)".to_owned()
    })?;
    Ok((program, expression, submission))
}

enum ProbeResult {
    Numeric,
    /// A symbolic result with no free names, such as `pi` or `sqrt(2)`.
    SymbolicConstant,
    Unbound(Vec<String>),
    NonNumeric(&'static str),
}

fn collect_unbound(
    expression: &SymbolicExpr,
    symbols: &SymbolTable,
    variable: &str,
) -> Vec<String> {
    let mut names = Vec::new();
    let mut stack = vec![expression];
    while let Some(node) = stack.pop() {
        match node {
            SymbolicExpr::Variable(symbol) => {
                let name = symbols.resolve(*symbol).to_owned();
                if name != variable && !names.contains(&name) {
                    names.push(name);
                }
            }
            SymbolicExpr::Unary { expr, .. } => stack.push(expr),
            SymbolicExpr::Call { args, .. } => {
                stack.extend(args.iter().map(|arg| arg.as_ref()));
            }
            SymbolicExpr::Binary { left, right, .. } => {
                stack.push(left);
                stack.push(right);
            }
            _ => {}
        }
    }
    names
}

fn probe_value(value: &Value, symbols: &SymbolTable, variable: &str) -> ProbeResult {
    match value {
        Value::Number(_) => ProbeResult::Numeric,
        Value::Symbolic(expression) => {
            let unbound = collect_unbound(expression, symbols, variable);
            if unbound.is_empty() {
                ProbeResult::SymbolicConstant
            } else {
                ProbeResult::Unbound(unbound)
            }
        }
        Value::Derivation { result, .. } => probe_value(result, symbols, variable),
        other => ProbeResult::NonNumeric(other.type_name()),
    }
}

/// Evaluates a parsed expression as a finite real number.
pub fn evaluate_expression_scalar<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    expression: &Expr,
) -> Result<f64, String> {
    let evalf_symbol = symbols.intern("evalf");
    interpreter.set_symbols(symbols.clone());
    let numeric_expression = Expr::new(
        expression.span,
        ExprKind::Call {
            callee: Box::new(Expr::new(expression.span, ExprKind::Variable(evalf_symbol))),
            args: vec![expression.clone()],
        },
    );
    let value = interpreter
        .evaluate_expression(&numeric_expression)
        .map_err(|error| error.message)?;
    value_to_f64(&value)
        .and_then(finite)
        .ok_or_else(|| "plot range must evaluate to a finite real number".to_owned())
}

/// Evaluates one SCAS source as a plain real number (used for plot bounds).
pub fn evaluate_scalar<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    source: &str,
) -> Result<f64, String> {
    let (_, expression, submission) = require_expression(parse_submission(symbols, source)?)?;
    interpreter.set_symbols(symbols.clone());
    let value = interpreter
        .evaluate_expression(&expression)
        .map_err(|error| error.render(&submission, "<plot>"))?;
    value_to_f64(&value).and_then(finite).ok_or_else(|| {
        format!(
            "plot range `{}` must evaluate to a real number",
            source.trim()
        )
    })
}

/// Samples an already-parsed expression over a domain. This is used by the
/// document renderer to plot exactly the expression in the user's AST without
/// formatting and reparsing source text.
#[allow(clippy::too_many_arguments)]
pub fn sample_expression<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    expression: &Expr,
    label: String,
    variable: &str,
    x_min: f64,
    x_max: f64,
    count: usize,
) -> Result<SampledSeries, String> {
    let count = count.clamp(2, 4_000);
    let span = x_max - x_min;
    if !span.is_finite() || span <= 0.0 {
        return Err("plot range must satisfy from A < to B".to_owned());
    }
    interpreter.set_symbols(symbols.clone());
    // Fast path: compiled f64 sampling with adaptive refinement. The exact
    // evaluator below remains the fallback for anything the compiler skips.
    if let Some(compiled) = compile_numeric(expression, symbols, variable, interpreter) {
        let samples = sample_numeric_adaptive(&compiled, x_min, x_max, count);
        if samples.iter().all(|(_, y)| y.is_none()) {
            return Err(format!(
                "no real values of the expression between x = {} and x = {}",
                format_number(x_min),
                format_number(x_max)
            ));
        }
        return Ok(SampledSeries {
            label,
            variable: variable.to_owned(),
            samples,
        });
    }
    let evalf_symbol = symbols.intern("evalf");
    let wrapped = Expr::new(
        expression.span,
        ExprKind::Call {
            callee: Box::new(Expr::new(expression.span, ExprKind::Variable(evalf_symbol))),
            args: vec![expression.clone()],
        },
    );
    let saved_precision = interpreter.decimal_precision();
    let _ = interpreter.set_decimal_precision(NonZeroU64::new(SAMPLE_PRECISION).expect("12 != 0"));
    let previous = interpreter.bind_global(
        variable,
        Value::Number(
            number_from_f64(x_min + span / 3.0)
                .unwrap_or_else(|| Number::Rational(BigRational::zero())),
        ),
    );
    let result = (|| {
        let mut samples = Vec::with_capacity(count);
        for index in 0..count {
            // Pin the endpoints exactly: `x_min + span` can overshoot `x_max` by
            // one ulp, which used to drop the last sample of the domain.
            let x = if index == 0 {
                x_min
            } else if index + 1 == count {
                x_max
            } else {
                x_min + span * index as f64 / (count - 1) as f64
            };
            let Some(number) = number_from_f64(x) else {
                samples.push((x, None));
                continue;
            };
            interpreter.bind_global(variable, Value::Number(number));
            match interpreter.evaluate_expression(&wrapped) {
                Ok(value) => samples.push((x, value_to_f64(&value).and_then(finite))),
                Err(_) => samples.push((x, None)),
            }
        }
        if samples.iter().all(|(_, y)| y.is_none()) {
            return Err(format!(
                "no real values of the expression between x = {} and x = {}",
                format_number(x_min),
                format_number(x_max)
            ));
        }
        Ok(SampledSeries {
            label,
            variable: variable.to_owned(),
            samples,
        })
    })();
    interpreter.restore_global(variable, previous);
    if let Some(precision) = NonZeroU64::new(saved_precision) {
        let _ = interpreter.set_decimal_precision(precision);
    }
    result
}

/// Samples `source` over `[x_min, x_max]` using the live interpreter. The
/// source may be an expression in `x` or a one-variable function definition
/// such as `g(t) = sin(t)`, which is defined in the session and then plotted
/// over its parameter. Any prefix statements run first.
pub fn sample_source<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    source: &str,
    x_min: f64,
    x_max: f64,
    count: usize,
) -> Result<SampledSeries, String> {
    sample_source_with_variable(interpreter, symbols, source, "x", x_min, x_max, count)
}

/// Like `sample_source`, but polls `should_cancel` between sample points so a
/// GUI can abandon an out-of-date run mid-flight. Returns `None` when the run
/// was cancelled and `Some(result)` when it ran to completion.
pub fn sample_source_cancellable<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    source: &str,
    x_min: f64,
    x_max: f64,
    count: usize,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Option<Result<SampledSeries, String>> {
    match sample_source_with_variable_cancellable(
        interpreter,
        symbols,
        source,
        "x",
        x_min,
        x_max,
        count,
        should_cancel,
    ) {
        Ok(series) => Some(Ok(series)),
        Err(SampleError::Message(message)) => Some(Err(message)),
        Err(SampleError::Cancelled) => None,
    }
}

/// Internal error channel: user-facing messages versus cooperative
/// cancellation of a sampling run.
enum SampleError {
    Message(String),
    Cancelled,
}

impl From<String> for SampleError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

/// Samples an expression over a caller-selected independent variable. A
/// trailing one-variable function definition keeps using its own parameter.
pub fn sample_source_with_variable<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    source: &str,
    independent_variable: &str,
    x_min: f64,
    x_max: f64,
    count: usize,
) -> Result<SampledSeries, String> {
    match sample_source_with_variable_cancellable(
        interpreter,
        symbols,
        source,
        independent_variable,
        x_min,
        x_max,
        count,
        &mut || false,
    ) {
        Ok(series) => Ok(series),
        Err(SampleError::Message(message)) => Err(message),
        Err(SampleError::Cancelled) => unreachable!("a run that never cancels cannot be cancelled"),
    }
}

/// Core of `sample_source_with_variable`; polls `should_cancel` between
/// sample points so long sampling runs stay interruptible for live GUIs.
#[allow(clippy::too_many_arguments)]
fn sample_source_with_variable_cancellable<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    source: &str,
    independent_variable: &str,
    x_min: f64,
    x_max: f64,
    count: usize,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<SampledSeries, SampleError> {
    let count = count.clamp(2, 4_000);
    let span = x_max - x_min;
    if !span.is_finite() || span <= 0.0 {
        return Err(SampleError::Message(
            "plot range must satisfy from A < to B".to_owned(),
        ));
    }
    let (program, parsed_expression, submission) = parse_submission(symbols, source)?;
    interpreter.set_symbols(symbols.clone());

    // A trailing one-variable function definition is executed and then
    // plotted over its own parameter name.
    let mut variable = independent_variable.to_owned();
    let last = program.len() - 1;
    let (mut expression, mut expression_source) =
        if let StatementKind::Function { name, params, .. } = &program[last].kind {
            if params.len() != 1 {
                return Err(SampleError::Message(
                    "plots need a function of exactly one variable".to_owned(),
                ));
            }
            let function_name = symbols.resolve(*name).to_owned();
            symbols.resolve(params[0]).clone_into(&mut variable);
            for statement in &program {
                interpreter
                    .execute_statement(statement)
                    .map_err(|error| error.render(&submission, "<plot>"))?;
            }
            let call_source = format!("{function_name}({variable})");
            let (_, call_expression, _call_submission) =
                require_expression(parse_submission(symbols, &call_source)?)?;
            (call_expression, call_source)
        } else {
            for statement in &program[..last] {
                interpreter
                    .execute_statement(statement)
                    .map_err(|error| error.render(&submission, "<plot>"))?;
            }
            let expression = parsed_expression.ok_or_else(|| {
                "plot the value of an expression or one-variable function, for example f(x)"
                    .to_owned()
            })?;
            (expression, submission.clone())
        };

    let label = source.trim().to_owned();
    let saved_precision = interpreter.decimal_precision();
    let _ = interpreter.set_decimal_precision(NonZeroU64::new(SAMPLE_PRECISION).expect("12 != 0"));
    let previous = interpreter.bind_global(
        &variable,
        Value::Number(
            number_from_f64(x_min + span * 0.5)
                .unwrap_or_else(|| Number::Rational(BigRational::zero())),
        ),
    );
    let result = sample_bound_expression(
        interpreter,
        symbols,
        &mut expression,
        &mut expression_source,
        &variable,
        x_min,
        x_max,
        count,
        &submission,
        should_cancel,
    );
    interpreter.restore_global(&variable, previous);
    if let Some(precision) = NonZeroU64::new(saved_precision) {
        let _ = interpreter.set_decimal_precision(precision);
    }
    result.map(|samples| SampledSeries {
        label,
        variable,
        samples,
    })
}

#[allow(clippy::too_many_arguments)]
fn sample_bound_expression<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    expression: &mut Expr,
    expression_source: &mut String,
    variable: &str,
    x_min: f64,
    x_max: f64,
    count: usize,
    submission: &str,
    should_cancel: &mut dyn FnMut() -> bool,
) -> Result<Vec<(f64, Option<f64>)>, SampleError> {
    let span = x_max - x_min;

    // Probe the raw expression away from the edges and the middle before
    // wrapping it: a zero at the midpoint can hide unbound names (`a*x`
    // folds to 0 there), while edges may sit on a domain gap. One third of
    // the way in avoids both for common functions. Probing the raw form
    // keeps symbolic results classifiable, which evalf would reject.
    let mut first_error: Option<String> = None;
    let probe_x = x_min + span / 3.0;
    if let Some(number) = number_from_f64(probe_x) {
        let saved = interpreter.bind_global(variable, Value::Number(number));
        let probe = interpreter.evaluate_expression(expression);
        match &probe {
            Ok(value) => match probe_value(value, symbols, variable) {
                ProbeResult::Numeric | ProbeResult::SymbolicConstant => {}
                ProbeResult::Unbound(names) => {
                    interpreter.restore_global(variable, saved);
                    return Err(SampleError::Message(unbound_message(&names)));
                }
                ProbeResult::NonNumeric(kind) => {
                    interpreter.restore_global(variable, saved);
                    return Err(SampleError::Message(non_numeric_message(kind)));
                }
            },
            Err(error) => first_error = Some(error.render(submission, "<plot>")),
        }
        interpreter.restore_global(variable, saved);
    }

    // Fast path: compiled f64 sampling with adaptive refinement. The exact
    // evaluator below remains the fallback for anything the compiler skips
    // (user functions, matrices, comparisons, ...).
    if let Some(compiled) = compile_numeric(expression, symbols, variable, interpreter) {
        let samples = sample_numeric_adaptive(&compiled, x_min, x_max, count);
        if samples.iter().all(|(_, y)| y.is_none()) {
            return Err(SampleError::Message(match first_error {
                Some(error) => error,
                None => format!(
                    "no real values of the expression between x = {} and x = {}",
                    format_number(x_min),
                    format_number(x_max)
                ),
            }));
        }
        return Ok(samples);
    }

    // Numeric evaluation must go through evalf: SCAS keeps exact irrational
    // results symbolic (`sqrt(2)` stays `sqrt(2)`), so a bare `sqrt(x)` would
    // sample to symbolic values that cannot be drawn.
    let base = expression_source.trim().trim_end_matches(';');
    let wrapped = format!("evalf({base})");
    let (_, wrapped_expression, wrapped_submission) =
        require_expression(parse_submission(symbols, &wrapped)?)?;
    interpreter.set_symbols(symbols.clone());
    *expression = wrapped_expression;
    *expression_source = wrapped;
    let submission = wrapped_submission.as_str();

    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        // Checked up front each chunk so a GUI can abandon an out-of-date
        // run (newer view or changed session) without waiting for the rest.
        if index % 16 == 0 && should_cancel() {
            return Err(SampleError::Cancelled);
        }
        // Pin the endpoints exactly: `x_min + span` can overshoot `x_max` by
        // one ulp, which used to drop the last sample of the domain.
        let x = if index == 0 {
            x_min
        } else if index + 1 == count {
            x_max
        } else {
            x_min + span * index as f64 / (count - 1) as f64
        };
        let Some(number) = number_from_f64(x) else {
            samples.push((x, None));
            continue;
        };
        interpreter.bind_global(variable, Value::Number(number));
        match interpreter.evaluate_expression(expression) {
            Ok(value) => samples.push((x, value_to_f64(&value).and_then(finite))),
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error.render(submission, "<plot>"));
                }
                samples.push((x, None));
            }
        }
    }

    if samples.iter().all(|(_, y)| y.is_none()) {
        return Err(SampleError::Message(match first_error {
            Some(error) => error,
            None => format!(
                "no real values of the expression between x = {} and x = {}",
                format_number(x_min),
                format_number(x_max)
            ),
        }));
    }
    Ok(samples)
}

fn unbound_message(names: &[String]) -> String {
    format!(
        "cannot plot: unbound name(s) {}. define them first, for example `let {} = 2;`",
        names.join(", "),
        names[0]
    )
}

fn non_numeric_message(kind: &str) -> String {
    format!(
        "cannot plot: the expression evaluates to a {kind} — plot a real-valued expression like f(x) or sin(x)"
    )
}

/// Chooses a y window from the sampled data, trimming outlier tails so
/// asymptotes do not flatten the interesting part of the curve.
pub fn auto_y_bounds(series: &[SampledSeries], x_min: f64, x_max: f64) -> PlotBounds {
    let mut values = series
        .iter()
        .flat_map(|entry| entry.samples.iter())
        .filter(|(x, _)| *x >= x_min && *x <= x_max)
        .filter_map(|(_, y)| *y)
        .collect::<Vec<_>>();
    if values.is_empty() {
        return PlotBounds::new(x_min, x_max, -1.0, 1.0);
    }
    values.sort_by(f64::total_cmp);
    let trim = if values.len() >= 20 {
        values.len() / 20
    } else {
        0
    };
    let mut low = values[trim].clamp(-1.0e6, 1.0e6);
    let mut high = values[values.len() - 1 - trim].clamp(-1.0e6, 1.0e6);
    if !low.is_finite() {
        low = -1.0;
    }
    if !high.is_finite() {
        high = 1.0;
    }
    if high - low < 1.0e-12 {
        let mid = (high + low) * 0.5;
        let spread = if mid.abs() > 1.0e-12 {
            mid.abs() * 0.5
        } else {
            1.0
        };
        low = mid - spread;
        high = mid + spread;
    }
    let pad = (high - low) * 0.08;
    PlotBounds::new(x_min, x_max, low - pad, high + pad)
}

/// The kind of a notable point found while scanning a sampled curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CriticalKind {
    /// Sign change between consecutive finite samples.
    Zero,
    /// A single undefined sample whose finite neighbours approach the same value.
    Hole,
    /// Highest finite sample among its neighbours.
    Maximum,
    /// Lowest finite sample among its neighbours.
    Minimum,
}

/// A notable point on a sampled curve: a zero crossing or local extremum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CriticalPoint {
    pub x: f64,
    pub y: f64,
    pub kind: CriticalKind,
}

/// Scans sampled points for zeros (sign changes) and local extrema.
///
/// Plateaus are resolved greedily: an extremum is reported once at the
/// centre of a run of equal values, so `floor`-shaped data does not emit
/// a dot per sample. Zeros interpolate linearly between the sign change.
pub fn detect_critical_points(samples: &[(f64, Option<f64>)]) -> Vec<CriticalPoint> {
    // Keep original indices so gap-separated points are never treated as
    // neighbours; a domain gap must not bridge into a false zero.
    let finite: Vec<(usize, f64, f64)> = samples
        .iter()
        .enumerate()
        .filter_map(|(index, (x, y))| y.filter(|y| y.is_finite()).map(|y| (index, *x, y)))
        .collect();
    let mut points = Vec::new();
    // A removable discontinuity appears as one undefined sample whose
    // one-sided linear estimates agree. Requiring two finite samples on each
    // side avoids misclassifying domain boundaries and vertical asymptotes.
    for index in 2..samples.len().saturating_sub(2) {
        let (hole_x, hole_y) = samples[index];
        if hole_y.is_some() {
            continue;
        }
        let [(x0, Some(y0)), (x1, Some(y1)), (x2, None), (x3, Some(y3)), (x4, Some(y4))] = [
            samples[index - 2],
            samples[index - 1],
            samples[index],
            samples[index + 1],
            samples[index + 2],
        ] else {
            continue;
        };
        if x2 != hole_x || x1 == x0 || x4 == x3 {
            continue;
        }
        let left = y1 + (hole_x - x1) * (y1 - y0) / (x1 - x0);
        let right = y3 + (hole_x - x3) * (y4 - y3) / (x4 - x3);
        let scale = left.abs().max(right.abs()).max(1.0);
        if left.is_finite() && right.is_finite() && (left - right).abs() <= scale * 0.08 {
            points.push(CriticalPoint {
                x: hole_x,
                y: (left + right) * 0.5,
                kind: CriticalKind::Hole,
            });
        }
    }
    for window in finite.windows(2) {
        let (index0, x0, y0) = window[0];
        let (index1, x1, y1) = window[1];
        if index1 == index0 + 1 {
            if (y0 < 0.0 && y1 > 0.0) || (y0 > 0.0 && y1 < 0.0) {
                let x = x0 + (x1 - x0) * (-y0 / (y1 - y0));
                points.push(CriticalPoint {
                    x,
                    y: 0.0,
                    kind: CriticalKind::Zero,
                });
            } else if y0 == 0.0 {
                points.push(CriticalPoint {
                    x: x0,
                    y: 0.0,
                    kind: CriticalKind::Zero,
                });
            }
        }
    }
    let mut position = 1;
    while position + 1 < finite.len() {
        let (previous_index, _x0, y0) = finite[position - 1];
        let (current_index, x1, y1) = finite[position];
        let mut end = position;
        while end + 1 < finite.len() && finite[end + 1].2 == y1 {
            end += 1;
        }
        let (next_index, _x2, y2) = finite[end + 1];
        let adjacent = previous_index + 1 == current_index && finite[end].0 + 1 == next_index;
        let (centre_index, centre_x) = if end == position {
            (position, x1)
        } else {
            ((position + end) / 2, (x1 + finite[end].1) * 0.5)
        };
        if adjacent && y1 < y0 && y1 < y2 {
            points.push(CriticalPoint {
                x: centre_x,
                y: finite[centre_index].2,
                kind: CriticalKind::Minimum,
            });
        } else if adjacent && y1 > y0 && y1 > y2 {
            points.push(CriticalPoint {
                x: centre_x,
                y: finite[centre_index].2,
                kind: CriticalKind::Maximum,
            });
        }
        position = end + 1;
    }
    points
}

/// Parses and plots a whole `:plot` command line, optionally writing the
/// result to a file. Relative redirect paths resolve under `base_directory`.
pub fn execute_plot_command<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    rest: &str,
    ascii_width: usize,
    ascii_height: usize,
    base_directory: &Path,
) -> Result<PlotOutcome, String> {
    execute_plot_command_with_range(
        interpreter,
        symbols,
        rest,
        ascii_width,
        ascii_height,
        base_directory,
        None,
    )
}

/// Plot the same command while sampling an explicit horizontal viewport.
/// Interactive canvases use this during pan and zoom, without rewriting the
/// user's expression or its optional source range.
pub fn execute_plot_command_with_range<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    rest: &str,
    ascii_width: usize,
    ascii_height: usize,
    base_directory: &Path,
    range_override: Option<(f64, f64)>,
) -> Result<PlotOutcome, String> {
    execute_plot_command_impl(
        interpreter,
        symbols,
        rest,
        ascii_width,
        ascii_height,
        base_directory,
        range_override,
        true,
    )
    .map(|(outcome, _)| outcome)
}

/// Samples a REPL plot without touching the host filesystem. The caller can
/// offer a browser download when the command includes a redirect target.
pub fn execute_plot_command_in_memory<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    rest: &str,
    ascii_width: usize,
    ascii_height: usize,
    range_override: Option<(f64, f64)>,
) -> Result<(PlotOutcome, Option<String>), String> {
    execute_plot_command_impl(
        interpreter,
        symbols,
        rest,
        ascii_width,
        ascii_height,
        Path::new(""),
        range_override,
        false,
    )
}

fn execute_plot_command_impl<W: Write>(
    interpreter: &mut Interpreter<W>,
    symbols: &mut SymbolTable,
    rest: &str,
    ascii_width: usize,
    ascii_height: usize,
    base_directory: &Path,
    range_override: Option<(f64, f64)>,
    write_file: bool,
) -> Result<(PlotOutcome, Option<String>), String> {
    let (svg_mode, terminal_mode, rest) = if rest == "svg" {
        (true, false, "")
    } else if let Some(expression) = rest.strip_prefix("svg ") {
        (true, false, expression.trim_start())
    } else if rest == "terminal" {
        (false, true, "")
    } else if let Some(expression) = rest.strip_prefix("terminal ") {
        (false, true, expression.trim_start())
    } else {
        (false, false, rest)
    };
    let command = parse_plot_command(rest)?;
    if !write_file && command.series.len() > 8 {
        return Err("Browser plots support at most 8 curves at a time".to_owned());
    }
    let (x_min, x_max) = match range_override {
        Some((low, high)) if low.is_finite() && high.is_finite() && low < high => (low, high),
        Some(_) => return Err("plot view requires finite ascending bounds".to_owned()),
        None => match (&command.from, &command.to) {
            (Some(from), Some(to)) => {
                let low = evaluate_scalar(interpreter, symbols, from)?;
                let high = evaluate_scalar(interpreter, symbols, to)?;
                if low.partial_cmp(&high) != Some(std::cmp::Ordering::Less) {
                    return Err(format!(
                        "plot range must satisfy from {} < to {}",
                        format_number(low),
                        format_number(high)
                    ));
                }
                (low, high)
            }
            (None, None) => (DEFAULT_X_MIN, DEFAULT_X_MAX),
            _ => return Err("plot ranges need both bounds: from A to B".to_owned()),
        },
    };

    let mut series = Vec::with_capacity(command.series.len());
    for source in &command.series {
        series.push(sample_source(
            interpreter,
            symbols,
            source,
            x_min,
            x_max,
            COMMAND_SAMPLE_COUNT,
        )?);
    }

    let bounds = auto_y_bounds(&series, x_min, x_max);
    let ascii = render_ascii(&series, &bounds, ascii_width, ascii_height);
    let wants_svg_file = command.redirect.as_ref().is_some_and(|target| {
        Path::new(target)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    });
    let svg = (svg_mode || (!terminal_mode && wants_svg_file))
        .then(|| render_svg(&series, &bounds, SVG_WIDTH, SVG_HEIGHT));
    let mut wrote_file = None;
    if write_file {
        if let Some(target) = &command.redirect {
            let path = resolve_path(base_directory, target);
            let content = if let Some(svg) = &svg {
                svg.clone()
            } else {
                format!("{ascii}\n")
            };
            fs::write(&path, content)
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
            wrote_file = Some(path);
        }
    }

    Ok((
        PlotOutcome {
            series,
            bounds,
            ascii,
            svg: if svg_mode { svg } else { None },
            wrote_file,
        },
        command.redirect,
    ))
}

fn resolve_path(base_directory: &Path, target: &str) -> PathBuf {
    let path = PathBuf::from(target);
    if path.is_absolute() {
        path
    } else {
        base_directory.join(path)
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Chooses a rounded tick step of roughly `target_lines` divisions across
/// `span`, used by both the ASCII and SVG renderers.
pub fn nice_step(span: f64, target_lines: f64) -> f64 {
    if span <= 0.0 || !span.is_finite() {
        return 1.0;
    }
    let raw = span / target_lines.max(1.0);
    let magnitude = 10f64.powf(raw.abs().log10().floor());
    let normalized = raw / magnitude;
    let step = if normalized < 1.5 {
        1.0
    } else if normalized < 3.5 {
        2.0
    } else if normalized < 7.5 {
        5.0
    } else {
        10.0
    };
    step * magnitude
}

/// Formats a value compactly for axis labels.
pub fn format_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return if value.is_nan() { "nan" } else { "inf" }.to_owned();
    }
    let magnitude = value.abs();
    if !(1.0e-3..1.0e5).contains(&magnitude) {
        return format!("{value:.2e}");
    }
    let decimals = if magnitude >= 100.0 {
        1
    } else if magnitude >= 1.0 {
        2
    } else {
        4
    };
    let text = format!("{value:.decimals$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.to_owned()
}

fn format_tick(value: f64, step: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let decimals = (-step.abs().log10().floor() + 0.5).clamp(0.0, 6.0) as usize;
    let text = format!("{value:.decimals$}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Draws a TI-style character-grid plot with axes and edge labels.
pub fn render_ascii(
    series: &[SampledSeries],
    bounds: &PlotBounds,
    width: usize,
    height: usize,
) -> String {
    let width = width.max(8);
    let height = height.max(4);
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if x_span <= 0.0 || y_span <= 0.0 {
        return "invalid plot window".to_owned();
    }

    let x_to_col = |x: f64| -> usize {
        (((x - bounds.x_min) / x_span) * (width - 1) as f64)
            .round()
            .clamp(0.0, (width - 1) as f64) as usize
    };
    let y_to_row = |y: f64| -> isize {
        (((bounds.y_max - y) / y_span) * (height - 1) as f64)
            .round()
            .clamp(0.0, (height - 1) as f64) as isize
    };

    let mut grid = vec![vec![' '; width]; height];

    // Axes first, so curves drawn afterwards take priority where they cross.
    let axis_col = (bounds.x_min <= 0.0 && bounds.x_max >= 0.0).then(|| x_to_col(0.0));
    let axis_row = (bounds.y_min <= 0.0 && bounds.y_max >= 0.0).then(|| y_to_row(0.0));
    if let Some(column) = axis_col {
        for line in grid.iter_mut().take(height) {
            line[column] = '|';
        }
    }
    if let Some(row) = axis_row {
        let row = row as usize;
        for column in 0..width {
            grid[row][column] = '-';
        }
        if let Some(column) = axis_col {
            grid[row][column] = '+';
        }
    }

    for (index, entry) in series.iter().enumerate() {
        let glyph = ASCII_GLYPHS[index % ASCII_GLYPHS.len()];
        let mut previous: Option<(usize, isize)> = None;
        for (x, y) in &entry.samples {
            let Some(y) = y else {
                previous = None;
                continue;
            };
            let column = x_to_col(*x);
            let row = y_to_row(*y);
            if let Some((previous_column, previous_row)) = previous {
                let columns = column.abs_diff(previous_column);
                if columns == 0 {
                    let (low, high) = if previous_row <= row {
                        (previous_row, row)
                    } else {
                        (row, previous_row)
                    };
                    for r in low..=high {
                        plot_cell(&mut grid, r, column, glyph);
                    }
                } else {
                    for step in 0..=columns {
                        let target_column = if column >= previous_column {
                            previous_column + step
                        } else {
                            previous_column - step
                        };
                        let interpolated = previous_row as f64
                            + (row - previous_row) as f64 * step as f64 / columns as f64;
                        plot_cell(
                            &mut grid,
                            interpolated.round() as isize,
                            target_column,
                            glyph,
                        );
                    }
                }
            } else {
                plot_cell(&mut grid, row, column, glyph);
            }
            previous = Some((column, row));
        }
    }

    // Gutter labels for the window edges and the y axis.
    let top_label = format_number(bounds.y_max);
    let bottom_label = format_number(bounds.y_min);
    let zero_label = axis_row.map(|_| "0".to_owned()).unwrap_or_default();
    let gutter = top_label
        .len()
        .max(bottom_label.len())
        .max(zero_label.len())
        .max(1)
        + 1;

    let mut lines = Vec::with_capacity(height + 1);
    for (row, line) in grid.iter().enumerate().take(height) {
        let label = if row == 0 {
            top_label.clone()
        } else if Some(row as isize) == axis_row {
            zero_label.clone()
        } else if row + 1 == height {
            bottom_label.clone()
        } else {
            String::new()
        };
        lines.push(format!(
            "{label:>gutter$} {}",
            line.iter().collect::<String>()
        ));
    }

    // x labels along the bottom edge.
    let mut footer = vec![' '; gutter + 1 + width];
    let place = |footer: &mut [char], text: &str, start: usize, end: usize| {
        let characters = text.chars().collect::<Vec<_>>();
        let span = end.saturating_sub(start);
        let offset = start + span.saturating_sub(characters.len()) / 2;
        for (index, character) in characters.into_iter().enumerate() {
            let index = offset + index;
            if index < footer.len() {
                footer[index] = character;
            }
        }
    };
    place(
        &mut footer,
        &format_number(bounds.x_min),
        gutter + 1,
        gutter + 1 + width / 3,
    );
    if let Some(column) = axis_col {
        place(
            &mut footer,
            "0",
            gutter + 1 + column.saturating_sub(2),
            gutter + 1 + column + 3,
        );
    }
    place(
        &mut footer,
        &format_number(bounds.x_max),
        gutter + 1 + width * 2 / 3,
        gutter + 1 + width,
    );
    lines.push(footer.iter().collect::<String>().trim_end().to_owned());
    lines.join("\n")
}

fn plot_cell(grid: &mut [Vec<char>], row: isize, column: usize, glyph: char) {
    if row >= 0 {
        let row = row as usize;
        if let Some(line) = grid.get_mut(row) {
            if let Some(cell) = line.get_mut(column) {
                *cell = glyph;
            }
        }
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Generates a self-contained LaTeX `picture` environment from sampled
/// curves. It uses only core LaTeX drawing primitives and therefore renders
/// in the same Computer Modern document pipeline without external graphics
/// packages or raster images.
///
/// The picture fills the current `\linewidth` exactly: `\unitlength` is set
/// to `\linewidth / width`, so the plot scales to whatever document column
/// contains it. A fixed-point picture wider than the surrounding text is
/// clipped on the right by the preview/export bounding box, which used to
/// cut the last piece of the domain off documents with prose.
pub fn render_latex_picture(
    series: &[SampledSeries],
    bounds: &PlotBounds,
    width: f64,
    height: f64,
) -> String {
    let width = width.clamp(160.0, 640.0);
    let height = height.clamp(100.0, 420.0);
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if !x_span.is_finite() || !y_span.is_finite() || x_span <= 0.0 || y_span <= 0.0 {
        return r"\text{invalid plot window}".to_owned();
    }
    let unit_divisor = width.round().max(1.0) as u64;
    let margin_left = 34.0;
    let margin_bottom = 22.0;
    let plot_width = width - margin_left - 8.0;
    let plot_height = height - margin_bottom - 10.0;
    let sx =
        |x: f64| margin_left + ((x - bounds.x_min) / x_span * plot_width).clamp(0.0, plot_width);
    let sy = |y: f64| {
        margin_bottom + ((y - bounds.y_min) / y_span * plot_height).clamp(0.0, plot_height)
    };
    let mut latex = format!(
        r"\vcenter{{\hbox{{\begingroup\setlength{{\unitlength}}{{\dimexpr\linewidth/{unit_divisor}\relax}}\begin{{picture}}({width:.1},{height:.1})",
    );
    let x_step = nice_step(x_span, 8.0);
    let y_step = nice_step(y_span, 6.0);
    let mut x_tick = (bounds.x_min / x_step).ceil() * x_step;
    while x_tick <= bounds.x_max + x_step * 0.001 {
        let x = sx(x_tick);
        latex.push_str(&format!(
            r"\put({x:.2},{margin_bottom:.2}){{\line(0,1){{{plot_height:.2}}}}}",
        ));
        latex.push_str(&format!(
            r"\put({x:.2},4){{\makebox(0,0)[t]{{\scriptsize ${}$}}}}",
            format_number(x_tick)
        ));
        x_tick += x_step;
    }
    let mut y_tick = (bounds.y_min / y_step).ceil() * y_step;
    while y_tick <= bounds.y_max + y_step * 0.001 {
        let y = sy(y_tick);
        latex.push_str(&format!(
            r"\put({margin_left:.2},{y:.2}){{\line(1,0){{{plot_width:.2}}}}}",
        ));
        latex.push_str(&format!(
            r"\put({:.2},{y:.2}){{\makebox(0,0)[r]{{\scriptsize ${}$}}}}",
            margin_left - 3.0,
            format_number(y_tick)
        ));
        y_tick += y_step;
    }
    // Darken the coordinate axes when they are in view.
    if bounds.contains_x(0.0) {
        let x = sx(0.0);
        latex.push_str(&format!(
            r"\linethickness{{0.8pt}}\put({x:.2},{margin_bottom:.2}){{\line(0,1){{{plot_height:.2}}}}}\linethickness{{0.4pt}}",
        ));
    }
    if bounds.contains_y(0.0) {
        let y = sy(0.0);
        latex.push_str(&format!(
            r"\linethickness{{0.8pt}}\put({margin_left:.2},{y:.2}){{\line(1,0){{{plot_width:.2}}}}}\linethickness{{0.4pt}}",
        ));
    }
    const COLORS: &[&str] = &[
        "blue", "red", "green", "violet", "orange", "cyan", "magenta", "black",
    ];
    for (index, entry) in series.iter().enumerate() {
        latex.push_str(&format!(r"\color{{{}}}", COLORS[index % COLORS.len()]));
        let mut previous: Option<(f64, f64)> = None;
        for (x, y) in &entry.samples {
            let Some(y) = y.filter(|y| y.is_finite()) else {
                previous = None;
                continue;
            };
            if !bounds.contains_x(*x) {
                previous = None;
                continue;
            }
            if let Some((previous_x, previous_y)) = previous {
                if (y - previous_y).abs() <= y_span * 2.0 {
                    let x0 = sx(previous_x);
                    let y0 = sy(previous_y);
                    let x1 = sx(*x);
                    let y1 = sy(y);
                    let xm = (x0 + x1) * 0.5;
                    let ym = (y0 + y1) * 0.5;
                    latex.push_str(&format!(
                        r"\qbezier({x0:.2},{y0:.2})({xm:.2},{ym:.2})({x1:.2},{y1:.2})",
                    ));
                }
            }
            previous = Some((*x, y));
        }
        // Critical points are part of the document plot as well as the GUI.
        for point in detect_critical_points(&entry.samples) {
            if bounds.contains_x(point.x) && bounds.contains_y(point.y) {
                let marker = if point.kind == CriticalKind::Hole {
                    r"\circle{3}"
                } else {
                    r"\circle*{3}"
                };
                latex.push_str(&format!(
                    r"\put({:.2},{:.2}){{{marker}}}",
                    sx(point.x),
                    sy(point.y)
                ));
            }
        }
    }
    latex.push_str(r"\end{picture}\endgroup}}");
    latex
}

pub fn render_svg(
    series: &[SampledSeries],
    bounds: &PlotBounds,
    width: u32,
    height: u32,
) -> String {
    let width = width.max(160);
    let height = height.max(120);
    let margin_left = 52.0_f64;
    let margin_right = 14.0_f64;
    let margin_top = 14.0_f64;
    let margin_bottom = 30.0_f64;
    let plot_width = f64::from(width) - margin_left - margin_right;
    let plot_height = f64::from(height) - margin_top - margin_bottom;
    let x_span = bounds.x_span();
    let y_span = bounds.y_span();
    if x_span <= 0.0 || y_span <= 0.0 || plot_width <= 0.0 || plot_height <= 0.0 {
        return "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_owned();
    }

    let to_screen_x = |x: f64| margin_left + (x - bounds.x_min) / x_span * plot_width;
    let to_screen_y = |y: f64| margin_top + (bounds.y_max - y) / y_span * plot_height;

    let mut body = String::new();
    body.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" font-family=\"Helvetica, Arial, sans-serif\">\n"
    ));
    body.push_str("  <rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");

    let x_step = nice_step(x_span, 9.0);
    let y_step = nice_step(y_span, 7.0);

    // Gridlines and tick labels.
    let mut tick = (bounds.x_min / x_step).ceil() * x_step;
    while tick <= bounds.x_max + x_step * 0.001 {
        if tick.is_finite() {
            let screen = to_screen_x(tick);
            body.push_str(&format!(
                "  <line x1=\"{screen:.1}\" y1=\"{margin_top:.1}\" x2=\"{screen:.1}\" y2=\"{:.1}\" stroke=\"#e4e7eb\" stroke-width=\"1\"/>\n",
                margin_top + plot_height
            ));
            let label = format_tick(tick, x_step);
            body.push_str(&format!(
                "  <text x=\"{screen:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"#57606a\" text-anchor=\"middle\">{}</text>\n",
                margin_top + plot_height + 16.0,
                xml_escape(&label)
            ));
        }
        tick += x_step;
    }
    let mut tick = (bounds.y_min / y_step).ceil() * y_step;
    while tick <= bounds.y_max + y_step * 0.001 {
        if tick.is_finite() {
            let screen = to_screen_y(tick);
            body.push_str(&format!(
                "  <line x1=\"{margin_left:.1}\" y1=\"{screen:.1}\" x2=\"{:.1}\" y2=\"{screen:.1}\" stroke=\"#e4e7eb\" stroke-width=\"1\"/>\n",
                margin_left + plot_width
            ));
            let label = format_tick(tick, y_step);
            body.push_str(&format!(
                "  <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"#57606a\" text-anchor=\"end\">{}</text>\n",
                margin_left - 6.0,
                screen + 4.0,
                xml_escape(&label)
            ));
        }
        tick += y_step;
    }

    // Axes.
    if bounds.x_min <= 0.0 && bounds.x_max >= 0.0 {
        let screen = to_screen_x(0.0);
        body.push_str(&format!(
            "  <line x1=\"{screen:.1}\" y1=\"{margin_top:.1}\" x2=\"{screen:.1}\" y2=\"{:.1}\" stroke=\"#57606a\" stroke-width=\"1.5\"/>\n",
            margin_top + plot_height
        ));
    }
    if bounds.y_min <= 0.0 && bounds.y_max >= 0.0 {
        let screen = to_screen_y(0.0);
        body.push_str(&format!(
            "  <line x1=\"{margin_left:.1}\" y1=\"{screen:.1}\" x2=\"{:.1}\" y2=\"{screen:.1}\" stroke=\"#57606a\" stroke-width=\"1.5\"/>\n",
            margin_left + plot_width
        ));
    }

    // Curves, broken wherever the function is undefined or jumps.
    for (index, entry) in series.iter().enumerate() {
        let color = SERIES_COLORS[index % SERIES_COLORS.len()];
        let mut path = String::new();
        let mut pen_down = false;
        let mut previous_y: Option<f64> = None;
        for (x, y) in &entry.samples {
            let Some(y) = y else {
                pen_down = false;
                previous_y = None;
                continue;
            };
            let discontinuity =
                previous_y.is_some_and(|previous| (y - previous).abs() > y_span * 2.0);
            if pen_down && !discontinuity {
                path.push_str(&format!(" L {:.1},{:.1}", to_screen_x(*x), to_screen_y(*y)));
            } else {
                path.push_str(&format!(" M {:.1},{:.1}", to_screen_x(*x), to_screen_y(*y)));
            }
            pen_down = true;
            previous_y = Some(*y);
        }
        if !path.is_empty() {
            body.push_str(&format!(
                "  <path d=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>\n",
                path.trim_start()
            ));
        }
    }

    // Legend.
    for (index, entry) in series.iter().enumerate() {
        let color = SERIES_COLORS[index % SERIES_COLORS.len()];
        let y = 20.0 + 16.0 * index as f64;
        let label = xml_escape(&entry.label);
        body.push_str(&format!(
            "  <line x1=\"12\" y1=\"{y:.1}\" x2=\"30\" y2=\"{y:.1}\" stroke=\"{color}\" stroke-width=\"3\"/>\n"
        ));
        body.push_str(&format!(
            "  <text x=\"36\" y=\"{:.1}\" font-size=\"12\" fill=\"#24292f\">{}</text>\n",
            y + 4.0,
            label
        ));
    }

    body.push_str("</svg>\n");
    body
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_zoom_keeps_pointer_value_fixed_and_pan_is_reversible() {
        let initial = PlotBounds::new(-10.0, 10.0, -5.0, 5.0);
        let zoomed = initial.zoomed_at(0.5, 0.75, 0.25).unwrap();
        assert_eq!((zoomed.x_min, zoomed.x_max), (-2.5, 7.5));
        assert_eq!((zoomed.y_min, zoomed.y_max), (-1.25, 3.75));
        assert_eq!(zoomed.x_min + 0.75 * zoomed.x_span(), 5.0);
        assert_eq!(zoomed.y_max - 0.25 * zoomed.y_span(), 2.5);
        let panned = initial
            .panned_by_pixels(100.0, -50.0, 1000.0, 500.0)
            .unwrap();
        assert_eq!(panned, PlotBounds::new(-12.0, 8.0, -6.0, 4.0));
        assert!(initial.zoomed_at(0.0, 0.5, 0.5).is_none());
    }

    fn linear_interpreter() -> Interpreter<Vec<u8>> {
        Interpreter::new(SymbolTable::new(), Vec::new())
    }

    #[test]
    fn command_parsing_handles_ranges_and_redirects() {
        let command = parse_plot_command("f(x), g(x) from -5 to 5 > out.svg").unwrap();
        assert_eq!(command.series, vec!["f(x)", "g(x)"]);
        assert_eq!(command.from.as_deref(), Some("-5"));
        assert_eq!(command.to.as_deref(), Some("5"));
        assert_eq!(command.redirect.as_deref(), Some("out.svg"));

        let plain = parse_plot_command("sin(x)").unwrap();
        assert_eq!(plain.series, vec!["sin(x)"]);
        assert!(plain.from.is_none() && plain.to.is_none() && plain.redirect.is_none());

        // Comparisons are not redirects.
        let comparison = parse_plot_command("x > 3").unwrap();
        assert_eq!(comparison.series, vec!["x > 3"]);
        assert!(comparison.redirect.is_none());

        assert!(parse_plot_command("").is_err());
        assert!(parse_plot_command("f(x) from -5").is_err());
    }

    #[test]
    fn in_memory_plot_preserves_redirect_without_writing() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        let (outcome, requested_file) = execute_plot_command_in_memory(
            &mut interpreter,
            &mut symbols,
            "svg sin(x) from -3 to 3 > graph.svg",
            64,
            18,
            None,
        )
        .unwrap();
        assert_eq!(requested_file.as_deref(), Some("graph.svg"));
        assert!(outcome.wrote_file.is_none());
        assert_eq!(outcome.bounds.x_min, -3.0);
        assert_eq!(outcome.bounds.x_max, 3.0);
        assert!(outcome.svg.as_deref().unwrap().contains("<svg"));
    }

    #[test]
    fn samples_an_expression_and_reports_unbound_names() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        let series = sample_source(&mut interpreter, &mut symbols, "x^2", -2.0, 2.0, 9).unwrap();
        assert_eq!(series.variable, "x");
        assert!(series.samples.len() >= 9, "adaptive sampling adds points");
        let (middle_x, middle_y) = series
            .samples
            .iter()
            .copied()
            .min_by(|left, right| left.0.abs().total_cmp(&right.0.abs()))
            .unwrap();
        assert!((middle_x).abs() < 1.0e-12);
        assert!(middle_y.unwrap().abs() < 1.0e-9);

        let error = sample_source(&mut interpreter, &mut symbols, "a*x", -1.0, 1.0, 5).unwrap_err();
        assert!(error.contains("unbound name(s) a"), "{error}");
    }

    #[test]
    fn samples_a_definition_over_its_parameter() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        let series =
            sample_source(&mut interpreter, &mut symbols, "g(t) = 2t + 1", 0.0, 4.0, 5).unwrap();
        assert_eq!(series.variable, "t");
        assert!((series.samples[0].1.unwrap() - 1.0).abs() < 1.0e-9);
        assert!((series.samples[4].1.unwrap() - 9.0).abs() < 1.0e-9);
        // The definition stays available afterwards.
        assert!(interpreter.definition_names().contains(&"g"));
    }

    #[test]
    fn critical_points_catch_zeros_and_extrema() {
        // sin over two periods, sampled slightly past the end so the zero
        // at 4*pi is a sign change rather than a floating-point endpoint.
        let samples: Vec<(f64, Option<f64>)> = (0..=200)
            .map(|step| {
                let x = step as f64 / 200.0 * (4.0 * std::f64::consts::PI + 0.1);
                (x, Some(x.sin()))
            })
            .collect();
        let points = detect_critical_points(&samples);
        let zeros = points
            .iter()
            .filter(|point| point.kind == CriticalKind::Zero)
            .count();
        let maxima = points
            .iter()
            .filter(|point| point.kind == CriticalKind::Maximum)
            .count();
        let minima = points
            .iter()
            .filter(|point| point.kind == CriticalKind::Minimum)
            .count();
        assert_eq!(zeros, 5, "zeros at 0, pi, 2pi, 3pi, 4pi: {points:?}");
        assert_eq!(maxima, 2);
        assert_eq!(minima, 2);
        let pi = std::f64::consts::PI;
        let peak = points
            .iter()
            .find(|point| point.kind == CriticalKind::Maximum && (point.x - pi / 2.0).abs() < 0.2)
            .expect("a maximum near pi/2");
        assert!((peak.y - 1.0).abs() < 0.01);

        // A straight line has no extrema; gaps are skipped cleanly.
        let line: Vec<(f64, Option<f64>)> = (0..=10)
            .map(|step| (step as f64, Some(step as f64)))
            .collect();
        assert!(detect_critical_points(&line)
            .iter()
            .all(|point| point.kind == CriticalKind::Zero));
        let gappy = vec![(-1.0, Some(-1.0)), (0.0, None), (1.0, Some(1.0))];
        assert!(detect_critical_points(&gappy).is_empty());

        let removable_hole = vec![
            (-2.0, Some(0.0)),
            (-1.0, Some(1.0)),
            (0.0, None),
            (1.0, Some(3.0)),
            (2.0, Some(4.0)),
        ];
        let hole = detect_critical_points(&removable_hole)
            .into_iter()
            .find(|point| point.kind == CriticalKind::Hole)
            .expect("matching one-sided limits should produce a hole");
        assert!((hole.x - 0.0).abs() < 1.0e-9);
        assert!((hole.y - 2.0).abs() < 1.0e-9);

        let vertical_asymptote = vec![
            (-2.0, Some(-0.5)),
            (-1.0, Some(-1.0)),
            (0.0, None),
            (1.0, Some(1.0)),
            (2.0, Some(0.5)),
        ];
        assert!(detect_critical_points(&vertical_asymptote)
            .iter()
            .all(|point| point.kind != CriticalKind::Hole));
    }

    #[test]
    fn complex_values_become_gaps_and_empty_windows_error() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        let series =
            sample_source(&mut interpreter, &mut symbols, "sqrt(x)", -4.0, 4.0, 9).unwrap();
        assert!(series.samples.iter().all(|(x, y)| if *x < 0.0 {
            y.is_none()
        } else {
            y.is_some()
        }));

        let error =
            sample_source(&mut interpreter, &mut symbols, "sqrt(x)", -4.0, -1.0, 5).unwrap_err();
        assert!(error.contains("no real values"), "{error}");
    }

    #[test]
    fn session_bindings_survive_sampling() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        symbols.intern("x");
        interpreter.set_symbols(symbols.clone());
        interpreter.bind_global("x", Value::Number(Number::Integer(7.into())));
        let (_, Some(expression), _) = parse_submission(&mut symbols, "x + 1").unwrap() else {
            unreachable!()
        };
        interpreter.set_symbols(symbols.clone());
        let value = interpreter.evaluate_expression(&expression).unwrap();
        assert_eq!(value_to_f64(&value), Some(8.0));

        sample_source(&mut interpreter, &mut symbols, "x", -1.0, 1.0, 5).unwrap();
        let (_, Some(expression), _) = parse_submission(&mut symbols, "x").unwrap() else {
            unreachable!()
        };
        interpreter.set_symbols(symbols.clone());
        let value = interpreter.evaluate_expression(&expression).unwrap();
        assert_eq!(
            value_to_f64(&value),
            Some(7.0),
            "the session binding must be restored"
        );
    }

    #[test]
    fn auto_bounds_trim_asymptotes_and_pad() {
        let mut samples = Vec::new();
        for index in 0..101 {
            let x = -5.0 + 0.1 * index as f64;
            let y = if (x - 0.0).abs() < 1.0e-9 {
                None
            } else {
                Some(1.0 / x)
            };
            samples.push((x, y));
        }
        let series = SampledSeries {
            label: "1/x".to_owned(),
            variable: "x".to_owned(),
            samples,
        };
        let bounds = auto_y_bounds(&[series], -5.0, 5.0);
        // tan-like data must not stretch to ±1e6.
        assert!(bounds.y_max < 1.0e4, "{bounds:?}");
        assert!(bounds.y_min > -1.0e4, "{bounds:?}");
        assert!(bounds.y_min < 0.0 && bounds.y_max > 0.0);
    }

    #[test]
    fn ascii_render_places_axes_and_curve() {
        let series = SampledSeries {
            label: "x".to_owned(),
            variable: "x".to_owned(),
            samples: (0..=40)
                .map(|index| {
                    let x = -2.0 + 0.1 * index as f64;
                    (x, Some(x))
                })
                .collect(),
        };
        let bounds = PlotBounds::new(-2.0, 2.0, -2.0, 2.0);
        let art = render_ascii(&[series], &bounds, 41, 11);
        let lines = art.lines().collect::<Vec<_>>();
        assert!(lines.len() >= 12, "{art}");
        // A steep segment covers the origin, so assert the axes instead.
        assert!(art.contains('-') && art.contains('|'), "{art}");
        assert!(art.contains('*'), "{art}");
        assert!(art.contains("-2"), "{art}");
        assert!(art.contains('2'), "{art}");
    }

    #[test]
    fn svg_render_is_well_formed_and_escapes_labels() {
        let series = SampledSeries {
            label: "a<b & c".to_owned(),
            variable: "x".to_owned(),
            samples: (0..=20)
                .map(|index| {
                    let x = -1.0 + 0.1 * index as f64;
                    (x, Some(x * x))
                })
                .collect(),
        };
        let bounds = auto_y_bounds(&[series.clone()], -1.0, 1.0);
        let svg = render_svg(&[series], &bounds, 320, 240);
        assert!(svg.starts_with("<svg xmlns"));
        assert!(svg.ends_with("</svg>\n"));
        assert!(svg.contains("a&lt;b &amp; c"));
        assert_eq!(svg.matches("<path").count(), 1);
        assert!(svg.contains("stroke=\"#1f6feb\""));
    }

    #[test]
    fn execute_plot_command_writes_svg_files() {
        let directory = std::env::temp_dir().join(format!(
            "scas-plot-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("graph.svg");

        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        let outcome = execute_plot_command(
            &mut interpreter,
            &mut symbols,
            &format!("sin(x) from -6 to 6 > {}", target.display()),
            60,
            16,
            &directory,
        )
        .unwrap();
        assert_eq!(outcome.wrote_file.as_deref(), Some(target.as_path()));
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("<svg"));
        assert!(!outcome.ascii.is_empty());
        assert!((outcome.bounds.x_min + 6.0).abs() < 1.0e-9);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn compiled_fast_path_matches_the_exact_evaluator() {
        let expressions = [
            "x + 2*x - 1",
            "x^2 / (x + 3)",
            "sin(x)^2 + cos(x)^2",
            "sqrt(x^2 + 1)",
            "cbrt(x)",
            "exp(-x^2)",
            "ln(x^2 + 1)",
            "log(x + 4)",
            "log(x + 4, 2)",
            "log10(x + 4)",
            "log2(x + 4)",
            "abs(x) - floor(x)",
            "ceil(x/3) + trunc(x/3)",
            "atan2(x, 2)",
            "hypot(x, 3)",
            "min(x, 2*x)",
            "max(x^2, 1)",
            "tan(x)",
            "asin(x/2)",
            "acos(x/2)",
            "atan(x)",
            "sinh(x/2)",
            "cosh(x/2)",
            "tanh(x)",
            "asinh(x)",
            "acosh(x + 2)",
            "atanh(x/2)",
            "sign(x + 0.5)",
            "degrees(x + 1)",
            "radians(x + 1)",
            "pi*x + e",
            "tau*x + phi",
            "2^(x/3)",
            "(x + 1)^(-2)",
            "evalf(sin(x) + 0.5)",
        ];
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        for source in expressions {
            let (_, Some(expression), _) = parse_submission(&mut symbols, source).unwrap() else {
                unreachable!()
            };
            // Mirror the samplers: installing the grown table binds the
            // builtins/constants the compiler (and evaluator) resolve.
            interpreter.set_symbols(symbols.clone());
            let compiled = compile_numeric(&expression, &symbols, "x", &interpreter)
                .unwrap_or_else(|| panic!("`{source}` should compile on the fast path"));
            for x in [-2.5_f64, -0.75, 0.0, 0.4, 1.25] {
                let bound = number_from_f64(x).unwrap();
                interpreter.bind_global("x", Value::Number(bound));
                let exact = evaluate_expression_scalar(&mut interpreter, &mut symbols, &expression);
                let fast = compiled.evaluate(x);
                match exact {
                    Ok(exact) => {
                        let tolerance = 1.0e-9 * (1.0 + exact.abs());
                        assert!(
                            (fast - exact).abs() <= tolerance,
                            "`{source}` at x = {x}: fast {fast} vs exact {exact}"
                        );
                    }
                    // Outside the real domain both paths must give it up:
                    // the exact evaluator errors, f64 yields NaN, and both
                    // become a gap in the drawn curve.
                    Err(_) => assert!(
                        !fast.is_finite(),
                        "`{source}` at x = {x}: exact errored but fast gave {fast}"
                    ),
                }
            }
        }
    }

    #[test]
    fn fast_path_refuses_to_compile_unmodeled_expressions() {
        let interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        // Unbound names must fall back so the unbound-name error still fires.
        let (_, Some(expression), _) = parse_submission(&mut symbols, "a*x").unwrap() else {
            unreachable!()
        };
        assert!(compile_numeric(&expression, &symbols, "x", &interpreter).is_none());
        // Comparisons and data structures are outside the numeric subset.
        let (_, Some(expression), _) = parse_submission(&mut symbols, "x > 3").unwrap() else {
            unreachable!()
        };
        assert!(compile_numeric(&expression, &symbols, "x", &interpreter).is_none());
        let (_, Some(expression), _) = parse_submission(&mut symbols, "[x, 1]").unwrap() else {
            unreachable!()
        };
        assert!(compile_numeric(&expression, &symbols, "x", &interpreter).is_none());
    }

    #[test]
    fn user_functions_and_session_values_compile_through_their_scopes() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        // A user function with an expression body inlines onto the fast path.
        let (program, _, _) = parse_submission(&mut symbols, "g(t) = 2t + 1;").unwrap();
        interpreter.set_symbols(symbols.clone());
        for statement in &program {
            interpreter.execute_statement(statement).unwrap();
        }
        let (_, Some(expression), _) = parse_submission(&mut symbols, "g(x) * 3").unwrap() else {
            unreachable!()
        };
        let compiled = compile_numeric(&expression, &symbols, "x", &interpreter)
            .expect("expression-bodied functions should compile");
        assert!((compiled.evaluate(2.0) - 15.0).abs() < 1.0e-12);

        // Session values resolve exactly like the evaluator's globals.
        let (program, _, _) = parse_submission(&mut symbols, "let a = 1/3;").unwrap();
        interpreter.set_symbols(symbols.clone());
        for statement in &program {
            interpreter.execute_statement(statement).unwrap();
        }
        let (_, Some(expression), _) = parse_submission(&mut symbols, "a * x").unwrap() else {
            unreachable!()
        };
        let compiled = compile_numeric(&expression, &symbols, "x", &interpreter)
            .expect("session constants should compile");
        assert!((compiled.evaluate(3.0) - 1.0).abs() < 1.0e-12);

        // Block-bodied functions keep using the exact evaluator.
        let (program, _, _) = parse_submission(&mut symbols, "fn h(t) { return t + 1; }").unwrap();
        interpreter.set_symbols(symbols.clone());
        for statement in &program {
            interpreter.execute_statement(statement).unwrap();
        }
        let (_, Some(expression), _) = parse_submission(&mut symbols, "h(x)").unwrap() else {
            unreachable!()
        };
        assert!(compile_numeric(&expression, &symbols, "x", &interpreter).is_none());
    }

    #[test]
    fn adaptive_sampling_captures_peaks_a_uniform_grid_would_miss() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        let (_, Some(expression), _) = parse_submission(&mut symbols, "sin(60*x)").unwrap() else {
            unreachable!()
        };
        interpreter.set_symbols(symbols.clone());
        let compiled = compile_numeric(&expression, &symbols, "x", &interpreter).unwrap();
        // A coarse base grid of 64 points cannot see every peak of sin(60x);
        // refinement must find them anyway.
        let samples = sample_numeric_adaptive(&compiled, -1.0, 1.0, 64);
        let peak = samples
            .iter()
            .filter_map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(peak > 0.999, "adaptive peaks reached only {peak}");
        assert!(samples[0].0 == -1.0 && samples.last().unwrap().0 == 1.0);
        assert!(samples.len() > 64, "refinement must add points");
    }

    #[test]
    fn sampling_pins_the_endpoints_of_the_domain() {
        let mut interpreter = linear_interpreter();
        let mut symbols = SymbolTable::new();
        for (x_min, x_max) in [(0.1, 0.3), (0.0, 1.0 / 3.0), (-2.0, 7.0), (1e8, 3e8)] {
            for source in ["x", "sin(x)"] {
                let series =
                    sample_source(&mut interpreter, &mut symbols, source, x_min, x_max, 5).unwrap();
                assert_eq!(series.samples.first().unwrap().0, x_min, "{source} {x_min}");
                assert_eq!(series.samples.last().unwrap().0, x_max, "{source} {x_max}");
            }
        }
    }

    #[test]
    fn latex_picture_fills_the_line_width_and_covers_the_domain() {
        let pi = std::f64::consts::PI;
        let series = SampledSeries {
            label: "sin(x)".to_owned(),
            variable: "x".to_owned(),
            samples: (0..=40)
                .map(|index| {
                    let x = -pi + 2.0 * pi * index as f64 / 40.0;
                    (x, Some(x.sin()))
                })
                .collect(),
        };
        let bounds = PlotBounds::new(-pi, pi, -1.2, 1.2);
        let picture = render_latex_picture(&[series], &bounds, 400.0, 220.0);
        // The picture must scale to the surrounding column: fixed-point
        // pictures wider than the text column are clipped on the right by
        // the preview/export bounding box, cutting the domain off.
        assert!(
            picture.contains(r"\setlength{\unitlength}{\dimexpr\linewidth/400\relax}"),
            "{picture}"
        );
        // The curve spans the whole domain edge to edge (34 .. width - 8).
        fn endpoints(fragment: &str) -> (f64, f64) {
            let body = fragment.strip_prefix(r"\qbezier(").unwrap();
            let mut pairs = body.split(')');
            let start = pairs.next().unwrap();
            let control = pairs.next().unwrap();
            let end = pairs.next().unwrap();
            let x_of = |pair: &str| {
                pair.trim_start_matches('(')
                    .split(',')
                    .next()
                    .unwrap()
                    .parse::<f64>()
                    .unwrap()
            };
            (x_of(start), x_of(end).max(x_of(control)))
        }
        let first = picture.find(r"\qbezier(").unwrap();
        let last = picture.rfind(r"\qbezier(").unwrap();
        let (first_x, _) = endpoints(&picture[first..]);
        let (_, last_x) = endpoints(&picture[last..]);
        assert!(
            (first_x - 34.0).abs() < 0.02,
            "curve must start at the left margin: {first_x}"
        );
        assert!(
            (last_x - 392.0).abs() < 0.02,
            "curve must end at the right margin: {last_x}"
        );
    }

    #[test]
    fn f64_roundtrip_through_exact_rationals() {
        for value in [0.1, -3.75, 1.0e-12, 123456.789, f64::MIN_POSITIVE] {
            let number = number_from_f64(value).unwrap();
            let recovered = number_to_f64(&number).unwrap();
            assert_eq!(recovered, value, "{value}");
        }
        assert!(number_from_f64(f64::NAN).is_none());
        assert!(number_from_f64(f64::INFINITY).is_none());
    }
}
