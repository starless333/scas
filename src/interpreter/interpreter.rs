use crate::ast::assign_op::AssignOp;
use crate::ast::binary_op::BinaryOp;
use crate::ast::expr::{Expr, ExprKind};
use crate::ast::statement::FunctionBody;
use crate::ast::statement::{Statement, StatementKind};
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::environment::{Env, Environment};
use crate::interpreter::flow::{ExecResult, Flow};
use crate::interpreter::polynomial::Polynomial;
use crate::interpreter::value::{Builtin, SymbolicExpr, UserFunction, Value};
use crate::lexer::symbol::{Symbol, SymbolTable};
use crate::number::Number;
use crate::{Limits, RuntimeError};
use bigdecimal::{BigDecimal, Context};
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::ToPrimitive;
use std::cell::RefCell;
use std::collections::HashSet;
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
    current_span: Option<crate::ast::SourceSpan>,
    limits: Limits,
    steps: u64,
    call_depth: usize,
    nonzero_symbols: RefCell<HashSet<Symbol>>,
    nonnegative_symbols: RefCell<HashSet<Symbol>>,
}

impl<W: Write> Interpreter<W> {
    pub fn new(symbols: SymbolTable, output: W) -> Self {
        Self::with_limits(symbols, output, Limits::default())
    }

    pub fn with_limits(symbols: SymbolTable, output: W, limits: Limits) -> Self {
        let mut interpreter = Interpreter {
            globals: Environment::new(),
            symbols,
            decimal_context: Context::default(),
            output,
            current_span: None,
            limits,
            steps: 0,
            call_depth: 0,
            nonzero_symbols: RefCell::new(HashSet::new()),
            nonnegative_symbols: RefCell::new(HashSet::new()),
        };
        interpreter.install_standard_library();
        interpreter
    }

    /// Adds standard-library names that are present in the current symbol
    /// table. The REPL's table grows between submissions, so this must also be
    /// called after a refresh. Existing globals are deliberately preserved so
    /// user definitions can shadow a built-in or constant.
    fn install_standard_library(&mut self) {
        for (name, builtin) in Builtin::ALL {
            if let Some(symbol) = self.symbols.get(name) {
                self.define_global_if_absent(symbol, Value::Builtin(*builtin));
            }
        }
        for (name, value) in [
            ("pi", std::f64::consts::PI),
            ("e", std::f64::consts::E),
            ("tau", std::f64::consts::TAU),
        ] {
            if let Some(symbol) = self.symbols.get(name) {
                if let Some(number) = Number::from_f64(value) {
                    self.define_global_if_absent(symbol, Value::Number(number));
                }
            }
        }
    }

    fn define_global_if_absent(&mut self, symbol: Symbol, value: Value) {
        if Environment::get(&self.globals, symbol).is_none() {
            self.globals.borrow_mut().define(symbol, value);
        }
    }

    pub fn output_mut(&mut self) -> &mut W {
        &mut self.output
    }

    /// Runs a whole program (as produced by `Parser::parse`) in the
    /// global scope. Returns a human-readable message on the first
    /// unhandled runtime error.
    pub fn run(&mut self, program: &[Statement]) -> Result<(), RuntimeError> {
        self.steps = 0;
        self.call_depth = 0;
        let env = Rc::clone(&self.globals);
        for stmt in program {
            self.execute(&env, stmt)
                .map_err(|flow| self.describe_top_level(flow))?;
        }
        Ok(())
    }

    /// Runs one REPL submission and returns the final bare expression.
    pub fn run_interactive(
        &mut self,
        program: &[Statement],
    ) -> Result<Option<String>, RuntimeError> {
        self.steps = 0;
        self.call_depth = 0;
        let env = Rc::clone(&self.globals);
        let mut result = None;
        for (index, stmt) in program.iter().enumerate() {
            if index + 1 == program.len() {
                if let StatementKind::Expr(expr) = &stmt.kind {
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
        self.install_standard_library();
    }

    /// Sets the significant-digit precision used by operations whose
    /// decimal expansion is infinite (division and negative powers).
    pub fn set_decimal_precision(&mut self, precision: NonZeroU64) -> Result<(), RuntimeError> {
        if precision.get() > self.limits.max_decimal_precision {
            return Err(RuntimeError::limit(
                format!(
                    "decimal precision exceeds the {} digit limit",
                    self.limits.max_decimal_precision
                ),
                self.current_span,
            ));
        }
        self.decimal_context = self.decimal_context.with_precision(precision);
        Ok(())
    }

    pub fn decimal_precision(&self) -> u64 {
        self.decimal_context.precision().get()
    }

    pub fn definition_names(&self) -> Vec<&str> {
        let mut names = Environment::local_names(&self.globals)
            .into_iter()
            .map(|symbol| self.symbols.resolve(symbol))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    fn describe_top_level(&self, flow: Flow) -> RuntimeError {
        match flow {
            Flow::Error(msg) => msg,
            Flow::Return(_) => {
                RuntimeError::new("'return' used outside of a function", self.current_span)
            }
            Flow::Break => RuntimeError::new("'break' used outside of a loop", self.current_span),
            Flow::Continue => {
                RuntimeError::new("'continue' used outside of a loop", self.current_span)
            }
        }
    }

    fn error(&self, message: impl Into<String>) -> Flow {
        Flow::Error(RuntimeError::new(message, self.current_span))
    }

    fn limit_error(&self, message: impl Into<String>) -> Flow {
        Flow::Error(RuntimeError::limit(message, self.current_span))
    }

    fn tick(&mut self) -> ExecResult<()> {
        self.steps = self.steps.saturating_add(1);
        if self.steps > self.limits.max_steps {
            Err(self.limit_error(format!(
                "execution exceeded the {} step limit",
                self.limits.max_steps
            )))
        } else {
            Ok(())
        }
    }

    fn undefined_variable(&self, name: Symbol) -> Flow {
        self.error(format!(
            "undefined variable '{}'",
            self.symbols.resolve(name)
        ))
    }

    // ---- statements -----------------------------------------------

    fn execute(&mut self, env: &Env, stmt: &Statement) -> ExecResult<()> {
        self.current_span = Some(stmt.span);
        self.tick()?;
        match &stmt.kind {
            StatementKind::Let { name, value } => {
                let v = match value {
                    Some(e) => self.evaluate(env, e)?,
                    None => Value::Nil,
                };
                env.borrow_mut().define(*name, v);
                Ok(())
            }

            StatementKind::Function { name, params, body } => {
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

            StatementKind::If {
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

            StatementKind::While { condition, body } => {
                while self.evaluate(env, condition)?.is_truthy() {
                    match self.execute(env, body) {
                        Err(Flow::Break) => break,
                        Err(Flow::Continue) => continue,
                        other => other?,
                    }
                }
                Ok(())
            }

            StatementKind::For {
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

            StatementKind::Return(value) => {
                let v = match value {
                    Some(e) => self.evaluate(env, e)?,
                    None => Value::Nil,
                };
                Err(Flow::Return(v))
            }

            StatementKind::Break => Err(Flow::Break),
            StatementKind::Continue => Err(Flow::Continue),

            StatementKind::Print(expr) => {
                let v = self.evaluate(env, expr)?;
                let rendered = self.stringify(&v);
                writeln!(self.output, "{rendered}")
                    .map_err(|error| self.error(format!("failed to write output: {error}")))?;
                Ok(())
            }

            StatementKind::Block(statements) => {
                let block_env = Environment::child(env);
                for s in statements {
                    self.execute(&block_env, s)?;
                }
                Ok(())
            }

            StatementKind::Expr(expr) => {
                self.evaluate(env, expr)?;
                Ok(())
            }

            StatementKind::Assign { name, op, value } => {
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
            StatementKind::DocComment(_) => Ok(()),
            StatementKind::RenderBlock(statements) => {
                for statement in statements {
                    self.execute(env, statement)?;
                }
                Ok(())
            }
            StatementKind::RenderLine { expression, .. } | StatementKind::Show(expression) => {
                self.evaluate(env, expression)?;
                Ok(())
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
        self.tick()?;
        match &expr.kind {
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Nil => Ok(Value::Nil),
            ExprKind::Number(text) => self.parse_number(text),
            ExprKind::String(s) => Ok(Value::Str(s.clone())),
            ExprKind::Variable(sym) => Ok(Environment::get(env, *sym)
                .unwrap_or_else(|| Value::Symbolic(Rc::new(SymbolicExpr::Variable(*sym))))),
            ExprKind::Group(inner) => self.evaluate(env, inner),

            ExprKind::Array(elements) => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.evaluate(env, element)?);
                }
                Ok(Value::Array(Rc::new(values)))
            }

            ExprKind::Index { target, index } => {
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

            ExprKind::Unary { op, expr } => {
                let v = self.evaluate(env, expr)?;
                self.apply_unary(*op, v)
            }

            ExprKind::Binary {
                left, op, right, ..
            } => match op {
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

            ExprKind::Call { callee, args } => {
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
        if let Value::Builtin(builtin) = callee {
            return self.call_builtin(builtin, args);
        }
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
        if self.call_depth >= self.limits.max_call_depth {
            return Err(self.limit_error(format!(
                "function calls exceeded the depth limit of {}",
                self.limits.max_call_depth
            )));
        }
        self.call_depth += 1;
        let call_env = Environment::child(&f.closure);
        for (param, arg) in f.params.iter().zip(args) {
            call_env.borrow_mut().define(*param, arg);
        }
        let result = match &f.body {
            FunctionBody::Expression(expr) => self.evaluate(&call_env, expr),
            FunctionBody::Block(statements) => self.execute_function_block(&call_env, statements),
        };
        self.call_depth -= 1;
        result
    }

    fn execute_function_block(&mut self, env: &Env, statements: &[Statement]) -> ExecResult<Value> {
        for statement in statements {
            match self.execute(env, statement) {
                Ok(()) => {}
                Err(Flow::Return(value)) => return Ok(value),
                Err(Flow::Break) => return Err(self.error("'break' used outside of a loop")),
                Err(Flow::Continue) => return Err(self.error("'continue' used outside of a loop")),
                Err(error @ Flow::Error(_)) => return Err(error),
            }
        }
        Ok(Value::Nil)
    }

    fn call_builtin(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        let (minimum, maximum) = match builtin {
            Builtin::Log => (1, 2),
            Builtin::Atan2
            | Builtin::Min
            | Builtin::Max
            | Builtin::Hypot
            | Builtin::Gcd
            | Builtin::Lcm
            | Builtin::Collect
            | Builtin::Solve
            | Builtin::Derivative
            | Builtin::Integral => (2, 2),
            Builtin::Subs => (3, 3),
            _ => (1, 1),
        };
        if args.len() < minimum || args.len() > maximum {
            let expected = if minimum == maximum {
                minimum.to_string()
            } else {
                format!("{minimum} or {maximum}")
            };
            return Err(self.error(format!(
                "{} expects {expected} argument(s) but got {}",
                builtin.name(),
                args.len()
            )));
        }

        if matches!(
            builtin,
            Builtin::Simplify
                | Builtin::Expand
                | Builtin::Factor
                | Builtin::Apart
                | Builtin::Horner
        ) {
            let value = args.into_iter().next().expect("arity checked");
            let Value::Symbolic(expression) = value else {
                return Ok(value);
            };
            if builtin == Builtin::Simplify {
                return Ok(Value::Symbolic(expression));
            }
            if builtin == Builtin::Expand {
                if let Some(expanded) = self.expand_function_identity(&expression)? {
                    return Ok(Self::symbolic_value(expanded));
                }
            }
            let result = if builtin == Builtin::Apart {
                Polynomial::partial_fraction_expression(&expression, &self.symbols)
            } else {
                Polynomial::from_expression(&expression, &self.symbols).map(|polynomial| {
                    if builtin == Builtin::Factor {
                        polynomial.factored_expression()
                    } else if builtin == Builtin::Horner {
                        polynomial.horner_expression()
                    } else {
                        polynomial.expanded_expression()
                    }
                })
            };
            return Ok(result
                .map(Self::symbolic_value)
                .unwrap_or(Value::Symbolic(expression)));
        }

        if matches!(builtin, Builtin::AssumeNonzero | Builtin::AssumeNonnegative) {
            let Value::Symbolic(expression) = &args[0] else {
                return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
            };
            let SymbolicExpr::Variable(symbol) = expression.as_ref() else {
                return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
            };
            if builtin == Builtin::AssumeNonzero {
                self.nonzero_symbols.borrow_mut().insert(*symbol);
            } else {
                self.nonnegative_symbols.borrow_mut().insert(*symbol);
            }
            return Ok(args[0].clone());
        }

        if matches!(
            builtin,
            Builtin::Collect
                | Builtin::Subs
                | Builtin::Solve
                | Builtin::Derivative
                | Builtin::Integral
        ) {
            return self.call_symbolic_algebra(builtin, args);
        }

        if args.iter().any(|value| matches!(value, Value::Symbolic(_))) {
            return self.symbolic_builtin(builtin, args);
        }

        match builtin {
            Builtin::Abs => match &args[0] {
                Value::Number(number) => Ok(Value::Number(number.abs())),
                value => {
                    Err(self.error(format!("abs expects a number, got {}", value.type_name())))
                }
            },
            Builtin::Sign => match &args[0] {
                Value::Number(number) => {
                    let zero = Number::Integer(BigInt::from(0));
                    let sign = match number.compare(&zero) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    };
                    Ok(Value::Number(Number::Integer(BigInt::from(sign))))
                }
                value => {
                    Err(self.error(format!("sign expects a number, got {}", value.type_name())))
                }
            },
            Builtin::Min | Builtin::Max => {
                let (Value::Number(left), Value::Number(right)) = (&args[0], &args[1]) else {
                    return Err(self.error(format!("{} expects two numbers", builtin.name())));
                };
                let take_left = if builtin == Builtin::Min {
                    !left.compare(right).is_gt()
                } else {
                    !left.compare(right).is_lt()
                };
                Ok(if take_left {
                    args[0].clone()
                } else {
                    args[1].clone()
                })
            }
            Builtin::Floor | Builtin::Ceil | Builtin::Round | Builtin::Trunc => {
                let number = self.builtin_number(builtin, &args[0])?;
                let value = self.number_as_f64(builtin, number)?;
                let result = match builtin {
                    Builtin::Floor => value.floor(),
                    Builtin::Ceil => value.ceil(),
                    Builtin::Round => value.round(),
                    Builtin::Trunc => value.trunc(),
                    _ => unreachable!(),
                };
                self.f64_value(builtin, result)
            }
            Builtin::Log if args.len() == 2 => {
                let value = self.number_as_f64(builtin, self.builtin_number(builtin, &args[0])?)?;
                let base = self.number_as_f64(builtin, self.builtin_number(builtin, &args[1])?)?;
                if value <= 0.0 || base <= 0.0 || base == 1.0 {
                    return Err(
                        self.error("log domain requires value > 0, base > 0, and base != 1")
                    );
                }
                self.f64_value(builtin, value.log(base))
            }
            Builtin::Atan2 | Builtin::Hypot => {
                let left = self.number_as_f64(builtin, self.builtin_number(builtin, &args[0])?)?;
                let right = self.number_as_f64(builtin, self.builtin_number(builtin, &args[1])?)?;
                let result = if builtin == Builtin::Atan2 {
                    left.atan2(right)
                } else {
                    left.hypot(right)
                };
                self.f64_value(builtin, result)
            }
            Builtin::Gcd | Builtin::Lcm => {
                let (Value::Number(Number::Integer(left)), Value::Number(Number::Integer(right))) =
                    (&args[0], &args[1])
                else {
                    return Err(
                        self.error(format!("{} expects two integer arguments", builtin.name()))
                    );
                };
                let result = if builtin == Builtin::Gcd {
                    left.gcd(right)
                } else {
                    left.lcm(right)
                };
                Ok(Value::Number(Number::Integer(result)))
            }
            _ => {
                let number = self.builtin_number(builtin, &args[0])?;
                if let Some(root) = match builtin {
                    Builtin::Sqrt => number.exact_root(2),
                    Builtin::Cbrt => number.exact_root(3),
                    _ => None,
                } {
                    return Ok(Value::Number(root));
                }
                if number.is_zero() {
                    return match builtin {
                        Builtin::Sqrt
                        | Builtin::Cbrt
                        | Builtin::Sin
                        | Builtin::Tan
                        | Builtin::Asin
                        | Builtin::Atan
                        | Builtin::Sinh
                        | Builtin::Tanh
                        | Builtin::Asinh => Ok(Value::Number(Number::Integer(BigInt::from(0)))),
                        Builtin::Exp | Builtin::Cos | Builtin::Cosh | Builtin::Sec => {
                            Ok(Value::Number(Number::Integer(BigInt::from(1))))
                        }
                        _ => self.approximate_unary(builtin, number),
                    };
                }
                if number.is_one()
                    && matches!(
                        builtin,
                        Builtin::Ln | Builtin::Log | Builtin::Log10 | Builtin::Log2
                    )
                {
                    return Ok(Value::Number(Number::Integer(BigInt::from(0))));
                }
                self.approximate_unary(builtin, number)
            }
        }
    }

    fn expand_function_identity(
        &self,
        expression: &Rc<SymbolicExpr>,
    ) -> ExecResult<Option<Rc<SymbolicExpr>>> {
        let SymbolicExpr::Call { function, args } = expression.as_ref() else {
            return Ok(None);
        };
        if args.len() != 1 {
            return Ok(None);
        }
        let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Add,
            right,
        } = args[0].as_ref()
        else {
            return Ok(None);
        };
        let call = |function, argument: Rc<SymbolicExpr>| {
            Rc::new(SymbolicExpr::Call {
                function,
                args: vec![argument],
            })
        };
        let expanded = match function {
            Builtin::Exp => Self::symbolic_binary_raw(
                call(Builtin::Exp, Rc::clone(left)),
                BinaryOp::Mul,
                call(Builtin::Exp, Rc::clone(right)),
            ),
            Builtin::Sin => {
                let first = Self::symbolic_binary_raw(
                    call(Builtin::Sin, Rc::clone(left)),
                    BinaryOp::Mul,
                    call(Builtin::Cos, Rc::clone(right)),
                );
                let second = Self::symbolic_binary_raw(
                    call(Builtin::Cos, Rc::clone(left)),
                    BinaryOp::Mul,
                    call(Builtin::Sin, Rc::clone(right)),
                );
                Self::symbolic_binary_raw(first, BinaryOp::Add, second)
            }
            Builtin::Cos => {
                let first = Self::symbolic_binary_raw(
                    call(Builtin::Cos, Rc::clone(left)),
                    BinaryOp::Mul,
                    call(Builtin::Cos, Rc::clone(right)),
                );
                let second = Self::symbolic_binary_raw(
                    call(Builtin::Sin, Rc::clone(left)),
                    BinaryOp::Mul,
                    call(Builtin::Sin, Rc::clone(right)),
                );
                Self::symbolic_binary_raw(first, BinaryOp::Sub, second)
            }
            _ => return Ok(None),
        };
        if *function == Builtin::Exp {
            Ok(Some(expanded))
        } else {
            Ok(Some(self.simplify_symbolic_tree(&expanded)?))
        }
    }

    fn call_symbolic_algebra(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        let expression = self.value_to_symbolic(args[0].clone())?;
        let Value::Symbolic(variable_expression) = &args[1] else {
            return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
        };
        let SymbolicExpr::Variable(variable) = variable_expression.as_ref() else {
            return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
        };

        match builtin {
            Builtin::Subs => {
                let replacement = self.value_to_symbolic(args[2].clone())?;
                let substituted = Self::substitute_symbolic(&expression, *variable, &replacement);
                let simplified = Polynomial::from_expression(&substituted, &self.symbols)
                    .map(|polynomial| polynomial.expanded_expression())
                    .unwrap_or(substituted);
                Ok(Self::symbolic_value(simplified))
            }
            Builtin::Solve => {
                let roots = Polynomial::solve_expression(&expression, *variable, &self.symbols)
                    .unwrap_or_default();
                Ok(Value::Array(Rc::new(
                    roots.into_iter().map(Self::symbolic_value).collect(),
                )))
            }
            Builtin::Derivative => {
                let derivative = self.derivative_symbolic(&expression, *variable);
                let simplified = self.simplify_symbolic_tree(&derivative)?;
                Ok(Self::symbolic_value(simplified))
            }
            Builtin::Integral => {
                let integral = self.integral_symbolic(&expression, *variable);
                let simplified = self.simplify_symbolic_tree(&integral)?;
                Ok(Self::symbolic_value(simplified))
            }
            Builtin::Collect => {
                let collected = Polynomial::from_expression(&expression, &self.symbols)
                    .map(|polynomial| polynomial.expanded_expression())
                    .unwrap_or(expression);
                Ok(Self::symbolic_value(collected))
            }
            _ => unreachable!("symbolic algebra builtin dispatched separately"),
        }
    }

    fn substitute_symbolic(
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
        replacement: &Rc<SymbolicExpr>,
    ) -> Rc<SymbolicExpr> {
        match expression.as_ref() {
            SymbolicExpr::Variable(symbol) if *symbol == variable => Rc::clone(replacement),
            SymbolicExpr::Unary { op, expr } => Rc::new(SymbolicExpr::Unary {
                op: *op,
                expr: Self::substitute_symbolic(expr, variable, replacement),
            }),
            SymbolicExpr::Binary { left, op, right } => Rc::new(SymbolicExpr::Binary {
                left: Self::substitute_symbolic(left, variable, replacement),
                op: *op,
                right: Self::substitute_symbolic(right, variable, replacement),
            }),
            SymbolicExpr::Call { function, args } => Rc::new(SymbolicExpr::Call {
                function: *function,
                args: args
                    .iter()
                    .map(|argument| Self::substitute_symbolic(argument, variable, replacement))
                    .collect(),
            }),
            _ => Rc::clone(expression),
        }
    }

    fn symbolic_binary_raw(
        left: Rc<SymbolicExpr>,
        op: BinaryOp,
        right: Rc<SymbolicExpr>,
    ) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Binary { left, op, right })
    }

    fn simplify_symbolic_tree(
        &self,
        expression: &Rc<SymbolicExpr>,
    ) -> ExecResult<Rc<SymbolicExpr>> {
        match expression.as_ref() {
            SymbolicExpr::Unary { op, expr } => {
                let inner = self.simplify_symbolic_tree(expr)?;
                self.apply_unary(*op, Self::symbolic_value(inner))
                    .and_then(|value| self.value_to_symbolic(value))
            }
            SymbolicExpr::Binary { left, op, right } => {
                let left = self.simplify_symbolic_tree(left)?;
                let right = self.simplify_symbolic_tree(right)?;
                self.symbolic_binary(*op, Self::symbolic_value(left), Self::symbolic_value(right))
                    .and_then(|value| self.value_to_symbolic(value))
            }
            SymbolicExpr::Call { function, args } => Ok(Rc::new(SymbolicExpr::Call {
                function: *function,
                args: args
                    .iter()
                    .map(|argument| self.simplify_symbolic_tree(argument))
                    .collect::<ExecResult<Vec<_>>>()?,
            })),
            _ => Ok(Rc::clone(expression)),
        }
    }

    fn symbolic_integer(value: i64) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(value))))
    }

    fn symbolic_call(function: Builtin, argument: Rc<SymbolicExpr>) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Call {
            function,
            args: vec![argument],
        })
    }

    fn derivative_symbolic(
        &self,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Rc<SymbolicExpr> {
        use BinaryOp::{Add, Div, Mul, Pow, Sub};
        match expression.as_ref() {
            SymbolicExpr::Number(_) => Self::symbolic_integer(0),
            SymbolicExpr::Variable(symbol) => {
                Self::symbolic_integer(i64::from(*symbol == variable))
            }
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Rc::new(SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr: self.derivative_symbolic(expr, variable),
            }),
            SymbolicExpr::Binary {
                left,
                op: Add | Sub,
                right,
            } => Self::symbolic_binary_raw(
                self.derivative_symbolic(left, variable),
                *match expression.as_ref() {
                    SymbolicExpr::Binary { op, .. } => op,
                    _ => unreachable!(),
                },
                self.derivative_symbolic(right, variable),
            ),
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } => Self::symbolic_binary_raw(
                Self::symbolic_binary_raw(
                    self.derivative_symbolic(left, variable),
                    Mul,
                    Rc::clone(right),
                ),
                Add,
                Self::symbolic_binary_raw(
                    Rc::clone(left),
                    Mul,
                    self.derivative_symbolic(right, variable),
                ),
            ),
            SymbolicExpr::Binary {
                left,
                op: Div,
                right,
            } => Self::symbolic_binary_raw(
                Self::symbolic_binary_raw(
                    Self::symbolic_binary_raw(
                        self.derivative_symbolic(left, variable),
                        Mul,
                        Rc::clone(right),
                    ),
                    Sub,
                    Self::symbolic_binary_raw(
                        Rc::clone(left),
                        Mul,
                        self.derivative_symbolic(right, variable),
                    ),
                ),
                Div,
                Self::symbolic_binary_raw(Rc::clone(right), Pow, Self::symbolic_integer(2)),
            ),
            SymbolicExpr::Binary {
                left,
                op: Pow,
                right,
            } => {
                if let SymbolicExpr::Number(Number::Integer(exponent)) = right.as_ref() {
                    let reduced = exponent - 1;
                    Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Rc::new(SymbolicExpr::Number(Number::Integer(exponent.clone()))),
                            Mul,
                            Self::symbolic_binary_raw(
                                Rc::clone(left),
                                Pow,
                                Rc::new(SymbolicExpr::Number(Number::Integer(reduced))),
                            ),
                        ),
                        Mul,
                        self.derivative_symbolic(left, variable),
                    )
                } else {
                    // d(f^g) = f^g (g' ln(f) + g f'/f)
                    Self::symbolic_binary_raw(
                        Rc::clone(expression),
                        Mul,
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(
                                self.derivative_symbolic(right, variable),
                                Mul,
                                Self::symbolic_call(Builtin::Ln, Rc::clone(left)),
                            ),
                            Add,
                            Self::symbolic_binary_raw(
                                Rc::clone(right),
                                Mul,
                                Self::symbolic_binary_raw(
                                    self.derivative_symbolic(left, variable),
                                    Div,
                                    Rc::clone(left),
                                ),
                            ),
                        ),
                    )
                }
            }
            SymbolicExpr::Call { function, args } if args.len() == 1 => {
                let inner = &args[0];
                let call = |function| Self::symbolic_call(function, Rc::clone(inner));
                let square =
                    |value| Self::symbolic_binary_raw(value, Pow, Self::symbolic_integer(2));
                let outer = match function {
                    Builtin::Sin => call(Builtin::Cos),
                    Builtin::Tan => square(call(Builtin::Sec)),
                    Builtin::Cos => Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: call(Builtin::Sin),
                    }),
                    Builtin::Sec => {
                        Self::symbolic_binary_raw(call(Builtin::Sec), Mul, call(Builtin::Tan))
                    }
                    Builtin::Csc => Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: Self::symbolic_binary_raw(
                            call(Builtin::Csc),
                            Mul,
                            call(Builtin::Cot),
                        ),
                    }),
                    Builtin::Cot => Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: square(call(Builtin::Csc)),
                    }),
                    Builtin::Exp => Rc::clone(expression),
                    Builtin::Ln | Builtin::Log => {
                        Self::symbolic_binary_raw(Self::symbolic_integer(1), Div, Rc::clone(inner))
                    }
                    Builtin::Sqrt => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(2),
                            Mul,
                            Rc::clone(expression),
                        ),
                    ),
                    Builtin::Asin | Builtin::Acos => {
                        let denominator = Self::symbolic_call(
                            Builtin::Sqrt,
                            Self::symbolic_binary_raw(
                                Self::symbolic_integer(1),
                                Sub,
                                square(Rc::clone(inner)),
                            ),
                        );
                        let result =
                            Self::symbolic_binary_raw(Self::symbolic_integer(1), Div, denominator);
                        if *function == Builtin::Acos {
                            Rc::new(SymbolicExpr::Unary {
                                op: UnaryOp::Neg,
                                expr: result,
                            })
                        } else {
                            result
                        }
                    }
                    Builtin::Atan => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(1),
                            Add,
                            square(Rc::clone(inner)),
                        ),
                    ),
                    Builtin::Sinh => call(Builtin::Cosh),
                    Builtin::Cosh => call(Builtin::Sinh),
                    Builtin::Tanh => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        square(call(Builtin::Cosh)),
                    ),
                    Builtin::Asinh => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        Self::symbolic_call(
                            Builtin::Sqrt,
                            Self::symbolic_binary_raw(
                                square(Rc::clone(inner)),
                                Add,
                                Self::symbolic_integer(1),
                            ),
                        ),
                    ),
                    Builtin::Acosh => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        Self::symbolic_binary_raw(
                            Self::symbolic_call(
                                Builtin::Sqrt,
                                Self::symbolic_binary_raw(
                                    Rc::clone(inner),
                                    Sub,
                                    Self::symbolic_integer(1),
                                ),
                            ),
                            Mul,
                            Self::symbolic_call(
                                Builtin::Sqrt,
                                Self::symbolic_binary_raw(
                                    Rc::clone(inner),
                                    Add,
                                    Self::symbolic_integer(1),
                                ),
                            ),
                        ),
                    ),
                    Builtin::Atanh => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(1),
                            Sub,
                            square(Rc::clone(inner)),
                        ),
                    ),
                    _ => {
                        return self.unevaluated_calculus(Builtin::Derivative, expression, variable)
                    }
                };
                Self::symbolic_binary_raw(outer, Mul, self.derivative_symbolic(inner, variable))
            }
            _ => self.unevaluated_calculus(Builtin::Derivative, expression, variable),
        }
    }

    fn integral_symbolic(
        &self,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Rc<SymbolicExpr> {
        use BinaryOp::{Add, Div, Mul, Pow, Sub};
        if !Self::contains_variable(expression, variable) {
            return Self::symbolic_binary_raw(
                Rc::clone(expression),
                Mul,
                Rc::new(SymbolicExpr::Variable(variable)),
            );
        }
        match expression.as_ref() {
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Rc::new(SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr: self.integral_symbolic(expr, variable),
            }),
            SymbolicExpr::Variable(symbol) if *symbol == variable => Self::symbolic_binary_raw(
                Self::symbolic_binary_raw(Rc::clone(expression), Pow, Self::symbolic_integer(2)),
                Div,
                Self::symbolic_integer(2),
            ),
            SymbolicExpr::Binary {
                left,
                op: Add | Sub,
                right,
            } => Self::symbolic_binary_raw(
                self.integral_symbolic(left, variable),
                *match expression.as_ref() {
                    SymbolicExpr::Binary { op, .. } => op,
                    _ => unreachable!(),
                },
                self.integral_symbolic(right, variable),
            ),
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } if !Self::contains_variable(left, variable) => Self::symbolic_binary_raw(
                Rc::clone(left),
                Mul,
                self.integral_symbolic(right, variable),
            ),
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } if !Self::contains_variable(right, variable) => Self::symbolic_binary_raw(
                Rc::clone(right),
                Mul,
                self.integral_symbolic(left, variable),
            ),
            SymbolicExpr::Binary {
                left,
                op: Div,
                right,
            } if !Self::contains_variable(right, variable) => Self::symbolic_binary_raw(
                self.integral_symbolic(left, variable),
                Div,
                Rc::clone(right),
            ),
            SymbolicExpr::Binary {
                left,
                op: Pow,
                right,
            } if Self::linear_coefficient(left, variable).is_some() => {
                if let SymbolicExpr::Number(Number::Integer(exponent)) = right.as_ref() {
                    let coefficient =
                        Self::linear_coefficient(left, variable).expect("guarded above");
                    if exponent == &BigInt::from(-1) {
                        return Self::symbolic_binary_raw(
                            Self::symbolic_call(Builtin::Ln, Rc::clone(left)),
                            Div,
                            coefficient,
                        );
                    }
                    let next = exponent + BigInt::from(1);
                    return Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(
                                Rc::clone(left),
                                Pow,
                                Rc::new(SymbolicExpr::Number(Number::Integer(next.clone()))),
                            ),
                            Div,
                            Rc::new(SymbolicExpr::Number(Number::Integer(next))),
                        ),
                        Div,
                        coefficient,
                    );
                }
                self.unevaluated_calculus(Builtin::Integral, expression, variable)
            }
            SymbolicExpr::Binary {
                left,
                op: Div,
                right,
            } if matches!(left.as_ref(), SymbolicExpr::Number(number) if number.is_one())
                && Self::linear_coefficient(right, variable).is_some() =>
            {
                Self::symbolic_binary_raw(
                    Self::symbolic_call(Builtin::Ln, Rc::clone(right)),
                    Div,
                    Self::linear_coefficient(right, variable).expect("guarded above"),
                )
            }
            SymbolicExpr::Call { function, args }
                if args.len() == 1
                    && Self::linear_coefficient(&args[0], variable).is_some()
                    && matches!(
                        function,
                        Builtin::Exp | Builtin::Sin | Builtin::Cos | Builtin::Tan
                    ) =>
            {
                let inner = Rc::clone(&args[0]);
                let primitive = match function {
                    Builtin::Exp => Rc::clone(expression),
                    Builtin::Sin => Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: Self::symbolic_call(Builtin::Cos, Rc::clone(&inner)),
                    }),
                    Builtin::Cos => Self::symbolic_call(Builtin::Sin, Rc::clone(&inner)),
                    Builtin::Tan => Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: Self::symbolic_call(
                            Builtin::Ln,
                            Self::symbolic_call(Builtin::Cos, Rc::clone(&inner)),
                        ),
                    }),
                    _ => unreachable!(),
                };
                Self::symbolic_binary_raw(
                    primitive,
                    Div,
                    Self::linear_coefficient(&inner, variable).expect("guarded above"),
                )
            }
            _ => self.unevaluated_calculus(Builtin::Integral, expression, variable),
        }
    }

    /// Return the constant derivative of an affine expression, or `None` when
    /// the expression is not provably linear in `variable`.
    fn linear_coefficient(
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Option<Rc<SymbolicExpr>> {
        use BinaryOp::{Add, Div, Mul, Sub};
        match expression.as_ref() {
            SymbolicExpr::Variable(symbol) if *symbol == variable => {
                Some(Self::symbolic_integer(1))
            }
            expression if !Self::contains_variable(expression, variable) => None,
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(Rc::new(SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr: Self::linear_coefficient(expr, variable)?,
            })),
            SymbolicExpr::Binary {
                left,
                op: op @ (Add | Sub),
                right,
            } => {
                let left_has = Self::contains_variable(left, variable);
                let right_has = Self::contains_variable(right, variable);
                match (left_has, right_has) {
                    (true, false) => Self::linear_coefficient(left, variable),
                    (false, true) if *op == Add => Self::linear_coefficient(right, variable),
                    (false, true) => Some(Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: Self::linear_coefficient(right, variable)?,
                    })),
                    _ => None,
                }
            }
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } if !Self::contains_variable(left, variable) => Some(Self::symbolic_binary_raw(
                Rc::clone(left),
                Mul,
                Self::linear_coefficient(right, variable)?,
            )),
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } if !Self::contains_variable(right, variable) => Some(Self::symbolic_binary_raw(
                Rc::clone(right),
                Mul,
                Self::linear_coefficient(left, variable)?,
            )),
            SymbolicExpr::Binary {
                left,
                op: Div,
                right,
            } if !Self::contains_variable(right, variable) => Some(Self::symbolic_binary_raw(
                Self::linear_coefficient(left, variable)?,
                Div,
                Rc::clone(right),
            )),
            _ => None,
        }
    }

    fn contains_variable(expression: &SymbolicExpr, variable: Symbol) -> bool {
        match expression {
            SymbolicExpr::Variable(symbol) => *symbol == variable,
            SymbolicExpr::Unary { expr, .. } => Self::contains_variable(expr, variable),
            SymbolicExpr::Binary { left, right, .. } => {
                Self::contains_variable(left, variable) || Self::contains_variable(right, variable)
            }
            SymbolicExpr::Call { args, .. } => args
                .iter()
                .any(|arg| Self::contains_variable(arg, variable)),
            SymbolicExpr::Number(_) => false,
        }
    }

    fn unevaluated_calculus(
        &self,
        operation: Builtin,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Call {
            function: operation,
            args: vec![
                Rc::clone(expression),
                Rc::new(SymbolicExpr::Variable(variable)),
            ],
        })
    }

    fn builtin_number<'a>(&self, builtin: Builtin, value: &'a Value) -> ExecResult<&'a Number> {
        let Value::Number(number) = value else {
            return Err(self.error(format!(
                "{} expects numeric arguments, got {}",
                builtin.name(),
                value.type_name()
            )));
        };
        Ok(number)
    }

    fn number_as_f64(&self, builtin: Builtin, number: &Number) -> ExecResult<f64> {
        number.to_f64().ok_or_else(|| {
            self.error(format!(
                "{} argument is outside the supported floating-point range",
                builtin.name()
            ))
        })
    }

    fn f64_value(&self, builtin: Builtin, value: f64) -> ExecResult<Value> {
        Number::from_f64(value).map(Value::Number).ok_or_else(|| {
            self.error(format!(
                "{} result is outside the real-number domain",
                builtin.name()
            ))
        })
    }

    fn approximate_unary(&self, builtin: Builtin, number: &Number) -> ExecResult<Value> {
        let value = self.number_as_f64(builtin, number)?;
        let mut result = match builtin {
            Builtin::Sqrt if value >= 0.0 => value.sqrt(),
            Builtin::Cbrt => value.cbrt(),
            Builtin::Exp => value.exp(),
            Builtin::Ln | Builtin::Log if value > 0.0 => value.ln(),
            Builtin::Log10 if value > 0.0 => value.log10(),
            Builtin::Log2 if value > 0.0 => value.log2(),
            Builtin::Sin => value.sin(),
            Builtin::Cos => value.cos(),
            Builtin::Tan if value.cos().abs() >= 1e-15 => value.tan(),
            Builtin::Sec if value.cos().abs() >= 1e-15 => 1.0 / value.cos(),
            Builtin::Csc if value.sin().abs() >= 1e-15 => 1.0 / value.sin(),
            Builtin::Cot if value.sin().abs() >= 1e-15 => 1.0 / value.tan(),
            Builtin::Asin if (-1.0..=1.0).contains(&value) => value.asin(),
            Builtin::Acos if (-1.0..=1.0).contains(&value) => value.acos(),
            Builtin::Atan => value.atan(),
            Builtin::Sinh => value.sinh(),
            Builtin::Cosh => value.cosh(),
            Builtin::Tanh => value.tanh(),
            Builtin::Asinh => value.asinh(),
            Builtin::Acosh if value >= 1.0 => value.acosh(),
            Builtin::Atanh if (-1.0..1.0).contains(&value) => value.atanh(),
            Builtin::Erf => libm::erf(value),
            Builtin::Erfc => libm::erfc(value),
            Builtin::Gamma => libm::tgamma(value),
            Builtin::Lgamma => libm::lgamma(value),
            Builtin::Degrees => value.to_degrees(),
            Builtin::Radians => value.to_radians(),
            _ => {
                return Err(self.error(format!(
                    "{} argument {number} is outside the real-number domain",
                    builtin.name()
                )))
            }
        };
        if matches!(
            builtin,
            Builtin::Sin | Builtin::Cos | Builtin::Tan | Builtin::Sec | Builtin::Csc | Builtin::Cot
        ) {
            if result.abs() < 1e-15 {
                result = 0.0;
            } else if (result - 1.0).abs() < 1e-15 {
                result = 1.0;
            } else if (result + 1.0).abs() < 1e-15 {
                result = -1.0;
            }
        }
        self.f64_value(builtin, result)
    }

    fn symbolic_builtin(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        let mut symbolic_args = Vec::with_capacity(args.len());
        for argument in args {
            symbolic_args.push(self.value_to_symbolic(argument)?);
        }
        let argument = symbolic_args.first().cloned();
        if let Some(argument) = argument {
            if matches!(builtin, Builtin::Ln | Builtin::Log) {
                if let SymbolicExpr::Call {
                    function: Builtin::Exp,
                    args,
                } = argument.as_ref()
                {
                    if args.len() == 1 {
                        return Ok(Self::symbolic_value(Rc::clone(&args[0])));
                    }
                }
            }
            if builtin == Builtin::Exp {
                if let SymbolicExpr::Call {
                    function: Builtin::Ln | Builtin::Log,
                    args,
                } = argument.as_ref()
                {
                    if args.len() == 1 {
                        return Ok(Self::symbolic_value(Rc::clone(&args[0])));
                    }
                }
            }
            if matches!(
                builtin,
                Builtin::Sin | Builtin::Tan | Builtin::Csc | Builtin::Cot | Builtin::Erf
            ) {
                if let SymbolicExpr::Unary {
                    op: UnaryOp::Neg,
                    expr,
                } = argument.as_ref()
                {
                    return self.apply_unary(
                        UnaryOp::Neg,
                        Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                            function: builtin,
                            args: vec![Rc::clone(expr)],
                        })),
                    );
                }
            }
            if matches!(builtin, Builtin::Cos | Builtin::Sec) {
                if let SymbolicExpr::Unary {
                    op: UnaryOp::Neg,
                    expr,
                } = argument.as_ref()
                {
                    return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                        function: builtin,
                        args: vec![Rc::clone(expr)],
                    })));
                }
            }
            if builtin == Builtin::Sqrt {
                if let SymbolicExpr::Binary {
                    left,
                    op: BinaryOp::Pow,
                    right,
                } = argument.as_ref()
                {
                    if matches!(right.as_ref(), SymbolicExpr::Number(Number::Integer(value)) if value == &BigInt::from(2))
                    {
                        if self.symbolic_nonnegative(left) {
                            return Ok(Self::symbolic_value(Rc::clone(left)));
                        }
                        return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                            function: Builtin::Abs,
                            args: vec![Rc::clone(left)],
                        })));
                    }
                }
            }
            if builtin == Builtin::Abs {
                if self.symbolic_nonnegative(&argument) {
                    return Ok(Self::symbolic_value(argument));
                }
                if let SymbolicExpr::Unary {
                    op: UnaryOp::Neg,
                    expr,
                } = argument.as_ref()
                {
                    return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                        function: Builtin::Abs,
                        args: vec![Rc::clone(expr)],
                    })));
                }
            }
        }
        Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
            function: builtin,
            args: symbolic_args,
        })))
    }

    fn symbolic_nonnegative(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(number) => !number.is_negative(),
            SymbolicExpr::Variable(symbol) => self.nonnegative_symbols.borrow().contains(symbol),
            SymbolicExpr::Binary {
                op: BinaryOp::Pow,
                right,
                ..
            } => {
                matches!(right.as_ref(), SymbolicExpr::Number(Number::Integer(value)) if value.is_even())
            }
            _ => false,
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
        let decimal = text
            .parse::<BigDecimal>()
            .map_err(|_| self.error(format!("invalid number literal '{text}'")))?;
        let (_, scale) = decimal.as_bigint_and_exponent();
        if scale.unsigned_abs() > self.limits.max_decimal_scale {
            return Err(self.limit_error(format!(
                "decimal scale exceeds the limit of {}",
                self.limits.max_decimal_scale
            )));
        }
        Ok(Value::Number(Number::Decimal(decimal)))
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
        if let (Value::Number(left), Value::Number(right)) = (&l, &r) {
            let estimated_bits = match op {
                Add | Sub => Some(Self::number_bits(left).max(Self::number_bits(right)) + 1),
                Mul | Div | Mod => Some(Self::number_bits(left) + Self::number_bits(right)),
                _ => None,
            };
            if estimated_bits.is_some_and(|bits| bits > self.limits.max_number_bits) {
                return Err(self.limit_error(format!(
                    "numeric result exceeds the {} bit limit",
                    self.limits.max_number_bits
                )));
            }
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
            self.limit_error(format!(
                "exponent magnitude exceeds the limit of {}",
                self.limits.max_exponent
            ))
        })?;
        if exponent.unsigned_abs() > u64::from(self.limits.max_exponent) {
            return Err(self.limit_error(format!(
                "exponent magnitude exceeds the limit of {}",
                self.limits.max_exponent
            )));
        }
        if Self::number_bits(&base).saturating_mul(exponent.unsigned_abs())
            > self.limits.max_number_bits
        {
            return Err(self.limit_error(format!(
                "numeric result exceeds the {} bit limit",
                self.limits.max_number_bits
            )));
        }
        if exponent >= 0 {
            let exponent = u32::try_from(exponent).expect("bounded exponent fits in u32");
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
                let positive = u32::try_from(exponent.unsigned_abs())
                    .expect("bounded exponent magnitude fits in u32");
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
                if amount > self.limits.max_shift {
                    return Err(self.limit_error(format!(
                        "shift amount exceeds the limit of {}",
                        self.limits.max_shift
                    )));
                }
                if a.bits().saturating_add(amount as u64) > self.limits.max_number_bits {
                    return Err(self.limit_error(format!(
                        "numeric result exceeds the {} bit limit",
                        self.limits.max_number_bits
                    )));
                }
                Ok(Value::Number(Number::Integer(op(a, amount))))
            }
            (l, r) => Err(self.type_error(sym, &l, &r)),
        }
    }

    fn number_bits(number: &Number) -> u64 {
        match number {
            Number::Integer(value) => value.bits(),
            Number::Rational(value) => value.numer().bits().saturating_add(value.denom().bits()),
            Number::Decimal(value) => value.as_bigint_and_exponent().0.bits(),
        }
    }

    fn symbolic_binary(&self, op: BinaryOp, left: Value, right: Value) -> ExecResult<Value> {
        let left = self.value_to_symbolic(left)?;
        let right = self.value_to_symbolic(right)?;

        if op == BinaryOp::Add {
            if let (Some((left_fn, left_arg)), Some((right_fn, right_arg))) = (
                Self::squared_unary_call(&left),
                Self::squared_unary_call(&right),
            ) {
                if left_arg.equals(&right_arg)
                    && matches!(
                        (left_fn, right_fn),
                        (Builtin::Sin, Builtin::Cos) | (Builtin::Cos, Builtin::Sin)
                    )
                {
                    return Ok(Value::Number(Number::Integer(BigInt::from(1))));
                }
            }
            let tangent = if Self::symbolic_is_one(&left) {
                Self::squared_unary_call(&right)
            } else if Self::symbolic_is_one(&right) {
                Self::squared_unary_call(&left)
            } else {
                None
            };
            if let Some((Builtin::Tan, argument)) = tangent {
                return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Binary {
                    left: Rc::new(SymbolicExpr::Call {
                        function: Builtin::Sec,
                        args: vec![argument],
                    }),
                    op: BinaryOp::Pow,
                    right: Self::symbolic_integer(2),
                })));
            }
        }

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
            BinaryOp::Div if Self::symbolic_is_zero(&right) => {
                return Err(self.error("division by zero in '/'"))
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

        if op == BinaryOp::Div {
            if let SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } = right.as_ref()
            {
                let numerator = self.apply_unary(UnaryOp::Neg, Self::symbolic_value(left))?;
                return self.symbolic_binary(
                    BinaryOp::Div,
                    numerator,
                    Self::symbolic_value(Rc::clone(expr)),
                );
            }
            if let SymbolicExpr::Number(number) = right.as_ref() {
                if number.is_negative() {
                    let numerator = self.apply_unary(UnaryOp::Neg, Self::symbolic_value(left))?;
                    return self.symbolic_binary(
                        BinaryOp::Div,
                        numerator,
                        Value::Number(number.abs()),
                    );
                }
            }
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

            if let Some(value) = self.combine_symbolic_fractions(op, &left, &right)? {
                return Ok(value);
            }
        }

        if op == BinaryOp::Mul {
            if let (
                SymbolicExpr::Call {
                    function: Builtin::Exp,
                    args: left_args,
                },
                SymbolicExpr::Call {
                    function: Builtin::Exp,
                    args: right_args,
                },
            ) = (left.as_ref(), right.as_ref())
            {
                if left_args.len() == 1 && right_args.len() == 1 {
                    let exponent = self.symbolic_binary(
                        BinaryOp::Add,
                        Self::symbolic_value(Rc::clone(&left_args[0])),
                        Self::symbolic_value(Rc::clone(&right_args[0])),
                    )?;
                    return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                        function: Builtin::Exp,
                        args: vec![self.value_to_symbolic(exponent)?],
                    })));
                }
            }
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
            if left_exponent == right_exponent && left_exponent != BigInt::from(1) {
                let product = self.symbolic_binary(
                    BinaryOp::Mul,
                    Self::symbolic_value(left_base),
                    Self::symbolic_value(right_base),
                )?;
                return Ok(Self::make_integer_power(
                    self.value_to_symbolic(product)?,
                    left_exponent,
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
            if Self::is_symbolic_integer(&right, 2) {
                if let SymbolicExpr::Call {
                    function: Builtin::Abs,
                    args,
                } = left.as_ref()
                {
                    if args.len() == 1 {
                        return Ok(Self::make_integer_power(
                            Rc::clone(&args[0]),
                            BigInt::from(2),
                        ));
                    }
                }
            }
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

        if op == BinaryOp::Div {
            if let Some((numerator, denominator)) =
                Polynomial::reduce_fraction(&left, &right, &self.symbols)
            {
                if Self::symbolic_is_one(&denominator) {
                    return Ok(Self::symbolic_value(numerator));
                }
                return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Binary {
                    left: numerator,
                    op: BinaryOp::Div,
                    right: denominator,
                })));
            }
            let (left_base, left_exponent) = Self::integer_power_parts(&left);
            let (right_base, right_exponent) = Self::integer_power_parts(&right);
            if left_base.equals(&right_base) {
                return Ok(Self::make_integer_power(
                    left_base,
                    left_exponent - right_exponent,
                ));
            }
            if let Some(cancelled) = self.cancel_symbolic_factors(&left, &right) {
                return Ok(Self::symbolic_value(cancelled));
            }
        }

        if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul) {
            let expression = SymbolicExpr::Binary {
                left: Rc::clone(&left),
                op,
                right: Rc::clone(&right),
            };
            if let Some(polynomial) = Polynomial::from_expression(&expression, &self.symbols) {
                if op != BinaryOp::Mul || polynomial.term_count() == 1 {
                    return Ok(Self::symbolic_value(polynomial.expanded_expression()));
                }
            }
        }

        if matches!(op, BinaryOp::Add | BinaryOp::Mul) {
            return Ok(Self::symbolic_value(
                self.canonical_commutative(op, left, right),
            ));
        }

        Ok(Value::Symbolic(Rc::new(SymbolicExpr::Binary {
            left,
            op,
            right,
        })))
    }

    fn is_symbolic_integer(expression: &SymbolicExpr, expected: i64) -> bool {
        matches!(expression, SymbolicExpr::Number(Number::Integer(value)) if value == &BigInt::from(expected))
    }

    fn squared_unary_call(expression: &SymbolicExpr) -> Option<(Builtin, Rc<SymbolicExpr>)> {
        let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Pow,
            right,
        } = expression
        else {
            return None;
        };
        if !Self::is_symbolic_integer(right, 2) {
            return None;
        }
        let SymbolicExpr::Call { function, args } = left.as_ref() else {
            return None;
        };
        (args.len() == 1).then(|| (*function, Rc::clone(&args[0])))
    }

    fn canonical_commutative(
        &self,
        op: BinaryOp,
        left: Rc<SymbolicExpr>,
        right: Rc<SymbolicExpr>,
    ) -> Rc<SymbolicExpr> {
        fn collect(
            expression: Rc<SymbolicExpr>,
            op: BinaryOp,
            operands: &mut Vec<Rc<SymbolicExpr>>,
        ) {
            if let SymbolicExpr::Binary {
                left,
                op: expression_op,
                right,
            } = expression.as_ref()
            {
                if *expression_op == op {
                    collect(Rc::clone(left), op, operands);
                    collect(Rc::clone(right), op, operands);
                    return;
                }
            }
            operands.push(expression);
        }

        let mut operands = Vec::new();
        collect(left, op, &mut operands);
        collect(right, op, &mut operands);
        operands.sort_by_key(|operand| self.format_symbolic(operand, 0));
        operands
            .into_iter()
            .reduce(|left, right| Rc::new(SymbolicExpr::Binary { left, op, right }))
            .expect("two operands were supplied")
    }

    fn combine_symbolic_fractions(
        &self,
        op: BinaryOp,
        left: &Rc<SymbolicExpr>,
        right: &Rc<SymbolicExpr>,
    ) -> ExecResult<Option<Value>> {
        let (left_numerator, left_denominator) = Self::fraction_parts(left);
        let (right_numerator, right_denominator) = Self::fraction_parts(right);
        if Self::symbolic_is_one(&left_denominator) && Self::symbolic_is_one(&right_denominator) {
            return Ok(None);
        }
        let left_cross = self.symbolic_binary(
            BinaryOp::Mul,
            Self::symbolic_value(left_numerator),
            Self::symbolic_value(Rc::clone(&right_denominator)),
        )?;
        let right_cross = self.symbolic_binary(
            BinaryOp::Mul,
            Self::symbolic_value(right_numerator),
            Self::symbolic_value(Rc::clone(&left_denominator)),
        )?;
        let numerator = self.symbolic_binary(op, left_cross, right_cross)?;
        let denominator = self.symbolic_binary(
            BinaryOp::Mul,
            Self::symbolic_value(left_denominator),
            Self::symbolic_value(right_denominator),
        )?;
        Ok(Some(self.symbolic_binary(
            BinaryOp::Div,
            numerator,
            denominator,
        )?))
    }

    fn fraction_parts(expression: &Rc<SymbolicExpr>) -> (Rc<SymbolicExpr>, Rc<SymbolicExpr>) {
        if let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Div,
            right,
        } = expression.as_ref()
        {
            return (Rc::clone(left), Rc::clone(right));
        }
        (
            Rc::clone(expression),
            Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(1)))),
        )
    }

    fn cancel_symbolic_factors(
        &self,
        numerator: &Rc<SymbolicExpr>,
        denominator: &Rc<SymbolicExpr>,
    ) -> Option<Rc<SymbolicExpr>> {
        let mut numerator_factors = Vec::new();
        let mut denominator_factors = Vec::new();
        Self::collect_product_factors(numerator, &mut numerator_factors);
        Self::collect_product_factors(denominator, &mut denominator_factors);
        let original_count = numerator_factors.len() + denominator_factors.len();

        let mut index = 0;
        while index < numerator_factors.len() {
            if let Some(match_index) = denominator_factors
                .iter()
                .position(|factor| numerator_factors[index].equals(factor))
            {
                numerator_factors.remove(index);
                denominator_factors.remove(match_index);
            } else {
                index += 1;
            }
        }
        if original_count == numerator_factors.len() + denominator_factors.len() {
            return None;
        }
        let numerator = Self::product_from_factors(numerator_factors);
        let denominator = Self::product_from_factors(denominator_factors);
        if Self::symbolic_is_one(&denominator) {
            Some(numerator)
        } else {
            Some(Rc::new(SymbolicExpr::Binary {
                left: numerator,
                op: BinaryOp::Div,
                right: denominator,
            }))
        }
    }

    fn collect_product_factors(expression: &Rc<SymbolicExpr>, factors: &mut Vec<Rc<SymbolicExpr>>) {
        if let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Mul,
            right,
        } = expression.as_ref()
        {
            Self::collect_product_factors(left, factors);
            Self::collect_product_factors(right, factors);
        } else {
            factors.push(Rc::clone(expression));
        }
    }

    fn product_from_factors(mut factors: Vec<Rc<SymbolicExpr>>) -> Rc<SymbolicExpr> {
        if factors.is_empty() {
            return Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(1))));
        }
        factors.sort_by_key(|factor| format!("{factor:?}"));
        factors
            .into_iter()
            .reduce(|left, right| {
                Rc::new(SymbolicExpr::Binary {
                    left,
                    op: BinaryOp::Mul,
                    right,
                })
            })
            .expect("non-empty factors")
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
            Value::Builtin(builtin) => format!("<builtin {}>", builtin.name()),
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
            SymbolicExpr::Call { function, args } => format!(
                "{}({})",
                function.name(),
                args.iter()
                    .map(|argument| self.format_symbolic(argument, 0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn format_expr(&self, expression: &Expr, parent_precedence: u8) -> String {
        match &expression.kind {
            ExprKind::Bool(value) => value.to_string(),
            ExprKind::Nil => "nil".to_string(),
            ExprKind::Number(value) => value.clone(),
            ExprKind::String(value) => format!("\"{value}\""),
            ExprKind::Variable(symbol) => self.symbols.resolve(*symbol).to_string(),
            ExprKind::Group(inner) => format!("({})", self.format_expr(inner, 0)),
            ExprKind::Array(elements) => format!(
                "[{}]",
                elements
                    .iter()
                    .map(|item| self.format_expr(item, 0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ExprKind::Index { target, index } => {
                format!(
                    "{}[{}]",
                    self.format_expr(target, 13),
                    self.format_expr(index, 0)
                )
            }
            ExprKind::Call { callee, args } => format!(
                "{}({})",
                self.format_expr(callee, 13),
                args.iter()
                    .map(|arg| self.format_expr(arg, 0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ExprKind::Unary { op, expr } => {
                let text = format!("{}{}", Self::unary_text(*op), self.format_expr(expr, 12));
                if 12 < parent_precedence {
                    format!("({text})")
                } else {
                    text
                }
            }
            ExprKind::Binary {
                left, op, right, ..
            } => {
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
