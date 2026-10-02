//! Fast, allocation-conscious rendering of source AST expressions to LaTeX.

use crate::ast::{BinaryOp, Expr, ExprKind, UnaryOp};
use crate::interpreter::value::{Builtin, SymbolicExpr, Value};
use crate::lexer::SymbolTable;
use crate::number::Number;
use num_traits::Signed;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct LatexConfig {
    /// Function templates use zero-based placeholders (`{0}`, `{1}`, ...).
    pub functions: HashMap<String, String>,
    /// Exact identifier replacements, applied before built-in Greek mapping.
    pub identifiers: HashMap<String, String>,
}

impl LatexConfig {
    /// Load the documenter's small TOML configuration surface. Supported
    /// tables are `[functions]` and `[identifiers]`; values may use TOML's
    /// single- or double-quoted strings.
    pub fn load(path: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        Self::from_toml(&source)
    }

    pub fn from_toml(source: &str) -> Result<Self, String> {
        enum Section {
            None,
            Functions,
            Identifiers,
        }
        let mut section = Section::None;
        let mut config = Self::default();
        for (index, raw_line) in source.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            section = match line {
                "[functions]" => Section::Functions,
                "[identifiers]" => Section::Identifiers,
                _ if line.starts_with('[') => {
                    return Err(format!("unsupported table on line {}", index + 1))
                }
                _ => {
                    let (key, value) = line
                        .split_once('=')
                        .ok_or_else(|| format!("expected `name = value` on line {}", index + 1))?;
                    let key = key
                        .trim()
                        .trim_matches(|character| character == '\'' || character == '"');
                    let value = quoted_value(value.trim())
                        .ok_or_else(|| format!("expected a quoted string on line {}", index + 1))?;
                    match section {
                        Section::Functions => {
                            config.functions.insert(key.to_owned(), value);
                        }
                        Section::Identifiers => {
                            config.identifiers.insert(key.to_owned(), value);
                        }
                        Section::None => {
                            return Err(format!("entry outside a table on line {}", index + 1))
                        }
                    }
                    continue;
                }
            };
        }
        Ok(config)
    }
}

fn quoted_value(value: &str) -> Option<String> {
    if let Some(value) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return Some(value.to_owned());
    }
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut output = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            output.push(match character {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else {
            output.push(character);
        }
    }
    (!escaped).then_some(output)
}

/// A reusable expression renderer that writes recursively into one buffer
/// without constructing intermediate expression strings.
pub struct LatexRenderer<'a> {
    symbols: &'a SymbolTable,
    config: Option<&'a LatexConfig>,
}

impl<'a> LatexRenderer<'a> {
    pub fn new(symbols: &'a SymbolTable) -> Self {
        Self {
            symbols,
            config: None,
        }
    }

    pub fn with_config(symbols: &'a SymbolTable, config: &'a LatexConfig) -> Self {
        Self {
            symbols,
            config: Some(config),
        }
    }

    pub fn expression(&self, expression: &Expr) -> String {
        let mut output = String::new();
        self.expression_into(expression, &mut output);
        output
    }

    /// Render into a caller-owned buffer, retaining its allocation for fast
    /// live-preview updates.
    pub fn expression_into(&self, expression: &Expr, output: &mut String) {
        output.clear();
        output.reserve(
            expression
                .span
                .byte_end
                .saturating_sub(expression.span.byte_start)
                .saturating_mul(2),
        );
        self.write_expr(expression, 0, output);
    }

    fn write_expr(&self, expression: &Expr, parent_precedence: u8, output: &mut String) {
        match &expression.kind {
            ExprKind::Bool(value) => output.push_str(if *value {
                r"\text{true}"
            } else {
                r"\text{false}"
            }),
            ExprKind::Nil => output.push_str(r"\varnothing"),
            ExprKind::Number(value) => output.push_str(value),
            // A source string in render position is the documented raw-LaTeX
            // escape hatch.
            ExprKind::String(value) => output.push_str(value),
            ExprKind::Variable(symbol) => {
                self.write_identifier(self.symbols.resolve(*symbol), output)
            }
            ExprKind::Group(inner) => {
                output.push_str(r"\left(");
                self.write_expr(inner, 0, output);
                output.push_str(r"\right)");
            }
            ExprKind::Array(elements) => self.write_array(elements, output),
            ExprKind::Index { target, index } => {
                self.write_expr(target, 13, output);
                output.push_str("_{");
                self.write_expr(index, 0, output);
                output.push('}');
            }
            ExprKind::Call { callee, args } => self.write_call(callee, args, output),
            ExprKind::Unary { op, expr } => {
                if *op == UnaryOp::Transpose {
                    self.write_expr(expr, 13, output);
                    output.push_str(r"^{\mathsf T}");
                    return;
                }
                let precedence = 12;
                let parenthesize = precedence < parent_precedence;
                if parenthesize {
                    output.push_str(r"\left(");
                }
                output.push_str(match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => r"\neg ",
                    UnaryOp::BitNot | UnaryOp::Tilde => r"\mathord{\sim}",
                    UnaryOp::Transpose => unreachable!("transpose is rendered as a postfix"),
                });
                self.write_expr(expr, precedence, output);
                if parenthesize {
                    output.push_str(r"\right)");
                }
            }
            ExprKind::Binary {
                left,
                op,
                right,
                implicit,
            } => self.write_binary(left, *op, right, *implicit, parent_precedence, output),
        }
    }

    fn write_binary(
        &self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        implicit: bool,
        parent_precedence: u8,
        output: &mut String,
    ) {
        let precedence = precedence(op);
        let parenthesize = precedence < parent_precedence;
        if parenthesize {
            output.push_str(r"\left(");
        }
        match op {
            BinaryOp::Div => {
                output.push_str(r"\frac{");
                self.write_expr(left, 0, output);
                output.push_str("}{");
                self.write_expr(right, 0, output);
                output.push('}');
            }
            BinaryOp::Pow => {
                self.write_expr(left, precedence + 1, output);
                output.push_str("^{");
                self.write_expr(right, 0, output);
                output.push('}');
            }
            _ => {
                self.write_expr(left, precedence + u8::from(op == BinaryOp::Sub), output);
                output.push_str(match op {
                    BinaryOp::Add => " + ",
                    BinaryOp::Sub => " - ",
                    BinaryOp::Mul if implicit => "",
                    BinaryOp::Mul => r" \cdot ",
                    BinaryOp::Mod => r" \bmod ",
                    BinaryOp::Eq => " = ",
                    BinaryOp::Ne => r" \ne ",
                    BinaryOp::Lt => " < ",
                    BinaryOp::Le => r" \le ",
                    BinaryOp::Gt => " > ",
                    BinaryOp::Ge => r" \ge ",
                    BinaryOp::And => r" \land ",
                    BinaryOp::Or => r" \lor ",
                    BinaryOp::BitAnd => r" \mathbin{\&} ",
                    BinaryOp::BitOr => r" \mathbin{|} ",
                    BinaryOp::BitXor => r" \oplus ",
                    BinaryOp::Shl => r" \ll ",
                    BinaryOp::Shr => r" \gg ",
                    BinaryOp::Div | BinaryOp::Pow => unreachable!(),
                });
                self.write_expr(right, precedence + u8::from(op != BinaryOp::Pow), output);
            }
        }
        if parenthesize {
            output.push_str(r"\right)");
        }
    }

    fn write_call(&self, callee: &Expr, args: &[Expr], output: &mut String) {
        let ExprKind::Variable(symbol) = &callee.kind else {
            self.write_expr(callee, 13, output);
            self.write_arguments(args, output);
            return;
        };
        let name = self.symbols.resolve(*symbol);
        if let Some(template) = self.config.and_then(|config| config.functions.get(name)) {
            let mut rendered = template.clone();
            for (index, argument) in args.iter().enumerate() {
                rendered = rendered.replace(&format!("{{{index}}}"), &self.expression(argument));
            }
            output.push_str(&rendered);
            return;
        }
        match (name, args) {
            ("sqrt", [argument]) => {
                output.push_str(r"\sqrt{");
                self.write_expr(argument, 0, output);
                output.push('}');
            }
            ("cbrt", [argument]) => {
                output.push_str(r"\sqrt[3]{");
                self.write_expr(argument, 0, output);
                output.push('}');
            }
            ("abs", [argument]) => {
                output.push_str(r"\left|");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right|");
            }
            ("transpose", [argument]) | ("transpose_steps", [argument]) => {
                self.write_expr(argument, 13, output);
                output.push_str(r"^{\mathsf T}");
            }
            ("det", [argument]) | ("det_steps", [argument]) => {
                output.push_str(r"\det\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            ("norm", [argument]) | ("norm_steps", [argument]) => {
                output.push_str(r"\left\lVert ");
                self.write_expr(argument, 0, output);
                output.push_str(r" \right\rVert");
            }
            ("dot", [left, right]) | ("dot_steps", [left, right]) => {
                self.write_expr(left, 0, output);
                output.push_str(r" \cdot ");
                self.write_expr(right, 0, output);
            }
            ("cross", [left, right]) | ("cross_steps", [left, right]) => {
                self.write_expr(left, 0, output);
                output.push_str(r" \times ");
                self.write_expr(right, 0, output);
            }
            ("derivative", [argument, variable]) => {
                output.push_str(r"\frac{d}{d");
                self.write_expr(variable, 0, output);
                output.push_str(r"}\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            ("integral", [argument, variable]) => {
                output.push_str(r"\int ");
                self.write_expr(argument, 0, output);
                output.push_str(r"\,d");
                self.write_expr(variable, 0, output);
            }
            ("definite_integral", [argument, variable, lower, upper]) => {
                output.push_str(r"\int_{");
                self.write_expr(lower, 0, output);
                output.push_str("}^{");
                self.write_expr(upper, 0, output);
                output.push_str("} ");
                self.write_expr(argument, 0, output);
                output.push_str(r"\,d");
                self.write_expr(variable, 0, output);
            }
            ("gradient", [argument, _variables]) => {
                output.push_str(r"\nabla\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            ("jacobian", [arguments, variables]) => {
                output.push_str(r"\frac{\partial ");
                self.write_expr(arguments, 0, output);
                output.push_str(r"}{\partial ");
                self.write_expr(variables, 0, output);
                output.push('}');
            }
            ("hessian", [argument, _variables]) => {
                output.push_str(r"\nabla^{2}\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            ("log", [argument, base]) => {
                output.push_str(r"\log_{");
                self.write_expr(base, 0, output);
                output.push_str(r"}\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            ("log10", [argument]) => {
                output.push_str(r"\log_{10}\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            ("log2", [argument]) => {
                output.push_str(r"\log_{2}\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            (name, [argument]) if unary_command(name).is_some() => {
                output.push_str(unary_command(name).expect("guarded above"));
                output.push_str(r"\left(");
                self.write_expr(argument, 0, output);
                output.push_str(r"\right)");
            }
            _ => {
                output.push_str(r"\operatorname{");
                self.write_identifier(name, output);
                output.push('}');
                self.write_arguments(args, output);
            }
        }
    }

    fn write_arguments(&self, args: &[Expr], output: &mut String) {
        output.push_str(r"\left(");
        for (index, argument) in args.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            self.write_expr(argument, 0, output);
        }
        output.push_str(r"\right)");
    }

    fn write_array(&self, elements: &[Expr], output: &mut String) {
        output.push_str(r"\begin{pmatrix}");
        let matrix = elements
            .first()
            .is_some_and(|element| matches!(element.kind, ExprKind::Array(_)));
        if matrix {
            for (row_index, row) in elements.iter().enumerate() {
                if row_index != 0 {
                    output.push_str(r" \\ ");
                }
                let ExprKind::Array(columns) = &row.kind else {
                    unreachable!("the parser only creates rectangular matrices");
                };
                for (column_index, value) in columns.iter().enumerate() {
                    if column_index != 0 {
                        output.push_str(" & ");
                    }
                    self.write_expr(value, 0, output);
                }
            }
        } else {
            for (index, value) in elements.iter().enumerate() {
                if index != 0 {
                    output.push_str(r" \\ ");
                }
                self.write_expr(value, 0, output);
            }
        }
        output.push_str(r"\end{pmatrix}");
    }

    fn write_identifier(&self, name: &str, output: &mut String) {
        if let Some(mapped) = self.config.and_then(|config| config.identifiers.get(name)) {
            output.push_str(mapped);
            return;
        }
        let (base, subscript) = name.split_once('_').unwrap_or((name, ""));
        if let Some(command) = upright_greek_command(base) {
            output.push_str(command);
        } else if base.chars().count() == 1 {
            // TeX math letters are italic by default. Single-letter names are
            // conventional mathematical variables; descriptive names remain
            // upright so `mass` is not typeset as m·a·s·s.
            output.push_str(base);
        } else {
            output.push_str(r"\mathrm{");
            output.push_str(base);
            output.push('}');
        }
        if !subscript.is_empty() {
            output.push_str(r"_{\mathrm{");
            output.push_str(subscript);
            output.push_str("}}");
        }
    }

    /// Render an identifier using the same upright style as expression variables.
    pub fn identifier(&self, name: &str) -> String {
        let mut output = String::new();
        self.write_identifier(name, &mut output);
        output
    }

    /// Render an evaluated runtime value using the same notation as source
    /// expressions. This is used by `=>` and prose interpolation.
    pub fn value(&self, value: &Value) -> String {
        let mut output = String::new();
        self.write_value(value, &mut output);
        output
    }

    fn write_value(&self, value: &Value, output: &mut String) {
        match value {
            Value::Number(Number::Integer(number)) => output.push_str(&number.to_string()),
            Value::Number(Number::Rational(number)) => {
                if number.numer().sign() == num_bigint::Sign::Minus {
                    output.push('-');
                }
                output.push_str(r"\frac{");
                output.push_str(&number.numer().abs().to_string());
                output.push_str("}{");
                output.push_str(&number.denom().to_string());
                output.push('}');
            }
            Value::Number(number) => output.push_str(&number.to_string()),
            Value::Str(text) => {
                output.push_str(r"\text{");
                write_text_escaped(text, output);
                output.push('}');
            }
            Value::Bool(boolean) => output.push_str(if *boolean {
                r"\text{true}"
            } else {
                r"\text{false}"
            }),
            Value::Nil => output.push_str(r"\varnothing"),
            Value::Array(elements) => {
                output.push_str(r"\begin{pmatrix}");
                let matrix = elements
                    .first()
                    .is_some_and(|value| matches!(value, Value::Array(_)));
                for (row, value) in elements.iter().enumerate() {
                    if row != 0 {
                        output.push_str(r" \\ ");
                    }
                    if matrix {
                        if let Value::Array(columns) = value {
                            for (column, value) in columns.iter().enumerate() {
                                if column != 0 {
                                    output.push_str(" & ");
                                }
                                self.write_value(value, output);
                            }
                        }
                    } else {
                        self.write_value(value, output);
                    }
                }
                output.push_str(r"\end{pmatrix}");
            }
            Value::Symbolic(expression) => self.write_symbolic(expression, 0, output),
            Value::Function(_) | Value::Builtin(_) => output.push_str(r"\text{function}"),
            Value::Derivation { result, .. } => self.write_value(result, output),
        }
    }

    fn write_symbolic(&self, expression: &SymbolicExpr, parent: u8, output: &mut String) {
        match expression {
            SymbolicExpr::Number(number) => {
                self.write_value(&Value::Number(number.clone()), output)
            }
            SymbolicExpr::Constant(constant) => self.write_identifier(constant.name(), output),
            SymbolicExpr::Variable(symbol) => {
                self.write_identifier(self.symbols.resolve(*symbol), output)
            }
            SymbolicExpr::Unary { op, expr } => {
                if *op == UnaryOp::Transpose {
                    self.write_symbolic(expr, 13, output);
                    output.push_str(r"^{\mathsf T}");
                    return;
                }
                output.push_str(match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => r"\neg ",
                    UnaryOp::BitNot | UnaryOp::Tilde => r"\mathord{\sim}",
                    UnaryOp::Transpose => unreachable!("transpose is rendered as a postfix"),
                });
                self.write_symbolic(expr, 12, output);
            }
            SymbolicExpr::Binary { left, op, right } => {
                let precedence = precedence(*op);
                let parens = precedence < parent;
                if parens {
                    output.push_str(r"\left(");
                }
                if *op == BinaryOp::Div {
                    output.push_str(r"\frac{");
                    self.write_symbolic(left, 0, output);
                    output.push_str("}{");
                    self.write_symbolic(right, 0, output);
                    output.push('}');
                } else if *op == BinaryOp::Pow {
                    self.write_symbolic(left, precedence + 1, output);
                    output.push_str("^{");
                    self.write_symbolic(right, 0, output);
                    output.push('}');
                } else {
                    self.write_symbolic(left, precedence, output);
                    output.push_str(binary_latex(*op));
                    self.write_symbolic(right, precedence + 1, output);
                }
                if parens {
                    output.push_str(r"\right)");
                }
            }
            SymbolicExpr::Call { function, args } => match (function, args.as_slice()) {
                (Builtin::Sqrt, [argument]) => {
                    output.push_str(r"\sqrt{");
                    self.write_symbolic(argument, 0, output);
                    output.push('}');
                }
                (Builtin::Cbrt, [argument]) => {
                    output.push_str(r"\sqrt[3]{");
                    self.write_symbolic(argument, 0, output);
                    output.push('}');
                }
                (Builtin::Log, [argument, base]) => {
                    output.push_str(r"\log_{");
                    self.write_symbolic(base, 0, output);
                    output.push_str(r"}\left(");
                    self.write_symbolic(argument, 0, output);
                    output.push_str(r"\right)");
                }
                (Builtin::Log10, [argument]) => {
                    output.push_str(r"\log_{10}\left(");
                    self.write_symbolic(argument, 0, output);
                    output.push_str(r"\right)");
                }
                (Builtin::Log2, [argument]) => {
                    output.push_str(r"\log_{2}\left(");
                    self.write_symbolic(argument, 0, output);
                    output.push_str(r"\right)");
                }
                _ => {
                    let name = function.name();
                    if let Some(command) = unary_command(name).filter(|_| args.len() == 1) {
                        output.push_str(command);
                    } else {
                        output.push_str(r"\operatorname{");
                        output.push_str(name);
                        output.push('}');
                    }
                    output.push_str(r"\left(");
                    for (index, argument) in args.iter().enumerate() {
                        if index != 0 {
                            output.push_str(", ");
                        }
                        self.write_symbolic(argument, 0, output);
                    }
                    output.push_str(r"\right)");
                }
            },
        }
    }
}

fn binary_latex(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => " + ",
        BinaryOp::Sub => " - ",
        BinaryOp::Mul => r" \cdot ",
        BinaryOp::Mod => r" \bmod ",
        BinaryOp::Eq => " = ",
        BinaryOp::Ne => r" \ne ",
        BinaryOp::Lt => " < ",
        BinaryOp::Le => r" \le ",
        BinaryOp::Gt => " > ",
        BinaryOp::Ge => r" \ge ",
        BinaryOp::And => r" \land ",
        BinaryOp::Or => r" \lor ",
        BinaryOp::BitAnd => r" \mathbin{\&} ",
        BinaryOp::BitOr => r" \mathbin{|} ",
        BinaryOp::BitXor => r" \oplus ",
        BinaryOp::Shl => r" \ll ",
        BinaryOp::Shr => r" \gg ",
        BinaryOp::Div | BinaryOp::Pow => "",
    }
}

fn write_text_escaped(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '\\' => output.push_str(r"\textbackslash{}"),
            '{' => output.push_str(r"\{"),
            '}' => output.push_str(r"\}"),
            '&' => output.push_str(r"\&"),
            '%' => output.push_str(r"\%"),
            '$' => output.push_str(r"\$"),
            '#' => output.push_str(r"\#"),
            '_' => output.push_str(r"\_"),
            '^' => output.push_str(r"\^{}"),
            '~' => output.push_str(r"\~{}"),
            _ => output.push(character),
        }
    }
}

fn precedence(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Or => 1,
        BinaryOp::And => 2,
        BinaryOp::Eq | BinaryOp::Ne => 3,
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 4,
        BinaryOp::BitOr => 5,
        BinaryOp::BitXor => 6,
        BinaryOp::BitAnd => 7,
        BinaryOp::Shl | BinaryOp::Shr => 8,
        BinaryOp::Add | BinaryOp::Sub => 9,
        BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 10,
        BinaryOp::Pow => 11,
    }
}

fn unary_command(name: &str) -> Option<&'static str> {
    Some(match name {
        "sin" => r"\sin",
        "cos" => r"\cos",
        "tan" => r"\tan",
        "sec" => r"\sec",
        "csc" => r"\csc",
        "cot" => r"\cot",
        "asin" => r"\arcsin",
        "acos" => r"\arccos",
        "atan" => r"\arctan",
        "sinh" => r"\sinh",
        "cosh" => r"\cosh",
        "tanh" => r"\tanh",
        "exp" => r"\exp",
        "ln" => r"\ln",
        "log" | "log10" | "log2" => r"\log",
        "det" => r"\det",
        _ => return None,
    })
}

fn greek_command(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" | "α" => r"\alpha",
        "beta" | "β" => r"\beta",
        "gamma" | "γ" => r"\gamma",
        "delta" | "δ" => r"\delta",
        "epsilon" | "ε" => r"\epsilon",
        "varepsilon" | "ϵ" => r"\varepsilon",
        "zeta" | "ζ" => r"\zeta",
        "eta" | "η" => r"\eta",
        "theta" | "θ" => r"\theta",
        "vartheta" | "ϑ" => r"\vartheta",
        "iota" | "ι" => r"\iota",
        "kappa" | "κ" => r"\kappa",
        "lambda" | "λ" => r"\lambda",
        "mu" | "μ" => r"\mu",
        "nu" | "ν" => r"\nu",
        "xi" | "ξ" => r"\xi",
        "omicron" | "ο" => "o",
        "pi" | "π" => r"\pi",
        "varpi" | "ϖ" => r"\varpi",
        "rho" | "ρ" => r"\rho",
        "varrho" | "ϱ" => r"\varrho",
        "sigma" | "σ" => r"\sigma",
        "varsigma" | "ς" => r"\varsigma",
        "tau" | "τ" => r"\tau",
        "upsilon" | "υ" => r"\upsilon",
        "phi" | "φ" => r"\phi",
        "varphi" | "ϕ" => r"\varphi",
        "chi" | "χ" => r"\chi",
        "psi" | "ψ" => r"\psi",
        "omega" | "ω" => r"\omega",
        "Alpha" | "Α" => "A",
        "Beta" | "Β" => "B",
        "Gamma" | "Γ" => r"\Gamma",
        "Delta" | "Δ" => r"\Delta",
        "Epsilon" | "Ε" => "E",
        "Zeta" | "Ζ" => "Z",
        "Eta" | "Η" => "H",
        "Theta" | "Θ" => r"\Theta",
        "Iota" | "Ι" => "I",
        "Kappa" | "Κ" => "K",
        "Lambda" | "Λ" => r"\Lambda",
        "Mu" | "Μ" => "M",
        "Nu" | "Ν" => "N",
        "Xi" | "Ξ" => r"\Xi",
        "Omicron" | "Ο" => "O",
        "Pi" | "Π" => r"\Pi",
        "Rho" | "Ρ" => "P",
        "Sigma" | "Σ" => r"\Sigma",
        "Tau" | "Τ" => "T",
        "Upsilon" | "Υ" => r"\Upsilon",
        "Phi" | "Φ" => r"\Phi",
        "Chi" | "Χ" => "X",
        "Psi" | "Ψ" => r"\Psi",
        "Omega" | "Ω" => r"\Omega",
        _ => return None,
    })
}

fn upright_greek_command(name: &str) -> Option<&'static str> {
    greek_command(name).map(|command| match command {
        r"\alpha" => r"\upalpha",
        r"\beta" => r"\upbeta",
        r"\gamma" => r"\upgamma",
        r"\delta" => r"\updelta",
        r"\epsilon" => r"\upepsilon",
        r"\varepsilon" => r"\upvarepsilon",
        r"\zeta" => r"\upzeta",
        r"\eta" => r"\upeta",
        r"\theta" => r"\uptheta",
        r"\vartheta" => r"\upvartheta",
        r"\iota" => r"\upiota",
        r"\kappa" => r"\upkappa",
        r"\lambda" => r"\uplambda",
        r"\mu" => r"\upmu",
        r"\nu" => r"\upnu",
        r"\xi" => r"\upxi",
        "o" => "o",
        r"\pi" => r"\uppi",
        r"\varpi" => r"\upvarpi",
        r"\rho" => r"\uprho",
        r"\varrho" => r"\upvarrho",
        r"\sigma" => r"\upsigma",
        r"\varsigma" => r"\upvarsigma",
        r"\tau" => r"\uptau",
        r"\upsilon" => r"\upupsilon",
        r"\phi" => r"\upphi",
        r"\varphi" => r"\upvarphi",
        r"\chi" => r"\upchi",
        r"\psi" => r"\uppsi",
        r"\omega" => r"\upomega",
        "A" | "B" | "E" | "Z" | "H" | "I" | "K" | "M" | "N" | "O" | "P" | "T" | "X" => command,
        r"\Gamma" => r"\Upgamma",
        r"\Delta" => r"\Updelta",
        r"\Theta" => r"\Uptheta",
        r"\Lambda" => r"\Uplambda",
        r"\Xi" => r"\Upxi",
        r"\Pi" => r"\Uppi",
        r"\Sigma" => r"\Upsigma",
        r"\Upsilon" => r"\Upupsilon",
        r"\Phi" => r"\Upphi",
        r"\Psi" => r"\Uppsi",
        r"\Omega" => r"\Upomega",
        _ => unreachable!("all Greek commands are mapped"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::StatementKind;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use std::rc::Rc;

    fn render(source: &str) -> String {
        let source = format!("print({source});");
        let mut scanner = Scanner::new(&source);
        let tokens = scanner.scan_tokens().clone();
        let symbols = scanner.into_symbols();
        let statements = Parser::new(tokens).parse().unwrap();
        let StatementKind::Print(expression) = &statements[0].kind else {
            panic!("expected print expression");
        };
        LatexRenderer::new(&symbols).expression(expression)
    }

    #[test]
    fn renders_precedence_calculus_and_implicit_products() {
        assert_eq!(
            render("(x + 1) / (2x^2)"),
            r"\frac{\left(x + 1\right)}{\left(2x^{2}\right)}"
        );
        assert_eq!(
            render("derivative(sin(x), x)"),
            r"\frac{d}{dx}\left(\sin\left(x\right)\right)"
        );
        assert_eq!(render("integral(sqrt(x), x)"), r"\int \sqrt{x}\,dx");
        assert_eq!(render("2 * x + 3x"), r"2 \cdot x + 3x");
    }

    #[test]
    fn renders_roots_and_logarithms_as_natural_math() {
        assert_eq!(render("sqrt(x)"), r"\sqrt{x}");
        assert_eq!(render("cbrt(x)"), r"\sqrt[3]{x}");
        assert_eq!(render("log(x)"), r"\log\left(x\right)");
        assert_eq!(render("log(x, b)"), r"\log_{b}\left(x\right)");
        assert_eq!(render("log10(x)"), r"\log_{10}\left(x\right)");
        assert_eq!(render("log2(x)"), r"\log_{2}\left(x\right)");
        assert_eq!(render("ln(x)"), r"\ln\left(x\right)");

        // Evaluated results take the value renderer, which must agree.
        let mut scanner = Scanner::new("x b");
        let _ = scanner.scan_tokens();
        let mut symbols = scanner.into_symbols();
        let x = symbols.intern("x");
        let b = symbols.intern("b");
        let renderer = LatexRenderer::new(&symbols);
        let variable = |symbol| Value::Symbolic(Rc::new(SymbolicExpr::Variable(symbol)));
        let call = |function: Builtin, arguments: Vec<Value>| {
            Value::Symbolic(Rc::new(SymbolicExpr::Call {
                function,
                args: arguments
                    .into_iter()
                    .map(|argument| match argument {
                        Value::Symbolic(expression) => expression,
                        _ => unreachable!("test builds symbolic arguments"),
                    })
                    .collect(),
            }))
        };
        assert_eq!(
            renderer.value(&call(Builtin::Sqrt, vec![variable(x)])),
            r"\sqrt{x}"
        );
        assert_eq!(
            renderer.value(&call(Builtin::Cbrt, vec![variable(x)])),
            r"\sqrt[3]{x}"
        );
        assert_eq!(
            renderer.value(&call(Builtin::Log, vec![variable(x), variable(b)])),
            r"\log_{b}\left(x\right)"
        );
        assert_eq!(
            renderer.value(&call(Builtin::Log10, vec![variable(x)])),
            r"\log_{10}\left(x\right)"
        );
        assert_eq!(
            renderer.value(&call(Builtin::Log2, vec![variable(x)])),
            r"\log_{2}\left(x\right)"
        );
    }

    #[test]
    fn renders_identifiers_vectors_and_matrices() {
        assert_eq!(render("alpha_1 + β"), r"\upalpha_{\mathrm{1}} + \upbeta");
        assert_eq!(
            render("[x, 1/2]"),
            r"\begin{pmatrix}x \\ \frac{1}{2}\end{pmatrix}"
        );
        assert_eq!(
            render("[1, 2; 3, 4]"),
            r"\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}"
        );
        assert_eq!(
            render("[1, 2; 3, 4]'"),
            r"\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}^{\mathsf T}"
        );
        assert_eq!(
            render("ζ + η + ϑ + ξ + υ + ψ + Ξ"),
            r"\upzeta + \upeta + \upvartheta + \upxi + \upupsilon + \uppsi + \Upxi"
        );
    }

    #[test]
    fn toml_templates_and_identifier_overrides_are_applied() {
        let config = LatexConfig::from_toml(
            "[functions]\ntranspose = '{0}^{\\top}'\n[identifiers]\nvelocity = '\\vec{v}'\n",
        )
        .unwrap();
        let source = "print(transpose(velocity));";
        let mut scanner = Scanner::new(source);
        let tokens = scanner.scan_tokens().clone();
        let symbols = scanner.into_symbols();
        let statements = Parser::new(tokens).parse().unwrap();
        let StatementKind::Print(expression) = &statements[0].kind else {
            panic!("expected print expression");
        };
        assert_eq!(
            LatexRenderer::with_config(&symbols, &config).expression(expression),
            r"\vec{v}^{\top}"
        );
    }

    #[test]
    fn representative_renderer_output_has_balanced_braces() {
        for expression in [
            "(x + 1) / (2x^2)",
            "sqrt(alpha_1 + 3)",
            "[1, 2; 3, 4]",
            "derivative(sin(x), x)",
            "f(g(x), [x, y])",
        ] {
            let rendered = render(expression);
            let mut depth = 0isize;
            for character in rendered.chars() {
                if character == '{' {
                    depth += 1;
                }
                if character == '}' {
                    depth -= 1;
                }
                assert!(depth >= 0, "unbalanced output: {rendered}");
            }
            assert_eq!(depth, 0, "unbalanced output: {rendered}");
        }
    }
}
