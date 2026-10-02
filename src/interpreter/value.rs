use crate::ast::binary_op::BinaryOp;
use crate::ast::statement::FunctionBody;
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::environment::Env;
use crate::lexer::symbol::Symbol;
use crate::number::{MathConstant, Number};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Abs,
    Sqrt,
    Cbrt,
    Exp,
    Ln,
    Log,
    Log10,
    Log2,
    Sin,
    Cos,
    Tan,
    Sec,
    Csc,
    Cot,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Erf,
    Erfc,
    Gamma,
    Lgamma,
    Floor,
    Ceil,
    Round,
    Trunc,
    Min,
    Max,
    Hypot,
    Gcd,
    Lcm,
    Sign,
    Degrees,
    Radians,
    Evalf,
    Simplify,
    Expand,
    Factor,
    Apart,
    Horner,
    Collect,
    Subs,
    Solve,
    Derivative,
    Integral,
    AssumeNonzero,
    AssumeNonnegative,
    AssumePositive,
    AssumeNegative,
    AssumeInteger,
    AssumeReal,
    Assume,
    ForgetAssumptions,
    Transpose,
    Trace,
    Determinant,
    Rank,
    Rref,
    Inverse,
    Matmul,
    Dot,
    Cross,
    Norm,
    SolveLinear,
    DeterminantSteps,
    RrefSteps,
    InverseSteps,
    MatmulSteps,
    SolveLinearSteps,
    TransposeSteps,
    TraceSteps,
    RankSteps,
    DotSteps,
    CrossSteps,
    NormSteps,
    Gradient,
    Jacobian,
    Hessian,
    DefiniteIntegral,
    Degree,
    Coefficients,
    Resultant,
    Discriminant,
    PolynomialGcd,
    PolynomialDivRem,
}

impl Builtin {
    pub const ALL: &'static [(&'static str, Builtin)] = &[
        ("abs", Builtin::Abs),
        ("sqrt", Builtin::Sqrt),
        ("cbrt", Builtin::Cbrt),
        ("exp", Builtin::Exp),
        ("ln", Builtin::Ln),
        ("log", Builtin::Log),
        ("log10", Builtin::Log10),
        ("log2", Builtin::Log2),
        ("sin", Builtin::Sin),
        ("cos", Builtin::Cos),
        ("tan", Builtin::Tan),
        ("sec", Builtin::Sec),
        ("csc", Builtin::Csc),
        ("cot", Builtin::Cot),
        ("asin", Builtin::Asin),
        ("acos", Builtin::Acos),
        ("atan", Builtin::Atan),
        ("atan2", Builtin::Atan2),
        ("sinh", Builtin::Sinh),
        ("cosh", Builtin::Cosh),
        ("tanh", Builtin::Tanh),
        ("asinh", Builtin::Asinh),
        ("acosh", Builtin::Acosh),
        ("atanh", Builtin::Atanh),
        ("erf", Builtin::Erf),
        ("erfc", Builtin::Erfc),
        ("gamma", Builtin::Gamma),
        ("lgamma", Builtin::Lgamma),
        ("floor", Builtin::Floor),
        ("ceil", Builtin::Ceil),
        ("round", Builtin::Round),
        ("trunc", Builtin::Trunc),
        ("min", Builtin::Min),
        ("max", Builtin::Max),
        ("hypot", Builtin::Hypot),
        ("gcd", Builtin::Gcd),
        ("lcm", Builtin::Lcm),
        ("sign", Builtin::Sign),
        ("degrees", Builtin::Degrees),
        ("radians", Builtin::Radians),
        ("evalf", Builtin::Evalf),
        ("simplify", Builtin::Simplify),
        ("expand", Builtin::Expand),
        ("factor", Builtin::Factor),
        ("apart", Builtin::Apart),
        ("horner", Builtin::Horner),
        ("collect", Builtin::Collect),
        ("subs", Builtin::Subs),
        ("solve", Builtin::Solve),
        ("derivative", Builtin::Derivative),
        ("integral", Builtin::Integral),
        ("assume_nonzero", Builtin::AssumeNonzero),
        ("assume_nonnegative", Builtin::AssumeNonnegative),
        ("assume_positive", Builtin::AssumePositive),
        ("assume_negative", Builtin::AssumeNegative),
        ("assume_integer", Builtin::AssumeInteger),
        ("assume_real", Builtin::AssumeReal),
        ("assume", Builtin::Assume),
        ("forget_assumptions", Builtin::ForgetAssumptions),
        ("transpose", Builtin::Transpose),
        ("trace", Builtin::Trace),
        ("det", Builtin::Determinant),
        ("rank", Builtin::Rank),
        ("rref", Builtin::Rref),
        ("inverse", Builtin::Inverse),
        ("matmul", Builtin::Matmul),
        ("dot", Builtin::Dot),
        ("cross", Builtin::Cross),
        ("norm", Builtin::Norm),
        ("solve_linear", Builtin::SolveLinear),
        ("det_steps", Builtin::DeterminantSteps),
        ("rref_steps", Builtin::RrefSteps),
        ("inverse_steps", Builtin::InverseSteps),
        ("matmul_steps", Builtin::MatmulSteps),
        ("solve_linear_steps", Builtin::SolveLinearSteps),
        ("transpose_steps", Builtin::TransposeSteps),
        ("trace_steps", Builtin::TraceSteps),
        ("rank_steps", Builtin::RankSteps),
        ("dot_steps", Builtin::DotSteps),
        ("cross_steps", Builtin::CrossSteps),
        ("norm_steps", Builtin::NormSteps),
        ("gradient", Builtin::Gradient),
        ("jacobian", Builtin::Jacobian),
        ("hessian", Builtin::Hessian),
        ("definite_integral", Builtin::DefiniteIntegral),
        ("degree", Builtin::Degree),
        ("coefficients", Builtin::Coefficients),
        ("resultant", Builtin::Resultant),
        ("discriminant", Builtin::Discriminant),
        ("polynomial_gcd", Builtin::PolynomialGcd),
        ("polynomial_divrem", Builtin::PolynomialDivRem),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find_map(|(name, builtin)| (*builtin == self).then_some(*name))
            .expect("every builtin has a name")
    }
}

/// A user-defined function and the lexical environment it closed over.
#[derive(Debug)]
pub struct UserFunction {
    pub name: Symbol,
    pub params: Vec<Symbol>,
    pub body: FunctionBody,
    /// A symbolically evaluated form used when displaying concise
    /// functions. This expands calls to functions already in scope.
    pub display_body: Option<Value>,
    pub closure: Env,
}

#[derive(Debug, Clone)]
pub enum StepOperation {
    Start,
    SwapRows {
        first: usize,
        second: usize,
    },
    ScaleRow {
        row: usize,
        divisor: Value,
    },
    AddRowMultiple {
        target: usize,
        source: usize,
        factor: Value,
    },
    Explanation(String),
}

#[derive(Debug, Clone)]
pub struct DerivationStep {
    pub value: Value,
    pub operation: StepOperation,
}

#[derive(Debug, Clone)]
pub enum SymbolicExpr {
    Number(Number),
    Constant(MathConstant),
    Variable(Symbol),
    Unary {
        op: UnaryOp,
        expr: Rc<SymbolicExpr>,
    },
    Binary {
        left: Rc<SymbolicExpr>,
        op: BinaryOp,
        right: Rc<SymbolicExpr>,
    },
    Call {
        function: Builtin,
        args: Vec<Rc<SymbolicExpr>>,
    },
}

impl SymbolicExpr {
    pub fn equals(&self, other: &SymbolicExpr) -> bool {
        match (self, other) {
            (SymbolicExpr::Number(a), SymbolicExpr::Number(b)) => a == b,
            (SymbolicExpr::Constant(a), SymbolicExpr::Constant(b)) => a == b,
            (SymbolicExpr::Variable(a), SymbolicExpr::Variable(b)) => a == b,
            (
                SymbolicExpr::Unary { op: a_op, expr: a },
                SymbolicExpr::Unary { op: b_op, expr: b },
            ) => a_op == b_op && a.equals(b),
            (
                SymbolicExpr::Binary {
                    left: a_left,
                    op: a_op,
                    right: a_right,
                },
                SymbolicExpr::Binary {
                    left: b_left,
                    op: b_op,
                    right: b_right,
                },
            ) => a_op == b_op && a_left.equals(b_left) && a_right.equals(b_right),
            (
                SymbolicExpr::Call {
                    function: a_function,
                    args: a_args,
                },
                SymbolicExpr::Call {
                    function: b_function,
                    args: b_args,
                },
            ) => {
                a_function == b_function
                    && a_args.len() == b_args.len()
                    && a_args.iter().zip(b_args).all(|(a, b)| a.equals(b))
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Number(Number),
    Str(String),
    Bool(bool),
    Nil,
    Array(Rc<Vec<Value>>),
    Symbolic(Rc<SymbolicExpr>),
    Function(Rc<UserFunction>),
    Builtin(Builtin),
    /// A computed value accompanied by structured working for document output.
    Derivation {
        result: Rc<Value>,
        steps: Rc<Vec<DerivationStep>>,
    },
}

impl Value {
    /// Lox-style truthiness: everything is truthy except `nil` and `false`.
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn equals(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Nil, Value::Nil) => true,
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| a.equals(b))
            }
            (Value::Symbolic(a), Value::Symbolic(b)) => a.equals(b),
            (Value::Function(a), Value::Function(b)) => Rc::ptr_eq(a, b),
            (Value::Builtin(a), Value::Builtin(b)) => a == b,
            (Value::Derivation { result: a, .. }, Value::Derivation { result: b, .. }) => {
                a.equals(b)
            }
            _ => false,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(Number::Integer(_)) => "integer",
            Value::Number(Number::Rational(_)) => "rational",
            Value::Number(Number::Decimal(_)) => "decimal",
            Value::Number(Number::Complex(_)) => "complex",
            Value::Str(_) => "string",
            Value::Bool(_) => "bool",
            Value::Nil => "nil",
            Value::Array(_) => "array",
            Value::Symbolic(_) => "symbolic expression",
            Value::Function(_) => "function",
            Value::Builtin(_) => "builtin function",
            Value::Derivation { result, .. } => result.type_name(),
        }
    }
}

/// One collected `assume(constraint)` fact. `Equal` facts substitute the
/// variable outright; inequality facts sharpen the range predicates the
/// simplifier already consults.
#[derive(Debug, Clone)]
pub enum ConstraintFact {
    Equal {
        symbol: Symbol,
        value: Rc<SymbolicExpr>,
    },
    RealCompare {
        symbol: Symbol,
        /// The value compared against, with any equality-fact substitutions
        /// already folded in so compound facts stay mutually consistent.
        bound: Rc<SymbolicExpr>,
        /// `bound op symbol`. A conjunction uses the tightest bound per kind.
        op: BinaryOp,
    },
    NotEqual {
        symbol: Symbol,
        value: Rc<SymbolicExpr>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;

    #[test]
    fn value_equality_is_structural_and_type_aware() {
        let one = Value::Number(Number::Integer(BigInt::from(1)));
        let left = Value::Array(Rc::new(vec![one.clone(), Value::Bool(true)]));
        let right = Value::Array(Rc::new(vec![one, Value::Bool(true)]));
        assert!(left.equals(&right));
        assert!(!Value::Bool(true).equals(&Value::Number(Number::Integer(1.into()))));
        assert_eq!(left.type_name(), "array");
        assert!(!Value::Nil.is_truthy());
    }
}
