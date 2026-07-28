#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
        // Arithmetic
        Add,
        Sub,
        Mul,
        Div,
        Mod,
        Pow,

        // Comparison
        Eq,
        Ne,
        Lt,
        Le,
        Gt,
        Ge,

        // Logical
        And,
        Or,

        // Bitwise
        BitAnd,
        BitOr,
        BitXor,
        Shl,
        Shr,
}
