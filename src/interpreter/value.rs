use crate::ast::binary_op::BinaryOp;
use crate::ast::statement::FunctionBody;
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::environment::Env;
use crate::lexer::symbol::Symbol;
use crate::number::Number;
use std::rc::Rc;

/// A user-defined function and the lexical environment it closed over.
#[derive(Debug)]
pub struct UserFunction {
    pub name: Symbol,
    pub params: Vec<Symbol>,
    pub body: FunctionBody,
    /// A symbolically evaluated form used when displaying concise
    /// functions. This expands calls to functions already in scope.
    pub display_body: Option<Value>,
    pub closure: Env,
}

#[derive(Debug, Clone)]
pub enum SymbolicExpr {
    Number(Number),
    Variable(Symbol),
    Unary {
        op: UnaryOp,
        expr: Rc<SymbolicExpr>,
    },
    Binary {
        left: Rc<SymbolicExpr>,
        op: BinaryOp,
        right: Rc<SymbolicExpr>,
    },
}

impl SymbolicExpr {
    pub fn equals(&self, other: &SymbolicExpr) -> bool {
        match (self, other) {
            (SymbolicExpr::Number(a), SymbolicExpr::Number(b)) => a == b,
            (SymbolicExpr::Variable(a), SymbolicExpr::Variable(b)) => a == b,
            (
                SymbolicExpr::Unary { op: a_op, expr: a },
                SymbolicExpr::Unary { op: b_op, expr: b },
            ) => a_op == b_op && a.equals(b),
            (
                SymbolicExpr::Binary {
                    left: a_left,
                    op: a_op,
                    right: a_right,
                },
                SymbolicExpr::Binary {
                    left: b_left,
                    op: b_op,
                    right: b_right,
                },
            ) => a_op == b_op && a_left.equals(b_left) && a_right.equals(b_right),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Number(Number),
    Str(String),
    Bool(bool),
    Nil,
    Array(Rc<Vec<Value>>),
    Symbolic(Rc<SymbolicExpr>),
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
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| a.equals(b))
            }
            (Value::Symbolic(a), Value::Symbolic(b)) => a.equals(b),
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(Number::Integer(_)) => "integer",
            Value::Number(Number::Rational(_)) => "rational",
            Value::Number(Number::Decimal(_)) => "decimal",
            Value::Str(_) => "string",
            Value::Bool(_) => "bool",
            Value::Nil => "nil",
            Value::Array(_) => "array",
            Value::Symbolic(_) => "symbolic expression",
            Value::Function(_) => "function",
        }
    }
}
