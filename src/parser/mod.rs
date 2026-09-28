pub mod error;
#[path = "parser.rs"]
mod syntax;

pub use error::ParseError;
pub use syntax::{ParseReport, Parser};
