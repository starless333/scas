#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
        Int(i64),
        Rational(f64),
        Str(String),
}
