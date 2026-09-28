use crate::ast::assign_op::AssignOp;
use crate::ast::binary_op::BinaryOp;
use crate::ast::expr::Expr;
use crate::ast::statement::FunctionBody;
use crate::ast::statement::Statement;
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::environment::{Env, Environment};
use crate::interpreter::flow::{ExecResult, Flow};
use crate::interpreter::value::{SymbolicExpr, UserFunction, Value};
use crate::lexer::symbol::{Symbol, SymbolTable};
use crate::number::Number;
use bigdecimal::{BigDecimal, Context};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::io::Write;
use std::num::NonZeroU64;
use std::rc::Rc;

pub struct Interpreter<W: Write> {
    globals: Env,
    /// Owned so error messages can resolve a `Symbol` back to its
    /// source text (`"undefined variable 'x'"`). Evaluation itself
    /// never needs this -- every name reference in the AST is already
    /// the interned id, not text.
    symbols: SymbolTable,
    decimal_context: Context,
    output: W,
}

impl<W: Write> Interpreter<W> {
    pub fn new(symbols: SymbolTable, output: W) -> Self {
        Interpreter {
            globals: Environment::new(),
            symbols,
            decimal_context: Context::default(),
            output,
        }
    }

    pub fn output_mut(&mut self) -> &mut W {
        &mut self.output
    }

    /// Runs a whole program (as produced by `Parser::parse`) in the
    /// global scope. Returns a human-readable message on the first
    /// unhandled runtime error.
    pub fn run(&mut self, program: &[Statement]) -> Result<(), String> {
        let env = Rc::clone(&self.globals);
        for stmt in program {
            self.execute(&env, stmt)
                .map_err(|flow| self.describe_top_level(flow))?;
        }
        Ok(())
    }

    /// Runs one REPL submission and returns the final bare expression.
    pub fn run_interactive(&mut self, program: &[Statement]) -> Result<Option<String>, String> {
        let env = Rc::clone(&self.globals);
        let mut result = None;
        for (index, stmt) in program.iter().enumerate() {
            if index + 1 == program.len() {
                if let Statement::Expr(expr) = stmt {
                    let value = self
                        .evaluate(&env, expr)
                        .map_err(|flow| self.describe_top_level(flow))?;
                    result = Some(self.stringify(&value));
                    continue;
                }
            }
            self.execute(&env, stmt)
                .map_err(|flow| self.describe_top_level(flow))?;
        }
        Ok(result)
    }

    /// Updates name resolution as the REPL's stable symbol table grows.
    pub fn set_symbols(&mut self, symbols: SymbolTable) {
        self.symbols = symbols;
    }

    /// Sets the significant-digit precision used by operations whose
    /// decimal expansion is infinite (division and negative powers).
    pub fn set_decimal_precision(&mut self, precision: NonZeroU64) {
        self.decimal_context = self.decimal_context.with_precision(precision);
    }

    pub fn decimal_precision(&self) -> u64 {
        self.decimal_context.precision().get()
    }

    fn describe_top_level(&self, flow: Flow) -> String {
        match flow {
            Flow::Error(msg) => msg,
            Flow::Return(_) => "'return' used outside of a function".to_string(),
            Flow::Break => "'break' used outside of a loop".to_string(),
            Flow::Continue => "'continue' used outside of a loop".to_string(),
        }
    }

    fn error(&self, message: impl Into<String>) -> Flow {
        Flow::Error(message.into())
    }

    fn undefined_variable(&self, name: Symbol) -> Flow {
        self.error(format!(
            "undefined variable '{}'",
            self.symbols.resolve(name)
        ))
    }

    // ---- statements -----------------------------------------------

    fn execute(&mut self, env: &Env, stmt: &Statement) -> ExecResult<()> {
        match stmt {
            Statement::Let { name, value } => {
                let v = match value {
                    Some(e) => self.evaluate(env, e)?,
                    None => Value::Nil,
                };
                env.borrow_mut().define(*name, v);
                Ok(())
            }

            Statement::Function { name, params, body } => {
                let display_body = self.normalize_function_body(env, params, body);
                let f = Value::Function(Rc::new(UserFunction {
                    name: *name,
                    params: params.clone(),
                    body: body.clone(),
                    display_body,
                    closure: Rc::clone(env),
                }));
                env.borrow_mut().define(*name, f);
                Ok(())
            }

            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if self.evaluate(env, condition)?.is_truthy() {
                    self.execute(env, then_branch)
                } else if let Some(else_b) = else_branch {
                    self.execute(env, else_b)
                } else {
                    Ok(())
                }
            }

            Statement::While { condition, body } => {
                while self.evaluate(env, condition)?.is_truthy() {
                    match self.execute(env, body) {
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => continue,
                        other => other?,
                    }
                }
                Ok(())
            }

            Statement::For {
                init,
                condition,
                update,
                body,
            } => {
                // Its own scope so a `let` in `init` doesn't leak
                // into the enclosing block.
                let loop_env = Environment::child(env);
                self.execute(&loop_env, init)?;
                while self.evaluate(&loop_env, condition)?.is_truthy() {
                    match self.execute(&loop_env, body) {
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => {
                            self.execute(&loop_env, update)?;
                            continue;
                        }
                        other => other?,
                    }
                    self.execute(&loop_env, update)?;
                }
                Ok(())
            }

            Statement::Return(value) => {
                let v = match value {
                    Some(e) => self.evaluate(env, e)?,
                    None => Value::Nil,
                };
                Err(Flow::Return(v))
            }

            Statement::Break => Err(Flow::Break),
            Statement::Continue => Err(Flow::Continue),

            Statement::Print(expr) => {
                let v = self.evaluate(env, expr)?;
                let rendered = self.stringify(&v);
                writeln!(self.output, "{rendered}")
                    .map_err(|error| self.error(format!("failed to write output: {error}")))?;
                Ok(())
            }

            Statement::Block(statements) => {
                let block_env = Environment::child(env);
                for s in statements {
                    self.execute(&block_env, s)?;
                }
                Ok(())
            }

            Statement::Expr(expr) => {
                self.evaluate(env, expr)?;
                Ok(())
            }

            Statement::Assign { name, op, value } => {
                let rhs = self.evaluate(env, value)?;
                let new_val = if *op == AssignOp::Assign {
                    rhs
                } else {
                    let current = Environment::get(env, *name)
                        .ok_or_else(|| self.undefined_variable(*name))?;
                    self.apply_compound(*op, current, rhs)?
                };
                if Environment::assign(env, *name, new_val) {
                    Ok(())
                } else {
                    Err(self.undefined_variable(*name))
                }
            }
        }
    }

    fn normalize_function_body(
        &mut self,
        env: &Env,
        params: &[Symbol],
        body: &FunctionBody,
    ) -> Option<Value> {
        let FunctionBody::Expression(expression) = body else {
            return None;
        };
        let symbolic_env = Environment::child(env);
        for param in params {
            symbolic_env.borrow_mut().define(
                *param,
                Value::Symbolic(Rc::new(SymbolicExpr::Variable(*param))),
            );
        }
        // Recursive definitions are not yet in `env`, so they fail
        // normalization quickly and retain their source form. Calls to
        // previously defined functions are expanded and simplified.
        self.evaluate(&symbolic_env, expression).ok()
    }

    // ---- expressions ------------------------------------------------

    fn evaluate(&mut self, env: &Env, expr: &Expr) -> ExecResult<Value> {
        match expr {
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Nil => Ok(Value::Nil),
            Expr::Number(text) => self.parse_number(text),
            Expr::String(s) => Ok(Value::Str(s.clone())),
            Expr::Variable(sym) => Ok(Environment::get(env, *sym)
                .unwrap_or_else(|| Value::Symbolic(Rc::new(SymbolicExpr::Variable(*sym))))),
            Expr::Group(inner) => self.evaluate(env, inner),

            Expr::Array(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.evaluate(env, element)?);
                }
                Ok(Value::Array(Rc::new(values)))
            }

            Expr::Index { target, index } => {
                let target = self.evaluate(env, target)?;
                let index = self.evaluate(env, index)?;
                let Value::Number(Number::Integer(index)) = index else {
                    return Err(self.error("array index must be an integer"));
                };
                let index = index.to_usize().ok_or_else(|| {
                    self.error("array index must be non-negative and fit in memory")
                })?;
                let Value::Array(elements) = target else {
                    return Err(self.error(format!("cannot index a {}", target.type_name())));
                };
                elements.get(index).cloned().ok_or_else(|| {
                    self.error(format!(
                        "array index {index} is out of bounds for length {}",
                        elements.len()
                    ))
                })
            }

            Expr::Unary { op, expr } => {
                let v = self.evaluate(env, expr)?;
                self.apply_unary(*op, v)
            }

            Expr::Binary { left, op, right } => match op {
                // Short-circuit: the right side must not be
                // evaluated (side effects, undefined vars) unless
                // the left side didn't already decide the result.
                BinaryOp::And => {
                    let l = self.evaluate(env, left)?;
                    if !l.is_truthy() {
                        return Ok(l);
                    }
                    self.evaluate(env, right)
                }
                BinaryOp::Or => {
                    let l = self.evaluate(env, left)?;
                    if l.is_truthy() {
                        return Ok(l);
                    }
                    self.evaluate(env, right)
                }
                _ => {
                    let l = self.evaluate(env, left)?;
                    let r = self.evaluate(env, right)?;
                    self.apply_binary(*op, l, r)
                }
            },

            Expr::Call { callee, args } => {
                let callee_val = self.evaluate(env, callee)?;
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    arg_vals.push(self.evaluate(env, a)?);
                }
                self.call(callee_val, arg_vals)
            }
        }
    }

    fn call(&mut self, callee: Value, args: Vec<Value>) -> ExecResult<Value> {
        let Value::Function(f) = callee else {
            return Err(self.error(format!("'{}' is not callable", callee.type_name())));
        };
        if args.len() != f.params.len() {
            return Err(self.error(format!(
                "expected {} argument(s) but got {}",
                f.params.len(),
                args.len()
            )));
        }
        let call_env = Environment::child(&f.closure);
        for (param, arg) in f.params.iter().zip(args) {
            call_env.borrow_mut().define(*param, arg);
        }
        match &f.body {
            FunctionBody::Expression(expr) => self.evaluate(&call_env, expr),
            FunctionBody::Block(statements) => {
                for statement in statements {
                    match self.execute(&call_env, statement) {
                        Ok(()) => {}
                        Err(Flow::Return(value)) => return Ok(value),
                        Err(Flow::Break) => {
                            return Err(self.error("'break' used outside of a loop"))
                        }
                        Err(Flow::Continue) => {
                            return Err(self.error("'continue' used outside of a loop"))
                        }
                        Err(error @ Flow::Error(_)) => return Err(error),
                    }
                }
                Ok(Value::Nil)
            }
        }
    }

    // ---- number parsing ----------------------------------------------

    fn parse_number(&self, text: &str) -> ExecResult<Value> {
        let integer = if let Some(hex) = text.strip_prefix("0x") {
            BigInt::parse_bytes(hex.as_bytes(), 16)
        } else if let Some(bin) = text.strip_prefix("0b") {
            BigInt::parse_bytes(bin.as_bytes(), 2)
        } else if let Some(oct) = text.strip_prefix("0o") {
            BigInt::parse_bytes(oct.as_bytes(), 8)
        } else if !text.contains(['.', 'e', 'E']) {
            BigInt::parse_bytes(text.as_bytes(), 10)
        } else {
            None
        };
        if let Some(integer) = integer {
            return Ok(Value::Number(Number::Integer(integer)));
        }
        text.parse::<BigDecimal>()
            .map(|decimal| Value::Number(Number::Decimal(decimal)))
            .map_err(|_| self.error(format!("invalid number literal '{text}'")))
    }

    // ---- operators -----------------------------------------------------

    fn apply_unary(&self, op: UnaryOp, v: Value) -> ExecResult<Value> {
        match (op, v) {
            (UnaryOp::Neg, Value::Number(Number::Integer(n))) => {
                Ok(Value::Number(Number::Integer(-n)))
            }
            (UnaryOp::Neg, Value::Number(Number::Decimal(n))) => {
                Ok(Value::Number(Number::Decimal(-n)))
            }
            (UnaryOp::Neg, Value::Symbolic(expr)) => {
                Ok(Value::Symbolic(Rc::new(match expr.as_ref() {
                    SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: inner,
                    } => {
                        return Ok(Self::symbolic_value(Rc::clone(inner)));
                    }
                    _ => SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr,
                    },
                })))
            }
            (UnaryOp::Not, value) => Ok(Value::Bool(!value.is_truthy())),
            (UnaryOp::BitNot, Value::Number(Number::Integer(n))) => {
                Ok(Value::Number(Number::Integer(!n)))
            }
            (op, value) => {
                Err(self.error(format!("cannot apply {op:?} to a {}", value.type_name())))
            }
        }
    }

    fn apply_binary(&self, op: BinaryOp, l: Value, r: Value) -> ExecResult<Value> {
        use BinaryOp::*;
        if matches!(op, Add | Sub | Mul | Div | Mod | Pow)
            && (matches!(&l, Value::Symbolic(_)) || matches!(&r, Value::Symbolic(_)))
        {
            return self.symbolic_binary(op, l, r);
        }
        match op {
            Add => match (&l, &r) {
                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
                (Value::Array(a), Value::Array(b)) => {
                    let mut values = Vec::with_capacity(a.len() + b.len());
                    values.extend(a.iter().cloned());
                    values.extend(b.iter().cloned());
                    Ok(Value::Array(Rc::new(values)))
                }
                _ => self.numeric(l, r, "+", |a, b| a + b, |a, b| a + b, |a, b| a + b),
            },
            Sub => self.numeric(l, r, "-", |a, b| a - b, |a, b| a - b, |a, b| a - b),
            Mul => self.numeric(l, r, "*", |a, b| a * b, |a, b| a * b, |a, b| a * b),
            Div => self.divide(l, r),
            Mod => self.remainder(l, r),
            Pow => self.power(l, r),

            Eq => Ok(Value::Bool(l.equals(&r))),
            Ne => Ok(Value::Bool(!l.equals(&r))),

            Lt => self.compare(l, r, "<", |o| o.is_lt()),
            Le => self.compare(l, r, "<=", |o| !o.is_gt()),
            Gt => self.compare(l, r, ">", |o| o.is_gt()),
            Ge => self.compare(l, r, ">=", |o| !o.is_lt()),

            BitAnd => self.bitwise(l, r, "&", |a, b| a & b),
            BitOr => self.bitwise(l, r, "|", |a, b| a | b),
            BitXor => self.bitwise(l, r, "xor", |a, b| a ^ b),
            Shl => self.shift(l, r, "<<", |a, b| a << b),
            Shr => self.shift(l, r, ">>", |a, b| a >> b),

            And | Or => unreachable!("short-circuit ops are handled directly in evaluate()"),
        }
    }

    fn apply_compound(&self, op: AssignOp, current: Value, rhs: Value) -> ExecResult<Value> {
        let bin_op = match op {
            AssignOp::AddAssign => BinaryOp::Add,
            AssignOp::SubAssign => BinaryOp::Sub,
            AssignOp::MulAssign => BinaryOp::Mul,
            AssignOp::DivAssign => BinaryOp::Div,
            AssignOp::ModAssign => BinaryOp::Mod,
            AssignOp::PowAssign => BinaryOp::Pow,
            AssignOp::BitAndAssign => BinaryOp::BitAnd,
            AssignOp::BitOrAssign => BinaryOp::BitOr,
            AssignOp::Assign => {
                unreachable!("plain '=' is handled without reading the current value")
            }
        };
        self.apply_binary(bin_op, current, rhs)
    }

    fn numeric(
        &self,
        l: Value,
        r: Value,
        sym: &str,
        integer_op: impl FnOnce(BigInt, BigInt) -> BigInt,
        rational_op: impl FnOnce(BigRational, BigRational) -> BigRational,
        decimal_op: impl FnOnce(BigDecimal, BigDecimal) -> BigDecimal,
    ) -> ExecResult<Value> {
        match (l, r) {
            (Value::Number(Number::Integer(a)), Value::Number(Number::Integer(b))) => {
                Ok(Value::Number(Number::Integer(integer_op(a, b))))
            }
            (Value::Number(a), Value::Number(b)) => {
                if matches!(a, Number::Decimal(_)) || matches!(b, Number::Decimal(_)) {
                    Ok(Value::Number(Number::Decimal(decimal_op(
                        a.to_decimal(&self.decimal_context),
                        b.to_decimal(&self.decimal_context),
                    ))))
                } else {
                    Ok(Value::Number(Number::from_rational(rational_op(
                        a.to_rational(),
                        b.to_rational(),
                    ))))
                }
            }
            (l, r) => Err(self.type_error(sym, &l, &r)),
        }
    }

    fn divide(&self, l: Value, r: Value) -> ExecResult<Value> {
        match (l, r) {
            (Value::Number(a), Value::Number(b)) => {
                if b.is_zero() {
                    return Err(self.error("division by zero in '/'"));
                }
                if matches!(a, Number::Decimal(_)) || matches!(b, Number::Decimal(_)) {
                    let numerator = a.to_decimal(&self.decimal_context);
                    let denominator = b.to_decimal(&self.decimal_context);
                    let reciprocal = self.decimal_context.invert(&denominator);
                    let quotient = self.decimal_context.multiply(&numerator, &reciprocal);
                    Ok(Value::Number(Number::Decimal(quotient)))
                } else {
                    Ok(Value::Number(Number::from_rational(
                        a.to_rational() / b.to_rational(),
                    )))
                }
            }
            (l, r) => Err(self.type_error("/", &l, &r)),
        }
    }

    fn remainder(&self, l: Value, r: Value) -> ExecResult<Value> {
        match (l, r) {
            (Value::Number(a), Value::Number(b)) => {
                if b.is_zero() {
                    return Err(self.error("division by zero in '%'"));
                }
                match (a, b) {
                    (Number::Integer(a), Number::Integer(b)) => {
                        Ok(Value::Number(Number::Integer(a % b)))
                    }
                    (Number::Decimal(a), b) => Ok(Value::Number(Number::Decimal(
                        a % b.to_decimal(&self.decimal_context),
                    ))),
                    (a, Number::Decimal(b)) => Ok(Value::Number(Number::Decimal(
                        a.to_decimal(&self.decimal_context) % b,
                    ))),
                    (a, b) => Ok(Value::Number(Number::from_rational(
                        a.to_rational() % b.to_rational(),
                    ))),
                }
            }
            (l, r) => Err(self.type_error("%", &l, &r)),
        }
    }

    fn power(&self, base: Value, exponent: Value) -> ExecResult<Value> {
        let (Value::Number(base), Value::Number(Number::Integer(exponent))) = (base, exponent)
        else {
            return Err(self.error("'^' requires a numeric base and an integer exponent"));
        };
        let exponent = exponent.to_i64().ok_or_else(|| {
            self.error("exponent is too large to evaluate (maximum magnitude is 2^63 - 1)")
        })?;
        if exponent >= 0 {
            let exponent = u32::try_from(exponent).map_err(|_| {
                self.error("exponent is too large to evaluate (maximum is 2^32 - 1)")
            })?;
            return Ok(Value::Number(match base {
                Number::Integer(value) => Number::Integer(value.pow(exponent)),
                Number::Rational(value) => {
                    Number::from_rational(Self::rational_pow(value, exponent))
                }
                Number::Decimal(value) => Number::Decimal(Self::decimal_pow(value, exponent)),
            }));
        }
        if base.is_zero() {
            return Err(self.error("zero cannot be raised to a negative power"));
        }
        match base {
            Number::Decimal(value) => Ok(Value::Number(Number::Decimal(
                value.powi_with_context(exponent, &self.decimal_context),
            ))),
            value => {
                let positive = u32::try_from(exponent.unsigned_abs()).map_err(|_| {
                    self.error("exponent magnitude is too large (maximum is 2^32 - 1)")
                })?;
                let powered = Self::rational_pow(value.to_rational(), positive);
                Ok(Value::Number(Number::from_rational(powered.recip())))
            }
        }
    }

    fn rational_pow(mut base: BigRational, mut exponent: u32) -> BigRational {
        let mut result = BigRational::from_integer(BigInt::from(1));
        while exponent > 0 {
            if exponent & 1 == 1 {
                result *= &base;
            }
            exponent >>= 1;
            if exponent > 0 {
                base = &base * &base;
            }
        }
        result
    }

    /// Positive integral decimal powers terminate, so keep every digit
    /// instead of applying the context used for non-terminating results.
    fn decimal_pow(mut base: BigDecimal, mut exponent: u32) -> BigDecimal {
        let mut result = BigDecimal::from(1);
        while exponent > 0 {
            if exponent & 1 == 1 {
                result *= &base;
            }
            exponent >>= 1;
            if exponent > 0 {
                base = &base * &base;
            }
        }
        result
    }

    fn compare(
        &self,
        l: Value,
        r: Value,
        sym: &str,
        f: impl Fn(std::cmp::Ordering) -> bool,
    ) -> ExecResult<Value> {
        match (&l, &r) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(f(a.compare(b)))),
            (Value::Str(a), Value::Str(b)) => Ok(Value::Bool(f(a.cmp(b)))),
            _ => Err(self.type_error(sym, &l, &r)),
        }
    }

    fn bitwise(
        &self,
        l: Value,
        r: Value,
        sym: &str,
        op: impl FnOnce(BigInt, BigInt) -> BigInt,
    ) -> ExecResult<Value> {
        match (l, r) {
            (Value::Number(Number::Integer(a)), Value::Number(Number::Integer(b))) => {
                Ok(Value::Number(Number::Integer(op(a, b))))
            }
            (l, r) => Err(self.type_error(sym, &l, &r)),
        }
    }

    fn shift(
        &self,
        l: Value,
        r: Value,
        sym: &str,
        op: impl FnOnce(BigInt, usize) -> BigInt,
    ) -> ExecResult<Value> {
        match (l, r) {
            (Value::Number(Number::Integer(a)), Value::Number(Number::Integer(b))) => {
                let amount = b.to_usize().ok_or_else(|| {
                    self.error("shift amount must be a non-negative machine-sized integer")
                })?;
                Ok(Value::Number(Number::Integer(op(a, amount))))
            }
            (l, r) => Err(self.type_error(sym, &l, &r)),
        }
    }

    fn symbolic_binary(&self, op: BinaryOp, left: Value, right: Value) -> ExecResult<Value> {
        let left = self.value_to_symbolic(left)?;
        let right = self.value_to_symbolic(right)?;

        match op {
            BinaryOp::Add if Self::symbolic_is_zero(&left) => {
                return Ok(Self::symbolic_value(right))
            }
            BinaryOp::Add if Self::symbolic_is_zero(&right) => {
                return Ok(Self::symbolic_value(left))
            }
            BinaryOp::Sub if Self::symbolic_is_zero(&right) => {
                return Ok(Self::symbolic_value(left))
            }
            BinaryOp::Sub if Self::symbolic_is_zero(&left) => {
                return self.apply_unary(UnaryOp::Neg, Self::symbolic_value(right));
            }
            BinaryOp::Mul if Self::symbolic_is_zero(&left) => {
                return Ok(Self::symbolic_value(left))
            }
            BinaryOp::Mul if Self::symbolic_is_zero(&right) => {
                return Ok(Self::symbolic_value(right))
            }
            BinaryOp::Mul if Self::symbolic_is_one(&left) => {
                return Ok(Self::symbolic_value(right))
            }
            BinaryOp::Mul if Self::symbolic_is_one(&right) => {
                return Ok(Self::symbolic_value(left))
            }
            BinaryOp::Div if Self::symbolic_is_zero(&left) => {
                return Ok(Self::symbolic_value(left))
            }
            BinaryOp::Div if Self::symbolic_is_one(&right) => {
                return Ok(Self::symbolic_value(left))
            }
            BinaryOp::Pow if Self::symbolic_is_zero(&right) => {
                return Ok(Value::Number(Number::Integer(BigInt::from(1))));
            }
            BinaryOp::Pow if Self::symbolic_is_one(&right) => {
                return Ok(Self::symbolic_value(left))
            }
            _ => {}
        }

        if left.equals(&right) {
            return Ok(match op {
                BinaryOp::Add => Self::make_integer_term(BigInt::from(2), left),
                BinaryOp::Sub | BinaryOp::Mod => Value::Number(Number::Integer(BigInt::from(0))),
                BinaryOp::Mul => Self::make_integer_power(left, BigInt::from(2)),
                BinaryOp::Div => Value::Number(Number::Integer(BigInt::from(1))),
                _ => Value::Symbolic(Rc::new(SymbolicExpr::Binary { left, op, right })),
            });
        }

        if matches!(op, BinaryOp::Add | BinaryOp::Sub) {
            let (left_coefficient, left_term) = Self::integer_coefficient(&left);
            let (right_coefficient, right_term) = Self::integer_coefficient(&right);
            if left_term.equals(&right_term) {
                let coefficient = if op == BinaryOp::Add {
                    left_coefficient + right_coefficient
                } else {
                    left_coefficient - right_coefficient
                };
                return Ok(Self::make_integer_term(coefficient, left_term));
            }
        }

        if op == BinaryOp::Mul {
            let (left_coefficient, left_term) = Self::integer_coefficient(&left);
            let (right_coefficient, right_term) = Self::integer_coefficient(&right);
            if left_term.equals(&right_term)
                && (left_coefficient != BigInt::from(1) || right_coefficient != BigInt::from(1))
            {
                let squared = Rc::new(SymbolicExpr::Binary {
                    left: left_term,
                    op: BinaryOp::Pow,
                    right: Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(2)))),
                });
                return Ok(Self::make_integer_term(
                    left_coefficient * right_coefficient,
                    squared,
                ));
            }
            let (left_base, left_exponent) = Self::integer_power_parts(&left);
            let (right_base, right_exponent) = Self::integer_power_parts(&right);
            if left_base.equals(&right_base) {
                return Ok(Self::make_integer_power(
                    left_base,
                    left_exponent + right_exponent,
                ));
            }
            if let SymbolicExpr::Number(Number::Integer(factor)) = right.as_ref() {
                let (coefficient, term) = Self::integer_coefficient(&left);
                if coefficient != BigInt::from(1) {
                    return Ok(Self::make_integer_term(coefficient * factor, term));
                }
            }
            if let SymbolicExpr::Number(Number::Integer(factor)) = left.as_ref() {
                let (coefficient, term) = Self::integer_coefficient(&right);
                if coefficient != BigInt::from(1) {
                    return Ok(Self::make_integer_term(coefficient * factor, term));
                }
            }
        }

        if op == BinaryOp::Pow {
            if let (
                SymbolicExpr::Binary {
                    left: base,
                    op: BinaryOp::Pow,
                    right: inner_exponent,
                },
                SymbolicExpr::Number(Number::Integer(outer_exponent)),
            ) = (left.as_ref(), right.as_ref())
            {
                if let SymbolicExpr::Number(Number::Integer(inner_exponent)) =
                    inner_exponent.as_ref()
                {
                    return Ok(Self::make_integer_power(
                        Rc::clone(base),
                        inner_exponent * outer_exponent,
                    ));
                }
            }
        }

        Ok(Value::Symbolic(Rc::new(SymbolicExpr::Binary {
            left,
            op,
            right,
        })))
    }

    fn integer_coefficient(expression: &Rc<SymbolicExpr>) -> (BigInt, Rc<SymbolicExpr>) {
        match expression.as_ref() {
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let (coefficient, term) = Self::integer_coefficient(expr);
                (-coefficient, term)
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Mul,
                right,
            } => match (left.as_ref(), right.as_ref()) {
                (SymbolicExpr::Number(Number::Integer(coefficient)), _) => {
                    (coefficient.clone(), Rc::clone(right))
                }
                (_, SymbolicExpr::Number(Number::Integer(coefficient))) => {
                    (coefficient.clone(), Rc::clone(left))
                }
                _ => (BigInt::from(1), Rc::clone(expression)),
            },
            _ => (BigInt::from(1), Rc::clone(expression)),
        }
    }

    fn integer_power_parts(expression: &Rc<SymbolicExpr>) -> (Rc<SymbolicExpr>, BigInt) {
        if let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Pow,
            right,
        } = expression.as_ref()
        {
            if let SymbolicExpr::Number(Number::Integer(exponent)) = right.as_ref() {
                return (Rc::clone(left), exponent.clone());
            }
        }
        (Rc::clone(expression), BigInt::from(1))
    }

    fn make_integer_term(coefficient: BigInt, term: Rc<SymbolicExpr>) -> Value {
        if coefficient == BigInt::from(0) {
            Value::Number(Number::Integer(coefficient))
        } else if coefficient == BigInt::from(1) {
            Self::symbolic_value(term)
        } else if coefficient == BigInt::from(-1) {
            Value::Symbolic(Rc::new(SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr: term,
            }))
        } else {
            Value::Symbolic(Rc::new(SymbolicExpr::Binary {
                left: Rc::new(SymbolicExpr::Number(Number::Integer(coefficient))),
                op: BinaryOp::Mul,
                right: term,
            }))
        }
    }

    fn make_integer_power(base: Rc<SymbolicExpr>, exponent: BigInt) -> Value {
        if exponent == BigInt::from(0) {
            Value::Number(Number::Integer(BigInt::from(1)))
        } else if exponent == BigInt::from(1) {
            Self::symbolic_value(base)
        } else {
            Value::Symbolic(Rc::new(SymbolicExpr::Binary {
                left: base,
                op: BinaryOp::Pow,
                right: Rc::new(SymbolicExpr::Number(Number::Integer(exponent))),
            }))
        }
    }

    fn value_to_symbolic(&self, value: Value) -> ExecResult<Rc<SymbolicExpr>> {
        match value {
            Value::Number(number) => Ok(Rc::new(SymbolicExpr::Number(number))),
            Value::Symbolic(expression) => Ok(expression),
            value => Err(self.error(format!(
                "cannot use a {} in a symbolic arithmetic expression",
                value.type_name()
            ))),
        }
    }

    fn symbolic_value(expression: Rc<SymbolicExpr>) -> Value {
        match expression.as_ref() {
            SymbolicExpr::Number(number) => Value::Number(number.clone()),
            _ => Value::Symbolic(expression),
        }
    }

    fn symbolic_is_zero(expression: &SymbolicExpr) -> bool {
        matches!(expression, SymbolicExpr::Number(number) if number.is_zero())
    }

    fn symbolic_is_one(expression: &SymbolicExpr) -> bool {
        matches!(expression, SymbolicExpr::Number(number) if number.is_one())
    }

    fn type_error(&self, sym: &str, l: &Value, r: &Value) -> Flow {
        self.error(format!(
            "cannot apply '{sym}' to {} and {}",
            l.type_name(),
            r.type_name()
        ))
    }

    fn stringify(&self, v: &Value) -> String {
        match v {
            Value::Number(n) => n.to_string(),
            Value::Str(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Nil => "nil".to_string(),
            Value::Array(elements) => {
                let values: Vec<String> = elements
                    .iter()
                    .map(|element| match element {
                        Value::Str(value) => format!("\"{value}\""),
                        _ => self.stringify(element),
                    })
                    .collect();
                format!("[{}]", values.join(", "))
            }
            Value::Symbolic(expression) => self.format_symbolic(expression, 0),
            Value::Function(function) => {
                let name = self.symbols.resolve(function.name);
                let params = function
                    .params
                    .iter()
                    .map(|param| self.symbols.resolve(*param))
                    .collect::<Vec<_>>()
                    .join(", ");
                match &function.body {
                    FunctionBody::Expression(body) => {
                        let formatted = function
                            .display_body
                            .as_ref()
                            .map(|value| self.stringify(value))
                            .unwrap_or_else(|| self.format_expr(body, 0));
                        format!("{name}({params}) = {formatted}")
                    }
                    FunctionBody::Block(_) => format!("<function {name}({params})>"),
                }
            }
        }
    }

    fn format_symbolic(&self, expression: &SymbolicExpr, parent_precedence: u8) -> String {
        match expression {
            SymbolicExpr::Number(number) => number.to_string(),
            SymbolicExpr::Variable(symbol) => self.symbols.resolve(*symbol).to_string(),
            SymbolicExpr::Unary { op, expr } => {
                let text = format!(
                    "{}{}",
                    Self::unary_text(*op),
                    self.format_symbolic(expr, 12)
                );
                if 12 < parent_precedence {
                    format!("({text})")
                } else {
                    text
                }
            }
            SymbolicExpr::Binary { left, op, right } => {
                let precedence = Self::operator_precedence(*op);
                let text = format!(
                    "{} {} {}",
                    self.format_symbolic(left, precedence + u8::from(*op == BinaryOp::Pow)),
                    Self::operator_text(*op),
                    self.format_symbolic(right, precedence + u8::from(*op != BinaryOp::Pow)),
                );
                if precedence < parent_precedence {
                    format!("({text})")
                } else {
                    text
                }
            }
        }
    }

    fn format_expr(&self, expression: &Expr, parent_precedence: u8) -> String {
        match expression {
            Expr::Bool(value) => value.to_string(),
            Expr::Nil => "nil".to_string(),
            Expr::Number(value) => value.clone(),
            Expr::String(value) => format!("\"{value}\""),
            Expr::Variable(symbol) => self.symbols.resolve(*symbol).to_string(),
            Expr::Group(inner) => format!("({})", self.format_expr(inner, 0)),
            Expr::Array(elements) => format!(
                "[{}]",
                elements
                    .iter()
                    .map(|item| self.format_expr(item, 0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Expr::Index { target, index } => {
                format!(
                    "{}[{}]",
                    self.format_expr(target, 13),
                    self.format_expr(index, 0)
                )
            }
            Expr::Call { callee, args } => format!(
                "{}({})",
                self.format_expr(callee, 13),
                args.iter()
                    .map(|arg| self.format_expr(arg, 0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Expr::Unary { op, expr } => {
                let text = format!("{}{}", Self::unary_text(*op), self.format_expr(expr, 12));
                if 12 < parent_precedence {
                    format!("({text})")
                } else {
                    text
                }
            }
            Expr::Binary { left, op, right } => {
                let precedence = Self::operator_precedence(*op);
                let text = format!(
                    "{} {} {}",
                    self.format_expr(left, precedence + u8::from(*op == BinaryOp::Pow)),
                    Self::operator_text(*op),
                    self.format_expr(right, precedence + u8::from(*op != BinaryOp::Pow)),
                );
                if precedence < parent_precedence {
                    format!("({text})")
                } else {
                    text
                }
            }
        }
    }

    fn operator_precedence(op: BinaryOp) -> u8 {
        match op {
            BinaryOp::Or => 1,
            BinaryOp::And => 2,
            BinaryOp::Eq | BinaryOp::Ne => 3,
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => 4,
            BinaryOp::BitOr => 5,
            BinaryOp::BitXor => 6,
            BinaryOp::BitAnd => 7,
            BinaryOp::Shl | BinaryOp::Shr => 8,
            BinaryOp::Add | BinaryOp::Sub => 9,
            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => 10,
            BinaryOp::Pow => 11,
        }
    }

    fn operator_text(op: BinaryOp) -> &'static str {
        match op {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Pow => "^",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::And => "and",
            BinaryOp::Or => "or",
            BinaryOp::BitAnd => "&",
            BinaryOp::BitOr => "|",
            BinaryOp::BitXor => "xor",
            BinaryOp::Shl => "<<",
            BinaryOp::Shr => ">>",
        }
    }

    fn unary_text(op: UnaryOp) -> &'static str {
        match op {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::BitNot | UnaryOp::Tilde => "~",
        }
    }
}
