//! Fast, allocation-conscious rendering of source AST expressions to LaTeX.

use crate::ast::{BinaryOp, Expr, ExprKind, UnaryOp};
use crate::lexer::SymbolTable;

/// A reusable expression renderer that writes recursively into one buffer
/// without constructing intermediate expression strings.
pub struct LatexRenderer<'a> {
    symbols: &'a SymbolTable,
}

impl<'a> LatexRenderer<'a> {
    pub fn new(symbols: &'a SymbolTable) -> Self {
        Self { symbols }
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
                let precedence = 12;
                let parenthesize = precedence < parent_precedence;
                if parenthesize {
                    output.push_str(r"\left(");
                }
                output.push_str(match op {
                    UnaryOp::Neg => "-",
                    UnaryOp::Not => r"\neg ",
                    UnaryOp::BitNot | UnaryOp::Tilde => r"\mathord{\sim}",
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
        let (base, subscript) = name.split_once('_').unwrap_or((name, ""));
        if let Some(command) = greek_command(base) {
            output.push_str(command);
        } else {
            output.push_str(base);
        }
        if !subscript.is_empty() {
            output.push_str("_{");
            output.push_str(subscript);
            output.push('}');
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
        "theta" | "θ" => r"\theta",
        "lambda" | "λ" => r"\lambda",
        "mu" | "μ" => r"\mu",
        "pi" | "π" => r"\pi",
        "rho" | "ρ" => r"\rho",
        "sigma" | "σ" => r"\sigma",
        "phi" | "φ" => r"\phi",
        "omega" | "ω" => r"\omega",
        "Gamma" | "Γ" => r"\Gamma",
        "Delta" | "Δ" => r"\Delta",
        "Theta" | "Θ" => r"\Theta",
        "Lambda" | "Λ" => r"\Lambda",
        "Pi" | "Π" => r"\Pi",
        "Sigma" | "Σ" => r"\Sigma",
        "Phi" | "Φ" => r"\Phi",
        "Omega" | "Ω" => r"\Omega",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::StatementKind;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

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
    fn renders_identifiers_vectors_and_matrices() {
        assert_eq!(render("alpha_1 + β"), r"\alpha_{1} + \beta");
        assert_eq!(
            render("[x, 1/2]"),
            r"\begin{pmatrix}x \\ \frac{1}{2}\end{pmatrix}"
        );
        assert_eq!(
            render("[1, 2; 3, 4]"),
            r"\begin{pmatrix}1 & 2 \\ 3 & 4\end{pmatrix}"
        );
    }
}
