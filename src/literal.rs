#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
        INT(i64),
        RATIONAL(f64),
        STRING(String),
}
