use crate::ast::expr::Expr;
use crate::interpreter::environment::Env;
use crate::lexer::symbol::Symbol;
use std::rc::Rc;

/// A user-defined function: its parameter names, its (single-expression)
/// body, and the environment it closed over at the point of definition.
#[derive(Debug)]
pub struct UserFunction {
        pub params: Vec<Symbol>,
        pub body: Expr,
        pub closure: Env,
}

#[derive(Debug, Clone)]
pub enum Value {
        /// f64 for now. `Expr::Number` still carries the exact source text
        /// all the way to this point, so nothing is lost *before*
        /// evaluation -- but this final text -> f64 conversion does lose
        /// precision (no arbitrary-precision integers/rationals yet). This
        /// is the one place in the whole pipeline that isn't safe for
        /// exact CAS arithmetic; swapping it for the BigInt/BigRational
        /// `Number` type discussed earlier only requires changing this
        /// variant and the arithmetic in `Interpreter::apply_binary`.
        Number(f64),
        Str(String),
        Bool(bool),
        Nil,
        Function(Rc<UserFunction>),
}

impl Value {
        /// Lox-style truthiness: everything is truthy except `nil` and `false`.
        pub fn is_truthy(&self) -> bool {
                !matches!(self, Value::Nil | Value::Bool(false))
        }

        pub fn equals(&self, other: &Value) -> bool {
                match (self, other) {
                        (Value::Number(a), Value::Number(b)) => a == b,
                        (Value::Str(a), Value::Str(b)) => a == b,
                        (Value::Bool(a), Value::Bool(b)) => a == b,
                        (Value::Nil, Value::Nil) => true,
                        (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
                        _ => false,
                }
        }

        pub fn type_name(&self) -> &'static str {
                match self {
                        Value::Number(_) => "number",
                        Value::Str(_) => "string",
                        Value::Bool(_) => "bool",
                        Value::Nil => "nil",
                        Value::Function(_) => "function",
                }
        }
}
