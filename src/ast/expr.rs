use crate::ast::binary_op::BinaryOp;
use crate::ast::span::SourceSpan;
use crate::ast::unary_op::UnaryOp;
use crate::lexer::symbol::Symbol;

/// The general-purpose expression tree for the host scripting language
/// (as opposed to a dedicated symbolic-math tree for CAS objects like
/// Hamiltonians -- that's a separate concern layered on top of this one,
/// e.g. via a builtin `Call` to a CAS function).
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub span: SourceSpan,
    pub kind: ExprKind,
}

impl Expr {
    pub fn new(span: SourceSpan, kind: ExprKind) -> Self {
        Self { span, kind }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Bool(bool),
    Nil,

    /// Raw numeric source text, exactly as scanned (`"10.5e-3"`,
    /// `"0xFF"`, ...). Kept as a string all the way through parsing so
    /// no precision is thrown away before an evaluator gets to decide
    /// how to interpret it -- parsing to a concrete numeric type is
    /// deferred to evaluation.
    Number(String),

    /// A string literal's *contents* (unquoted).
    String(String),

    /// A variable reference. Stores the interned `Symbol` id from the
    /// lexer, not the resolved name -- comparing/hashing/copying a
    /// `u32` during parsing and evaluation is far cheaper than doing
    /// the same with an owned `String` on every reference, and the
    /// name text is only needed for error messages, which is rare
    /// enough to pay the `SymbolTable::resolve` cost there instead.
    Variable(Symbol),

    Group(Box<Expr>),

    Array(Vec<Expr>),

    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },

    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },

    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        /// Whether multiplication was written by adjacency (`2x`) rather
        /// than with an explicit operator (`2 * x`). Always false for
        /// non-multiplication operators.
        implicit: bool,
    },

    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
}
