#[derive(Debug, Clone)]
mod unary_op;
mod binary_op;
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
