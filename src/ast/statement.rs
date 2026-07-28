use crate::ast::expr::*;
use crate::ast::assign_op::*;
#[derive(Debug, Clone)]
pub enum Statement {
        Block(Vec<Statement>),

        Let {
                name: String,
                value: Option<Expr>,
        },

        Assign {
                name: String,
                op: AssignOp,
                value: Expr,
        },

        Function {
                name: String,
                params: Vec<String>,
                body: Expr,
        },

        If {
                condition: Expr,
                then_branch: Box<Statement>,
                else_branch: Option<Box<Statement>>,
        },

        While {
                condition: Expr,
                body: Box<Statement>,
        },

        For {
                init: Box<Statement>,
                condition: Expr,
                update: Box<Statement>,
                body: Box<Statement>,
        },

        Return(Option<Expr>),

        Break,

        Continue,

        Print(Expr),

        Expr(Expr),
}
