use crate::interpreter::value::Value;
use crate::RuntimeError;

/// Everything that can interrupt normal top-to-bottom statement execution.
/// Modeled as the `Err` side of a `Result` so `?` propagates it through
/// `execute`/`evaluate` for free -- a runtime error, a `break`, a
/// `continue`, and a `return` all need to unwind through the same nested
/// `if`/`while`/`block` structure, they just get caught at different
/// points (`Break`/`Continue` at the nearest loop, `Return` at the
/// nearest function call, `Error` all the way out to the top level).
#[derive(Debug, Clone)]
pub enum Flow {
    Error(RuntimeError),
    Return(Value),
    Break,
    Continue,
}

pub type ExecResult<T> = Result<T, Flow>;
