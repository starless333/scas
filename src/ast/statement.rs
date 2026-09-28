use crate::ast::assign_op::AssignOp;
use crate::ast::expr::Expr;
use crate::ast::span::SourceSpan;
use crate::lexer::symbol::Symbol;

#[derive(Debug, Clone, PartialEq)]
pub enum FunctionBody {
    Expression(Expr),
    Block(Vec<Statement>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub span: SourceSpan,
    pub kind: StatementKind,
}

impl Statement {
    pub fn new(span: SourceSpan, kind: StatementKind) -> Self {
        Self { span, kind }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    Let {
        name: Symbol,
        value: Option<Expr>,
    },
    Function {
        name: Symbol,
        params: Vec<Symbol>,
        body: FunctionBody,
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
    Block(Vec<Statement>),
    Expr(Expr),
    Assign {
        name: Symbol,
        op: AssignOp,
        value: Expr,
    },
    DocComment(String),
    RenderBlock(Vec<Statement>),
    RenderLine {
        expression: Expr,
        show_result: bool,
    },
    Show(Expr),
}
