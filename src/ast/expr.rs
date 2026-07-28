use crate::ast::unary_op::*;
use crate::ast::binary_op::*;
#[derive(Debug, Clone)]
pub enum Expr {
        // Literals
        Number(String),
        String(String),
        Bool(bool),
        Nil,

        // Variables
        Variable(String),

        // Unary
        Unary {
                op: UnaryOp,
                expr: Box<Expr>,
        },

        // Binary
        Binary {
                left: Box<Expr>,
                op: BinaryOp,
                right: Box<Expr>,
        },

        // Function call
        Call {
                callee: Box<Expr>,
                args: Vec<Expr>,
        },

        // Grouping
        Group(Box<Expr>),
}
