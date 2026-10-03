use crate::ast::assign_op::AssignOp;
use crate::ast::binary_op::BinaryOp;
use crate::ast::expr::{Expr, ExprKind};
use crate::ast::span::SourceSpan;
use crate::ast::statement::FunctionBody;
use crate::ast::statement::{Statement, StatementKind};
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::environment::{Env, Environment};
use crate::interpreter::flow::{ExecResult, Flow};
use crate::interpreter::polynomial::Polynomial;
use crate::interpreter::value::{
    Builtin, ConstraintFact, DerivationStep, StepOperation, SymbolicExpr, UserFunction, Value,
};
use crate::lexer::symbol::{Symbol, SymbolTable};
use crate::number::{BinaryMathFunction, ComplexNumber, MathConstant, Number, UnaryMathFunction};
use crate::{Limits, RuntimeError};
use bigdecimal::{BigDecimal, Context};
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write;
use std::num::NonZeroU64;
use std::rc::Rc;

type Matrix = Vec<Vec<Value>>;
type Reduction = (Matrix, Vec<DerivationStep>, usize);

/// Replaces every free occurrence of `symbol` in `expression` with
/// `replacement`, returning `None` when nothing matched.
fn rewrite_variable(
    expression: &Rc<SymbolicExpr>,
    symbol: Symbol,
    replacement: &Rc<SymbolicExpr>,
) -> Option<Rc<SymbolicExpr>> {
    match expression.as_ref() {
        SymbolicExpr::Variable(found) if *found == symbol => Some(Rc::clone(replacement)),
        SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => None,
        SymbolicExpr::Unary { op, expr } => {
            let inner = rewrite_variable(expr, symbol, replacement)?;
            Some(Rc::new(SymbolicExpr::Unary {
                op: *op,
                expr: inner,
            }))
        }
        SymbolicExpr::Binary { left, op, right } => {
            let new_left = rewrite_variable(left, symbol, replacement);
            let new_right = rewrite_variable(right, symbol, replacement);
            if new_left.is_none() && new_right.is_none() {
                return None;
            }
            Some(Rc::new(SymbolicExpr::Binary {
                left: new_left.unwrap_or_else(|| Rc::clone(left)),
                op: *op,
                right: new_right.unwrap_or_else(|| Rc::clone(right)),
            }))
        }
        SymbolicExpr::Call { function, args } => {
            let rewritten = args
                .iter()
                .map(|argument| rewrite_variable(argument, symbol, replacement))
                .collect::<Option<Vec<_>>>()?;
            Some(Rc::new(SymbolicExpr::Call {
                function: *function,
                args: rewritten,
            }))
        }
        SymbolicExpr::Variable(_) => None,
    }
}

/// Whether a symbolic expression is even or odd in a variable, used for the
/// definite-integral symmetry shortcuts (`∫_{-a}^{a}`).
enum Parity {
    Even,
    Odd,
}

/// Greatest polynomial degree `integrate_by_parts` will expand. The tabular
/// rule runs one integration step per degree, so this bounds the rewrite:
/// larger products stay unevaluated instead of exploding.
const MAX_BY_PARTS_DEGREE: u32 = 8;

/// Cap on the number of monomial terms `polynomial_terms` will expand a
/// product to before giving up on the by-parts rule.
const MAX_POLYNOMIAL_TERMS: usize = 16;

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
    positive_symbols: RefCell<HashSet<Symbol>>,
    negative_symbols: RefCell<HashSet<Symbol>>,
    integer_symbols: RefCell<HashSet<Symbol>>,
    real_symbols: RefCell<HashSet<Symbol>>,
    /// Facts collected from `assume(constraint)` calls, such as `x == 3`,
    /// `x > 0 and x < 10`, or `x != 0`. They generalize the per-variable
    /// assumption sets: equality facts substitute directly, inequality
    /// facts sharpen the range predicates the simplifier already consults.
    constraints: RefCell<Vec<ConstraintFact>>,
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
            positive_symbols: RefCell::new(HashSet::new()),
            negative_symbols: RefCell::new(HashSet::new()),
            integer_symbols: RefCell::new(HashSet::new()),
            real_symbols: RefCell::new(HashSet::new()),
            constraints: RefCell::new(Vec::new()),
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
        // Unicode spellings used by the GUI remain executable when a source
        // file is later run through the CLI. Uppercase gamma follows the
        // conventional notation for the Gamma function; lowercase gamma is
        // left available as an ordinary symbolic variable.
        for (name, builtin) in [("Γ", Builtin::Gamma)] {
            if let Some(symbol) = self.symbols.get(name) {
                self.define_global_if_absent(symbol, Value::Builtin(builtin));
            }
        }
        for (name, constant) in [
            ("pi", MathConstant::Pi),
            ("π", MathConstant::Pi),
            ("e", MathConstant::E),
            ("tau", MathConstant::Tau),
            ("τ", MathConstant::Tau),
            ("phi", MathConstant::Phi),
            ("φ", MathConstant::Phi),
        ] {
            if let Some(symbol) = self.symbols.get(name) {
                self.define_global_if_absent(
                    symbol,
                    Value::Symbolic(Rc::new(SymbolicExpr::Constant(constant))),
                );
            }
        }
        if let Some(symbol) = self.symbols.get("i") {
            self.define_global_if_absent(
                symbol,
                Value::Number(Number::Complex(ComplexNumber::new(
                    BigDecimal::from(0),
                    BigDecimal::from(1),
                ))),
            );
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
        self.run_interactive_value(program)
            .map(|value| value.map(|value| self.stringify(&value)))
    }

    /// Runs one REPL submission and returns its final bare expression as a
    /// structured value. GUI clients use this to typeset results without
    /// reparsing their display strings.
    pub fn run_interactive_value(
        &mut self,
        program: &[Statement],
    ) -> Result<Option<Value>, RuntimeError> {
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
                    result = Some(value);
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

    /// Binds `name` in the global scope, returning the previously held value
    /// so it can be restored. Plotting uses this to drive the independent
    /// variable without disturbing session definitions.
    pub fn bind_global(&mut self, name: &str, value: Value) -> Option<Value> {
        let symbol = self.symbols.intern(name);
        let previous = Environment::remove(&self.globals, symbol);
        self.globals.borrow_mut().define(symbol, value);
        previous
    }

    /// Restores a binding displaced by `bind_global`. `previous` of `None`
    /// removes the name entirely; `Some(value)` reinstates it.
    pub fn restore_global(&mut self, name: &str, previous: Option<Value>) {
        let symbol = self.symbols.intern(name);
        match previous {
            Some(value) => self.globals.borrow_mut().define(symbol, value),
            None => {
                Environment::remove(&self.globals, symbol);
            }
        }
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

    fn transcendental_precision(&self) -> ExecResult<u64> {
        let precision = self.decimal_precision();
        if precision > self.limits.max_transcendental_precision {
            return Err(self.limit_error(format!(
                "transcendental precision exceeds the {} digit limit",
                self.limits.max_transcendental_precision
            )));
        }
        Ok(precision)
    }

    /// Looks up the current global binding for `name`. Plotting uses this to
    /// inline user functions and session constants into compiled samplers.
    pub fn global_value(&self, name: &str) -> Option<Value> {
        let symbol = self.symbols.get(name)?;
        Environment::get(&self.globals, symbol)
    }

    pub fn definition_names(&self) -> Vec<&str> {
        let mut names = Environment::local_names(&self.globals)
            .into_iter()
            .map(|symbol| self.symbols.resolve(symbol))
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }

    /// Execute one statement in the persistent top-level environment.
    ///
    /// This is primarily used by the documenter, which must interleave
    /// evaluation with fragment emission while preserving source order.
    pub fn execute_statement(&mut self, statement: &Statement) -> Result<(), RuntimeError> {
        let env = Rc::clone(&self.globals);
        self.execute(&env, statement)
            .map_err(|flow| self.describe_top_level(flow))
    }

    /// Evaluate one expression in the persistent top-level environment.
    /// Equality facts from `assume(x == v)` are substituted first, so
    /// constrained variables behave as if defined to their bound value.
    pub fn evaluate_expression(&mut self, expression: &Expr) -> Result<Value, RuntimeError> {
        let env = Rc::clone(&self.globals);
        self.current_span = Some(expression.span);
        let effective = match self.substitute_expression_constraints(expression) {
            Some(substituted) => substituted,
            None => expression.clone(),
        };
        self.evaluate(&env, &effective)
            .map_err(|flow| self.describe_top_level(flow))
    }

    /// Evaluate an expression for a dynamic document result. Operations with
    /// meaningful internal work select their structured variant here, keeping
    /// ordinary execution concise while making `=>` maximally informative.
    pub fn evaluate_documented_expression(
        &mut self,
        expression: &Expr,
    ) -> Result<Value, RuntimeError> {
        let ExprKind::Call { callee, args } = &expression.kind else {
            return self.evaluate_expression(expression);
        };
        let ExprKind::Variable(symbol) = callee.kind else {
            return self.evaluate_expression(expression);
        };
        let documented = match self.symbols.resolve(symbol) {
            "transpose" => Some(Builtin::TransposeSteps),
            "trace" => Some(Builtin::TraceSteps),
            "det" => Some(Builtin::DeterminantSteps),
            "rank" => Some(Builtin::RankSteps),
            "rref" => Some(Builtin::RrefSteps),
            "inverse" => Some(Builtin::InverseSteps),
            "matmul" => Some(Builtin::MatmulSteps),
            "dot" => Some(Builtin::DotSteps),
            "cross" => Some(Builtin::CrossSteps),
            "norm" => Some(Builtin::NormSteps),
            "solve_linear" => Some(Builtin::SolveLinearSteps),
            _ => None,
        };
        let Some(documented) = documented else {
            return self.evaluate_expression(expression);
        };
        let env = Rc::clone(&self.globals);
        self.current_span = Some(expression.span);
        let values = args
            .iter()
            .map(|argument| self.evaluate(&env, argument))
            .collect::<ExecResult<Vec<_>>>()
            .and_then(|values| self.call_builtin(documented, values));
        values.map_err(|flow| self.describe_top_level(flow))
    }

    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }

    /// Replace the symbol table after parsing an embedded document
    /// interpolation. Existing symbol ids remain stable because the scanner
    /// extends a clone of this table.
    pub fn extend_symbols(&mut self, symbols: SymbolTable) {
        self.set_symbols(symbols);
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
            StatementKind::Chain(statements) => {
                for statement in statements {
                    self.execute(env, statement)?;
                }
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
            ExprKind::Variable(sym) => {
                if let Some(value) = Environment::get(env, *sym) {
                    return Ok(value);
                }
                // An equality fact from `assume(x == v)` acts as a
                // definition for symbolic reads.
                if let Some(definition) = self.equality_fact_for(*sym) {
                    return Ok(Self::symbolic_value(definition));
                }
                Ok(Value::Symbolic(Rc::new(SymbolicExpr::Variable(*sym))))
            }
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
                    // A symbolic operand keeps the conjunction intact so
                    // `assume(x > 0 and x < 10)` reaches the collector.
                    // Symbolic values are truthy, so this must be checked
                    // before the short-circuit below.
                    if matches!(l, Value::Symbolic(_)) {
                        let r = self.evaluate(env, right)?;
                        if matches!(r, Value::Symbolic(_)) {
                            let left_tree = self.value_to_symbolic(l)?;
                            let right_tree = self.value_to_symbolic(r)?;
                            return Ok(Value::Symbolic(
                                self.symbolic_conjunction(&left_tree, &right_tree),
                            ));
                        }
                        // `true` is the identity of `and`; `false`
                        // annihilates the undecided left side.
                        return Ok(if r.is_truthy() { l } else { r });
                    }
                    if l.is_truthy() {
                        return self.evaluate(env, right);
                    }
                    Ok(l)
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
            | Builtin::Integral
            | Builtin::Degree
            | Builtin::Coefficients
            | Builtin::Discriminant
            | Builtin::CharacteristicPolynomial
            | Builtin::Projection
            | Builtin::ChangeBasis => (2, 2),
            Builtin::Matmul
            | Builtin::Dot
            | Builtin::Cross
            | Builtin::SolveLinear
            | Builtin::MatmulSteps
            | Builtin::SolveLinearSteps
            | Builtin::DotSteps
            | Builtin::CrossSteps => (2, 2),
            Builtin::Subs
            | Builtin::Resultant
            | Builtin::PolynomialGcd
            | Builtin::PolynomialDivRem => (3, 3),
            Builtin::Gradient | Builtin::Jacobian | Builtin::Hessian => (2, 2),
            Builtin::DefiniteIntegral | Builtin::Sum | Builtin::Product => (4, 4),
            Builtin::Limit => (3, 4),
            Builtin::Series => (4, 4),
            Builtin::MultipleIntegral => (2, 2),
            Builtin::Dsolve => (2, 4),
            Builtin::Assume => (1, 1),
            Builtin::ForgetAssumptions => (0, 0),
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
            Builtin::Transpose
                | Builtin::Trace
                | Builtin::Determinant
                | Builtin::Rank
                | Builtin::Rref
                | Builtin::Inverse
                | Builtin::Matmul
                | Builtin::Dot
                | Builtin::Cross
                | Builtin::Norm
                | Builtin::SolveLinear
                | Builtin::DeterminantSteps
                | Builtin::RrefSteps
                | Builtin::InverseSteps
                | Builtin::MatmulSteps
                | Builtin::SolveLinearSteps
                | Builtin::TransposeSteps
                | Builtin::TraceSteps
                | Builtin::RankSteps
                | Builtin::DotSteps
                | Builtin::CrossSteps
                | Builtin::NormSteps
                | Builtin::CharacteristicPolynomial
                | Builtin::Eigenvalues
                | Builtin::Eigenvectors
                | Builtin::Lu
                | Builtin::Qr
                | Builtin::Cholesky
                | Builtin::LuSteps
                | Builtin::QrSteps
                | Builtin::CholeskySteps
                | Builtin::NullSpace
                | Builtin::ColumnSpace
                | Builtin::GramSchmidt
                | Builtin::Projection
                | Builtin::ChangeBasis
                | Builtin::MatrixExp
                | Builtin::JordanForm
        ) {
            return self.call_linear_algebra(builtin, args);
        }

        if matches!(
            builtin,
            Builtin::Gradient
                | Builtin::Jacobian
                | Builtin::Hessian
                | Builtin::DefiniteIntegral
                | Builtin::Limit
                | Builtin::Series
                | Builtin::Sum
                | Builtin::Product
                | Builtin::MultipleIntegral
                | Builtin::Dsolve
        ) {
            return self.call_calculus_tool(builtin, args);
        }

        if matches!(
            builtin,
            Builtin::Degree
                | Builtin::Coefficients
                | Builtin::Resultant
                | Builtin::Discriminant
                | Builtin::PolynomialGcd
                | Builtin::PolynomialDivRem
        ) {
            return self.call_polynomial_tool(builtin, args);
        }

        if matches!(
            builtin,
            Builtin::Simplify
                | Builtin::Expand
                | Builtin::Factor
                | Builtin::Apart
                | Builtin::Together
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
                if let Some(expanded) = self.expand_trigonometric(&expression)? {
                    let simplified = self.simplify_symbolic_tree(&expanded)?;
                    return Ok(Self::symbolic_value(simplified));
                }
            }
            if builtin == Builtin::Factor {
                if let Some(factored) = self.trig_sum_to_product(&expression)? {
                    let simplified = self.simplify_symbolic_tree(&factored)?;
                    return Ok(Self::symbolic_value(simplified));
                }
            }
            let result = if builtin == Builtin::Apart {
                Polynomial::partial_fraction_expression(&expression, &self.symbols)
            } else if builtin == Builtin::Together {
                Polynomial::together_expression(&expression, &self.symbols)
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

        if matches!(builtin, Builtin::Assume | Builtin::ForgetAssumptions) {
            return self.call_constraint_tool(builtin, args);
        }

        if matches!(
            builtin,
            Builtin::AssumeNonzero
                | Builtin::AssumeNonnegative
                | Builtin::AssumePositive
                | Builtin::AssumeNegative
                | Builtin::AssumeInteger
                | Builtin::AssumeReal
        ) {
            let Value::Symbolic(expression) = &args[0] else {
                // Equality facts substitute arguments, so `assume(x == 0)`
                // followed by `assume_positive(x)` arrives as
                // `assume_positive(0)`. Recover the variable from the
                // matching fact and report the claim's consistency with
                // the concrete value.
                if let Some(symbol) = self.equality_fact_symbol(&args[0]) {
                    let concrete = match &args[0] {
                        Value::Number(number) => Some(SymbolicExpr::Number(number.clone())),
                        _ => None,
                    };
                    let holds = concrete.as_ref().map_or(false, |value| match builtin {
                        Builtin::AssumePositive => self.symbolic_positive(value),
                        Builtin::AssumeNegative => self.symbolic_negative(value),
                        Builtin::AssumeNonnegative => {
                            self.symbolic_positive(value)
                                || matches!(value, SymbolicExpr::Number(number)
                                        if number.is_zero())
                        }
                        Builtin::AssumeNonzero => self.symbolic_nonzero(value),
                        Builtin::AssumeInteger => self.symbolic_is_integer(value),
                        Builtin::AssumeReal => self.symbolic_real(value),
                        _ => false,
                    });
                    if holds {
                        return Ok(args[0].clone());
                    }
                    return Err(self.error(format!(
                        "{} contradicts an existing assumption for '{}'",
                        builtin.name(),
                        self.symbols.resolve(symbol)
                    )));
                }
                return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
            };
            let SymbolicExpr::Variable(symbol) = expression.as_ref() else {
                return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
            };
            let contradictory = match builtin {
                Builtin::AssumeNonnegative | Builtin::AssumePositive => {
                    self.negative_symbols.borrow().contains(symbol)
                }
                Builtin::AssumeNegative => {
                    self.nonnegative_symbols.borrow().contains(symbol)
                        || self.positive_symbols.borrow().contains(symbol)
                }
                _ => false,
            };
            if contradictory {
                return Err(self.error(format!(
                    "{} contradicts an existing assumption for '{}'",
                    builtin.name(),
                    self.symbols.resolve(*symbol)
                )));
            }
            // Mirror the domain claim as a fact so assume/assume_* calls
            // cross-check against each other in both directions.
            let zero = Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(0))));
            let fact = match builtin {
                Builtin::AssumePositive => Some(ConstraintFact::RealCompare {
                    symbol: *symbol,
                    bound: Rc::clone(&zero),
                    op: BinaryOp::Lt,
                }),
                Builtin::AssumeNegative => Some(ConstraintFact::RealCompare {
                    symbol: *symbol,
                    bound: Rc::clone(&zero),
                    op: BinaryOp::Gt,
                }),
                Builtin::AssumeNonnegative => Some(ConstraintFact::RealCompare {
                    symbol: *symbol,
                    bound: Rc::clone(&zero),
                    op: BinaryOp::Le,
                }),
                Builtin::AssumeNonzero => Some(ConstraintFact::NotEqual {
                    symbol: *symbol,
                    value: zero,
                }),
                _ => None,
            };
            if let Some(fact) = fact {
                let contradicts = self
                    .constraints
                    .borrow()
                    .iter()
                    .any(|existing| Self::facts_conflict(existing, &fact));
                if contradicts {
                    return Err(self.error(format!(
                        "{} contradicts an existing assumption for '{}'",
                        builtin.name(),
                        self.symbols.resolve(*symbol)
                    )));
                }
                self.constraints.borrow_mut().push(fact);
            }
            match builtin {
                Builtin::AssumeNonzero => {
                    self.nonzero_symbols.borrow_mut().insert(*symbol);
                }
                Builtin::AssumeNonnegative => {
                    self.nonnegative_symbols.borrow_mut().insert(*symbol);
                    self.real_symbols.borrow_mut().insert(*symbol);
                }
                Builtin::AssumePositive => {
                    self.positive_symbols.borrow_mut().insert(*symbol);
                    self.nonnegative_symbols.borrow_mut().insert(*symbol);
                    self.nonzero_symbols.borrow_mut().insert(*symbol);
                    self.real_symbols.borrow_mut().insert(*symbol);
                }
                Builtin::AssumeNegative => {
                    self.negative_symbols.borrow_mut().insert(*symbol);
                    self.nonzero_symbols.borrow_mut().insert(*symbol);
                    self.real_symbols.borrow_mut().insert(*symbol);
                }
                Builtin::AssumeInteger => {
                    self.integer_symbols.borrow_mut().insert(*symbol);
                    self.real_symbols.borrow_mut().insert(*symbol);
                }
                Builtin::AssumeReal => {
                    self.real_symbols.borrow_mut().insert(*symbol);
                }
                _ => unreachable!(),
            }
            return Ok(args[0].clone());
        }

        if builtin == Builtin::Evalf {
            return self.evalf_value(args.into_iter().next().expect("arity checked"));
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
                Value::Number(Number::Complex(number)) => Number::Decimal(number.real.clone())
                    .evaluate_binary(
                        &Number::Decimal(number.imaginary.clone()),
                        BinaryMathFunction::Hypot,
                        self.transcendental_precision()?,
                    )
                    .map(Value::Number)
                    .map_err(|error| self.error(error.to_string())),
                Value::Number(number) => Ok(Value::Number(number.abs())),
                value => {
                    Err(self.error(format!("abs expects a number, got {}", value.type_name())))
                }
            },
            Builtin::Sign => match &args[0] {
                Value::Number(number) => {
                    let zero = Number::Integer(BigInt::from(0));
                    let sign = match number.compare(&zero) {
                        Some(std::cmp::Ordering::Less) => -1,
                        Some(std::cmp::Ordering::Equal) => 0,
                        Some(std::cmp::Ordering::Greater) => 1,
                        None => return Err(self.error("sign expects a real number")),
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
                let ordering = left.compare(right).ok_or_else(|| {
                    self.error(format!("{} expects real numbers", builtin.name()))
                })?;
                let take_left = if builtin == Builtin::Min {
                    !ordering.is_gt()
                } else {
                    !ordering.is_lt()
                };
                Ok(if take_left {
                    args[0].clone()
                } else {
                    args[1].clone()
                })
            }
            Builtin::Floor | Builtin::Ceil | Builtin::Round | Builtin::Trunc => {
                let number = self.builtin_number(builtin, &args[0])?;
                self.rounding_builtin(builtin, number)
            }
            Builtin::Log if args.len() == 2 => {
                let value = self.builtin_number(builtin, &args[0])?;
                let base = self.builtin_number(builtin, &args[1])?;
                if base.is_zero() || base.is_one() {
                    return Err(self.error("log base must be nonzero and different from 1"));
                }
                if value.is_exact() && base.is_exact() {
                    if let Some(exponent) = Self::exact_integer_log(value, base) {
                        return Ok(Value::Number(Number::Integer(exponent)));
                    }
                    return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                        function: Builtin::Log,
                        args: vec![
                            Rc::new(SymbolicExpr::Number(value.clone())),
                            Rc::new(SymbolicExpr::Number(base.clone())),
                        ],
                    })));
                }
                value
                    .evaluate_binary(
                        base,
                        BinaryMathFunction::Log,
                        self.transcendental_precision()?,
                    )
                    .map(Value::Number)
                    .map_err(|error| self.error(format!("log failed: {error}")))
            }
            Builtin::Atan2 | Builtin::Hypot => {
                let left = self.builtin_number(builtin, &args[0])?;
                let right = self.builtin_number(builtin, &args[1])?;
                let function = if builtin == Builtin::Atan2 {
                    BinaryMathFunction::Atan2
                } else {
                    BinaryMathFunction::Hypot
                };
                left.evaluate_binary(right, function, self.transcendental_precision()?)
                    .map(Value::Number)
                    .map_err(|error| self.error(format!("{} failed: {error}", builtin.name())))
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
                if let Some(value) = self.exact_special_value(builtin, number)? {
                    return Ok(value);
                }
                if builtin == Builtin::Sqrt && number.is_exact() && number.is_negative() {
                    let magnitude = number.abs();
                    let root = magnitude.exact_root(2).map_or_else(
                        || {
                            Rc::new(SymbolicExpr::Call {
                                function: Builtin::Sqrt,
                                args: vec![Rc::new(SymbolicExpr::Number(magnitude))],
                            })
                        },
                        |root| Rc::new(SymbolicExpr::Number(root)),
                    );
                    let imaginary_unit = Rc::new(SymbolicExpr::Number(Number::Complex(
                        ComplexNumber::new(BigDecimal::from(0), BigDecimal::from(1)),
                    )));
                    return self.symbolic_binary(
                        BinaryOp::Mul,
                        Self::symbolic_value(root),
                        Self::symbolic_value(imaginary_unit),
                    );
                }
                if let Some(root) = match builtin {
                    Builtin::Sqrt => number.exact_root(2),
                    Builtin::Cbrt => number.exact_root(3),
                    _ => None,
                } {
                    return Ok(Value::Number(root));
                }
                if number.is_zero() {
                    let exact = match builtin {
                        Builtin::Sqrt
                        | Builtin::Cbrt
                        | Builtin::Sin
                        | Builtin::Tan
                        | Builtin::Asin
                        | Builtin::Atan
                        | Builtin::Sinh
                        | Builtin::Tanh
                        | Builtin::Asinh
                        | Builtin::Atanh
                        | Builtin::Erf => Some(Value::Number(Number::Integer(BigInt::from(0)))),
                        Builtin::Exp
                        | Builtin::Cos
                        | Builtin::Cosh
                        | Builtin::Sec
                        | Builtin::Erfc => Some(Value::Number(Number::Integer(BigInt::from(1)))),
                        _ => None,
                    };
                    if let Some(value) = exact {
                        return Ok(value);
                    }
                }
                if number.is_one()
                    && matches!(
                        builtin,
                        Builtin::Ln
                            | Builtin::Log
                            | Builtin::Log10
                            | Builtin::Log2
                            | Builtin::Acosh
                    )
                {
                    return Ok(Value::Number(Number::Integer(BigInt::from(0))));
                }
                if let Some(function) = Self::unary_math_function(builtin) {
                    if number.is_exact() {
                        return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                            function: builtin,
                            args: vec![Rc::new(SymbolicExpr::Number(number.clone()))],
                        })));
                    }
                    return number
                        .evaluate_unary(function, self.transcendental_precision()?)
                        .map(Value::Number)
                        .map_err(|error| {
                            self.error(format!("{} failed: {error}", builtin.name()))
                        });
                }
                Err(self.error(format!(
                    "{} has no numeric implementation for this argument",
                    builtin.name()
                )))
            }
        }
    }

    fn call_linear_algebra(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        match builtin {
            Builtin::Transpose | Builtin::TransposeSteps => {
                let matrix = self.matrix(&args[0], "transpose")?;
                let result = Self::matrix_value(Self::transpose_matrix(&matrix));
                if builtin == Builtin::TransposeSteps {
                    Ok(Self::derivation(
                        result.clone(),
                        vec![DerivationStep {
                            value: result,
                            operation: StepOperation::Explanation(
                                r"(A^{\mathsf T})_{ij}=A_{ji}".to_owned(),
                            ),
                        }],
                    ))
                } else {
                    Ok(result)
                }
            }
            Builtin::Trace | Builtin::TraceSteps => {
                let matrix = self.square_matrix(&args[0], "trace")?;
                let mut total = Self::integer_value(0);
                for (index, row) in matrix.iter().enumerate() {
                    total = self.apply_binary(BinaryOp::Add, total, row[index].clone())?;
                }
                if builtin == Builtin::TraceSteps {
                    let terms = matrix
                        .iter()
                        .enumerate()
                        .map(|(index, row)| row[index].clone())
                        .collect::<Vec<_>>();
                    Ok(Self::derivation(
                        total.clone(),
                        vec![
                            DerivationStep {
                                value: Value::Array(Rc::new(terms)),
                                operation: StepOperation::Explanation(
                                    r"\operatorname{tr}(A)=\sum_i A_{ii}".to_owned(),
                                ),
                            },
                            DerivationStep {
                                value: total,
                                operation: StepOperation::Explanation(
                                    "sum the diagonal".to_owned(),
                                ),
                            },
                        ],
                    ))
                } else {
                    Ok(total)
                }
            }
            Builtin::Determinant | Builtin::DeterminantSteps => {
                let matrix = self.square_matrix(&args[0], builtin.name())?;
                let (determinant, history) = self.determinant_with_steps(matrix)?;
                if builtin == Builtin::DeterminantSteps {
                    Ok(Self::derivation(determinant, history))
                } else {
                    Ok(determinant)
                }
            }
            Builtin::Rref | Builtin::Rank | Builtin::RrefSteps | Builtin::RankSteps => {
                let matrix = self.matrix(&args[0], builtin.name())?;
                let (reduced, history, pivots) = self.rref_with_steps(matrix, None)?;
                match builtin {
                    Builtin::Rank => Ok(Self::integer_value(pivots as i64)),
                    Builtin::RankSteps => {
                        let result = Self::integer_value(pivots as i64);
                        Ok(Self::derivation(result, history))
                    }
                    Builtin::RrefSteps => Ok(Self::derivation(
                        Self::matrix_value(reduced.clone()),
                        history,
                    )),
                    _ => Ok(Self::matrix_value(reduced)),
                }
            }
            Builtin::Inverse | Builtin::InverseSteps => {
                let matrix = self.square_matrix(&args[0], builtin.name())?;
                let size = matrix.len();
                let mut augmented = Vec::with_capacity(size);
                for (row_index, mut row) in matrix.into_iter().enumerate() {
                    for column in 0..size {
                        row.push(Self::integer_value(i64::from(row_index == column)));
                    }
                    augmented.push(row);
                }
                let (reduced, history, pivots) = self.rref_with_steps(augmented, Some(size))?;
                if pivots < size {
                    return Err(self.error("matrix is singular and has no inverse"));
                }
                let inverse = reduced
                    .into_iter()
                    .map(|row| row[size..].to_vec())
                    .collect::<Vec<_>>();
                if builtin == Builtin::InverseSteps {
                    Ok(Self::derivation(Self::matrix_value(inverse), history))
                } else {
                    Ok(Self::matrix_value(inverse))
                }
            }
            Builtin::Matmul | Builtin::MatmulSteps => {
                let product = self.matrix_product(&args[0], &args[1], builtin.name())?;
                if builtin == Builtin::MatmulSteps {
                    Ok(Self::derivation(
                        product.clone(),
                        vec![DerivationStep {
                            value: product,
                            operation: StepOperation::Explanation(
                                r"(AB)_{ij}=\sum_k A_{ik}B_{kj}\quad\text{or}\quad(Av)_i=\sum_k A_{ik}v_k"
                                    .to_owned(),
                            ),
                        }],
                    ))
                } else {
                    Ok(product)
                }
            }
            Builtin::Dot | Builtin::DotSteps => {
                let left = self.vector(&args[0], "dot")?;
                let right = self.vector(&args[1], "dot")?;
                if left.len() != right.len() {
                    return Err(self.error("dot expects vectors of equal length"));
                }
                let mut total = Self::integer_value(0);
                for (left, right) in left.into_iter().zip(right) {
                    let product = self.apply_binary(BinaryOp::Mul, left, right)?;
                    total = self.apply_binary(BinaryOp::Add, total, product)?;
                }
                if builtin == Builtin::DotSteps {
                    Ok(Self::derivation(
                        total.clone(),
                        vec![DerivationStep {
                            value: total,
                            operation: StepOperation::Explanation(
                                r"u\cdot v=\sum_i u_i v_i".to_owned(),
                            ),
                        }],
                    ))
                } else {
                    Ok(total)
                }
            }
            Builtin::Cross | Builtin::CrossSteps => {
                let left = self.vector(&args[0], "cross")?;
                let right = self.vector(&args[1], "cross")?;
                if left.len() != 3 || right.len() != 3 {
                    return Err(self.error("cross expects two three-dimensional vectors"));
                }
                let component = |a: usize, b: usize, c: usize, d: usize| -> ExecResult<Value> {
                    let first =
                        self.apply_binary(BinaryOp::Mul, left[a].clone(), right[b].clone())?;
                    let second =
                        self.apply_binary(BinaryOp::Mul, left[c].clone(), right[d].clone())?;
                    self.apply_binary(BinaryOp::Sub, first, second)
                };
                let result = Value::Array(Rc::new(vec![
                    component(1, 2, 2, 1)?,
                    component(2, 0, 0, 2)?,
                    component(0, 1, 1, 0)?,
                ]));
                if builtin == Builtin::CrossSteps {
                    Ok(Self::derivation(
                        result.clone(),
                        vec![DerivationStep {
                            value: result,
                            operation: StepOperation::Explanation(
                                r"u\times v=\det\!\begin{pmatrix}\mathbf i&\mathbf j&\mathbf k\\u_1&u_2&u_3\\v_1&v_2&v_3\end{pmatrix}".to_owned(),
                            ),
                        }],
                    ))
                } else {
                    Ok(result)
                }
            }
            Builtin::Norm | Builtin::NormSteps => {
                let vector = self.vector(&args[0], "norm")?;
                let mut squared = Self::integer_value(0);
                for value in vector {
                    let term = self.apply_binary(BinaryOp::Mul, value.clone(), value)?;
                    squared = self.apply_binary(BinaryOp::Add, squared, term)?;
                }
                let result = self.call_builtin(Builtin::Sqrt, vec![squared.clone()])?;
                if builtin == Builtin::NormSteps {
                    Ok(Self::derivation(
                        result.clone(),
                        vec![
                            DerivationStep {
                                value: squared,
                                operation: StepOperation::Explanation(
                                    r"\lVert v\rVert^2=\sum_i v_i^2".to_owned(),
                                ),
                            },
                            DerivationStep {
                                value: result,
                                operation: StepOperation::Explanation(
                                    r"\lVert v\rVert=\sqrt{\lVert v\rVert^2}".to_owned(),
                                ),
                            },
                        ],
                    ))
                } else {
                    Ok(result)
                }
            }
            Builtin::CharacteristicPolynomial => {
                let matrix = self.square_matrix(&args[0], builtin.name())?;
                if matrix.len() > 6 {
                    return Err(self.limit_error(
                        "characteristic_polynomial exact expansion is limited to 6x6 matrices",
                    ));
                }
                let variable = self.value_to_symbolic(args[1].clone())?;
                let SymbolicExpr::Variable(_) = variable.as_ref() else {
                    return Err(self.error(
                        "characteristic_polynomial expects a symbolic variable as its second argument",
                    ));
                };
                let mut characteristic = Vec::with_capacity(matrix.len());
                for (row_index, row) in matrix.into_iter().enumerate() {
                    let mut result_row = Vec::with_capacity(row.len());
                    for (column_index, entry) in row.into_iter().enumerate() {
                        let diagonal = if row_index == column_index {
                            Self::symbolic_value(Rc::clone(&variable))
                        } else {
                            Self::integer_value(0)
                        };
                        result_row.push(self.apply_binary(BinaryOp::Sub, diagonal, entry)?);
                    }
                    characteristic.push(result_row);
                }
                let result = self.determinant_cofactor(&characteristic)?;
                match result {
                    Value::Symbolic(expression) => {
                        let simplified = self.simplify_symbolic_tree(&expression)?;
                        let expanded = Polynomial::from_expression(&simplified, &self.symbols)
                            .map(|polynomial| polynomial.expanded_expression())
                            .unwrap_or(simplified);
                        Ok(Self::symbolic_value(expanded))
                    }
                    value => Ok(value),
                }
            }
            Builtin::Eigenvalues => self.eigenvalues_value(&args[0]),
            Builtin::Eigenvectors => self.eigenvectors_value(&args[0]),
            Builtin::Lu | Builtin::LuSteps => {
                let matrix = self.square_matrix(&args[0], builtin.name())?;
                let (lower, upper) = self.lu_decomposition(matrix)?;
                let result = Value::Array(Rc::new(vec![
                    Self::matrix_value(lower),
                    Self::matrix_value(upper),
                ]));
                if builtin == Builtin::LuSteps {
                    Ok(Self::derivation(
                        result.clone(),
                        vec![DerivationStep {
                            value: result,
                            operation: StepOperation::Explanation(
                                r"A=LU\quad\text{(Doolittle elimination)}".to_owned(),
                            ),
                        }],
                    ))
                } else {
                    Ok(result)
                }
            }
            Builtin::Qr | Builtin::QrSteps => {
                let matrix = self.matrix(&args[0], builtin.name())?;
                let (q, r) = self.qr_decomposition(matrix)?;
                let result =
                    Value::Array(Rc::new(vec![Self::matrix_value(q), Self::matrix_value(r)]));
                if builtin == Builtin::QrSteps {
                    Ok(Self::derivation(
                        result.clone(),
                        vec![DerivationStep {
                            value: result,
                            operation: StepOperation::Explanation(
                                r"A=QR\quad\text{(modified Gram--Schmidt)}".to_owned(),
                            ),
                        }],
                    ))
                } else {
                    Ok(result)
                }
            }
            Builtin::Cholesky | Builtin::CholeskySteps => {
                let matrix = self.square_matrix(&args[0], builtin.name())?;
                let lower = self.cholesky_decomposition(matrix)?;
                let result = Self::matrix_value(lower);
                if builtin == Builtin::CholeskySteps {
                    Ok(Self::derivation(
                        result.clone(),
                        vec![DerivationStep {
                            value: result,
                            operation: StepOperation::Explanation(r"A=LL^{\mathsf T}".to_owned()),
                        }],
                    ))
                } else {
                    Ok(result)
                }
            }
            Builtin::NullSpace => self.null_space_value(&args[0]),
            Builtin::ColumnSpace => self.column_space_value(&args[0]),
            Builtin::GramSchmidt => {
                let vectors = self.matrix(&args[0], "gram_schmidt")?;
                Ok(Value::Array(Rc::new(
                    self.orthogonalize(vectors, true)?
                        .into_iter()
                        .map(|vector| Value::Array(Rc::new(vector)))
                        .collect(),
                )))
            }
            Builtin::Projection => {
                let vector = self.vector(&args[0], "projection")?;
                let onto = self.vector(&args[1], "projection")?;
                Ok(Value::Array(Rc::new(self.project_vector(&vector, &onto)?)))
            }
            Builtin::ChangeBasis => {
                let vector = self.vector(&args[0], "change_basis")?;
                let basis = self.square_matrix(&args[1], "change_basis")?;
                let transposed = Self::transpose_matrix(&basis);
                let solution = self.call_linear_algebra(
                    Builtin::SolveLinear,
                    vec![
                        Self::matrix_value(transposed),
                        Value::Array(Rc::new(vector)),
                    ],
                )?;
                Ok(solution)
            }
            Builtin::MatrixExp => {
                let matrix = self.square_matrix(&args[0], "matrix_exp")?;
                let size = matrix.len();
                for (row, values) in matrix.iter().enumerate() {
                    for (column, value) in values.iter().enumerate() {
                        if row != column && !Self::definitely_zero(value) {
                            return Err(
                                self.error("matrix_exp currently requires a diagonal matrix")
                            );
                        }
                    }
                }
                let mut result = vec![vec![Self::integer_value(0); size]; size];
                for index in 0..size {
                    result[index][index] =
                        self.call_builtin(Builtin::Exp, vec![matrix[index][index].clone()])?;
                }
                Ok(Self::matrix_value(result))
            }
            Builtin::JordanForm => self.jordan_form_value(&args[0]),
            Builtin::SolveLinear | Builtin::SolveLinearSteps => {
                let matrix = self.matrix(&args[0], builtin.name())?;
                let vector = self.vector(&args[1], builtin.name())?;
                if matrix.len() != vector.len()
                    || matrix.is_empty()
                    || matrix[0].len() != matrix.len()
                {
                    return Err(
                        self.error("solve_linear expects a square matrix and matching vector")
                    );
                }
                let size = matrix.len();
                let augmented = matrix
                    .into_iter()
                    .zip(vector)
                    .map(|(mut row, value)| {
                        row.push(value);
                        row
                    })
                    .collect::<Vec<_>>();
                let (reduced, history, pivots) = self.rref_with_steps(augmented, Some(size))?;
                if pivots < size {
                    return Err(self.error("linear system does not have a unique solution"));
                }
                let solution = Value::Array(Rc::new(
                    reduced.iter().map(|row| row[size].clone()).collect(),
                ));
                if builtin == Builtin::SolveLinearSteps {
                    Ok(Self::derivation(solution, history))
                } else {
                    Ok(solution)
                }
            }
            _ => unreachable!("linear algebra builtin dispatched separately"),
        }
    }

    fn integer_value(value: i64) -> Value {
        Value::Number(Number::Integer(BigInt::from(value)))
    }

    fn derivation(result: Value, steps: Vec<DerivationStep>) -> Value {
        Value::Derivation {
            result: Rc::new(result),
            steps: Rc::new(steps),
        }
    }

    fn matrix_value(rows: Vec<Vec<Value>>) -> Value {
        Value::Array(Rc::new(
            rows.into_iter()
                .map(|row| Value::Array(Rc::new(row)))
                .collect(),
        ))
    }

    fn matrix(&self, value: &Value, function: &str) -> ExecResult<Vec<Vec<Value>>> {
        let Value::Array(rows) = value else {
            return Err(self.error(format!("{function} expects a matrix")));
        };
        let mut matrix = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let Value::Array(columns) = row else {
                return Err(self.error(format!("{function} expects a matrix, not a vector")));
            };
            matrix.push(columns.as_ref().clone());
        }
        let width = matrix.first().map_or(0, Vec::len);
        if width == 0 || matrix.iter().any(|row| row.len() != width) {
            return Err(self.error(format!("{function} expects a non-empty rectangular matrix")));
        }
        Ok(matrix)
    }

    fn square_matrix(&self, value: &Value, function: &str) -> ExecResult<Vec<Vec<Value>>> {
        let matrix = self.matrix(value, function)?;
        if matrix.len() != matrix[0].len() {
            return Err(self.error(format!("{function} expects a square matrix")));
        }
        Ok(matrix)
    }

    fn vector(&self, value: &Value, function: &str) -> ExecResult<Vec<Value>> {
        let Value::Array(values) = value else {
            return Err(self.error(format!("{function} expects a vector")));
        };
        if values.iter().any(|value| matches!(value, Value::Array(_))) {
            return Err(self.error(format!("{function} expects a vector")));
        }
        Ok(values.as_ref().clone())
    }

    fn transpose_matrix(matrix: &[Vec<Value>]) -> Vec<Vec<Value>> {
        (0..matrix[0].len())
            .map(|column| matrix.iter().map(|row| row[column].clone()).collect())
            .collect()
    }

    fn dot_values(&self, left: &[Value], right: &[Value]) -> ExecResult<Value> {
        if left.len() != right.len() {
            return Err(self.error("vectors must have equal length"));
        }
        let mut total = Self::integer_value(0);
        for (left, right) in left.iter().zip(right) {
            let term = self.apply_binary(BinaryOp::Mul, left.clone(), right.clone())?;
            total = self.apply_binary(BinaryOp::Add, total, term)?;
        }
        Ok(total)
    }

    fn project_vector(&self, vector: &[Value], onto: &[Value]) -> ExecResult<Vec<Value>> {
        if vector.len() != onto.len() {
            return Err(self.error("projection expects vectors of equal length"));
        }
        let denominator = self.dot_values(onto, onto)?;
        if Self::definitely_zero(&denominator) {
            return Err(self.error("cannot project onto the zero vector"));
        }
        let coefficient =
            self.apply_binary(BinaryOp::Div, self.dot_values(vector, onto)?, denominator)?;
        onto.iter()
            .map(|value| self.apply_binary(BinaryOp::Mul, coefficient.clone(), value.clone()))
            .collect()
    }

    fn orthogonalize(
        &self,
        vectors: Vec<Vec<Value>>,
        normalize: bool,
    ) -> ExecResult<Vec<Vec<Value>>> {
        if vectors.is_empty() || vectors.iter().any(|row| row.len() != vectors[0].len()) {
            return Err(self.error("gram_schmidt expects equally sized non-empty vectors"));
        }
        let mut result: Vec<Vec<Value>> = Vec::with_capacity(vectors.len());
        for vector in vectors {
            let mut orthogonal = vector;
            for basis in &result {
                let projection = self.project_vector(&orthogonal, basis)?;
                for (entry, projected) in orthogonal.iter_mut().zip(projection) {
                    *entry = self.apply_binary(BinaryOp::Sub, entry.clone(), projected)?;
                }
            }
            let squared = self.dot_values(&orthogonal, &orthogonal)?;
            if Self::definitely_zero(&squared) {
                return Err(self.error("vectors are linearly dependent"));
            }
            if normalize {
                let length = self.call_builtin(Builtin::Sqrt, vec![squared])?;
                for entry in &mut orthogonal {
                    *entry = self.apply_binary(BinaryOp::Div, entry.clone(), length.clone())?;
                }
            }
            result.push(orthogonal);
        }
        Ok(result)
    }

    fn lu_decomposition(&self, matrix: Matrix) -> ExecResult<(Matrix, Matrix)> {
        let size = matrix.len();
        let mut lower = vec![vec![Self::integer_value(0); size]; size];
        let mut upper = vec![vec![Self::integer_value(0); size]; size];
        for (index, row) in lower.iter_mut().enumerate() {
            row[index] = Self::integer_value(1);
        }
        for pivot in 0..size {
            for column in pivot..size {
                let mut value = matrix[pivot][column].clone();
                for (index, upper_row) in upper.iter().enumerate().take(pivot) {
                    let term = self.apply_binary(
                        BinaryOp::Mul,
                        lower[pivot][index].clone(),
                        upper_row[column].clone(),
                    )?;
                    value = self.apply_binary(BinaryOp::Sub, value, term)?;
                }
                upper[pivot][column] = value;
            }
            if Self::definitely_zero(&upper[pivot][pivot]) {
                return Err(self.error("lu requires nonzero leading pivots"));
            }
            for row in pivot + 1..size {
                let mut value = matrix[row][pivot].clone();
                for (index, upper_row) in upper.iter().enumerate().take(pivot) {
                    let term = self.apply_binary(
                        BinaryOp::Mul,
                        lower[row][index].clone(),
                        upper_row[pivot].clone(),
                    )?;
                    value = self.apply_binary(BinaryOp::Sub, value, term)?;
                }
                lower[row][pivot] =
                    self.apply_binary(BinaryOp::Div, value, upper[pivot][pivot].clone())?;
            }
        }
        Ok((lower, upper))
    }

    fn qr_decomposition(&self, matrix: Matrix) -> ExecResult<(Matrix, Matrix)> {
        let rows = matrix.len();
        let columns = matrix[0].len();
        if rows < columns {
            return Err(self.error("qr expects a matrix with at least as many rows as columns"));
        }
        let input_columns = Self::transpose_matrix(&matrix);
        let q_columns = self.orthogonalize(input_columns.clone(), true)?;
        let mut r = vec![vec![Self::integer_value(0); columns]; columns];
        for row in 0..columns {
            for (column, input) in input_columns.iter().enumerate().skip(row) {
                r[row][column] = self.dot_values(&q_columns[row], input)?;
            }
        }
        Ok((Self::transpose_matrix(&q_columns), r))
    }

    fn cholesky_decomposition(&self, matrix: Matrix) -> ExecResult<Matrix> {
        let size = matrix.len();
        for row in 0..size {
            for column in 0..size {
                if !matrix[row][column].equals(&matrix[column][row]) {
                    return Err(self.error("cholesky expects a symmetric matrix"));
                }
            }
        }
        let mut lower = vec![vec![Self::integer_value(0); size]; size];
        for row in 0..size {
            for column in 0..=row {
                let mut value = matrix[row][column].clone();
                for index in 0..column {
                    let term = self.apply_binary(
                        BinaryOp::Mul,
                        lower[row][index].clone(),
                        lower[column][index].clone(),
                    )?;
                    value = self.apply_binary(BinaryOp::Sub, value, term)?;
                }
                if row == column {
                    if matches!(&value, Value::Number(number) if number.is_negative() || number.is_zero())
                    {
                        return Err(self.error("cholesky expects a positive-definite matrix"));
                    }
                    lower[row][column] = self.call_builtin(Builtin::Sqrt, vec![value])?;
                } else {
                    lower[row][column] =
                        self.apply_binary(BinaryOp::Div, value, lower[column][column].clone())?;
                }
            }
        }
        Ok(lower)
    }

    fn eigenvalues_value(&self, value: &Value) -> ExecResult<Value> {
        let matrix = self.square_matrix(value, "eigenvalues")?;
        let triangular = (0..matrix.len())
            .all(|row| (0..row).all(|column| Self::definitely_zero(&matrix[row][column])))
            || (0..matrix.len()).all(|row| {
                (row + 1..matrix.len()).all(|column| Self::definitely_zero(&matrix[row][column]))
            });
        if matrix.len() > 2 && triangular {
            return Ok(Value::Array(Rc::new(
                matrix
                    .iter()
                    .enumerate()
                    .map(|(index, row)| row[index].clone())
                    .collect(),
            )));
        }
        match matrix.len() {
            1 => Ok(Value::Array(Rc::new(vec![matrix[0][0].clone()]))),
            2 => {
                let trace =
                    self.apply_binary(BinaryOp::Add, matrix[0][0].clone(), matrix[1][1].clone())?;
                let four_ad = self.apply_binary(
                    BinaryOp::Mul,
                    Self::integer_value(4),
                    self.apply_binary(BinaryOp::Mul, matrix[0][0].clone(), matrix[1][1].clone())?,
                )?;
                let four_bc = self.apply_binary(
                    BinaryOp::Mul,
                    Self::integer_value(4),
                    self.apply_binary(BinaryOp::Mul, matrix[0][1].clone(), matrix[1][0].clone())?,
                )?;
                let trace_squared =
                    self.apply_binary(BinaryOp::Mul, trace.clone(), trace.clone())?;
                let discriminant = self.apply_binary(
                    BinaryOp::Add,
                    self.apply_binary(BinaryOp::Sub, trace_squared, four_ad)?,
                    four_bc,
                )?;
                let root = self.call_builtin(Builtin::Sqrt, vec![discriminant])?;
                let plus = self.apply_binary(BinaryOp::Add, trace.clone(), root.clone())?;
                let minus = self.apply_binary(BinaryOp::Sub, trace, root)?;
                Ok(Value::Array(Rc::new(vec![
                    self.apply_binary(BinaryOp::Div, plus, Self::integer_value(2))?,
                    self.apply_binary(BinaryOp::Div, minus, Self::integer_value(2))?,
                ])))
            }
            _ => Err(self.error("exact eigenvalues currently support matrices up to 2x2")),
        }
    }

    fn eigenvectors_value(&self, value: &Value) -> ExecResult<Value> {
        let matrix = self.square_matrix(value, "eigenvectors")?;
        if matrix.len() == 1 {
            return Ok(Value::Array(Rc::new(vec![Value::Array(Rc::new(vec![
                Self::integer_value(1),
            ]))])));
        }
        if matrix.len() != 2 {
            return Err(self.error("exact eigenvectors currently support matrices up to 2x2"));
        }
        let Value::Array(values) = self.eigenvalues_value(value)? else {
            unreachable!()
        };
        let mut vectors = Vec::new();
        for eigenvalue in values.iter() {
            let vector = if !Self::definitely_zero(&matrix[0][1]) {
                vec![
                    matrix[0][1].clone(),
                    self.apply_binary(BinaryOp::Sub, eigenvalue.clone(), matrix[0][0].clone())?,
                ]
            } else if !Self::definitely_zero(&matrix[1][0]) {
                vec![
                    self.apply_binary(BinaryOp::Sub, eigenvalue.clone(), matrix[1][1].clone())?,
                    matrix[1][0].clone(),
                ]
            } else if eigenvalue.equals(&matrix[0][0]) {
                vec![Self::integer_value(1), Self::integer_value(0)]
            } else {
                vec![Self::integer_value(0), Self::integer_value(1)]
            };
            if !vectors
                .iter()
                .any(|existing: &Value| existing.equals(&Value::Array(Rc::new(vector.clone()))))
            {
                vectors.push(Value::Array(Rc::new(vector)));
            }
        }
        Ok(Value::Array(Rc::new(vectors)))
    }

    fn null_space_value(&self, value: &Value) -> ExecResult<Value> {
        let matrix = self.matrix(value, "null_space")?;
        let columns = matrix[0].len();
        let (reduced, _, _) = self.rref_with_steps(matrix, None)?;
        let mut pivot_columns = Vec::new();
        for row in &reduced {
            if let Some(column) = row.iter().position(|entry| !Self::definitely_zero(entry)) {
                pivot_columns.push(column);
            }
        }
        let mut basis = Vec::new();
        for free in 0..columns {
            if pivot_columns.contains(&free) {
                continue;
            }
            let mut vector = vec![Self::integer_value(0); columns];
            vector[free] = Self::integer_value(1);
            for (row, pivot) in pivot_columns.iter().enumerate() {
                vector[*pivot] = self.apply_unary(UnaryOp::Neg, reduced[row][free].clone())?;
            }
            basis.push(Value::Array(Rc::new(vector)));
        }
        Ok(Value::Array(Rc::new(basis)))
    }

    fn column_space_value(&self, value: &Value) -> ExecResult<Value> {
        let matrix = self.matrix(value, "column_space")?;
        let (reduced, _, _) = self.rref_with_steps(matrix.clone(), None)?;
        let pivot_columns = reduced
            .iter()
            .filter_map(|row| row.iter().position(|entry| !Self::definitely_zero(entry)))
            .collect::<Vec<_>>();
        let columns = Self::transpose_matrix(&matrix);
        Ok(Value::Array(Rc::new(
            pivot_columns
                .into_iter()
                .map(|column| Value::Array(Rc::new(columns[column].clone())))
                .collect(),
        )))
    }

    fn jordan_form_value(&self, value: &Value) -> ExecResult<Value> {
        let matrix = self.square_matrix(value, "jordan_form")?;
        if matrix.len() == 1 {
            return Ok(Self::matrix_value(matrix));
        }
        if matrix.len() != 2 {
            return Err(self.error("jordan_form currently supports matrices up to 2x2"));
        }
        let Value::Array(values) = self.eigenvalues_value(value)? else {
            unreachable!()
        };
        if values[0].equals(&values[1]) {
            if Self::definitely_zero(&matrix[0][1])
                && Self::definitely_zero(&matrix[1][0])
                && matrix[0][0].equals(&matrix[1][1])
            {
                return Ok(Self::matrix_value(matrix));
            }
            return Ok(Self::matrix_value(vec![
                vec![values[0].clone(), Self::integer_value(1)],
                vec![Self::integer_value(0), values[0].clone()],
            ]));
        }
        Ok(Self::matrix_value(vec![
            vec![values[0].clone(), Self::integer_value(0)],
            vec![Self::integer_value(0), values[1].clone()],
        ]))
    }

    fn definitely_zero(value: &Value) -> bool {
        matches!(value, Value::Number(number) if number.is_zero())
    }

    fn rref_with_steps(
        &self,
        mut matrix: Matrix,
        pivot_columns: Option<usize>,
    ) -> ExecResult<Reduction> {
        let rows = matrix.len();
        let columns = matrix[0].len();
        let mut pivot_row = 0;
        let mut history = vec![DerivationStep {
            value: Self::matrix_value(matrix.clone()),
            operation: StepOperation::Start,
        }];
        for column in 0..pivot_columns.unwrap_or(columns).min(columns) {
            let Some(found) =
                (pivot_row..rows).find(|row| !Self::definitely_zero(&matrix[*row][column]))
            else {
                continue;
            };
            if found != pivot_row {
                matrix.swap(found, pivot_row);
                history.push(DerivationStep {
                    value: Self::matrix_value(matrix.clone()),
                    operation: StepOperation::SwapRows {
                        first: pivot_row,
                        second: found,
                    },
                });
            }
            let pivot = matrix[pivot_row][column].clone();
            for entry in &mut matrix[pivot_row] {
                *entry = self.apply_binary(BinaryOp::Div, entry.clone(), pivot.clone())?;
            }
            if !matches!(&pivot, Value::Number(number) if number.is_one()) {
                history.push(DerivationStep {
                    value: Self::matrix_value(matrix.clone()),
                    operation: StepOperation::ScaleRow {
                        row: pivot_row,
                        divisor: pivot.clone(),
                    },
                });
            }
            for row in 0..rows {
                if row == pivot_row || Self::definitely_zero(&matrix[row][column]) {
                    continue;
                }
                let factor = matrix[row][column].clone();
                for index in 0..columns {
                    let product = self.apply_binary(
                        BinaryOp::Mul,
                        factor.clone(),
                        matrix[pivot_row][index].clone(),
                    )?;
                    matrix[row][index] =
                        self.apply_binary(BinaryOp::Sub, matrix[row][index].clone(), product)?;
                }
                history.push(DerivationStep {
                    value: Self::matrix_value(matrix.clone()),
                    operation: StepOperation::AddRowMultiple {
                        target: row,
                        source: pivot_row,
                        factor,
                    },
                });
            }
            pivot_row += 1;
            if pivot_row == rows {
                break;
            }
        }
        Ok((matrix, history, pivot_row))
    }

    fn determinant_with_steps(
        &self,
        mut matrix: Vec<Vec<Value>>,
    ) -> ExecResult<(Value, Vec<DerivationStep>)> {
        let size = matrix.len();
        let mut determinant = Self::integer_value(1);
        let mut history = vec![DerivationStep {
            value: Self::matrix_value(matrix.clone()),
            operation: StepOperation::Start,
        }];
        for column in 0..size {
            let Some(found) =
                (column..size).find(|row| !Self::definitely_zero(&matrix[*row][column]))
            else {
                return Ok((Self::integer_value(0), history));
            };
            if found != column {
                matrix.swap(found, column);
                determinant = self.apply_unary(UnaryOp::Neg, determinant)?;
                history.push(DerivationStep {
                    value: Self::matrix_value(matrix.clone()),
                    operation: StepOperation::SwapRows {
                        first: column,
                        second: found,
                    },
                });
            }
            let pivot = matrix[column][column].clone();
            determinant = self.apply_binary(BinaryOp::Mul, determinant, pivot.clone())?;
            for row in column + 1..size {
                if Self::definitely_zero(&matrix[row][column]) {
                    continue;
                }
                let factor =
                    self.apply_binary(BinaryOp::Div, matrix[row][column].clone(), pivot.clone())?;
                for index in column..size {
                    let product = self.apply_binary(
                        BinaryOp::Mul,
                        factor.clone(),
                        matrix[column][index].clone(),
                    )?;
                    matrix[row][index] =
                        self.apply_binary(BinaryOp::Sub, matrix[row][index].clone(), product)?;
                }
                history.push(DerivationStep {
                    value: Self::matrix_value(matrix.clone()),
                    operation: StepOperation::AddRowMultiple {
                        target: row,
                        source: column,
                        factor,
                    },
                });
            }
        }
        Ok((determinant, history))
    }

    /// Division-free Laplace expansion used for characteristic polynomials.
    /// The ordinary determinant path uses elimination; that is faster, but
    /// introduces removable symbolic denominators into `det(xI - A)`.
    fn determinant_cofactor(&self, matrix: &[Vec<Value>]) -> ExecResult<Value> {
        if matrix.len() == 1 {
            return Ok(matrix[0][0].clone());
        }
        let mut total = Self::integer_value(0);
        for column in 0..matrix.len() {
            let minor = matrix[1..]
                .iter()
                .map(|row| {
                    row.iter()
                        .enumerate()
                        .filter(|(index, _)| *index != column)
                        .map(|(_, value)| value.clone())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            let mut term = self.apply_binary(
                BinaryOp::Mul,
                matrix[0][column].clone(),
                self.determinant_cofactor(&minor)?,
            )?;
            if column % 2 == 1 {
                term = self.apply_unary(UnaryOp::Neg, term)?;
            }
            total = self.apply_binary(BinaryOp::Add, total, term)?;
        }
        Ok(total)
    }

    fn matmul_values(
        &self,
        left: &[Vec<Value>],
        right: &[Vec<Value>],
        operation: &str,
    ) -> ExecResult<Vec<Vec<Value>>> {
        if left[0].len() != right.len() {
            return Err(self.error(format!("{operation} matrix dimensions do not align")));
        }
        let mut product = vec![vec![Self::integer_value(0); right[0].len()]; left.len()];
        for row in 0..left.len() {
            for column in 0..right[0].len() {
                for (index, right_row) in right.iter().enumerate() {
                    let term = self.apply_binary(
                        BinaryOp::Mul,
                        left[row][index].clone(),
                        right_row[column].clone(),
                    )?;
                    product[row][column] =
                        self.apply_binary(BinaryOp::Add, product[row][column].clone(), term)?;
                }
            }
        }
        Ok(product)
    }

    fn matrix_product(&self, left: &Value, right: &Value, operation: &str) -> ExecResult<Value> {
        let left = self.matrix(left, operation)?;
        let Value::Array(right_values) = right else {
            return Err(self.error(format!(
                "{operation} expects a matrix or vector as its right operand"
            )));
        };

        if right_values
            .iter()
            .any(|value| matches!(value, Value::Array(_)))
        {
            let right = self.matrix(right, operation)?;
            return self
                .matmul_values(&left, &right, operation)
                .map(Self::matrix_value);
        }

        let right = self.vector(right, operation)?;
        if left[0].len() != right.len() {
            return Err(self.error(format!(
                "{operation} matrix and vector dimensions do not align"
            )));
        }
        let mut product = Vec::with_capacity(left.len());
        for row in left {
            let mut total = Self::integer_value(0);
            for (left, right) in row.into_iter().zip(&right) {
                let term = self.apply_binary(BinaryOp::Mul, left, right.clone())?;
                total = self.apply_binary(BinaryOp::Add, total, term)?;
            }
            product.push(total);
        }
        Ok(Value::Array(Rc::new(product)))
    }

    fn call_calculus_tool(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        match builtin {
            Builtin::Gradient | Builtin::Hessian => {
                let expression = self.value_to_symbolic(args[0].clone())?;
                let variables = self.calculus_variables(&args[1], builtin.name())?;
                let gradient = variables
                    .iter()
                    .map(|variable| {
                        let derivative = self.derivative_symbolic(&expression, *variable);
                        self.simplify_symbolic_tree(&derivative)
                            .map(Self::symbolic_value)
                    })
                    .collect::<ExecResult<Vec<_>>>()?;
                if builtin == Builtin::Gradient {
                    return Ok(Value::Array(Rc::new(gradient)));
                }
                let mut rows = Vec::with_capacity(variables.len());
                for first in variables {
                    let first_derivative = self.derivative_symbolic(&expression, first);
                    let row = self
                        .calculus_variables(&args[1], builtin.name())?
                        .into_iter()
                        .map(|second| {
                            let derivative = self.derivative_symbolic(&first_derivative, second);
                            self.simplify_symbolic_tree(&derivative)
                                .map(Self::symbolic_value)
                        })
                        .collect::<ExecResult<Vec<_>>>()?;
                    rows.push(Value::Array(Rc::new(row)));
                }
                Ok(Value::Array(Rc::new(rows)))
            }
            Builtin::Jacobian => {
                let Value::Array(expressions) = &args[0] else {
                    return Err(self.error("jacobian expects an array of expressions"));
                };
                let variables = self.calculus_variables(&args[1], "jacobian")?;
                let mut rows = Vec::with_capacity(expressions.len());
                for expression in expressions.iter() {
                    let expression = self.value_to_symbolic(expression.clone())?;
                    let row = variables
                        .iter()
                        .map(|variable| {
                            let derivative = self.derivative_symbolic(&expression, *variable);
                            self.simplify_symbolic_tree(&derivative)
                                .map(Self::symbolic_value)
                        })
                        .collect::<ExecResult<Vec<_>>>()?;
                    rows.push(Value::Array(Rc::new(row)));
                }
                Ok(Value::Array(Rc::new(rows)))
            }
            Builtin::DefiniteIntegral => {
                let expression = self.value_to_symbolic(args[0].clone())?;
                let Value::Symbolic(variable_expression) = &args[1] else {
                    return Err(self.error("definite_integral expects a symbolic variable"));
                };
                let SymbolicExpr::Variable(variable) = variable_expression.as_ref() else {
                    return Err(self.error("definite_integral expects a symbolic variable"));
                };
                let lower = self.value_to_symbolic(args[2].clone())?;
                let upper = self.value_to_symbolic(args[3].clone())?;
                let result =
                    self.definite_integral_symbolic(&expression, *variable, &lower, &upper)?;
                Ok(Self::symbolic_value(result))
            }
            Builtin::Limit => {
                let expression = self.value_to_symbolic(args[0].clone())?;
                let variable = self.calculus_variable(&args[1], "limit")?;
                if args.len() == 4 {
                    let Value::Str(side) = &args[3] else {
                        return Err(self.error("limit side must be \"left\" or \"right\""));
                    };
                    if side != "left" && side != "right" {
                        return Err(self.error("limit side must be \"left\" or \"right\""));
                    }
                }
                let point = self.value_to_symbolic(args[2].clone())?;
                let result = self.limit_symbolic(expression, variable, &point)?;
                Ok(Self::symbolic_value(result))
            }
            Builtin::Series => {
                let expression = self.value_to_symbolic(args[0].clone())?;
                let variable = self.calculus_variable(&args[1], "series")?;
                let point = self.value_to_symbolic(args[2].clone())?;
                let order = self.nonnegative_integer(&args[3], "series order")?;
                if order > 64 {
                    return Err(self.limit_error("series order exceeds the limit of 64"));
                }
                let variable_expression = Rc::new(SymbolicExpr::Variable(variable));
                let offset = Self::symbolic_binary_raw(
                    variable_expression,
                    BinaryOp::Sub,
                    Rc::clone(&point),
                );
                let mut derivative = expression;
                let mut result = Self::symbolic_integer(0);
                let mut factorial = BigInt::one();
                for degree in 0..=order {
                    let coefficient = Self::substitute_symbolic(&derivative, variable, &point);
                    let coefficient = self.simplify_symbolic_tree(&coefficient)?;
                    let term = Self::symbolic_binary_raw(
                        coefficient,
                        BinaryOp::Mul,
                        Self::symbolic_binary_raw(
                            Rc::clone(&offset),
                            BinaryOp::Pow,
                            Self::symbolic_integer(degree as i64),
                        ),
                    );
                    let term = Self::symbolic_binary_raw(
                        term,
                        BinaryOp::Div,
                        Rc::new(SymbolicExpr::Number(Number::Integer(factorial.clone()))),
                    );
                    result = self.simplify_symbolic_tree(&Self::symbolic_binary_raw(
                        result,
                        BinaryOp::Add,
                        term,
                    ))?;
                    derivative = self.derivative_symbolic(&derivative, variable);
                    factorial *= BigInt::from(degree + 1);
                }
                Ok(Self::symbolic_value(result))
            }
            Builtin::Sum | Builtin::Product => {
                let expression = self.value_to_symbolic(args[0].clone())?;
                let variable = self.calculus_variable(&args[1], builtin.name())?;
                if let (Ok(lower), Ok(upper)) = (
                    self.integer_argument(&args[2], builtin.name()),
                    self.integer_argument(&args[3], builtin.name()),
                ) {
                    if upper < lower {
                        return Ok(Self::integer_value(i64::from(builtin == Builtin::Product)));
                    }
                    if upper - lower > 10_000 {
                        return Err(self.limit_error(format!(
                            "{} range exceeds the limit of 10001 terms",
                            builtin.name()
                        )));
                    }
                    let mut result = Self::integer_value(i64::from(builtin == Builtin::Product));
                    for index in lower..=upper {
                        let replacement =
                            Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(index))));
                        let term = Self::substitute_symbolic(&expression, variable, &replacement);
                        let term = Self::symbolic_value(self.simplify_symbolic_tree(&term)?);
                        result = self.apply_binary(
                            if builtin == Builtin::Sum {
                                BinaryOp::Add
                            } else {
                                BinaryOp::Mul
                            },
                            result,
                            term,
                        )?;
                    }
                    return Ok(result);
                }
                let lower = self.value_to_symbolic(args[2].clone())?;
                let upper = self.value_to_symbolic(args[3].clone())?;
                if builtin == Builtin::Sum {
                    if let Some(result) =
                        self.faulhaber_sum(&expression, variable, &lower, &upper)?
                    {
                        return Ok(Self::symbolic_value(result));
                    }
                }
                Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                    function: builtin,
                    args: vec![
                        expression,
                        Rc::new(SymbolicExpr::Variable(variable)),
                        lower,
                        upper,
                    ],
                })))
            }
            Builtin::MultipleIntegral => {
                let mut expression = self.value_to_symbolic(args[0].clone())?;
                let variables = self.calculus_variables(&args[1], "multiple_integral")?;
                for variable in variables {
                    expression = self.integral_symbolic(&expression, variable);
                    expression = self.simplify_symbolic_tree(&expression)?;
                }
                Ok(Self::symbolic_value(expression))
            }
            Builtin::Dsolve => {
                if args.len() == 4 {
                    let coefficient = self.value_to_symbolic(args[0].clone())?;
                    let forcing = self.value_to_symbolic(args[1].clone())?;
                    let variable = self.calculus_variable(&args[2], "dsolve")?;
                    let constant = self.value_to_symbolic(args[3].clone())?;
                    let integral_coefficient = self.integral_symbolic(&coefficient, variable);
                    let integrating_factor =
                        Self::symbolic_call(Builtin::Exp, Rc::clone(&integral_coefficient));
                    let weighted_forcing = Self::symbolic_binary_raw(
                        forcing,
                        BinaryOp::Mul,
                        Rc::clone(&integrating_factor),
                    );
                    let particular = self.integral_symbolic(&weighted_forcing, variable);
                    let numerator = Self::symbolic_binary_raw(constant, BinaryOp::Add, particular);
                    let result =
                        Self::symbolic_binary_raw(numerator, BinaryOp::Div, integrating_factor);
                    return Ok(Self::symbolic_value(self.simplify_symbolic_tree(&result)?));
                }
                if args.len() != 2 {
                    return Err(self.error(
                        "dsolve expects (coefficients, variable) or (p, q, variable, constant)",
                    ));
                }
                let coefficients = self.vector(&args[0], "dsolve")?;
                let variable = self.calculus_variable(&args[1], "dsolve")?;
                self.constant_coefficient_ode_basis(&coefficients, variable)
            }
            _ => unreachable!("calculus tool dispatched separately"),
        }
    }

    fn calculus_variable(&self, value: &Value, function: &str) -> ExecResult<Symbol> {
        let Value::Symbolic(expression) = value else {
            return Err(self.error(format!("{function} expects a symbolic variable")));
        };
        let SymbolicExpr::Variable(variable) = expression.as_ref() else {
            return Err(self.error(format!("{function} expects a symbolic variable")));
        };
        Ok(*variable)
    }

    fn integer_argument(&self, value: &Value, function: &str) -> ExecResult<i64> {
        let Value::Number(Number::Integer(integer)) = value else {
            return Err(self.error(format!("{function} expects integer bounds")));
        };
        integer
            .to_i64()
            .ok_or_else(|| self.limit_error(format!("{function} bound is too large")))
    }

    fn nonnegative_integer(&self, value: &Value, name: &str) -> ExecResult<u32> {
        let Value::Number(Number::Integer(integer)) = value else {
            return Err(self.error(format!("{name} must be a nonnegative integer")));
        };
        integer
            .to_u32()
            .ok_or_else(|| self.error(format!("{name} must be a nonnegative integer")))
    }

    fn limit_symbolic(
        &self,
        mut expression: Rc<SymbolicExpr>,
        variable: Symbol,
        point: &Rc<SymbolicExpr>,
    ) -> ExecResult<Rc<SymbolicExpr>> {
        for _ in 0..=8 {
            let substituted = Self::substitute_symbolic(&expression, variable, point);
            if let SymbolicExpr::Binary {
                left,
                op: BinaryOp::Div,
                right,
            } = substituted.as_ref()
            {
                let numerator = self.simplify_symbolic_tree(left)?;
                let denominator = self.simplify_symbolic_tree(right)?;
                let numerator_zero =
                    matches!(numerator.as_ref(), SymbolicExpr::Number(number) if number.is_zero());
                let denominator_zero = matches!(denominator.as_ref(), SymbolicExpr::Number(number) if number.is_zero());
                if numerator_zero && denominator_zero {
                    let SymbolicExpr::Binary { left, right, .. } = expression.as_ref() else {
                        unreachable!()
                    };
                    expression = Self::symbolic_binary_raw(
                        self.derivative_symbolic(left, variable),
                        BinaryOp::Div,
                        self.derivative_symbolic(right, variable),
                    );
                    continue;
                }
            }
            return self.simplify_symbolic_tree(&substituted);
        }
        Ok(Rc::new(SymbolicExpr::Call {
            function: Builtin::Limit,
            args: vec![
                expression,
                Rc::new(SymbolicExpr::Variable(variable)),
                Rc::clone(point),
            ],
        }))
    }

    fn faulhaber_sum(
        &self,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
        lower: &Rc<SymbolicExpr>,
        upper: &Rc<SymbolicExpr>,
    ) -> ExecResult<Option<Rc<SymbolicExpr>>> {
        fn power_of(expression: &SymbolicExpr, variable: Symbol) -> Option<u32> {
            match expression {
                SymbolicExpr::Variable(found) if *found == variable => Some(1),
                SymbolicExpr::Binary {
                    left,
                    op: BinaryOp::Pow,
                    right,
                } if matches!(left.as_ref(), SymbolicExpr::Variable(found) if *found == variable) =>
                {
                    let SymbolicExpr::Number(Number::Integer(power)) = right.as_ref() else {
                        return None;
                    };
                    power.to_u32()
                }
                SymbolicExpr::Number(number) if number.is_one() => Some(0),
                _ => None,
            }
        }
        let Some(power) = power_of(expression, variable) else {
            return Ok(None);
        };
        if power > 3 {
            return Ok(None);
        }
        let prefix = |end: Rc<SymbolicExpr>| {
            let one = Self::symbolic_integer(1);
            let next = Self::symbolic_binary_raw(Rc::clone(&end), BinaryOp::Add, one);
            match power {
                0 => end,
                1 => Self::symbolic_binary_raw(
                    Self::symbolic_binary_raw(end, BinaryOp::Mul, next),
                    BinaryOp::Div,
                    Self::symbolic_integer(2),
                ),
                2 => {
                    let twice = Self::symbolic_binary_raw(
                        Self::symbolic_integer(2),
                        BinaryOp::Mul,
                        Rc::clone(&end),
                    );
                    let final_factor =
                        Self::symbolic_binary_raw(twice, BinaryOp::Add, Self::symbolic_integer(1));
                    Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(end, BinaryOp::Mul, next),
                            BinaryOp::Mul,
                            final_factor,
                        ),
                        BinaryOp::Div,
                        Self::symbolic_integer(6),
                    )
                }
                3 => Self::symbolic_binary_raw(
                    Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(end, BinaryOp::Mul, next),
                        BinaryOp::Div,
                        Self::symbolic_integer(2),
                    ),
                    BinaryOp::Pow,
                    Self::symbolic_integer(2),
                ),
                _ => unreachable!(),
            }
        };
        let before_lower =
            Self::symbolic_binary_raw(Rc::clone(lower), BinaryOp::Sub, Self::symbolic_integer(1));
        let result = Self::symbolic_binary_raw(
            prefix(Rc::clone(upper)),
            BinaryOp::Sub,
            prefix(before_lower),
        );
        Ok(Some(self.simplify_symbolic_tree(&result)?))
    }

    fn constant_coefficient_ode_basis(
        &self,
        coefficients: &[Value],
        variable: Symbol,
    ) -> ExecResult<Value> {
        if !(2..=3).contains(&coefficients.len()) {
            return Err(self.error(
                "dsolve constant-coefficient form currently supports first- and second-order equations",
            ));
        }
        if Self::definitely_zero(&coefficients[0]) {
            return Err(self.error("dsolve leading coefficient must be nonzero"));
        }
        let roots = if coefficients.len() == 2 {
            vec![self.apply_binary(
                BinaryOp::Div,
                self.apply_unary(UnaryOp::Neg, coefficients[1].clone())?,
                coefficients[0].clone(),
            )?]
        } else {
            let four_ac = self.apply_binary(
                BinaryOp::Mul,
                Self::integer_value(4),
                self.apply_binary(
                    BinaryOp::Mul,
                    coefficients[0].clone(),
                    coefficients[2].clone(),
                )?,
            )?;
            let b_squared = self.apply_binary(
                BinaryOp::Mul,
                coefficients[1].clone(),
                coefficients[1].clone(),
            )?;
            let discriminant = self.apply_binary(BinaryOp::Sub, b_squared, four_ac)?;
            let root = self.call_builtin(Builtin::Sqrt, vec![discriminant])?;
            let negative_b = self.apply_unary(UnaryOp::Neg, coefficients[1].clone())?;
            let denominator = self.apply_binary(
                BinaryOp::Mul,
                Self::integer_value(2),
                coefficients[0].clone(),
            )?;
            vec![
                self.apply_binary(
                    BinaryOp::Div,
                    self.apply_binary(BinaryOp::Add, negative_b.clone(), root.clone())?,
                    denominator.clone(),
                )?,
                self.apply_binary(
                    BinaryOp::Div,
                    self.apply_binary(BinaryOp::Sub, negative_b, root)?,
                    denominator,
                )?,
            ]
        };
        let variable_expression = Rc::new(SymbolicExpr::Variable(variable));
        let mut basis = roots
            .iter()
            .cloned()
            .map(|root| {
                let root = self.value_to_symbolic(root)?;
                let exponent =
                    Self::symbolic_binary_raw(root, BinaryOp::Mul, Rc::clone(&variable_expression));
                self.simplify_symbolic_tree(&Self::symbolic_call(Builtin::Exp, exponent))
                    .map(Self::symbolic_value)
            })
            .collect::<ExecResult<Vec<_>>>()?;
        if roots.len() == 2 && roots[0].equals(&roots[1]) {
            basis[1] = self.apply_binary(
                BinaryOp::Mul,
                Self::symbolic_value(variable_expression),
                basis[1].clone(),
            )?;
        }
        Ok(Value::Array(Rc::new(basis)))
    }

    fn calculus_variables(&self, value: &Value, function: &str) -> ExecResult<Vec<Symbol>> {
        let Value::Array(values) = value else {
            return Err(self.error(format!("{function} expects an array of variables")));
        };
        values
            .iter()
            .map(|value| {
                let Value::Symbolic(expression) = value else {
                    return Err(
                        self.error(format!("{function} expects an array of symbolic variables"))
                    );
                };
                let SymbolicExpr::Variable(variable) = expression.as_ref() else {
                    return Err(
                        self.error(format!("{function} expects an array of symbolic variables"))
                    );
                };
                Ok(*variable)
            })
            .collect()
    }

    fn unary_math_function(builtin: Builtin) -> Option<UnaryMathFunction> {
        Some(match builtin {
            Builtin::Sqrt => UnaryMathFunction::Sqrt,
            Builtin::Cbrt => UnaryMathFunction::Cbrt,
            Builtin::Exp => UnaryMathFunction::Exp,
            Builtin::Ln | Builtin::Log => UnaryMathFunction::Ln,
            Builtin::Log10 => UnaryMathFunction::Log10,
            Builtin::Log2 => UnaryMathFunction::Log2,
            Builtin::Sin => UnaryMathFunction::Sin,
            Builtin::Cos => UnaryMathFunction::Cos,
            Builtin::Tan => UnaryMathFunction::Tan,
            Builtin::Sec => UnaryMathFunction::Sec,
            Builtin::Csc => UnaryMathFunction::Csc,
            Builtin::Cot => UnaryMathFunction::Cot,
            Builtin::Asin => UnaryMathFunction::Asin,
            Builtin::Acos => UnaryMathFunction::Acos,
            Builtin::Atan => UnaryMathFunction::Atan,
            Builtin::Sinh => UnaryMathFunction::Sinh,
            Builtin::Cosh => UnaryMathFunction::Cosh,
            Builtin::Tanh => UnaryMathFunction::Tanh,
            Builtin::Asinh => UnaryMathFunction::Asinh,
            Builtin::Acosh => UnaryMathFunction::Acosh,
            Builtin::Atanh => UnaryMathFunction::Atanh,
            Builtin::Degrees => UnaryMathFunction::Degrees,
            Builtin::Radians => UnaryMathFunction::Radians,
            Builtin::Erf => UnaryMathFunction::Erf,
            Builtin::Erfc => UnaryMathFunction::Erfc,
            Builtin::Gamma => UnaryMathFunction::Gamma,
            Builtin::Lgamma => UnaryMathFunction::Lgamma,
            _ => return None,
        })
    }

    fn exact_special_value(&self, builtin: Builtin, number: &Number) -> ExecResult<Option<Value>> {
        let pi_fraction = |numerator: i64, denominator: i64| {
            let pi = Rc::new(SymbolicExpr::Constant(MathConstant::Pi));
            if numerator == 0 {
                return Value::Number(Number::Integer(BigInt::from(0)));
            }
            if numerator == denominator {
                return Self::symbolic_value(pi);
            }
            let fraction = Number::from_rational(BigRational::new(
                BigInt::from(numerator),
                BigInt::from(denominator),
            ));
            Self::symbolic_value(Rc::new(SymbolicExpr::Binary {
                left: Rc::new(SymbolicExpr::Number(fraction)),
                op: BinaryOp::Mul,
                right: pi,
            }))
        };
        if number.is_one() {
            let value = match builtin {
                Builtin::Exp => Some(Self::symbolic_value(Rc::new(SymbolicExpr::Constant(
                    MathConstant::E,
                )))),
                Builtin::Asin => Some(pi_fraction(1, 2)),
                Builtin::Acos => Some(Value::Number(Number::Integer(BigInt::from(0)))),
                Builtin::Atan => Some(pi_fraction(1, 4)),
                _ => None,
            };
            if value.is_some() {
                return Ok(value);
            }
        }
        if number == &Number::Integer(BigInt::from(-1)) {
            let value = match builtin {
                Builtin::Asin => Some(pi_fraction(-1, 2)),
                Builtin::Acos => Some(pi_fraction(1, 1)),
                Builtin::Atan => Some(pi_fraction(-1, 4)),
                _ => None,
            };
            if value.is_some() {
                return Ok(value);
            }
        }
        if builtin == Builtin::Radians && number == &Number::Integer(BigInt::from(180)) {
            return Ok(Some(Self::symbolic_value(Rc::new(SymbolicExpr::Constant(
                MathConstant::Pi,
            )))));
        }
        if let Some(base) = match builtin {
            Builtin::Log10 => Some(Number::Integer(BigInt::from(10))),
            Builtin::Log2 => Some(Number::Integer(BigInt::from(2))),
            _ => None,
        } {
            if let Some(exponent) = Self::exact_integer_log(number, &base) {
                return Ok(Some(Value::Number(Number::Integer(exponent))));
            }
        }
        let Number::Integer(integer) = number else {
            return Ok(None);
        };
        if !matches!(builtin, Builtin::Gamma | Builtin::Lgamma) || integer <= &BigInt::from(0) {
            return Ok(None);
        }
        let limit = integer.to_u32().ok_or_else(|| {
            self.limit_error("gamma integer argument is too large to evaluate exactly")
        })?;
        if limit > self.limits.max_exponent {
            return Err(self.limit_error(format!(
                "gamma integer argument exceeds the exact evaluation limit of {}",
                self.limits.max_exponent
            )));
        }
        let mut factorial = BigInt::from(1);
        for factor in 2..limit {
            factorial *= factor;
            if factorial.bits() > self.limits.max_number_bits {
                return Err(self.limit_error(format!(
                    "numeric result exceeds the {} bit limit",
                    self.limits.max_number_bits
                )));
            }
        }
        if builtin == Builtin::Gamma {
            return Ok(Some(Value::Number(Number::Integer(factorial))));
        }
        if factorial.is_one() {
            Ok(Some(Value::Number(Number::Integer(BigInt::from(0)))))
        } else {
            Ok(Some(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                function: Builtin::Ln,
                args: vec![Rc::new(SymbolicExpr::Number(Number::Integer(factorial)))],
            }))))
        }
    }

    fn exact_integer_log(value: &Number, base: &Number) -> Option<BigInt> {
        let (Number::Integer(value), Number::Integer(base)) = (value, base) else {
            return None;
        };
        if value <= &BigInt::from(0) || base <= &BigInt::from(1) {
            return None;
        }
        if value == &BigInt::from(1) {
            return Some(BigInt::from(0));
        }
        let mut current = value.clone();
        let mut exponent = BigInt::from(0);
        while current > BigInt::from(1) {
            let (quotient, remainder) = current.div_rem(base);
            if !remainder.is_zero() {
                return None;
            }
            current = quotient;
            exponent += 1;
        }
        Some(exponent)
    }

    fn evalf_value(&self, value: Value) -> ExecResult<Value> {
        match value {
            Value::Number(_) => Ok(value),
            Value::Array(values) => values
                .iter()
                .cloned()
                .map(|value| self.evalf_value(value))
                .collect::<ExecResult<Vec<_>>>()
                .map(|values| Value::Array(Rc::new(values))),
            Value::Symbolic(expression) => self.evalf_symbolic(&expression),
            value => Err(self.error(format!(
                "evalf expects a numeric or symbolic expression, got {}",
                value.type_name()
            ))),
        }
    }

    fn evalf_symbolic(&self, expression: &Rc<SymbolicExpr>) -> ExecResult<Value> {
        match expression.as_ref() {
            SymbolicExpr::Number(number) => Ok(Value::Number(number.clone())),
            SymbolicExpr::Constant(constant) => {
                Number::constant(*constant, self.transcendental_precision()?)
                    .map(Value::Number)
                    .map_err(|error| self.error(format!("evalf failed: {error}")))
            }
            SymbolicExpr::Variable(symbol) => Err(self.error(format!(
                "evalf cannot evaluate unbound variable '{}'",
                self.symbols.resolve(*symbol)
            ))),
            SymbolicExpr::Unary { op, expr } => {
                let value = self.evalf_symbolic(expr)?;
                self.apply_unary(*op, value)
            }
            SymbolicExpr::Binary { left, op, right } => {
                let left = self.evalf_symbolic(left)?;
                let right = self.evalf_symbolic(right)?;
                if *op == BinaryOp::Pow {
                    let left_number = self.builtin_number(Builtin::Exp, &left)?;
                    let right_number = self.builtin_number(Builtin::Exp, &right)?;
                    return left_number
                        .evaluate_binary(
                            right_number,
                            BinaryMathFunction::Pow,
                            self.transcendental_precision()?,
                        )
                        .map(Value::Number)
                        .map_err(|error| self.error(format!("power failed: {error}")));
                }
                self.apply_binary(*op, left, right)
            }
            SymbolicExpr::Call { function, args } => {
                let args = args
                    .iter()
                    .map(|argument| self.evalf_symbolic(argument))
                    .collect::<ExecResult<Vec<_>>>()?;
                if args.len() == 1 {
                    if let Some(math_function) = Self::unary_math_function(*function) {
                        let number = self.builtin_number(*function, &args[0])?;
                        return number
                            .evaluate_unary(math_function, self.transcendental_precision()?)
                            .map(Value::Number)
                            .map_err(|error| {
                                self.error(format!("{} failed: {error}", function.name()))
                            });
                    }
                }
                if args.len() == 2 {
                    let math_function = match function {
                        Builtin::Log => Some(BinaryMathFunction::Log),
                        Builtin::Atan2 => Some(BinaryMathFunction::Atan2),
                        Builtin::Hypot => Some(BinaryMathFunction::Hypot),
                        _ => None,
                    };
                    if let Some(math_function) = math_function {
                        let left = self.builtin_number(*function, &args[0])?;
                        let right = self.builtin_number(*function, &args[1])?;
                        return left
                            .evaluate_binary(right, math_function, self.transcendental_precision()?)
                            .map(Value::Number)
                            .map_err(|error| {
                                self.error(format!("{} failed: {error}", function.name()))
                            });
                    }
                }
                self.call_builtin(*function, args)
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

    /// Expands trigonometric multiple angles, squares, and mixed products:
    /// `cos(3*x)` into powers of `sin(x)`/`cos(x)`, `sin(x)^2` into
    /// `(1 - cos(2*x))/2`, and `sin(a)*cos(b)` into a sum of sines. Hyperbolic
    /// analogues are included.
    fn expand_trigonometric(
        &self,
        expression: &Rc<SymbolicExpr>,
    ) -> ExecResult<Option<Rc<SymbolicExpr>>> {
        use BinaryOp::{Add, Mul, Sub};
        const MAX_ANGLE_MULTIPLE: i64 = 12;
        let call = |function, argument: Rc<SymbolicExpr>| {
            Rc::new(SymbolicExpr::Call {
                function,
                args: vec![argument],
            })
        };
        // Squares reduce to doubled angles: sin(x)^2 -> (1 - cos(2x))/2.
        if let Some((function, argument)) = Self::squared_unary_call(expression) {
            let doubled =
                Self::symbolic_binary_raw(Self::symbolic_integer(2), Mul, Rc::clone(&argument));
            let reduced = match function {
                Builtin::Sin => Some(Self::symbolic_binary_raw(
                    Self::symbolic_integer(1),
                    Sub,
                    call(Builtin::Cos, doubled),
                )),
                Builtin::Cos => Some(Self::symbolic_binary_raw(
                    Self::symbolic_integer(1),
                    Add,
                    call(Builtin::Cos, doubled),
                )),
                Builtin::Sinh => Some(Self::symbolic_binary_raw(
                    call(Builtin::Cosh, doubled),
                    Sub,
                    Self::symbolic_integer(1),
                )),
                Builtin::Cosh => Some(Self::symbolic_binary_raw(
                    call(Builtin::Cosh, doubled),
                    Add,
                    Self::symbolic_integer(1),
                )),
                _ => None,
            };
            if let Some(reduced) = reduced {
                return Ok(Some(Self::symbolic_binary_raw(
                    reduced,
                    BinaryOp::Div,
                    Self::symbolic_integer(2),
                )));
            }
        }
        // Products of two trigonometric calls expand into sums of the summed
        // and differenced angles: sin(a)*cos(b) -> (sin(a+b) + sin(a-b))/2.
        if matches!(expression.as_ref(), SymbolicExpr::Binary { op: Mul, .. }) {
            let mut factors = Vec::new();
            Self::mul_factors(expression, &mut factors);
            let mut coefficient = Self::symbolic_integer(1);
            let mut trigonometric = Vec::new();
            for factor in factors {
                match factor.as_ref() {
                    SymbolicExpr::Call { function, args }
                        if args.len() == 1
                            && matches!(
                                function,
                                Builtin::Sin | Builtin::Cos | Builtin::Sinh | Builtin::Cosh
                            ) =>
                    {
                        if trigonometric.len() == 2 {
                            return Ok(None);
                        }
                        trigonometric.push((*function, Rc::clone(&args[0])));
                    }
                    SymbolicExpr::Number(_) => {
                        coefficient =
                            Self::symbolic_binary_raw(coefficient, Mul, Rc::clone(&factor));
                    }
                    _ => return Ok(None),
                }
            }
            if trigonometric.len() == 2 {
                let (left_fn, left_arg) = trigonometric[0].clone();
                let (right_fn, right_arg) = trigonometric[1].clone();
                let sum =
                    Self::symbolic_binary_raw(Rc::clone(&left_arg), Add, Rc::clone(&right_arg));
                let difference =
                    Self::symbolic_binary_raw(Rc::clone(&left_arg), Sub, Rc::clone(&right_arg));
                let (first_fn, first_angle, second_fn, second_angle, combine) =
                    match (left_fn, right_fn) {
                        (Builtin::Sin, Builtin::Cos) => {
                            (Builtin::Sin, &sum, Builtin::Sin, &difference, Add)
                        }
                        (Builtin::Cos, Builtin::Sin) => {
                            (Builtin::Sin, &sum, Builtin::Sin, &difference, Sub)
                        }
                        (Builtin::Sin, Builtin::Sin) => {
                            (Builtin::Cos, &difference, Builtin::Cos, &sum, Sub)
                        }
                        (Builtin::Cos, Builtin::Cos) => {
                            (Builtin::Cos, &difference, Builtin::Cos, &sum, Add)
                        }
                        (Builtin::Sinh, Builtin::Cosh) => {
                            (Builtin::Sinh, &sum, Builtin::Sinh, &difference, Add)
                        }
                        (Builtin::Cosh, Builtin::Sinh) => {
                            (Builtin::Sinh, &sum, Builtin::Sinh, &difference, Sub)
                        }
                        (Builtin::Sinh, Builtin::Sinh) => {
                            (Builtin::Cosh, &sum, Builtin::Cosh, &difference, Sub)
                        }
                        (Builtin::Cosh, Builtin::Cosh) => {
                            (Builtin::Cosh, &sum, Builtin::Cosh, &difference, Add)
                        }
                        _ => return Ok(None),
                    };
                let total = Self::symbolic_binary_raw(
                    call(first_fn, Rc::clone(first_angle)),
                    combine,
                    call(second_fn, Rc::clone(second_angle)),
                );
                let halved =
                    Self::symbolic_binary_raw(total, BinaryOp::Div, Self::symbolic_integer(2));
                return Ok(Some(Self::symbolic_binary_raw(coefficient, Mul, halved)));
            }
        }
        // Addition formulas for the hyperbolic and tangent functions that
        // the plain angle-sum rules do not cover.
        if let SymbolicExpr::Call { function, args } = expression.as_ref() {
            if args.len() == 1 {
                if let SymbolicExpr::Binary {
                    left,
                    op: op @ (Add | Sub),
                    right,
                } = args[0].as_ref()
                {
                    let plus = *op == Add;
                    let product = |first: Builtin, second: Builtin| {
                        Self::symbolic_binary_raw(
                            call(first, Rc::clone(left)),
                            Mul,
                            call(second, Rc::clone(right)),
                        )
                    };
                    let combine = if plus { Add } else { Sub };
                    let expanded = match function {
                        // sinh(a ∓ b) = sinh(a)cosh(b) ∓ cosh(a)sinh(b)
                        Builtin::Sinh => Some(Self::symbolic_binary_raw(
                            product(Builtin::Sinh, Builtin::Cosh),
                            combine,
                            product(Builtin::Cosh, Builtin::Sinh),
                        )),
                        // cosh(a ∓ b) = cosh(a)cosh(b) ∓ sinh(a)sinh(b)
                        Builtin::Cosh => Some(Self::symbolic_binary_raw(
                            product(Builtin::Cosh, Builtin::Cosh),
                            combine,
                            product(Builtin::Sinh, Builtin::Sinh),
                        )),
                        _ => None,
                    };
                    if let Some(expanded) = expanded {
                        return Ok(Some(expanded));
                    }
                    // tan(a ∓ b) = (tan(a) ∓ tan(b)) / (1 ± tan(a)tan(b))
                    if matches!(function, Builtin::Tan | Builtin::Tanh) {
                        let numerator = Self::symbolic_binary_raw(
                            call(*function, Rc::clone(left)),
                            if plus { Add } else { Sub },
                            call(*function, Rc::clone(right)),
                        );
                        let denominator = Self::symbolic_binary_raw(
                            Self::symbolic_integer(1),
                            if plus { Sub } else { Add },
                            Self::symbolic_binary_raw(
                                call(*function, Rc::clone(left)),
                                Mul,
                                call(*function, Rc::clone(right)),
                            ),
                        );
                        return Ok(Some(Self::symbolic_binary_raw(
                            numerator,
                            BinaryOp::Div,
                            denominator,
                        )));
                    }
                }
            }
        }
        // Multiple angles expand into powers of the single angle through the
        // addition recurrence, tracking the sine-like and cosine-like halves
        // of the pair together.
        if let SymbolicExpr::Call { function, args } = expression.as_ref() {
            if args.len() == 1
                && matches!(
                    function,
                    Builtin::Sin | Builtin::Cos | Builtin::Sinh | Builtin::Cosh
                )
            {
                if let SymbolicExpr::Binary {
                    left,
                    op: Mul,
                    right,
                } = args[0].as_ref()
                {
                    let (coefficient, base) = match (left.as_ref(), right.as_ref()) {
                        (SymbolicExpr::Number(Number::Integer(value)), _) => {
                            (value.clone(), Rc::clone(right))
                        }
                        (_, SymbolicExpr::Number(Number::Integer(value))) => {
                            (value.clone(), Rc::clone(left))
                        }
                        _ => return Ok(None),
                    };
                    if coefficient.abs() > BigInt::from(MAX_ANGLE_MULTIPLE) {
                        return Ok(None);
                    }
                    let steps = coefficient.abs().to_i64().unwrap_or(0);
                    if steps < 2 {
                        return Ok(None);
                    }
                    let hyperbolic = matches!(function, Builtin::Sinh | Builtin::Cosh);
                    let (sine_fn, cosine_fn) = if hyperbolic {
                        (Builtin::Sinh, Builtin::Cosh)
                    } else {
                        (Builtin::Sin, Builtin::Cos)
                    };
                    // Pair recurrences: s((n+1)u) = s(nu)cu + c(nu)su and
                    // c((n+1)u) = c(nu)cu ∓ s(nu)su (minus for cosine,
                    // plus for cosh).
                    let s_u = call(sine_fn, Rc::clone(&base));
                    let c_u = call(cosine_fn, Rc::clone(&base));
                    let mut s = Rc::clone(&s_u);
                    let mut c = Rc::clone(&c_u);
                    for _ in 1..steps {
                        let next_s = Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(Rc::clone(&s), Mul, Rc::clone(&c_u)),
                            Add,
                            Self::symbolic_binary_raw(Rc::clone(&c), Mul, Rc::clone(&s_u)),
                        );
                        let next_c = Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(Rc::clone(&c), Mul, Rc::clone(&c_u)),
                            if hyperbolic || *function != Builtin::Cos {
                                Add
                            } else {
                                Sub
                            },
                            Self::symbolic_binary_raw(Rc::clone(&s), Mul, Rc::clone(&s_u)),
                        );
                        s = next_s;
                        c = next_c;
                    }
                    let mut result = if matches!(function, Builtin::Sin | Builtin::Sinh) {
                        s
                    } else {
                        c
                    };
                    if coefficient < BigInt::from(0)
                        && matches!(function, Builtin::Sin | Builtin::Sinh)
                    {
                        result = Rc::new(SymbolicExpr::Unary {
                            op: UnaryOp::Neg,
                            expr: result,
                        });
                    }
                    return Ok(Some(result));
                }
            }
        }
        Ok(None)
    }

    /// Rewrites a sum or difference of like trigonometric calls as a product:
    /// `sin(a) + sin(b) -> 2*sin((a + b)/2)*cos((a - b)/2)`.
    fn trig_sum_to_product(
        &self,
        expression: &Rc<SymbolicExpr>,
    ) -> ExecResult<Option<Rc<SymbolicExpr>>> {
        use BinaryOp::{Add, Mul, Sub};
        let (left, op, right) = match expression.as_ref() {
            SymbolicExpr::Binary {
                left,
                op: op @ (Add | Sub),
                right,
            } => (Rc::clone(left), *op, Rc::clone(right)),
            _ => return Ok(None),
        };
        let call_parts = |expression: &Rc<SymbolicExpr>| match expression.as_ref() {
            SymbolicExpr::Call { function, args }
                if args.len() == 1 && matches!(function, Builtin::Sin | Builtin::Cos) =>
            {
                Some((*function, Rc::clone(&args[0])))
            }
            _ => None,
        };
        let Some((left_fn, left_arg)) = call_parts(&left) else {
            return Ok(None);
        };
        let Some((right_fn, right_arg)) = call_parts(&right) else {
            return Ok(None);
        };
        if left_fn != right_fn {
            return Ok(None);
        }
        let call = |function, argument: Rc<SymbolicExpr>| {
            Rc::new(SymbolicExpr::Call {
                function,
                args: vec![argument],
            })
        };
        let average = Self::symbolic_binary_raw(
            Self::symbolic_binary_raw(Rc::clone(&left_arg), Add, Rc::clone(&right_arg)),
            BinaryOp::Div,
            Self::symbolic_integer(2),
        );
        let semi_difference = Self::symbolic_binary_raw(
            Self::symbolic_binary_raw(left_arg, Sub, right_arg),
            BinaryOp::Div,
            Self::symbolic_integer(2),
        );
        // sin(a) + sin(b) = 2*sin((a + b)/2)*cos((a - b)/2); the other three
        // combinations follow the standard sum-to-product table.
        let (first_fn, second_fn, negate) = match (left_fn, op) {
            (Builtin::Sin, Add) => (Builtin::Sin, Builtin::Cos, false),
            (Builtin::Sin, Sub) => (Builtin::Cos, Builtin::Sin, false),
            (Builtin::Cos, Add) => (Builtin::Cos, Builtin::Cos, false),
            (Builtin::Cos, Sub) => (Builtin::Sin, Builtin::Sin, true),
            _ => return Ok(None),
        };
        let product = Self::symbolic_binary_raw(
            call(first_fn, average),
            Mul,
            call(second_fn, semi_difference),
        );
        let scaled = Self::symbolic_binary_raw(Self::symbolic_integer(2), Mul, product);
        Ok(Some(if negate {
            Rc::new(SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr: scaled,
            })
        } else {
            scaled
        }))
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
                    .or_else(|| Self::solve_transcendental(&expression, *variable))
                    .unwrap_or_default();
                let mut simplified = Vec::with_capacity(roots.len());
                for root in roots {
                    let root = self.simplify_symbolic_tree(&root)?;
                    if !simplified
                        .iter()
                        .any(|known: &Rc<SymbolicExpr>| known.equals(&root))
                    {
                        simplified.push(root);
                    }
                }
                Ok(Value::Array(Rc::new(
                    simplified.into_iter().map(Self::symbolic_value).collect(),
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

    fn call_polynomial_tool(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        let expression = self.value_to_symbolic(args[0].clone())?;
        let variable_index = match builtin {
            Builtin::Degree | Builtin::Coefficients | Builtin::Discriminant => 1,
            Builtin::Resultant | Builtin::PolynomialGcd | Builtin::PolynomialDivRem => 2,
            _ => unreachable!("polynomial builtin dispatched separately"),
        };
        let Value::Symbolic(variable_expression) = &args[variable_index] else {
            return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
        };
        let SymbolicExpr::Variable(variable) = variable_expression.as_ref() else {
            return Err(self.error(format!("{} expects a symbolic variable", builtin.name())));
        };
        let failure = |message: String| self.error(format!("{}: {message}", builtin.name()));
        match builtin {
            Builtin::Degree => Polynomial::degree_expression(&expression, *variable, &self.symbols)
                .map(|degree| Self::integer_value(degree as i64))
                .map_err(failure),
            Builtin::Coefficients => {
                Polynomial::coefficient_expressions(&expression, *variable, &self.symbols)
                    .map(|coefficients| {
                        Value::Array(Rc::new(
                            coefficients.into_iter().map(Self::symbolic_value).collect(),
                        ))
                    })
                    .map_err(failure)
            }
            Builtin::Discriminant => {
                Polynomial::discriminant_expression(&expression, *variable, &self.symbols)
                    .map(|value| Value::Number(Number::from_rational(value)))
                    .map_err(failure)
            }
            Builtin::Resultant | Builtin::PolynomialGcd | Builtin::PolynomialDivRem => {
                let other = self.value_to_symbolic(args[1].clone())?;
                match builtin {
                    Builtin::Resultant => Polynomial::resultant_expression(
                        &expression,
                        &other,
                        *variable,
                        &self.symbols,
                    )
                    .map(|value| Value::Number(Number::from_rational(value)))
                    .map_err(failure),
                    Builtin::PolynomialGcd => {
                        Polynomial::gcd_expression(&expression, &other, *variable, &self.symbols)
                            .map(Self::symbolic_value)
                            .map_err(failure)
                    }
                    Builtin::PolynomialDivRem => Polynomial::divrem_expressions(
                        &expression,
                        &other,
                        *variable,
                        &self.symbols,
                    )
                    .map(|(quotient, remainder)| {
                        Value::Array(Rc::new(vec![
                            Self::symbolic_value(quotient),
                            Self::symbolic_value(remainder),
                        ]))
                    })
                    .map_err(failure),
                    _ => unreachable!(),
                }
            }
            _ => unreachable!("polynomial builtin dispatched separately"),
        }
    }

    fn solve_transcendental(
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Option<Vec<Rc<SymbolicExpr>>> {
        let zero = Self::symbolic_integer(0);
        let (target, right) = match expression.as_ref() {
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Sub,
                right,
            } if Self::contains_symbol(left, variable)
                && !Self::contains_symbol(right, variable) =>
            {
                (Rc::clone(left), Rc::clone(right))
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Sub,
                right,
            } if !Self::contains_symbol(left, variable)
                && Self::contains_symbol(right, variable) =>
            {
                (Rc::clone(right), Rc::clone(left))
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Add,
                right,
            } if Self::contains_symbol(left, variable)
                && !Self::contains_symbol(right, variable) =>
            {
                (
                    Rc::clone(left),
                    Self::symbolic_binary_raw(zero, BinaryOp::Sub, Rc::clone(right)),
                )
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Add,
                right,
            } if !Self::contains_symbol(left, variable)
                && Self::contains_symbol(right, variable) =>
            {
                (
                    Rc::clone(right),
                    Self::symbolic_binary_raw(zero, BinaryOp::Sub, Rc::clone(left)),
                )
            }
            _ => (Rc::clone(expression), zero),
        };

        if let SymbolicExpr::Binary {
            left: base,
            op: BinaryOp::Pow,
            right: exponent,
        } = target.as_ref()
        {
            if !Self::contains_symbol(base, variable) {
                let logarithm = Rc::new(SymbolicExpr::Call {
                    function: Builtin::Log,
                    args: vec![right, Rc::clone(base)],
                });
                return Self::solve_affine_argument(exponent, variable, logarithm)
                    .map(|root| vec![root]);
            }
        }

        let SymbolicExpr::Call { function, args } = target.as_ref() else {
            return None;
        };
        let argument = args.first()?;
        let inverse = |function, argument| {
            Rc::new(SymbolicExpr::Call {
                function,
                args: vec![argument],
            })
        };
        let candidates = match (*function, args.as_slice()) {
            (Builtin::Exp, [_]) => vec![inverse(Builtin::Ln, right)],
            (Builtin::Ln | Builtin::Log, [_]) => vec![inverse(Builtin::Exp, right)],
            (Builtin::Log, [_, base]) => vec![Self::symbolic_binary_raw(
                Rc::clone(base),
                BinaryOp::Pow,
                right,
            )],
            (Builtin::Log10, [_]) => vec![Self::symbolic_binary_raw(
                Self::symbolic_integer(10),
                BinaryOp::Pow,
                right,
            )],
            (Builtin::Log2, [_]) => vec![Self::symbolic_binary_raw(
                Self::symbolic_integer(2),
                BinaryOp::Pow,
                right,
            )],
            (Builtin::Sin, [_]) => {
                let principal = inverse(Builtin::Asin, right);
                vec![
                    Rc::clone(&principal),
                    Self::symbolic_binary_raw(
                        Rc::new(SymbolicExpr::Constant(MathConstant::Pi)),
                        BinaryOp::Sub,
                        principal,
                    ),
                ]
            }
            (Builtin::Cos, [_]) => {
                let principal = inverse(Builtin::Acos, right);
                vec![
                    Rc::clone(&principal),
                    Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: principal,
                    }),
                ]
            }
            (Builtin::Tan, [_]) => vec![inverse(Builtin::Atan, right)],
            (Builtin::Sinh, [_]) => vec![inverse(Builtin::Asinh, right)],
            (Builtin::Cosh, [_]) => {
                let principal = inverse(Builtin::Acosh, right);
                vec![
                    Rc::clone(&principal),
                    Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: principal,
                    }),
                ]
            }
            (Builtin::Tanh, [_]) => vec![inverse(Builtin::Atanh, right)],
            _ => return None,
        };
        candidates
            .into_iter()
            .map(|candidate| Self::solve_affine_argument(argument, variable, candidate))
            .collect()
    }

    fn contains_symbol(expression: &SymbolicExpr, variable: Symbol) -> bool {
        match expression {
            SymbolicExpr::Variable(symbol) => *symbol == variable,
            SymbolicExpr::Unary { expr, .. } => Self::contains_symbol(expr, variable),
            SymbolicExpr::Binary { left, right, .. } => {
                Self::contains_symbol(left, variable) || Self::contains_symbol(right, variable)
            }
            SymbolicExpr::Call { args, .. } => {
                args.iter().any(|arg| Self::contains_symbol(arg, variable))
            }
            SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => false,
        }
    }

    fn affine_coefficients(
        expression: &SymbolicExpr,
        variable: Symbol,
    ) -> Option<(BigRational, BigRational)> {
        match expression {
            SymbolicExpr::Variable(symbol) if *symbol == variable => {
                Some((BigRational::one(), BigRational::zero()))
            }
            SymbolicExpr::Number(number) => Some((BigRational::zero(), number.try_to_rational()?)),
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let (coefficient, constant) = Self::affine_coefficients(expr, variable)?;
                Some((-coefficient, -constant))
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Add | BinaryOp::Sub,
                right,
            } => {
                let (left_coefficient, left_constant) = Self::affine_coefficients(left, variable)?;
                let (right_coefficient, right_constant) =
                    Self::affine_coefficients(right, variable)?;
                if matches!(
                    expression,
                    SymbolicExpr::Binary {
                        op: BinaryOp::Add,
                        ..
                    }
                ) {
                    Some((
                        left_coefficient + right_coefficient,
                        left_constant + right_constant,
                    ))
                } else {
                    Some((
                        left_coefficient - right_coefficient,
                        left_constant - right_constant,
                    ))
                }
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Mul,
                right,
            } => {
                let left_constant = Self::affine_coefficients(left, variable)?;
                let right_constant = Self::affine_coefficients(right, variable)?;
                match (left_constant, right_constant) {
                    ((coefficient, constant), (factor, offset)) if coefficient.is_zero() => {
                        Some((constant.clone() * factor, constant * offset))
                    }
                    ((factor, offset), (coefficient, constant)) if coefficient.is_zero() => {
                        Some((factor * constant.clone(), offset * constant))
                    }
                    _ => None,
                }
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Div,
                right,
            } => {
                let (coefficient, constant) = Self::affine_coefficients(left, variable)?;
                let (right_coefficient, divisor) = Self::affine_coefficients(right, variable)?;
                if !right_coefficient.is_zero() || divisor.is_zero() {
                    None
                } else {
                    Some((coefficient / &divisor, constant / divisor))
                }
            }
            _ => None,
        }
    }

    fn solve_affine_argument(
        argument: &SymbolicExpr,
        variable: Symbol,
        value: Rc<SymbolicExpr>,
    ) -> Option<Rc<SymbolicExpr>> {
        let (coefficient, constant) = Self::affine_coefficients(argument, variable)?;
        if coefficient.is_zero() {
            return None;
        }
        let numerator = if constant.is_zero() {
            value
        } else {
            Self::symbolic_binary_raw(
                value,
                BinaryOp::Sub,
                Rc::new(SymbolicExpr::Number(Number::from_rational(constant))),
            )
        };
        if coefficient.is_one() {
            Some(numerator)
        } else {
            Some(Self::symbolic_binary_raw(
                numerator,
                BinaryOp::Div,
                Rc::new(SymbolicExpr::Number(Number::from_rational(coefficient))),
            ))
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
            SymbolicExpr::Call { function, args } => {
                let args = args
                    .iter()
                    .map(|argument| self.simplify_symbolic_tree(argument))
                    .collect::<ExecResult<Vec<_>>>()?
                    .into_iter()
                    .map(Self::symbolic_value)
                    .collect();
                self.symbolic_builtin(*function, args)
                    .and_then(|value| self.value_to_symbolic(value))
            }
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
            SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => Self::symbolic_integer(0),
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
            SymbolicExpr::Call { function, args } if args.len() == 2 => {
                let left = &args[0];
                let right = &args[1];
                match function {
                    Builtin::Log => {
                        // log_b(u) = ln(u) / ln(b), including the case where
                        // the base is itself a function of the variable.
                        let rewritten = Self::symbolic_binary_raw(
                            Self::symbolic_call(Builtin::Ln, Rc::clone(left)),
                            Div,
                            Self::symbolic_call(Builtin::Ln, Rc::clone(right)),
                        );
                        self.derivative_symbolic(&rewritten, variable)
                    }
                    Builtin::Atan2 => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(
                                Rc::clone(right),
                                Mul,
                                self.derivative_symbolic(left, variable),
                            ),
                            Sub,
                            Self::symbolic_binary_raw(
                                Rc::clone(left),
                                Mul,
                                self.derivative_symbolic(right, variable),
                            ),
                        ),
                        Div,
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(
                                Rc::clone(left),
                                Pow,
                                Self::symbolic_integer(2),
                            ),
                            Add,
                            Self::symbolic_binary_raw(
                                Rc::clone(right),
                                Pow,
                                Self::symbolic_integer(2),
                            ),
                        ),
                    ),
                    Builtin::Hypot => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(
                                Rc::clone(left),
                                Mul,
                                self.derivative_symbolic(left, variable),
                            ),
                            Add,
                            Self::symbolic_binary_raw(
                                Rc::clone(right),
                                Mul,
                                self.derivative_symbolic(right, variable),
                            ),
                        ),
                        Div,
                        Rc::clone(expression),
                    ),
                    _ => self.unevaluated_calculus(Builtin::Derivative, expression, variable),
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
                    Builtin::Cbrt => Self::symbolic_binary_raw(
                        Self::symbolic_integer(1),
                        Div,
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(3),
                            Mul,
                            square(Rc::clone(expression)),
                        ),
                    ),
                    Builtin::Log10 | Builtin::Log2 => {
                        let base = if *function == Builtin::Log10 { 10 } else { 2 };
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(1),
                            Div,
                            Self::symbolic_binary_raw(
                                Rc::clone(inner),
                                Mul,
                                Self::symbolic_call(Builtin::Ln, Self::symbolic_integer(base)),
                            ),
                        )
                    }
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
                    Builtin::Erf | Builtin::Erfc => {
                        let exponent = Rc::new(SymbolicExpr::Unary {
                            op: UnaryOp::Neg,
                            expr: square(Rc::clone(inner)),
                        });
                        let numerator = Self::symbolic_binary_raw(
                            Self::symbolic_integer(2),
                            Mul,
                            Self::symbolic_call(Builtin::Exp, exponent),
                        );
                        let derivative = Self::symbolic_binary_raw(
                            numerator,
                            Div,
                            Self::symbolic_call(
                                Builtin::Sqrt,
                                Rc::new(SymbolicExpr::Constant(MathConstant::Pi)),
                            ),
                        );
                        if *function == Builtin::Erfc {
                            Rc::new(SymbolicExpr::Unary {
                                op: UnaryOp::Neg,
                                expr: derivative,
                            })
                        } else {
                            derivative
                        }
                    }
                    Builtin::Abs if self.symbolic_positive(inner) => Self::symbolic_integer(1),
                    Builtin::Abs if self.symbolic_real(inner) && self.symbolic_nonzero(inner) => {
                        Self::symbolic_binary_raw(Rc::clone(inner), Div, Rc::clone(expression))
                    }
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
        if let Some((function, argument)) = Self::squared_unary_call(expression) {
            if let Some(coefficient) = Self::linear_coefficient(&argument, variable) {
                // Double-angle halves: ∫ sin(a*x + b)^2 dx =
                // x/2 - sin(2*(a*x + b))/(4*a), and cos squares are the
                // same with a plus.
                if matches!(function, Builtin::Sin | Builtin::Cos) {
                    let doubled = Self::symbolic_binary_raw(
                        Self::symbolic_integer(2),
                        Mul,
                        Rc::clone(&argument),
                    );
                    let correction = Self::symbolic_binary_raw(
                        Self::symbolic_call(Builtin::Sin, doubled),
                        Div,
                        Self::symbolic_binary_raw(Self::symbolic_integer(4), Mul, coefficient),
                    );
                    let half_variable = Self::symbolic_binary_raw(
                        Rc::new(SymbolicExpr::Variable(variable)),
                        Div,
                        Self::symbolic_integer(2),
                    );
                    return Self::symbolic_binary_raw(
                        half_variable,
                        if function == Builtin::Sin { Sub } else { Add },
                        correction,
                    );
                }
                let primitive = match function {
                    Builtin::Sec => Some(Self::symbolic_call(Builtin::Tan, argument)),
                    Builtin::Csc => Some(Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: Self::symbolic_call(Builtin::Cot, argument),
                    })),
                    _ => None,
                };
                if let Some(primitive) = primitive {
                    return Self::symbolic_binary_raw(primitive, Div, coefficient);
                }
            }
        }
        if let Some((left_function, right_function, argument)) =
            Self::product_unary_calls(expression)
        {
            if let Some(coefficient) = Self::linear_coefficient(&argument, variable) {
                let primitive = match (left_function, right_function) {
                    (Builtin::Sec, Builtin::Tan) | (Builtin::Tan, Builtin::Sec) => {
                        Some(Self::symbolic_call(Builtin::Sec, argument))
                    }
                    (Builtin::Csc, Builtin::Cot) | (Builtin::Cot, Builtin::Csc) => {
                        Some(Rc::new(SymbolicExpr::Unary {
                            op: UnaryOp::Neg,
                            expr: Self::symbolic_call(Builtin::Csc, argument),
                        }))
                    }
                    _ => None,
                };
                if let Some(primitive) = primitive {
                    return Self::symbolic_binary_raw(primitive, Div, coefficient);
                }
            }
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
            } if !Self::contains_variable(left, variable)
                && Self::linear_coefficient(right, variable).is_some() =>
            {
                // ∫ c / (a*x + b) dx = c * ln(a*x + b) / a
                let logarithm = Self::symbolic_binary_raw(
                    Rc::clone(left),
                    Mul,
                    Self::symbolic_call(Builtin::Ln, Rc::clone(right)),
                );
                Self::symbolic_binary_raw(
                    logarithm,
                    Div,
                    Self::linear_coefficient(right, variable).expect("guarded above"),
                )
            }
            // Remaining rational forms: the tangent substitution for
            // `c / (a*x^2 + b)` first, then proper rational functions with
            // distinct rational linear factors integrate through partial
            // fractions (`apart` output feeds the arms above term by term).
            // Anything neither strategy handles stays unevaluated.
            SymbolicExpr::Binary { op: Div, .. } => {
                if let Some(primitive) = self.integrate_quadratic_reciprocal(expression, variable) {
                    primitive
                } else if let Some(decomposed) =
                    Polynomial::partial_fraction_expression(expression, &self.symbols)
                {
                    self.integral_symbolic(&decomposed, variable)
                } else {
                    self.unevaluated_calculus(Builtin::Integral, expression, variable)
                }
            }
            SymbolicExpr::Call { function, args }
                if args.len() == 1
                    && Self::linear_coefficient(&args[0], variable).is_some()
                    && matches!(
                        function,
                        Builtin::Exp
                            | Builtin::Ln
                            | Builtin::Log10
                            | Builtin::Log2
                            | Builtin::Sin
                            | Builtin::Cos
                            | Builtin::Tan
                            | Builtin::Sec
                            | Builtin::Csc
                            | Builtin::Sinh
                            | Builtin::Cosh
                            | Builtin::Asin
                            | Builtin::Acos
                            | Builtin::Atan
                            | Builtin::Asinh
                            | Builtin::Acosh
                            | Builtin::Atanh
                            | Builtin::Erf
                            | Builtin::Erfc
                            | Builtin::Sqrt
                            | Builtin::Cbrt
                    ) =>
            {
                let inner = Rc::clone(&args[0]);
                let square = |value: Rc<SymbolicExpr>| {
                    Self::symbolic_binary_raw(value, Pow, Self::symbolic_integer(2))
                };
                let sqrt = |value| Self::symbolic_call(Builtin::Sqrt, value);
                let primitive = match function {
                    Builtin::Exp => Rc::clone(expression),
                    Builtin::Ln => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Sub,
                        Rc::clone(&inner),
                    ),
                    Builtin::Log10 | Builtin::Log2 => {
                        let base = if *function == Builtin::Log10 { 10 } else { 2 };
                        Self::symbolic_binary_raw(
                            Self::symbolic_binary_raw(
                                Self::symbolic_binary_raw(
                                    Rc::clone(&inner),
                                    Mul,
                                    Self::symbolic_call(Builtin::Ln, Rc::clone(&inner)),
                                ),
                                Sub,
                                Rc::clone(&inner),
                            ),
                            Div,
                            Self::symbolic_call(Builtin::Ln, Self::symbolic_integer(base)),
                        )
                    }
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
                    Builtin::Sec => Self::symbolic_call(
                        Builtin::Ln,
                        Self::symbolic_binary_raw(
                            Self::symbolic_call(Builtin::Sec, Rc::clone(&inner)),
                            Add,
                            Self::symbolic_call(Builtin::Tan, Rc::clone(&inner)),
                        ),
                    ),
                    Builtin::Csc => Self::symbolic_call(
                        Builtin::Ln,
                        Self::symbolic_binary_raw(
                            Self::symbolic_call(Builtin::Csc, Rc::clone(&inner)),
                            Sub,
                            Self::symbolic_call(Builtin::Cot, Rc::clone(&inner)),
                        ),
                    ),
                    Builtin::Sinh => Self::symbolic_call(Builtin::Cosh, Rc::clone(&inner)),
                    Builtin::Cosh => Self::symbolic_call(Builtin::Sinh, Rc::clone(&inner)),
                    Builtin::Asin => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Add,
                        sqrt(Self::symbolic_binary_raw(
                            Self::symbolic_integer(1),
                            Sub,
                            square(Rc::clone(&inner)),
                        )),
                    ),
                    Builtin::Acos => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Sub,
                        sqrt(Self::symbolic_binary_raw(
                            Self::symbolic_integer(1),
                            Sub,
                            square(Rc::clone(&inner)),
                        )),
                    ),
                    Builtin::Atan => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Sub,
                        Self::symbolic_binary_raw(
                            Self::symbolic_call(
                                Builtin::Ln,
                                Self::symbolic_binary_raw(
                                    Self::symbolic_integer(1),
                                    Add,
                                    square(Rc::clone(&inner)),
                                ),
                            ),
                            Div,
                            Self::symbolic_integer(2),
                        ),
                    ),
                    Builtin::Asinh => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Sub,
                        sqrt(Self::symbolic_binary_raw(
                            square(Rc::clone(&inner)),
                            Add,
                            Self::symbolic_integer(1),
                        )),
                    ),
                    Builtin::Acosh => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Sub,
                        Self::symbolic_binary_raw(
                            sqrt(Self::symbolic_binary_raw(
                                Rc::clone(&inner),
                                Sub,
                                Self::symbolic_integer(1),
                            )),
                            Mul,
                            sqrt(Self::symbolic_binary_raw(
                                Rc::clone(&inner),
                                Add,
                                Self::symbolic_integer(1),
                            )),
                        ),
                    ),
                    Builtin::Atanh => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(Rc::clone(&inner), Mul, Rc::clone(expression)),
                        Add,
                        Self::symbolic_binary_raw(
                            Self::symbolic_call(
                                Builtin::Ln,
                                Self::symbolic_binary_raw(
                                    Self::symbolic_integer(1),
                                    Sub,
                                    square(Rc::clone(&inner)),
                                ),
                            ),
                            Div,
                            Self::symbolic_integer(2),
                        ),
                    ),
                    Builtin::Erf | Builtin::Erfc => {
                        let correction = Self::symbolic_binary_raw(
                            Self::symbolic_call(
                                Builtin::Exp,
                                Rc::new(SymbolicExpr::Unary {
                                    op: UnaryOp::Neg,
                                    expr: square(Rc::clone(&inner)),
                                }),
                            ),
                            Div,
                            sqrt(Rc::new(SymbolicExpr::Constant(MathConstant::Pi))),
                        );
                        let product = Self::symbolic_binary_raw(
                            Rc::clone(&inner),
                            Mul,
                            Rc::clone(expression),
                        );
                        Self::symbolic_binary_raw(
                            product,
                            if *function == Builtin::Erf { Add } else { Sub },
                            correction,
                        )
                    }
                    Builtin::Sqrt => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(2),
                            Mul,
                            Self::symbolic_binary_raw(
                                Rc::clone(expression),
                                Pow,
                                Self::symbolic_integer(3),
                            ),
                        ),
                        Div,
                        Self::symbolic_integer(3),
                    ),
                    Builtin::Cbrt => Self::symbolic_binary_raw(
                        Self::symbolic_binary_raw(
                            Self::symbolic_integer(3),
                            Mul,
                            Self::symbolic_binary_raw(
                                Rc::clone(expression),
                                Pow,
                                Self::symbolic_integer(4),
                            ),
                        ),
                        Div,
                        Self::symbolic_integer(4),
                    ),
                    _ => unreachable!(),
                };
                Self::symbolic_binary_raw(
                    primitive,
                    Div,
                    Self::linear_coefficient(&inner, variable).expect("guarded above"),
                )
            }
            // Products of a polynomial and a kernel call (`x*exp(x)`,
            // `x^2*sin(x)`, ...) integrate by parts; anything else that
            // reached this arm has no rule and stays unevaluated.
            SymbolicExpr::Binary { op: Mul, .. } => self
                .integrate_by_parts(expression, variable)
                .unwrap_or_else(|| {
                    self.unevaluated_calculus(Builtin::Integral, expression, variable)
                }),
            _ => self.unevaluated_calculus(Builtin::Integral, expression, variable),
        }
    }

    /// Integrates `polynomial * kernel(linear)` products with the tabular
    /// by-parts rule: `∫ p·g = Σ_j (−1)^j · p^(j) · V_j` where `V_j` is the
    /// `j`-th antiderivative of `g`. Kernels are restricted to `exp`, `sin`,
    /// `cos`, `sinh`, and `cosh` of a linear argument — the families closed
    /// under repeated integration — so every `V_j` stays inside the rule set
    /// and the expansion terminates after one step per polynomial degree.
    /// Returns `None` (leaving the integral unevaluated) for any other shape.
    fn integrate_by_parts(
        &self,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Option<Rc<SymbolicExpr>> {
        use BinaryOp::{Add, Mul};
        let mut factors = Vec::new();
        Self::mul_factors(expression, &mut factors);
        let mut constant = Self::symbolic_integer(1);
        let mut polynomial = vec![(Self::symbolic_integer(1), 0_u32)];
        let mut kernel = None;
        for factor in factors {
            if !Self::contains_variable(&factor, variable) {
                constant = Self::symbolic_binary_raw(constant, Mul, factor);
                continue;
            }
            match factor.as_ref() {
                SymbolicExpr::Call { function, args }
                    if args.len() == 1
                        && matches!(
                            function,
                            Builtin::Exp
                                | Builtin::Sin
                                | Builtin::Cos
                                | Builtin::Sinh
                                | Builtin::Cosh
                        )
                        && Self::linear_coefficient(&args[0], variable).is_some() =>
                {
                    if kernel.is_some() {
                        return None;
                    }
                    kernel = Some(Rc::clone(&factor));
                }
                _ => {
                    let terms = Self::polynomial_terms(&factor, variable)?;
                    polynomial = Self::polynomial_product(&polynomial, &terms)?;
                }
            }
        }
        let kernel = kernel?;
        let degree = polynomial.iter().map(|(_, power)| *power).max()?;
        if degree == 0 || degree > MAX_BY_PARTS_DEGREE {
            return None;
        }
        // Successive antiderivatives of the kernel; each stays closed form
        // for the five kernel families, so no check is strictly needed, but
        // bail out rather than build on an unevaluated integral if that ever
        // changes.
        let mut antiderivatives = Vec::with_capacity(degree as usize + 1);
        let mut current = Rc::clone(&kernel);
        for _ in 0..=degree {
            current = self.integral_symbolic(&current, variable);
            if !Self::is_closed_antiderivative(&current) {
                return None;
            }
            antiderivatives.push(Rc::clone(&current));
        }
        let mut total = None;
        for (coefficient, power) in polynomial {
            let mut sum = None;
            for step in 0..=power {
                // d^j/dx^j [c*x^power] up to sign: falling factorial in the
                // exponent, and x^(power - j) after differentiation.
                let scale = (0..step).fold(1_i64, |acc, k| acc * i64::from(power - k))
                    * if step % 2 == 1 { -1 } else { 1 };
                let mut term = Rc::clone(&coefficient);
                let remaining = power - step;
                if remaining > 0 {
                    let variable_expression = Rc::new(SymbolicExpr::Variable(variable));
                    let x_power = if remaining == 1 {
                        variable_expression
                    } else {
                        Self::symbolic_binary_raw(
                            variable_expression,
                            BinaryOp::Pow,
                            Self::symbolic_integer(i64::from(remaining)),
                        )
                    };
                    term = Self::symbolic_binary_raw(term, Mul, x_power);
                }
                term = Self::symbolic_binary_raw(
                    term,
                    Mul,
                    Rc::clone(&antiderivatives[step as usize]),
                );
                if scale != 1 {
                    term = Self::symbolic_binary_raw(Self::symbolic_integer(scale), Mul, term);
                }
                sum = Some(match sum {
                    Some(sum) => Self::symbolic_binary_raw(sum, Add, term),
                    None => term,
                });
            }
            let Some(sum) = sum else {
                continue;
            };
            let sum = Self::symbolic_binary_raw(Rc::clone(&constant), Mul, sum);
            total = Some(match total {
                Some(total) => Self::symbolic_binary_raw(total, Add, sum),
                None => sum,
            });
        }
        total
    }

    /// Integrates `c / (a*x^2 + b)` for positive rational `a` and `b` via
    /// the tangent substitution `x = sqrt(b/a) * tan(t)`:
    /// `∫ c/(a*x^2 + b) dx = c * atan(x / sqrt(b/a)) / (a * sqrt(b/a))`.
    fn integrate_quadratic_reciprocal(
        &self,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Option<Rc<SymbolicExpr>> {
        use BinaryOp::{Div, Mul};
        let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Div,
            right,
        } = expression.as_ref()
        else {
            return None;
        };
        if Self::contains_variable(left, variable) {
            return None;
        }
        let mut quadratic = None;
        let mut constant = None;
        for (coefficient, power) in Self::polynomial_terms(right, variable)? {
            let value = Self::positive_rational_number(&coefficient)?;
            match power {
                2 if quadratic.is_none() => quadratic = Some(value),
                0 if constant.is_none() => constant = Some(value),
                _ => return None,
            }
        }
        let (quadratic, constant) = (quadratic?, constant?);
        let ratio = constant.to_rational() / quadratic.to_rational();
        let root = Self::symbolic_call(
            Builtin::Sqrt,
            Rc::new(SymbolicExpr::Number(Number::from_rational(ratio))),
        );
        let angle = Self::symbolic_binary_raw(
            Rc::new(SymbolicExpr::Variable(variable)),
            Div,
            Rc::clone(&root),
        );
        let numerator = Self::symbolic_binary_raw(
            Rc::clone(left),
            Mul,
            Self::symbolic_call(Builtin::Atan, angle),
        );
        let denominator =
            Self::symbolic_binary_raw(Rc::new(SymbolicExpr::Number(quadratic)), Mul, root);
        Some(Self::symbolic_binary_raw(numerator, Div, denominator))
    }

    /// The exact positive rational value of a numeric coefficient, if any.
    fn positive_rational_number(expression: &Rc<SymbolicExpr>) -> Option<Number> {
        let value = Self::exact_rational_value(expression)?;
        (value.to_rational().numer() > &BigInt::from(0)).then_some(value)
    }

    /// The exact rational value of a numeric-only expression — products,
    /// quotients, and negations of rational literals as produced by the
    /// polynomial decompositions — if it has one.
    fn exact_rational_value(expression: &Rc<SymbolicExpr>) -> Option<Number> {
        use BinaryOp::{Div, Mul};
        match expression.as_ref() {
            SymbolicExpr::Number(number) if !matches!(number, Number::Complex(_)) => {
                Some(number.clone())
            }
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Some(Number::from_rational(
                -Self::exact_rational_value(expr)?.to_rational(),
            )),
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } => Some(Number::from_rational(
                Self::exact_rational_value(left)?.to_rational()
                    * Self::exact_rational_value(right)?.to_rational(),
            )),
            SymbolicExpr::Binary {
                left,
                op: Div,
                right,
            } => {
                let numerator = Self::exact_rational_value(left)?.to_rational();
                let denominator = Self::exact_rational_value(right)?.to_rational();
                if denominator.numer() == &BigInt::from(0) {
                    return None;
                }
                Some(Number::from_rational(numerator / denominator))
            }
            _ => None,
        }
    }

    /// Flattens a raw product tree into its factors.
    fn mul_factors(expression: &Rc<SymbolicExpr>, factors: &mut Vec<Rc<SymbolicExpr>>) {
        match expression.as_ref() {
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Mul,
                right,
            } => {
                Self::mul_factors(left, factors);
                Self::mul_factors(right, factors);
            }
            _ => factors.push(Rc::clone(expression)),
        }
    }

    /// Decomposes `expression` into monomial terms `(coefficient, power)` in
    /// `variable` with coefficients free of `variable`, expanding products
    /// and small positive powers. `None` when the expression is not a
    /// polynomial in `variable` or the expansion exceeds the term cap.
    fn polynomial_terms(
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
    ) -> Option<Vec<(Rc<SymbolicExpr>, u32)>> {
        use BinaryOp::{Add, Div, Mul, Pow, Sub};
        if !Self::contains_variable(expression, variable) {
            return Some(vec![(Rc::clone(expression), 0)]);
        }
        match expression.as_ref() {
            SymbolicExpr::Variable(symbol) if *symbol == variable => {
                Some(vec![(Self::symbolic_integer(1), 1)])
            }
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let terms = Self::polynomial_terms(expr, variable)?;
                Some(
                    terms
                        .into_iter()
                        .map(|(coefficient, power)| {
                            (
                                Rc::new(SymbolicExpr::Unary {
                                    op: UnaryOp::Neg,
                                    expr: coefficient,
                                }),
                                power,
                            )
                        })
                        .collect(),
                )
            }
            SymbolicExpr::Binary {
                left,
                op: op @ (Add | Sub),
                right,
            } => {
                let mut terms = Self::polynomial_terms(left, variable)?;
                let mut right_terms = Self::polynomial_terms(right, variable)?;
                if *op == Sub {
                    right_terms = right_terms
                        .into_iter()
                        .map(|(coefficient, power)| {
                            (
                                Rc::new(SymbolicExpr::Unary {
                                    op: UnaryOp::Neg,
                                    expr: coefficient,
                                }),
                                power,
                            )
                        })
                        .collect();
                }
                terms.append(&mut right_terms);
                Some(terms)
            }
            SymbolicExpr::Binary {
                left,
                op: Mul,
                right,
            } => Self::polynomial_product(
                &Self::polynomial_terms(left, variable)?,
                &Self::polynomial_terms(right, variable)?,
            ),
            SymbolicExpr::Binary {
                left,
                op: Div,
                right,
            } if !Self::contains_variable(right, variable) => {
                let terms = Self::polynomial_terms(left, variable)?;
                Some(
                    terms
                        .into_iter()
                        .map(|(coefficient, power)| {
                            (
                                Self::symbolic_binary_raw(coefficient, Div, Rc::clone(right)),
                                power,
                            )
                        })
                        .collect(),
                )
            }
            SymbolicExpr::Binary {
                left,
                op: Pow,
                right,
            } => {
                let exponent = match right.as_ref() {
                    SymbolicExpr::Number(Number::Integer(value)) => value.to_u32()?,
                    _ => return None,
                };
                if exponent > MAX_BY_PARTS_DEGREE {
                    return None;
                }
                let base_terms = Self::polynomial_terms(left, variable)?;
                let mut result = vec![(Self::symbolic_integer(1), 0_u32)];
                for _ in 0..exponent {
                    result = Self::polynomial_product(&result, &base_terms)?;
                }
                Some(result)
            }
            _ => None,
        }
    }

    /// Multiplies two monomial-term lists, keeping the expansion bounded.
    fn polynomial_product(
        left: &[(Rc<SymbolicExpr>, u32)],
        right: &[(Rc<SymbolicExpr>, u32)],
    ) -> Option<Vec<(Rc<SymbolicExpr>, u32)>> {
        let mut product = Vec::new();
        for (left_coefficient, left_power) in left {
            for (right_coefficient, right_power) in right {
                let power = left_power + right_power;
                if power > MAX_BY_PARTS_DEGREE {
                    return None;
                }
                product.push((
                    Self::symbolic_binary_raw(
                        Rc::clone(left_coefficient),
                        BinaryOp::Mul,
                        Rc::clone(right_coefficient),
                    ),
                    power,
                ));
                if product.len() > MAX_POLYNOMIAL_TERMS {
                    return None;
                }
            }
        }
        Some(product)
    }

    /// Whether `expression` is a closed-form antiderivative — that is, it
    /// contains no unevaluated integral left to substitute into.
    fn is_closed_antiderivative(expression: &Rc<SymbolicExpr>) -> bool {
        match expression.as_ref() {
            SymbolicExpr::Call {
                function: Builtin::Integral | Builtin::DefiniteIntegral,
                ..
            } => false,
            SymbolicExpr::Unary { expr, .. } => Self::is_closed_antiderivative(expr),
            SymbolicExpr::Binary { left, right, .. } => {
                Self::is_closed_antiderivative(left) && Self::is_closed_antiderivative(right)
            }
            SymbolicExpr::Call { args, .. } => args
                .iter()
                .all(|argument| Self::is_closed_antiderivative(argument)),
            _ => true,
        }
    }

    /// Classifies `expression` as even or odd in `variable` for the
    /// definite-integral symmetry shortcuts. `None` when parity is unknown.
    fn expression_parity(expression: &Rc<SymbolicExpr>, variable: Symbol) -> Option<Parity> {
        use BinaryOp::{Add, Div, Mul, Sub};
        if !Self::contains_variable(expression, variable) {
            return Some(Parity::Even);
        }
        match expression.as_ref() {
            SymbolicExpr::Variable(symbol) if *symbol == variable => Some(Parity::Odd),
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Self::expression_parity(expr, variable),
            SymbolicExpr::Binary {
                left,
                op: Add | Sub,
                right,
            } => {
                let left = Self::expression_parity(left, variable)?;
                let right = Self::expression_parity(right, variable)?;
                // Only same-parity sums stay classified; odd + even is
                // neither even nor odd.
                match (left, right) {
                    (Parity::Even, Parity::Even) => Some(Parity::Even),
                    (Parity::Odd, Parity::Odd) => Some(Parity::Odd),
                    _ => None,
                }
            }
            SymbolicExpr::Binary {
                left,
                op: Mul | Div,
                right,
            } => {
                let left = Self::expression_parity(left, variable)?;
                let right = Self::expression_parity(right, variable)?;
                // Parity multiplies: even*even and odd*odd are even, one
                // factor of each is odd. Division behaves the same because
                // 1/f has the parity of f.
                match (left, right) {
                    (Parity::Even, Parity::Even) => Some(Parity::Even),
                    (Parity::Odd, Parity::Odd) => Some(Parity::Even),
                    _ => Some(Parity::Odd),
                }
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Pow,
                right,
            } => match Self::expression_parity(left, variable)? {
                // An even base stays even under any power.
                Parity::Even => Some(Parity::Even),
                Parity::Odd => match right.as_ref() {
                    SymbolicExpr::Number(Number::Integer(exponent)) => {
                        // Odd base: even powers are even, odd powers odd.
                        Some(if exponent.bit(0) {
                            Parity::Odd
                        } else {
                            Parity::Even
                        })
                    }
                    _ => None,
                },
            },
            SymbolicExpr::Call { function, args } if args.len() == 1 => {
                match Self::expression_parity(&args[0], variable)? {
                    // Any function composed with an even inner expression is
                    // even; with an odd inner one it inherits the function's
                    // own parity.
                    Parity::Even => Some(Parity::Even),
                    Parity::Odd => match function {
                        Builtin::Sin
                        | Builtin::Tan
                        | Builtin::Cot
                        | Builtin::Csc
                        | Builtin::Asin
                        | Builtin::Atan
                        | Builtin::Sinh
                        | Builtin::Tanh
                        | Builtin::Asinh
                        | Builtin::Atanh
                        | Builtin::Erf
                        | Builtin::Cbrt
                        | Builtin::Sign => Some(Parity::Odd),
                        Builtin::Cos | Builtin::Sec | Builtin::Cosh | Builtin::Abs => {
                            Some(Parity::Even)
                        }
                        _ => None,
                    },
                }
            }
            _ => None,
        }
    }

    /// Applies linearity, odd/even symmetry, and the fundamental theorem of
    /// calculus to `∫_{lower}^{upper} expression dx`. Stays unevaluated when
    /// the rule set has no closed primitive — substituting limits into an
    /// unevaluated integral would corrupt it.
    fn definite_integral_symbolic(
        &self,
        expression: &Rc<SymbolicExpr>,
        variable: Symbol,
        lower: &Rc<SymbolicExpr>,
        upper: &Rc<SymbolicExpr>,
    ) -> ExecResult<Rc<SymbolicExpr>> {
        use BinaryOp::{Add, Mul, Sub};
        match expression.as_ref() {
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let inner = self.definite_integral_symbolic(expr, variable, lower, upper)?;
                self.apply_unary(UnaryOp::Neg, Self::symbolic_value(inner))
                    .and_then(|value| self.value_to_symbolic(value))
            }
            SymbolicExpr::Binary {
                left,
                op: op @ (Add | Sub),
                right,
            } => {
                let left = self.definite_integral_symbolic(left, variable, lower, upper)?;
                let right = self.definite_integral_symbolic(right, variable, lower, upper)?;
                self.symbolic_binary(*op, Self::symbolic_value(left), Self::symbolic_value(right))
                    .and_then(|value| self.value_to_symbolic(value))
            }
            _ => {
                if self.limits_are_negations(lower, upper) {
                    match Self::expression_parity(expression, variable) {
                        // ∫_{-a}^{a} of an odd integrand vanishes, even
                        // without a closed antiderivative.
                        Some(Parity::Odd) => return Ok(Self::symbolic_integer(0)),
                        Some(Parity::Even) => {
                            let zero = Self::symbolic_integer(0);
                            let half = self
                                .definite_integral_symbolic(expression, variable, &zero, upper)?;
                            return self
                                .symbolic_binary(
                                    Mul,
                                    Self::symbolic_value(Self::symbolic_integer(2)),
                                    Self::symbolic_value(half),
                                )
                                .and_then(|value| self.value_to_symbolic(value));
                        }
                        None => {}
                    }
                }
                let primitive = self.integral_symbolic(expression, variable);
                if !Self::is_closed_antiderivative(&primitive) {
                    return Ok(Rc::new(SymbolicExpr::Call {
                        function: Builtin::DefiniteIntegral,
                        args: vec![
                            Rc::clone(expression),
                            Rc::new(SymbolicExpr::Variable(variable)),
                            Rc::clone(lower),
                            Rc::clone(upper),
                        ],
                    }));
                }
                let at_upper = Self::substitute_symbolic(&primitive, variable, upper);
                let at_lower = Self::substitute_symbolic(&primitive, variable, lower);
                let difference = Self::symbolic_binary_raw(at_upper, Sub, at_lower);
                let simplified = self.simplify_symbolic_tree(&difference)?;
                let simplified = Polynomial::from_expression(&simplified, &self.symbols)
                    .map(|polynomial| polynomial.expanded_expression())
                    .unwrap_or(simplified);
                Ok(simplified)
            }
        }
    }

    /// Whether `lower == -upper` after simplification: the symmetric interval
    /// case that lets parity shortcuts fire.
    fn limits_are_negations(&self, lower: &Rc<SymbolicExpr>, upper: &Rc<SymbolicExpr>) -> bool {
        let sum = Self::symbolic_binary_raw(Rc::clone(lower), BinaryOp::Add, Rc::clone(upper));
        matches!(
            self.simplify_symbolic_tree(&sum),
            Ok(sum) if matches!(sum.as_ref(), SymbolicExpr::Number(number) if number.is_zero())
        )
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
            SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => false,
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

    fn rounding_builtin(&self, builtin: Builtin, number: &Number) -> ExecResult<Value> {
        if matches!(number, Number::Complex(_)) {
            return Err(self.error(format!("{} expects a real number", builtin.name())));
        }
        let rational = number.to_rational();
        let numerator = rational.numer();
        let denominator = rational.denom();
        let integer = match builtin {
            Builtin::Floor => numerator.div_floor(denominator),
            Builtin::Ceil => numerator.div_ceil(denominator),
            Builtin::Trunc => numerator / denominator,
            Builtin::Round => {
                let quotient = numerator / denominator;
                let remainder = numerator % denominator;
                if remainder.abs() * BigInt::from(2) >= *denominator {
                    quotient
                        + if numerator.sign() == num_bigint::Sign::Minus {
                            -1
                        } else {
                            1
                        }
                } else {
                    quotient
                }
            }
            _ => unreachable!("rounding helper called with non-rounding builtin"),
        };
        Ok(Value::Number(Number::Integer(integer)))
    }

    fn symbolic_builtin(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        let mut symbolic_args = Vec::with_capacity(args.len());
        for argument in args {
            symbolic_args.push(self.value_to_symbolic(argument)?);
        }
        if let Some(value) = self.exact_symbolic_function(builtin, &symbolic_args)? {
            return Ok(value);
        }
        let argument = symbolic_args.first().cloned();
        if let Some(argument) = argument {
            if matches!(builtin, Builtin::Ln | Builtin::Log) {
                if let SymbolicExpr::Call {
                    function: Builtin::Exp,
                    args,
                } = argument.as_ref()
                {
                    if args.len() == 1 && self.symbolic_real(&args[0]) {
                        return Ok(Self::symbolic_value(Rc::clone(&args[0])));
                    }
                }
            }
            // log(a^n) -> n*log(a) for positive bases.
            if matches!(
                builtin,
                Builtin::Ln | Builtin::Log | Builtin::Log10 | Builtin::Log2
            ) {
                if let SymbolicExpr::Binary {
                    left: base,
                    op: BinaryOp::Pow,
                    right: exponent,
                } = argument.as_ref()
                {
                    if self.symbolic_positive(base) {
                        return Ok(Self::symbolic_value(Self::symbolic_binary_raw(
                            Rc::clone(exponent),
                            BinaryOp::Mul,
                            Rc::new(SymbolicExpr::Call {
                                function: builtin,
                                args: vec![Rc::clone(base)],
                            }),
                        )));
                    }
                }
            }
            if builtin == Builtin::Exp {
                if let SymbolicExpr::Call {
                    function: Builtin::Ln | Builtin::Log,
                    args,
                } = argument.as_ref()
                {
                    if args.len() == 1 && self.symbolic_nonzero(&args[0]) {
                        return Ok(Self::symbolic_value(Rc::clone(&args[0])));
                    }
                }
            }
            // f(f⁻¹(x)) -> x on the principal branches: each function is the
            // left inverse of its arc-function over the whole complex plane.
            // The reverse direction (f⁻¹(f(x)) -> x) needs range facts and is
            // deliberately not applied.
            if let SymbolicExpr::Call {
                function: inner,
                args: inner_args,
            } = argument.as_ref()
            {
                let inverse_pair = inner_args.len() == 1
                    && matches!(
                        (builtin, inner),
                        (Builtin::Sin, Builtin::Asin)
                            | (Builtin::Cos, Builtin::Acos)
                            | (Builtin::Tan, Builtin::Atan)
                            | (Builtin::Sinh, Builtin::Asinh)
                            | (Builtin::Cosh, Builtin::Acosh)
                            | (Builtin::Tanh, Builtin::Atanh)
                    );
                if inverse_pair {
                    return Ok(Self::symbolic_value(Rc::clone(&inner_args[0])));
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
                        if self.symbolic_negative(left) {
                            return self
                                .apply_unary(UnaryOp::Neg, Self::symbolic_value(Rc::clone(left)));
                        }
                        if self.symbolic_real(left) {
                            return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                                function: Builtin::Abs,
                                args: vec![Rc::clone(left)],
                            })));
                        }
                    }
                }
            }
            if builtin == Builtin::Abs {
                if self.symbolic_nonnegative(&argument) {
                    return Ok(Self::symbolic_value(argument));
                }
                if self.symbolic_negative(&argument) {
                    return self.apply_unary(UnaryOp::Neg, Self::symbolic_value(argument));
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

    fn exact_symbolic_function(
        &self,
        builtin: Builtin,
        args: &[Rc<SymbolicExpr>],
    ) -> ExecResult<Option<Value>> {
        let [argument] = args else {
            return Ok(None);
        };
        // Exact numeric arguments fold to closed forms so simplification can
        // finish arithmetic like `sin(0) -> 0` or `ln(1) -> 0` inside larger
        // expressions. Only provably exact results fold; everything else
        // (like `sin(1)`) stays symbolic.
        if let SymbolicExpr::Number(number) = argument.as_ref() {
            let folded = if number.is_zero() {
                match builtin {
                    Builtin::Sin
                    | Builtin::Tan
                    | Builtin::Sinh
                    | Builtin::Tanh
                    | Builtin::Asin
                    | Builtin::Atan
                    | Builtin::Asinh
                    | Builtin::Atanh
                    | Builtin::Erf
                    | Builtin::Floor
                    | Builtin::Ceil
                    | Builtin::Trunc
                    | Builtin::Abs
                    | Builtin::Sign
                    | Builtin::Sqrt
                    | Builtin::Cbrt => Some(Self::symbolic_value(Self::symbolic_integer(0))),
                    Builtin::Cos | Builtin::Cosh | Builtin::Exp | Builtin::Sec => {
                        Some(Self::symbolic_value(Self::symbolic_integer(1)))
                    }
                    _ => None,
                }
            } else if number.is_one() {
                match builtin {
                    Builtin::Ln | Builtin::Log10 | Builtin::Log2 => {
                        Some(Self::symbolic_value(Self::symbolic_integer(0)))
                    }
                    Builtin::Floor
                    | Builtin::Ceil
                    | Builtin::Trunc
                    | Builtin::Abs
                    | Builtin::Sign
                    | Builtin::Sqrt
                    | Builtin::Cbrt => Some(Self::symbolic_value(Self::symbolic_integer(1))),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(folded) = folded {
                return Ok(Some(folded));
            }
            // Perfect rational powers fold as well: `sqrt(4) -> 2`,
            // `cbrt(-8) -> -2`.
            if matches!(builtin, Builtin::Sqrt | Builtin::Cbrt)
                && !matches!(number, Number::Complex(_))
            {
                let rational = number.to_rational();
                let (numerator, denominator) = (rational.numer(), rational.denom());
                let odd = builtin == Builtin::Cbrt;
                let negative = numerator < &BigInt::from(0);
                if odd || !negative {
                    let magnitude = numerator.abs();
                    let (degree, numerator_root, denominator_root) = if odd {
                        (3_u32, magnitude.cbrt(), denominator.cbrt())
                    } else {
                        (2_u32, magnitude.sqrt(), denominator.sqrt())
                    };
                    if numerator_root.clone().pow(degree) == magnitude
                        && denominator_root.clone().pow(degree) == *denominator
                    {
                        let root = BigRational::new(numerator_root, denominator_root);
                        let root = if negative { -root } else { root };
                        return Ok(Some(Self::symbolic_value(Rc::new(SymbolicExpr::Number(
                            Number::from_rational(root),
                        )))));
                    }
                }
            }
        }
        if matches!(
            builtin,
            Builtin::Sin | Builtin::Cos | Builtin::Tan | Builtin::Sec | Builtin::Csc | Builtin::Cot
        ) {
            if let Some(multiple) = Self::pi_multiple(argument) {
                if let Some((sine, cosine)) = Self::exact_sine_cosine(multiple) {
                    let value = match builtin {
                        Builtin::Sin => Self::symbolic_value(sine),
                        Builtin::Cos => Self::symbolic_value(cosine),
                        Builtin::Tan => {
                            if Self::symbolic_is_zero(&cosine) {
                                return Err(
                                    self.error("tan is undefined at an odd multiple of pi/2")
                                );
                            }
                            self.symbolic_binary(
                                BinaryOp::Div,
                                Self::symbolic_value(sine),
                                Self::symbolic_value(cosine),
                            )?
                        }
                        Builtin::Sec => {
                            if Self::symbolic_is_zero(&cosine) {
                                return Err(
                                    self.error("sec is undefined at an odd multiple of pi/2")
                                );
                            }
                            self.symbolic_binary(
                                BinaryOp::Div,
                                Value::Number(Number::Integer(BigInt::from(1))),
                                Self::symbolic_value(cosine),
                            )?
                        }
                        Builtin::Csc => {
                            if Self::symbolic_is_zero(&sine) {
                                return Err(
                                    self.error("csc is undefined at an integer multiple of pi")
                                );
                            }
                            self.symbolic_binary(
                                BinaryOp::Div,
                                Value::Number(Number::Integer(BigInt::from(1))),
                                Self::symbolic_value(sine),
                            )?
                        }
                        Builtin::Cot => {
                            if Self::symbolic_is_zero(&sine) {
                                return Err(
                                    self.error("cot is undefined at an integer multiple of pi")
                                );
                            }
                            self.symbolic_binary(
                                BinaryOp::Div,
                                Self::symbolic_value(cosine),
                                Self::symbolic_value(sine),
                            )?
                        }
                        _ => unreachable!(),
                    };
                    return Ok(Some(value));
                }
            }
        }
        match (builtin, argument.as_ref()) {
            (Builtin::Ln | Builtin::Log, SymbolicExpr::Constant(MathConstant::E)) => {
                Ok(Some(Value::Number(Number::Integer(BigInt::from(1)))))
            }
            (Builtin::Exp, expression) if Self::is_symbolic_integer(expression, 1) => Ok(Some(
                Self::symbolic_value(Rc::new(SymbolicExpr::Constant(MathConstant::E))),
            )),
            (Builtin::Exp, expression) if Self::is_i_times_pi(expression) => {
                Ok(Some(Value::Number(Number::Integer(BigInt::from(-1)))))
            }
            (Builtin::Degrees, SymbolicExpr::Constant(MathConstant::Pi)) => {
                Ok(Some(Value::Number(Number::Integer(BigInt::from(180)))))
            }
            (Builtin::Degrees, SymbolicExpr::Constant(MathConstant::Tau)) => {
                Ok(Some(Value::Number(Number::Integer(BigInt::from(360)))))
            }
            (Builtin::Radians, SymbolicExpr::Number(Number::Integer(value)))
                if value == &BigInt::from(180) =>
            {
                Ok(Some(Self::symbolic_value(Rc::new(SymbolicExpr::Constant(
                    MathConstant::Pi,
                )))))
            }
            _ => Ok(None),
        }
    }

    fn is_i_times_pi(expression: &SymbolicExpr) -> bool {
        let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Mul,
            right,
        } = expression
        else {
            return false;
        };
        let is_i = |expression: &SymbolicExpr| {
            matches!(expression, SymbolicExpr::Number(Number::Complex(value))
                if value.real.is_zero() && value.imaginary.is_one())
        };
        (is_i(left) && matches!(right.as_ref(), SymbolicExpr::Constant(MathConstant::Pi)))
            || (is_i(right) && matches!(left.as_ref(), SymbolicExpr::Constant(MathConstant::Pi)))
    }

    fn pi_multiple(expression: &SymbolicExpr) -> Option<BigRational> {
        match expression {
            SymbolicExpr::Constant(MathConstant::Pi) => {
                Some(BigRational::from_integer(BigInt::from(1)))
            }
            SymbolicExpr::Constant(MathConstant::Tau) => {
                Some(BigRational::from_integer(BigInt::from(2)))
            }
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => Self::pi_multiple(expr).map(std::ops::Neg::neg),
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Mul,
                right,
            } => {
                if let SymbolicExpr::Number(number) = left.as_ref() {
                    return number
                        .try_to_rational()
                        .and_then(|factor| Self::pi_multiple(right).map(|value| factor * value));
                }
                if let SymbolicExpr::Number(number) = right.as_ref() {
                    return number
                        .try_to_rational()
                        .and_then(|factor| Self::pi_multiple(left).map(|value| factor * value));
                }
                None
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Div,
                right,
            } => {
                let divisor = match right.as_ref() {
                    SymbolicExpr::Number(number) => number.try_to_rational()?,
                    _ => return None,
                };
                (!divisor.is_zero())
                    .then(|| Self::pi_multiple(left).map(|value| value / divisor))
                    .flatten()
            }
            _ => None,
        }
    }

    fn exact_sine_cosine(multiple: BigRational) -> Option<(Rc<SymbolicExpr>, Rc<SymbolicExpr>)> {
        let two = BigRational::from_integer(BigInt::from(2));
        let mut reduced = multiple % &two;
        if reduced < BigRational::from_integer(BigInt::from(0)) {
            reduced += &two;
        }
        let numerator = reduced.numer().to_i64()?;
        let denominator = reduced.denom().to_i64()?;
        let integer = |value: i64| Self::symbolic_integer(value);
        let half = |sign: i64| {
            Rc::new(SymbolicExpr::Number(Number::from_rational(
                BigRational::new(BigInt::from(sign), BigInt::from(2)),
            )))
        };
        let radical_half = |radicand: i64, sign: i64| {
            let radical = Rc::new(SymbolicExpr::Call {
                function: Builtin::Sqrt,
                args: vec![integer(radicand)],
            });
            let quotient = Rc::new(SymbolicExpr::Binary {
                left: radical,
                op: BinaryOp::Div,
                right: integer(2),
            });
            if sign < 0 {
                Rc::new(SymbolicExpr::Unary {
                    op: UnaryOp::Neg,
                    expr: quotient,
                })
            } else {
                quotient
            }
        };
        Some(match (numerator, denominator) {
            (0, 1) => (integer(0), integer(1)),
            (1, 6) => (half(1), radical_half(3, 1)),
            (1, 4) => (radical_half(2, 1), radical_half(2, 1)),
            (1, 3) => (radical_half(3, 1), half(1)),
            (1, 2) => (integer(1), integer(0)),
            (2, 3) => (radical_half(3, 1), half(-1)),
            (3, 4) => (radical_half(2, 1), radical_half(2, -1)),
            (5, 6) => (half(1), radical_half(3, -1)),
            (1, 1) => (integer(0), integer(-1)),
            (7, 6) => (half(-1), radical_half(3, -1)),
            (5, 4) => (radical_half(2, -1), radical_half(2, -1)),
            (4, 3) => (radical_half(3, -1), half(-1)),
            (3, 2) => (integer(-1), integer(0)),
            (5, 3) => (radical_half(3, -1), half(1)),
            (7, 4) => (radical_half(2, -1), radical_half(2, 1)),
            (11, 6) => (half(-1), radical_half(3, 1)),
            _ => return None,
        })
    }

    fn symbolic_nonnegative(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(Number::Complex(_)) => false,
            SymbolicExpr::Number(number) => !number.is_negative(),
            SymbolicExpr::Constant(
                MathConstant::Pi | MathConstant::E | MathConstant::Tau | MathConstant::Phi,
            ) => true,
            SymbolicExpr::Variable(symbol) => {
                self.nonnegative_symbols.borrow().contains(symbol)
                    || self.positive_symbols.borrow().contains(symbol)
            }
            SymbolicExpr::Call {
                function: Builtin::Abs,
                ..
            } => true,
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

    fn symbolic_positive(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(Number::Complex(_)) => false,
            SymbolicExpr::Number(number) => number
                .compare(&Number::Integer(BigInt::from(0)))
                .is_some_and(std::cmp::Ordering::is_gt),
            SymbolicExpr::Constant(
                MathConstant::Pi | MathConstant::E | MathConstant::Tau | MathConstant::Phi,
            ) => true,
            SymbolicExpr::Variable(symbol) => self.positive_symbols.borrow().contains(symbol),
            SymbolicExpr::Call {
                function: Builtin::Exp,
                args,
            } => args.len() == 1 && self.symbolic_real(&args[0]),
            SymbolicExpr::Call {
                function: Builtin::Sqrt,
                args,
            } => args.len() == 1 && self.symbolic_positive(&args[0]),
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Pow,
                right,
            } => {
                self.symbolic_nonzero(left)
                    && matches!(right.as_ref(), SymbolicExpr::Number(Number::Integer(value)) if value.is_even())
            }
            _ => false,
        }
    }

    fn symbolic_negative(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(Number::Complex(_)) => false,
            SymbolicExpr::Number(number) => number
                .compare(&Number::Integer(BigInt::from(0)))
                .is_some_and(std::cmp::Ordering::is_lt),
            SymbolicExpr::Variable(symbol) => self.negative_symbols.borrow().contains(symbol),
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => self.symbolic_positive(expr),
            _ => false,
        }
    }

    // ---- general constraints (`assume`) ---------------------------------

    /// Handles `assume(constraint)` and `forget_assumptions()`.
    /// The constraint argument arrives already evaluated, so comparisons
    /// over purely numeric operands return `true`/`false` before this
    /// method ever runs; everything that reaches here still contains a
    /// symbolic operand and is collected as a fact instead of answering.
    fn call_constraint_tool(&self, builtin: Builtin, args: Vec<Value>) -> ExecResult<Value> {
        if builtin == Builtin::ForgetAssumptions {
            self.constraints.borrow_mut().clear();
            self.nonzero_symbols.borrow_mut().clear();
            self.nonnegative_symbols.borrow_mut().clear();
            self.positive_symbols.borrow_mut().clear();
            self.negative_symbols.borrow_mut().clear();
            self.integer_symbols.borrow_mut().clear();
            self.real_symbols.borrow_mut().clear();
            return Ok(Value::Nil);
        }
        let constraint = args.into_iter().next().expect("arity checked");
        // A constraint whose operands were both resolved through equality
        // facts folds into a concrete boolean, as in `assume(x == 2)`
        // followed by `assume(x < 10)`, which evaluates `2 < 10`. That is
        // a consistency check rather than a new fact: `true` is already
        // implied by the recorded assumptions while `false` contradicts
        // them. Without recorded facts a boolean cannot be a folded
        // constraint, so literal booleans keep failing type checks.
        if let Value::Bool(decided) = constraint {
            if self.constraints.borrow().is_empty() {
                return Err(self.error(format!(
                    "assume expects a constraint such as x == 3 or x > 0 and x < 10, got {}",
                    constraint.type_name()
                )));
            }
            if decided {
                return Ok(Value::Bool(true));
            }
            return Err(self.error("assume contradicts the current assumptions"));
        }
        let Value::Symbolic(expression) = constraint else {
            return Err(self.error(format!(
                "assume expects a constraint such as x == 3 or x > 0 and x < 10, got {}",
                constraint.type_name()
            )));
        };
        let mut collected = Vec::new();
        if let Err(message) = self.collect_constraints(&expression, &mut collected) {
            return Err(self.error(message));
        }
        // Detect contradictions before committing any of the new facts.
        let previous = self.constraints.borrow().clone();
        let mut facts = previous.clone();
        facts.extend(collected.iter().cloned());
        let contradictions = facts.iter().enumerate().any(|(index, fact)| {
            facts[..index]
                .iter()
                .any(|earlier| Self::facts_conflict(fact, earlier))
        });
        if contradictions {
            return Err(self.error("assume contradicts an existing constraint"));
        }
        *self.constraints.borrow_mut() = facts;
        // Mirror inequality facts into the per-variable sets so the existing
        // range predicates see them.
        for fact in &collected {
            self.apply_fact_side_effects(fact);
        }
        Ok(Value::Symbolic(expression))
    }

    /// Flattens `and` combinations into unit facts. `or` and top-level
    /// boolean variables are not expressible as facts and are rejected.
    fn collect_constraints(
        &self,
        expression: &SymbolicExpr,
        facts: &mut Vec<ConstraintFact>,
    ) -> Result<(), String> {
        match expression {
            SymbolicExpr::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.collect_constraints(left, facts)?;
                self.collect_constraints(right, facts)?;
                Ok(())
            }
            SymbolicExpr::Binary {
                op:
                    op @ (BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge),
                left,
                right,
            } => {
                // Normalize to `variable rel bound` first; the stored fact
                // then flips the comparison to always read `bound op variable`.
                let (variable, bound, rel) = match (&**left, &**right) {
                    (SymbolicExpr::Variable(symbol), _) => (*symbol, Rc::clone(right), *op),
                    (_, SymbolicExpr::Variable(symbol)) => {
                        (*symbol, Rc::clone(left), Self::flip_comparison(*op))
                    }
                    _ => Self::linear_assumption(left, right, *op)?,
                };
                let bound = self.substitute_constraints(&bound);
                // The stored fact always reads `bound op variable`.
                let op = Self::flip_comparison(rel);
                match op {
                    BinaryOp::Eq => facts.push(ConstraintFact::Equal {
                        symbol: variable,
                        value: bound,
                    }),
                    BinaryOp::Ne => facts.push(ConstraintFact::NotEqual {
                        symbol: variable,
                        value: bound,
                    }),
                    _ => facts.push(ConstraintFact::RealCompare {
                        symbol: variable,
                        bound,
                        op,
                    }),
                }
                Ok(())
            }
            SymbolicExpr::Binary {
                op: BinaryOp::Or,
                ..
            } => Err(
                "assume does not support `or`; list each case as a separate call".to_owned(),
            ),
            _ => Err(
                "assume expects a comparison such as x == 3, x > 0, or an `and` combination of them"
                    .to_owned(),
            ),
        }
    }

    /// Flips a comparison so `a op b` reads `b flipped(op) a`.
    fn flip_comparison(op: BinaryOp) -> BinaryOp {
        match op {
            BinaryOp::Lt => BinaryOp::Gt,
            BinaryOp::Le => BinaryOp::Ge,
            BinaryOp::Gt => BinaryOp::Lt,
            BinaryOp::Ge => BinaryOp::Le,
            other => other,
        }
    }

    /// Normalizes an assumption comparison whose sides are not plain
    /// variables into `variable rel bound`, as in `assume(x + 1 > 0)`
    /// becoming `x > -1`. Only linear expressions in exactly one symbol
    /// are accepted.
    fn linear_assumption(
        left: &SymbolicExpr,
        right: &SymbolicExpr,
        op: BinaryOp,
    ) -> Result<(Symbol, Rc<SymbolicExpr>, BinaryOp), String> {
        let not_linear = || {
            "assume comparisons need a plain variable on one side (for example x == 3 or y < x + 1) \
             or a linear expression in one variable (for example x + 1 > 0 or 2 * x < 6)"
                .to_owned()
        };
        let difference = Rc::new(SymbolicExpr::Binary {
            left: Rc::new(left.clone()),
            op: BinaryOp::Sub,
            right: Rc::new(right.clone()),
        });
        let mut symbols = Vec::new();
        Self::collect_free_symbols(difference.as_ref(), &mut symbols);
        let [variable] = symbols.as_slice() else {
            return Err(not_linear());
        };
        let variable = *variable;
        let Some((coefficient, rest)) = Self::linear_coefficients(difference.as_ref(), variable)
        else {
            return Err(not_linear());
        };
        if coefficient.is_zero() {
            return Err(not_linear());
        }
        // `coefficient * variable + rest op 0` isolates as
        // `variable op -rest / coefficient`, flipping the comparison when
        // the scale is negative.
        let negated_rest = Self::fold_constant_binary(
            BinaryOp::Sub,
            Rc::new(SymbolicExpr::Number(Number::Integer(BigInt::from(0)))),
            rest,
        );
        let bound = Self::fold_constant_binary(
            BinaryOp::Div,
            negated_rest,
            Rc::new(SymbolicExpr::Number(Number::from_rational(
                coefficient.clone(),
            ))),
        );
        let rel = if coefficient.is_negative() {
            Self::flip_comparison(op)
        } else {
            op
        };
        Ok((variable, bound, rel))
    }

    /// Collects the free symbols of `expression` in first-seen order.
    fn collect_free_symbols(expression: &SymbolicExpr, out: &mut Vec<Symbol>) {
        match expression {
            SymbolicExpr::Variable(symbol) => {
                if !out.contains(symbol) {
                    out.push(*symbol);
                }
            }
            SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => {}
            SymbolicExpr::Unary { expr, .. } => Self::collect_free_symbols(expr, out),
            SymbolicExpr::Binary { left, right, .. } => {
                Self::collect_free_symbols(left, out);
                Self::collect_free_symbols(right, out);
            }
            SymbolicExpr::Call { args, .. } => {
                for argument in args {
                    Self::collect_free_symbols(argument, out);
                }
            }
        }
    }

    /// Decomposes `expression` as `coefficient * variable + rest`, where
    /// the coefficient is a rational number and `rest` never mentions
    /// `variable`.
    fn linear_coefficients(
        expression: &SymbolicExpr,
        variable: Symbol,
    ) -> Option<(BigRational, Rc<SymbolicExpr>)> {
        let zero = || {
            Rc::new(SymbolicExpr::Number(Number::from_rational(
                BigRational::zero(),
            )))
        };
        match expression {
            SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => {
                Some((BigRational::zero(), Rc::new(expression.clone())))
            }
            SymbolicExpr::Variable(symbol) if *symbol == variable => {
                Some((BigRational::one(), zero()))
            }
            SymbolicExpr::Variable(_) => Some((BigRational::zero(), Rc::new(expression.clone()))),
            SymbolicExpr::Unary {
                op: UnaryOp::Neg,
                expr,
            } => {
                let (coefficient, rest) = Self::linear_coefficients(expr, variable)?;
                Some((
                    -coefficient,
                    Self::fold_constant_binary(
                        BinaryOp::Mul,
                        Rc::new(SymbolicExpr::Number(Number::from_rational(
                            BigRational::from_integer(BigInt::from(-1)),
                        ))),
                        rest,
                    ),
                ))
            }
            SymbolicExpr::Unary { .. } => None,
            SymbolicExpr::Binary {
                left,
                op: op @ (BinaryOp::Add | BinaryOp::Sub),
                right,
            } => {
                let (left_coefficient, left_rest) = Self::linear_coefficients(left, variable)?;
                let (right_coefficient, right_rest) = Self::linear_coefficients(right, variable)?;
                let coefficient = if *op == BinaryOp::Add {
                    left_coefficient + right_coefficient
                } else {
                    left_coefficient - right_coefficient
                };
                Some((
                    coefficient,
                    Self::fold_constant_binary(*op, left_rest, right_rest),
                ))
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Mul,
                right,
            } => {
                let scalar = |operand: &Rc<SymbolicExpr>| match operand.as_ref() {
                    SymbolicExpr::Number(number) => number.try_to_rational(),
                    _ => None,
                };
                if let Some(factor) = scalar(right) {
                    let (coefficient, rest) = Self::linear_coefficients(left, variable)?;
                    let scale =
                        Rc::new(SymbolicExpr::Number(Number::from_rational(factor.clone())));
                    Some((
                        coefficient * factor,
                        Self::fold_constant_binary(BinaryOp::Mul, scale, rest),
                    ))
                } else if let Some(factor) = scalar(left) {
                    let (coefficient, rest) = Self::linear_coefficients(right, variable)?;
                    let scale =
                        Rc::new(SymbolicExpr::Number(Number::from_rational(factor.clone())));
                    Some((
                        coefficient * factor,
                        Self::fold_constant_binary(BinaryOp::Mul, scale, rest),
                    ))
                } else {
                    None
                }
            }
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Div,
                right,
            } => {
                let divisor = match right.as_ref() {
                    SymbolicExpr::Number(number) => number.try_to_rational(),
                    _ => None,
                };
                let divisor = divisor.filter(|divisor| !divisor.is_zero())?;
                let (coefficient, rest) = Self::linear_coefficients(left, variable)?;
                let scale = Rc::new(SymbolicExpr::Number(Number::from_rational(divisor.clone())));
                Some((
                    coefficient / divisor,
                    Self::fold_constant_binary(BinaryOp::Div, rest, scale),
                ))
            }
            _ => None,
        }
    }

    /// Builds a binary node while folding rational constants and additive
    /// identities, keeping stored bounds readable (`-1` instead of
    /// `-(1 - 0) / 1`).
    fn fold_constant_binary(
        op: BinaryOp,
        left: Rc<SymbolicExpr>,
        right: Rc<SymbolicExpr>,
    ) -> Rc<SymbolicExpr> {
        if let (SymbolicExpr::Number(a), SymbolicExpr::Number(b)) = (left.as_ref(), right.as_ref())
        {
            if let (Some(a), Some(b)) = (a.try_to_rational(), b.try_to_rational()) {
                let folded = match op {
                    BinaryOp::Add => Some(a + b),
                    BinaryOp::Sub => Some(a - b),
                    BinaryOp::Mul => Some(a * b),
                    BinaryOp::Div if !b.is_zero() => Some(a / b),
                    _ => None,
                };
                if let Some(folded) = folded {
                    return Rc::new(SymbolicExpr::Number(Number::from_rational(folded)));
                }
            }
        }
        if matches!(op, BinaryOp::Add | BinaryOp::Sub)
            && matches!(right.as_ref(), SymbolicExpr::Number(number) if number.is_zero())
        {
            return left;
        }
        if op == BinaryOp::Add
            && matches!(left.as_ref(), SymbolicExpr::Number(number) if number.is_zero())
        {
            return right;
        }
        Rc::new(SymbolicExpr::Binary { left, op, right })
    }

    /// Finds the symbol an equality fact assigned to a concrete value, so
    /// tools like `assume_positive` can still name the variable when fact
    /// substitution rewrote their argument.
    fn equality_fact_symbol(&self, value: &Value) -> Option<Symbol> {
        self.constraints
            .borrow()
            .iter()
            .find_map(|fact| match fact {
                ConstraintFact::Equal {
                    symbol,
                    value: assigned,
                } => match (value, assigned.as_ref()) {
                    (Value::Number(number), SymbolicExpr::Number(fact_number))
                        if number == fact_number =>
                    {
                        Some(*symbol)
                    }
                    (Value::Symbolic(expression), other) if expression.equals(other) => {
                        Some(*symbol)
                    }
                    _ => None,
                },
                _ => None,
            })
    }

    /// Rewrites an expression with every equality fact applied, so compound
    /// bounds stay consistent with previously accepted constraints.
    fn substitute_constraints(&self, expression: &SymbolicExpr) -> Rc<SymbolicExpr> {
        if self.constraints.borrow().is_empty() {
            return Rc::new(expression.clone());
        }
        let mut current = Rc::new(expression.clone());
        let mut remaining = self.constraints.borrow().clone();
        loop {
            let mut changed = false;
            remaining.retain(|fact| {
                if let ConstraintFact::Equal { symbol, value } = fact {
                    if let Some(rewritten) = rewrite_variable(&current, *symbol, value) {
                        current = rewritten;
                        changed = true;
                        return false;
                    }
                }
                true
            });
            if !changed || remaining.is_empty() {
                break;
            }
        }
        current
    }

    fn apply_fact_side_effects(&self, fact: &ConstraintFact) {
        match fact {
            ConstraintFact::Equal { symbol, value } => {
                if self.symbolic_real(value) {
                    self.real_symbols.borrow_mut().insert(*symbol);
                }
                if self.symbolic_is_integer(value) {
                    self.integer_symbols.borrow_mut().insert(*symbol);
                }
                match self.classify_bound(value) {
                    Some(true) => {
                        self.positive_symbols.borrow_mut().insert(*symbol);
                        self.nonnegative_symbols.borrow_mut().insert(*symbol);
                        self.nonzero_symbols.borrow_mut().insert(*symbol);
                    }
                    Some(false) => {
                        self.negative_symbols.borrow_mut().insert(*symbol);
                        self.nonzero_symbols.borrow_mut().insert(*symbol);
                    }
                    None => {
                        if matches!(value.as_ref(), SymbolicExpr::Number(number) if number.is_zero())
                        {
                            self.nonnegative_symbols.borrow_mut().insert(*symbol);
                        }
                    }
                }
            }
            ConstraintFact::RealCompare { symbol, bound, op } => {
                // The fact reads `bound op symbol`, so Lt/Le bound the symbol
                // from below and Gt/Ge from above. Sign knowledge about the
                // symbol follows from the bound's own sign.
                let sign = self.classify_bound(bound);
                let bound_zero =
                    matches!(bound.as_ref(), SymbolicExpr::Number(number) if number.is_zero());
                self.real_symbols.borrow_mut().insert(*symbol);
                match op {
                    // symbol > bound with bound >= 0.
                    BinaryOp::Lt if matches!(sign, Some(true)) || bound_zero => {
                        self.positive_symbols.borrow_mut().insert(*symbol);
                        self.nonnegative_symbols.borrow_mut().insert(*symbol);
                        self.nonzero_symbols.borrow_mut().insert(*symbol);
                    }
                    // symbol >= bound with bound > 0 (or bound >= 0).
                    BinaryOp::Le => match sign {
                        Some(true) => {
                            self.positive_symbols.borrow_mut().insert(*symbol);
                            self.nonnegative_symbols.borrow_mut().insert(*symbol);
                            self.nonzero_symbols.borrow_mut().insert(*symbol);
                        }
                        _ if bound_zero => {
                            self.nonnegative_symbols.borrow_mut().insert(*symbol);
                        }
                        _ => {}
                    },
                    // symbol < bound with bound <= 0.
                    BinaryOp::Gt if matches!(sign, Some(false)) || bound_zero => {
                        self.negative_symbols.borrow_mut().insert(*symbol);
                        self.nonzero_symbols.borrow_mut().insert(*symbol);
                    }
                    // symbol <= bound with bound < 0.
                    BinaryOp::Ge if matches!(sign, Some(false)) => {
                        self.negative_symbols.borrow_mut().insert(*symbol);
                        self.nonzero_symbols.borrow_mut().insert(*symbol);
                    }
                    _ => {}
                }
            }
            ConstraintFact::NotEqual { symbol, value } => {
                if matches!(value.as_ref(), SymbolicExpr::Number(number) if number.is_zero()) {
                    self.nonzero_symbols.borrow_mut().insert(*symbol);
                }
                self.real_symbols.borrow_mut().insert(*symbol);
            }
        }
    }

    /// Static sign of a (possibly symbolic) bound: `Some(true)` for
    /// provably positive, `Some(false)` for provably negative.
    fn classify_bound(&self, bound: &SymbolicExpr) -> Option<bool> {
        if self.symbolic_positive(bound) {
            Some(true)
        } else if self.symbolic_negative(bound) {
            Some(false)
        } else {
            None
        }
    }

    /// Detects direct conflicts between two facts about the same variable.
    /// Only numeric bounds are compared; symbolic bounds never conflict.
    fn facts_conflict(left: &ConstraintFact, right: &ConstraintFact) -> bool {
        use crate::interpreter::value::ConstraintFact::*;
        match (left, right) {
            (
                Equal {
                    symbol: a,
                    value: a_value,
                },
                Equal {
                    symbol: b,
                    value: b_value,
                },
            ) => a == b && !a_value.equals(b_value.as_ref()),
            (
                Equal {
                    symbol: a,
                    value: a_value,
                },
                RealCompare {
                    symbol: b,
                    bound: b_bound,
                    op,
                },
            ) => {
                a == b
                    && match (a_value.as_ref(), b_bound.as_ref()) {
                        (SymbolicExpr::Number(a_number), SymbolicExpr::Number(b_number)) => {
                            // The fact reads `bound op symbol`, so the
                            // equality value must satisfy `bound op value`.
                            let ordering = a_number.compare(b_number);
                            let satisfied = match op {
                                BinaryOp::Lt => {
                                    matches!(ordering, Some(std::cmp::Ordering::Greater))
                                }
                                BinaryOp::Le => matches!(
                                    ordering,
                                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                                ),
                                BinaryOp::Gt => {
                                    matches!(ordering, Some(std::cmp::Ordering::Less))
                                }
                                BinaryOp::Ge => matches!(
                                    ordering,
                                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                                ),
                                _ => true,
                            };
                            !satisfied
                        }
                        _ => false,
                    }
            }
            (
                RealCompare {
                    symbol: a,
                    bound: a_bound,
                    op: a_op,
                },
                Equal {
                    symbol: b,
                    value: b_value,
                },
            ) => Self::facts_conflict(
                &Equal {
                    symbol: *b,
                    value: Rc::clone(b_value),
                },
                &RealCompare {
                    symbol: *a,
                    bound: Rc::clone(a_bound),
                    op: *a_op,
                },
            ),
            (
                Equal {
                    symbol: a,
                    value: a_value,
                },
                NotEqual {
                    symbol: b,
                    value: b_value,
                },
            ) => a == b && a_value.equals(b_value.as_ref()),
            (
                NotEqual {
                    symbol: a,
                    value: a_value,
                },
                Equal {
                    symbol: b,
                    value: b_value,
                },
            ) => a == b && a_value.equals(b_value.as_ref()),
            (
                RealCompare {
                    symbol: a,
                    bound: a_bound,
                    op: a_op,
                },
                RealCompare {
                    symbol: b,
                    bound: b_bound,
                    op: b_op,
                },
            ) => {
                if a != b {
                    return false;
                }
                let (SymbolicExpr::Number(a_number), SymbolicExpr::Number(b_number)) =
                    (a_bound.as_ref(), b_bound.as_ref())
                else {
                    return false;
                };
                // Lt/Le bound the symbol from below, Gt/Ge from above; two
                // bounds conflict when the lower one meets or exceeds the
                // upper one (strictly on either side).
                let a_lower = matches!(a_op, BinaryOp::Lt | BinaryOp::Le);
                let b_lower = matches!(b_op, BinaryOp::Lt | BinaryOp::Le);
                if a_lower == b_lower {
                    return false;
                }
                let (lower, lower_strict, upper, upper_strict) = if a_lower {
                    (
                        a_number,
                        matches!(a_op, BinaryOp::Lt),
                        b_number,
                        matches!(b_op, BinaryOp::Gt),
                    )
                } else {
                    (
                        b_number,
                        matches!(b_op, BinaryOp::Lt),
                        a_number,
                        matches!(a_op, BinaryOp::Gt),
                    )
                };
                match lower.compare(upper) {
                    Some(std::cmp::Ordering::Greater) => true,
                    Some(std::cmp::Ordering::Equal) => lower_strict || upper_strict,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Rewrites a source expression with equality facts, keeping the
    /// original when nothing applies (or the rewrite cannot succeed).
    fn substitute_expression_constraints(&self, expression: &Expr) -> Option<Expr> {
        let equalities = self
            .constraints
            .borrow()
            .iter()
            .filter_map(|fact| match fact {
                ConstraintFact::Equal { symbol, value } => Some((*symbol, Rc::clone(value))),
                _ => None,
            })
            .collect::<Vec<_>>();
        if equalities.is_empty() {
            return None;
        }
        let symbolic = self.expression_to_symbolic(expression)?;
        let mut current = Rc::new(symbolic);
        let mut changed = false;
        for (symbol, value) in equalities {
            if let Some(rewritten) = rewrite_variable(&current, symbol, &value) {
                current = rewritten;
                changed = true;
            }
        }
        changed.then(|| self.symbolic_to_expression(&current))
    }

    /// Converts a source expression tree into the symbolic tree, or `None`
    /// for constructs without a symbolic form (assignments, blocks, ...).
    fn expression_to_symbolic(&self, expression: &Expr) -> Option<SymbolicExpr> {
        match &expression.kind {
            ExprKind::Bool(value) => Some(SymbolicExpr::Number(Number::Integer(BigInt::from(
                u8::from(*value),
            )))),
            ExprKind::Nil => Some(SymbolicExpr::Number(Number::Integer(BigInt::from(0)))),
            ExprKind::Number(text) => {
                let (integer, decimal) = if let Some(hex) = text.strip_prefix("0x") {
                    (BigInt::parse_bytes(hex.as_bytes(), 16), None)
                } else if let Some(binary) = text.strip_prefix("0b") {
                    (BigInt::parse_bytes(binary.as_bytes(), 2), None)
                } else if let Some(octal) = text.strip_prefix("0o") {
                    (BigInt::parse_bytes(octal.as_bytes(), 8), None)
                } else if !text.contains(['.', 'e', 'E']) {
                    (BigInt::parse_bytes(text.as_bytes(), 10), None)
                } else {
                    (None, text.parse::<BigDecimal>().ok())
                };
                if let Some(integer) = integer {
                    return Some(SymbolicExpr::Number(Number::Integer(integer)));
                }
                decimal.map(Number::Decimal).map(SymbolicExpr::Number)
            }
            ExprKind::String(_) => None,
            ExprKind::Variable(symbol) => Some(SymbolicExpr::Variable(*symbol)),
            ExprKind::Group(inner) => self.expression_to_symbolic(inner),
            ExprKind::Array(_) | ExprKind::Index { .. } => None,
            ExprKind::Unary { op, expr } => {
                let inner = Rc::new(self.expression_to_symbolic(expr)?);
                Some(SymbolicExpr::Unary {
                    op: *op,
                    expr: inner,
                })
            }
            ExprKind::Binary {
                left,
                op,
                right,
                implicit: _,
            } => {
                let left = Rc::new(self.expression_to_symbolic(left)?);
                let right = Rc::new(self.expression_to_symbolic(right)?);
                Some(SymbolicExpr::Binary {
                    left,
                    op: *op,
                    right,
                })
            }
            ExprKind::Call { callee, args } => {
                let ExprKind::Variable(symbol) = callee.kind else {
                    return None;
                };
                let function = self.symbol_function(symbol)?;
                let mut converted = Vec::with_capacity(args.len());
                for argument in args {
                    converted.push(Rc::new(self.expression_to_symbolic(argument)?));
                }
                Some(SymbolicExpr::Call {
                    function,
                    args: converted,
                })
            }
        }
    }

    /// Resolves a called name to its builtin, consulting the environment
    /// first so user shadowing wins over the standard library.
    fn symbol_function(&self, symbol: Symbol) -> Option<Builtin> {
        if let Some(value) = Environment::get(&self.globals, symbol) {
            return match value {
                Value::Builtin(builtin) => Some(builtin),
                _ => None,
            };
        }
        let name = self.symbols.resolve(symbol);
        Builtin::ALL
            .iter()
            .find(|(builtin_name, _)| *builtin_name == name)
            .map(|(_, builtin)| *builtin)
    }

    /// Public accessor for embedders (GUI status lines, REPL diagnostics).
    pub fn constraint_count(&self) -> usize {
        self.constraints.borrow().len()
    }

    /// The substituted value of an equality fact for `symbol`, if one exists.
    fn equality_fact_for(&self, symbol: Symbol) -> Option<Rc<SymbolicExpr>> {
        self.constraints
            .borrow()
            .iter()
            .find_map(|fact| match fact {
                ConstraintFact::Equal {
                    symbol: found,
                    value,
                } if *found == symbol => Some(Rc::clone(value)),
                _ => None,
            })
    }

    fn symbolic_nonzero(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(number) => !number.is_zero(),
            SymbolicExpr::Constant(_) => true,
            SymbolicExpr::Variable(symbol) => self.nonzero_symbols.borrow().contains(symbol),
            SymbolicExpr::Unary { expr, .. } => self.symbolic_nonzero(expr),
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Mul | BinaryOp::Div,
                right,
            } => self.symbolic_nonzero(left) && self.symbolic_nonzero(right),
            SymbolicExpr::Binary {
                left,
                op: BinaryOp::Pow,
                ..
            } => self.symbolic_nonzero(left),
            SymbolicExpr::Call {
                function: Builtin::Exp,
                ..
            } => true,
            SymbolicExpr::Call {
                function: Builtin::Sqrt,
                args,
            } => args.len() == 1 && self.symbolic_positive(&args[0]),
            _ => self.symbolic_positive(expression),
        }
    }

    fn symbolic_real(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(Number::Complex(_)) => false,
            SymbolicExpr::Number(_) | SymbolicExpr::Constant(_) => true,
            SymbolicExpr::Variable(symbol) => self.real_symbols.borrow().contains(symbol),
            SymbolicExpr::Unary { expr, .. } => self.symbolic_real(expr),
            SymbolicExpr::Binary { left, right, .. } => {
                self.symbolic_real(left) && self.symbolic_real(right)
            }
            SymbolicExpr::Call { function, args } => match function {
                Builtin::Sqrt => args.len() == 1 && self.symbolic_nonnegative(&args[0]),
                Builtin::Ln | Builtin::Log => args.len() == 1 && self.symbolic_positive(&args[0]),
                _ => args.iter().all(|argument| self.symbolic_real(argument)),
            },
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
            (UnaryOp::Neg, Value::Number(Number::Rational(n))) => {
                Ok(Value::Number(Number::Rational(-n)))
            }
            (UnaryOp::Neg, Value::Number(Number::Complex(n))) => Ok(Value::Number(
                Number::Complex(ComplexNumber::new(-n.real, -n.imaginary)),
            )),
            (UnaryOp::Neg, Value::Symbolic(expr)) => {
                Ok(Value::Symbolic(Rc::new(match expr.as_ref() {
                    SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr: inner,
                    } => {
                        return Ok(Self::symbolic_value(Rc::clone(inner)));
                    }
                    // Keep the sign in the leading numeric coefficient:
                    // `-(c * r) -> (-c) * r`. Products without a numeric
                    // leading factor keep the negation outside.
                    SymbolicExpr::Binary {
                        left,
                        op: BinaryOp::Mul,
                        right,
                    } if matches!(left.as_ref(), SymbolicExpr::Number(_)) => {
                        let negated =
                            self.apply_unary(UnaryOp::Neg, Self::symbolic_value(Rc::clone(left)))?;
                        return self.apply_binary(
                            BinaryOp::Mul,
                            negated,
                            Self::symbolic_value(Rc::clone(right)),
                        );
                    }
                    _ => SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr,
                    },
                })))
            }
            (UnaryOp::Not, Value::Symbolic(expression)) => {
                // Boolean algebra on symbolic predicates: comparison
                // negation and De Morgan, with double negation cancelling.
                let negated = match expression.as_ref() {
                    SymbolicExpr::Unary {
                        op: UnaryOp::Not,
                        expr,
                    } => return Ok(Self::symbolic_value(Rc::clone(expr))),
                    SymbolicExpr::Binary {
                        left,
                        op:
                            op @ (BinaryOp::Eq
                            | BinaryOp::Ne
                            | BinaryOp::Lt
                            | BinaryOp::Le
                            | BinaryOp::Gt
                            | BinaryOp::Ge),
                        right,
                    } => {
                        let flipped = match op {
                            BinaryOp::Eq => BinaryOp::Ne,
                            BinaryOp::Ne => BinaryOp::Eq,
                            BinaryOp::Lt => BinaryOp::Ge,
                            BinaryOp::Le => BinaryOp::Gt,
                            BinaryOp::Gt => BinaryOp::Le,
                            BinaryOp::Ge => BinaryOp::Lt,
                            _ => unreachable!("guarded above"),
                        };
                        Rc::new(SymbolicExpr::Binary {
                            left: Rc::clone(left),
                            op: flipped,
                            right: Rc::clone(right),
                        })
                    }
                    SymbolicExpr::Binary {
                        left,
                        op: op @ (BinaryOp::And | BinaryOp::Or),
                        right,
                    } => {
                        let flipped = if *op == BinaryOp::And {
                            BinaryOp::Or
                        } else {
                            BinaryOp::And
                        };
                        let left_negated =
                            self.apply_unary(UnaryOp::Not, Self::symbolic_value(Rc::clone(left)))?;
                        let right_negated =
                            self.apply_unary(UnaryOp::Not, Self::symbolic_value(Rc::clone(right)))?;
                        return self.symbolic_binary(flipped, left_negated, right_negated);
                    }
                    _ => Rc::new(SymbolicExpr::Unary {
                        op: UnaryOp::Not,
                        expr: Rc::clone(&expression),
                    }),
                };
                Ok(Value::Symbolic(negated))
            }
            (UnaryOp::Not, value) => Ok(Value::Bool(!value.is_truthy())),
            (UnaryOp::BitNot, Value::Number(Number::Integer(n))) => {
                Ok(Value::Number(Number::Integer(!n)))
            }
            (UnaryOp::Transpose, value) => {
                let matrix = self.matrix(&value, "transpose")?;
                Ok(Self::matrix_value(Self::transpose_matrix(&matrix)))
            }
            (op, value) => {
                Err(self.error(format!("cannot apply {op:?} to a {}", value.type_name())))
            }
        }
    }

    fn apply_binary(&self, op: BinaryOp, l: Value, r: Value) -> ExecResult<Value> {
        use BinaryOp::*;
        if op == Pow && matches!(&l, Value::Array(_)) {
            return self.power(l, r);
        }
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
            Mul if matches!(&l, Value::Array(rows) if rows.iter().any(|row| matches!(row, Value::Array(_)))) => {
                self.matrix_product(&l, &r, "*")
            }
            Mul => self.numeric(l, r, "*", |a, b| a * b, |a, b| a * b, |a, b| a * b),
            Div => self.divide(l, r),
            Mod => self.remainder(l, r),
            Pow => self.power(l, r),

            Eq => {
                if matches!(&l, Value::Symbolic(_)) || matches!(&r, Value::Symbolic(_)) {
                    // Canonically identical sides are trivially equal even
                    // when symbolic, so `print(a*x + 0 == a*x)` stays a
                    // boolean; anything else becomes a constraint tree for
                    // `assume` to consume.
                    if l.equals(&r) {
                        return Ok(Value::Bool(true));
                    }
                    return self.symbolic_constraint(BinaryOp::Eq, l, r);
                }
                Ok(Value::Bool(l.equals(&r)))
            }
            Ne => {
                if matches!(&l, Value::Symbolic(_)) || matches!(&r, Value::Symbolic(_)) {
                    if l.equals(&r) {
                        return Ok(Value::Bool(false));
                    }
                    return self.symbolic_constraint(BinaryOp::Ne, l, r);
                }
                Ok(Value::Bool(!l.equals(&r)))
            }

            Lt if self.constraint_comparison(&l, &r) => {
                self.symbolic_constraint(BinaryOp::Lt, l, r)
            }
            Le if self.constraint_comparison(&l, &r) => {
                self.symbolic_constraint(BinaryOp::Le, l, r)
            }
            Gt if self.constraint_comparison(&l, &r) => {
                self.symbolic_constraint(BinaryOp::Gt, l, r)
            }
            Ge if self.constraint_comparison(&l, &r) => {
                self.symbolic_constraint(BinaryOp::Ge, l, r)
            }
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
                if matches!(a, Number::Complex(_)) || matches!(b, Number::Complex(_)) {
                    return self.complex_arithmetic(a, b, sym);
                }
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
                if matches!(a, Number::Complex(_)) || matches!(b, Number::Complex(_)) {
                    return self.complex_arithmetic(a, b, "/");
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
                if matches!(a, Number::Complex(_)) || matches!(b, Number::Complex(_)) {
                    return Err(self.error("remainder is not defined for complex numbers"));
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
        if matches!(base, Value::Array(_)) {
            let Value::Number(Number::Integer(exponent)) = exponent else {
                return Err(self.error("matrix exponent must be an integer"));
            };
            return self.matrix_integer_power(&base, exponent);
        }
        let (Value::Number(base), Value::Number(exponent)) = (base, exponent) else {
            return Err(self.error("'^' requires numeric operands"));
        };
        let Number::Integer(integer_exponent) = exponent else {
            if base.is_exact() && exponent.is_exact() {
                if let Number::Rational(rational_exponent) = &exponent {
                    if let (Some(degree), Some(power)) = (
                        rational_exponent.denom().to_u32(),
                        rational_exponent.numer().to_i64(),
                    ) {
                        if degree > 0 && degree <= self.limits.max_exponent {
                            if let Some(root) = base.exact_root(degree) {
                                return self.power(
                                    Value::Number(root),
                                    Value::Number(Number::Integer(BigInt::from(power))),
                                );
                            }
                        }
                    }
                }
                return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Binary {
                    left: Rc::new(SymbolicExpr::Number(base)),
                    op: BinaryOp::Pow,
                    right: Rc::new(SymbolicExpr::Number(exponent)),
                })));
            }
            return base
                .evaluate_binary(
                    &exponent,
                    BinaryMathFunction::Pow,
                    self.transcendental_precision()?,
                )
                .map(Value::Number)
                .map_err(|error| self.error(format!("power failed: {error}")));
        };
        let exponent = integer_exponent;
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
                Number::Complex(value) => {
                    return self.complex_integer_power(value, i64::from(exponent));
                }
            }));
        }
        if base.is_zero() {
            return Err(self.error("zero cannot be raised to a negative power"));
        }
        match base {
            Number::Complex(value) => self.complex_integer_power(value, exponent),
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

    fn matrix_integer_power(&self, base: &Value, exponent: BigInt) -> ExecResult<Value> {
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

        let mut base = self.square_matrix(base, "matrix power")?;
        let size = base.len();
        if exponent < 0 {
            let inverse =
                self.call_linear_algebra(Builtin::Inverse, vec![Self::matrix_value(base)])?;
            base = self.matrix(&inverse, "matrix power")?;
        }

        let mut result = (0..size)
            .map(|row| {
                (0..size)
                    .map(|column| Self::integer_value(i64::from(row == column)))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut remaining = exponent.unsigned_abs();
        while remaining > 0 {
            if remaining & 1 == 1 {
                result = self.matmul_values(&result, &base, "matrix power")?;
            }
            remaining >>= 1;
            if remaining > 0 {
                base = self.matmul_values(&base, &base, "matrix power")?;
            }
        }
        Ok(Self::matrix_value(result))
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
            (Value::Number(a), Value::Number(b)) => a
                .compare(b)
                .map(|ordering| Value::Bool(f(ordering)))
                .ok_or_else(|| self.error("complex numbers are not ordered")),
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
            Number::Complex(value) => value
                .real
                .as_bigint_and_exponent()
                .0
                .bits()
                .saturating_add(value.imaginary.as_bigint_and_exponent().0.bits()),
        }
    }

    fn complex_parts(&self, number: Number) -> (BigDecimal, BigDecimal) {
        match number {
            Number::Complex(value) => (value.real, value.imaginary),
            value => (value.to_decimal(&self.decimal_context), BigDecimal::from(0)),
        }
    }

    fn complex_value(&self, real: BigDecimal, imaginary: BigDecimal) -> Value {
        let real = self.decimal_context.round_decimal(real);
        let imaginary = self.decimal_context.round_decimal(imaginary);
        if imaginary == 0 {
            Value::Number(Number::Decimal(real))
        } else {
            Value::Number(Number::Complex(ComplexNumber::new(real, imaginary)))
        }
    }

    fn complex_arithmetic(&self, left: Number, right: Number, op: &str) -> ExecResult<Value> {
        let (a, b) = self.complex_parts(left);
        let (c, d) = self.complex_parts(right);
        match op {
            "+" => Ok(self.complex_value(a + c, b + d)),
            "-" => Ok(self.complex_value(a - c, b - d)),
            "*" => {
                let ac = self.decimal_context.multiply(&a, &c);
                let bd = self.decimal_context.multiply(&b, &d);
                let ad = self.decimal_context.multiply(&a, &d);
                let bc = self.decimal_context.multiply(&b, &c);
                Ok(self.complex_value(ac - bd, ad + bc))
            }
            "/" => {
                let c2 = self.decimal_context.multiply(&c, &c);
                let d2 = self.decimal_context.multiply(&d, &d);
                let denominator = c2 + d2;
                if denominator == 0 {
                    return Err(self.error("division by zero in '/'"));
                }
                let reciprocal = self.decimal_context.invert(&denominator);
                let ac = self.decimal_context.multiply(&a, &c);
                let bd = self.decimal_context.multiply(&b, &d);
                let bc = self.decimal_context.multiply(&b, &c);
                let ad = self.decimal_context.multiply(&a, &d);
                let real_numerator = ac + bd;
                let imaginary_numerator = bc - ad;
                Ok(self.complex_value(
                    self.decimal_context.multiply(&real_numerator, &reciprocal),
                    self.decimal_context
                        .multiply(&imaginary_numerator, &reciprocal),
                ))
            }
            _ => Err(self.error(format!("unsupported complex operator '{op}'"))),
        }
    }

    fn complex_integer_power(&self, base: ComplexNumber, exponent: i64) -> ExecResult<Value> {
        if exponent == 0 {
            return Ok(Value::Number(Number::Integer(BigInt::from(1))));
        }
        let negative = exponent < 0;
        let mut remaining = exponent.unsigned_abs();
        let mut factor = Number::Complex(base);
        let mut result = Number::Integer(BigInt::from(1));
        while remaining > 0 {
            if remaining & 1 == 1 {
                let Value::Number(value) = self.complex_arithmetic(result, factor.clone(), "*")?
                else {
                    unreachable!("complex multiplication returns a number")
                };
                result = value;
            }
            remaining >>= 1;
            if remaining > 0 {
                let Value::Number(value) = self.complex_arithmetic(factor.clone(), factor, "*")?
                else {
                    unreachable!("complex multiplication returns a number")
                };
                factor = value;
            }
        }
        if negative {
            self.complex_arithmetic(Number::Integer(BigInt::from(1)), result, "/")
        } else {
            Ok(Value::Number(result))
        }
    }

    fn symbolic_binary(&self, op: BinaryOp, left: Value, right: Value) -> ExecResult<Value> {
        let left = self.value_to_symbolic(left)?;
        let right = self.value_to_symbolic(right)?;

        if op == BinaryOp::Add || op == BinaryOp::Sub {
            // Squared terms of `left [+-] right` with their effective sign.
            let mut squares = Vec::new();
            for (expression, positive) in [(&left, true), (&right, op != BinaryOp::Sub)] {
                let direct = Self::squared_unary_call(expression);
                let negated = match expression.as_ref() {
                    SymbolicExpr::Unary {
                        op: UnaryOp::Neg,
                        expr,
                    } => Self::squared_unary_call(expr),
                    _ => None,
                };
                if let Some((function, argument)) = direct {
                    squares.push((function, argument, positive));
                } else if let Some((function, argument)) = negated {
                    squares.push((function, argument, !positive));
                }
            }
            if let [(left_fn, left_arg, left_pos), (right_fn, right_arg, right_pos)] =
                squares.as_slice()
            {
                if left_arg.equals(right_arg) {
                    // sin^2 + cos^2 -> 1
                    if *left_pos
                        && *right_pos
                        && matches!(
                            (left_fn, right_fn),
                            (Builtin::Sin, Builtin::Cos) | (Builtin::Cos, Builtin::Sin)
                        )
                    {
                        return Ok(Value::Number(Number::Integer(BigInt::from(1))));
                    }
                    // cosh^2 - sinh^2 -> 1, sinh^2 - cosh^2 -> -1
                    if left_pos != right_pos
                        && matches!(
                            (left_fn, right_fn),
                            (Builtin::Cosh, Builtin::Sinh) | (Builtin::Sinh, Builtin::Cosh)
                        )
                    {
                        let positive_fn = if *left_pos { *left_fn } else { *right_fn };
                        let value = if positive_fn == Builtin::Cosh { 1 } else { -1 };
                        return Ok(Value::Number(Number::Integer(BigInt::from(value))));
                    }
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

        // log(a) + log(b) -> log(a*b) and log(a) - log(b) -> log(a/b) for
        // positive arguments (the principal-branch condition).
        if matches!(op, BinaryOp::Add | BinaryOp::Sub) {
            if let (
                SymbolicExpr::Call {
                    function: left_fn,
                    args: left_args,
                },
                SymbolicExpr::Call {
                    function: right_fn,
                    args: right_args,
                },
            ) = (left.as_ref(), right.as_ref())
            {
                if left_fn == right_fn
                    && matches!(
                        left_fn,
                        Builtin::Ln | Builtin::Log | Builtin::Log10 | Builtin::Log2
                    )
                    && left_args.len() == 1
                    && right_args.len() == 1
                    && self.symbolic_positive(&left_args[0])
                    && self.symbolic_positive(&right_args[0])
                {
                    let combined = if op == BinaryOp::Add {
                        Self::symbolic_binary_raw(
                            Rc::clone(&left_args[0]),
                            BinaryOp::Mul,
                            Rc::clone(&right_args[0]),
                        )
                    } else {
                        Self::symbolic_binary_raw(
                            Rc::clone(&left_args[0]),
                            BinaryOp::Div,
                            Rc::clone(&right_args[0]),
                        )
                    };
                    return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                        function: *left_fn,
                        args: vec![combined],
                    })));
                }
            }
        }
        // sqrt(a)*sqrt(b) -> sqrt(a*b) for nonnegative arguments.
        if op == BinaryOp::Mul {
            if let (
                SymbolicExpr::Call {
                    function: Builtin::Sqrt,
                    args: left_args,
                },
                SymbolicExpr::Call {
                    function: Builtin::Sqrt,
                    args: right_args,
                },
            ) = (left.as_ref(), right.as_ref())
            {
                if left_args.len() == 1
                    && right_args.len() == 1
                    && self.symbolic_nonnegative(&left_args[0])
                    && self.symbolic_nonnegative(&right_args[0])
                {
                    return Ok(Self::symbolic_value(Rc::new(SymbolicExpr::Call {
                        function: Builtin::Sqrt,
                        args: vec![Self::symbolic_binary_raw(
                            Rc::clone(&left_args[0]),
                            BinaryOp::Mul,
                            Rc::clone(&right_args[0]),
                        )],
                    })));
                }
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
                BinaryOp::Div if self.symbolic_nonzero(&left) => {
                    Value::Number(Number::Integer(BigInt::from(1)))
                }
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
            if matches!(left.as_ref(), SymbolicExpr::Constant(MathConstant::Phi))
                && Self::is_symbolic_integer(&right, 2)
            {
                return Ok(Self::symbolic_value(Self::symbolic_binary_raw(
                    Rc::clone(&left),
                    BinaryOp::Add,
                    Self::symbolic_integer(1),
                )));
            }
            if Self::is_symbolic_integer(&right, 2) {
                if let SymbolicExpr::Call {
                    function: Builtin::Abs,
                    args,
                } = left.as_ref()
                {
                    if args.len() == 1 && self.symbolic_real(&args[0]) {
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
            if Self::symbolic_is_one(&left)
                && matches!(right.as_ref(), SymbolicExpr::Constant(MathConstant::Phi))
            {
                return Ok(Self::symbolic_value(Self::symbolic_binary_raw(
                    Rc::clone(&right),
                    BinaryOp::Sub,
                    Self::symbolic_integer(1),
                )));
            }
            if self.symbolic_nonzero(&right) {
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
            }
            let (left_base, left_exponent) = Self::integer_power_parts(&left);
            let (right_base, right_exponent) = Self::integer_power_parts(&right);
            if left_base.equals(&right_base) && self.symbolic_nonzero(&left_base) {
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

    /// Whether a comparison over these operands should stay symbolic as a
    /// constraint: exactly when an operand is a symbolic expression.
    fn constraint_comparison(&self, left: &Value, right: &Value) -> bool {
        matches!(left, Value::Symbolic(_)) || matches!(right, Value::Symbolic(_))
    }

    /// Builds a symbolic constraint tree from a comparison whose operands
    /// stay symbolic. Equality facts fold in so `assume(x == 3)` followed
    /// by `assume(x > 2 and x < 5)` stores bounds against `3`.
    fn symbolic_constraint(&self, op: BinaryOp, left: Value, right: Value) -> ExecResult<Value> {
        let left = self.value_to_symbolic(left)?;
        let right = self.value_to_symbolic(right)?;
        Ok(Value::Symbolic(Rc::new(SymbolicExpr::Binary {
            left,
            op,
            right,
        })))
    }

    /// Builds a symbolic conjunction of constraint trees. Like `and` in
    /// the evaluator, order matters: each operand is attempted in turn.
    fn symbolic_conjunction(&self, left: &SymbolicExpr, right: &SymbolicExpr) -> Rc<SymbolicExpr> {
        Rc::new(SymbolicExpr::Binary {
            left: Rc::new(left.clone()),
            op: BinaryOp::And,
            right: Rc::new(right.clone()),
        })
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

    fn product_unary_calls(
        expression: &SymbolicExpr,
    ) -> Option<(Builtin, Builtin, Rc<SymbolicExpr>)> {
        let SymbolicExpr::Binary {
            left,
            op: BinaryOp::Mul,
            right,
        } = expression
        else {
            return None;
        };
        let (
            SymbolicExpr::Call {
                function: left_function,
                args: left_args,
            },
            SymbolicExpr::Call {
                function: right_function,
                args: right_args,
            },
        ) = (left.as_ref(), right.as_ref())
        else {
            return None;
        };
        (left_args.len() == 1 && right_args.len() == 1 && left_args[0].equals(&right_args[0]))
            .then(|| (*left_function, *right_function, Rc::clone(&left_args[0])))
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
            if let Some(match_index) = denominator_factors.iter().position(|factor| {
                numerator_factors[index].equals(factor) && self.symbolic_nonzero(factor)
            }) {
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

    /// Rebuilds a source expression from a symbolic tree so constraint
    /// substitutions can flow through the ordinary evaluator.
    fn symbolic_to_expression(&self, expression: &Rc<SymbolicExpr>) -> Expr {
        let span = SourceSpan::diagnostic(1, 1, 0);
        let kind = match expression.as_ref() {
            SymbolicExpr::Number(number) => match number {
                Number::Integer(value) => ExprKind::Number(value.to_string()),
                Number::Decimal(value) => ExprKind::Number(value.to_string()),
                Number::Rational(value) => ExprKind::Binary {
                    left: Box::new(Expr::new(span, ExprKind::Number(value.numer().to_string()))),
                    op: BinaryOp::Div,
                    right: Box::new(Expr::new(span, ExprKind::Number(value.denom().to_string()))),
                    implicit: false,
                },
                Number::Complex(_) => ExprKind::Nil,
            },
            SymbolicExpr::Constant(constant) => {
                let name = match constant {
                    MathConstant::Pi => "pi",
                    MathConstant::E => "e",
                    MathConstant::Tau => "tau",
                    MathConstant::Phi => "phi",
                };
                let Some(symbol) = self.symbols.get(name) else {
                    // The constant is unknown to this table; leave it
                    // symbolic rather than mutating shared state.
                    return Expr::new(span, ExprKind::Nil);
                };
                ExprKind::Variable(symbol)
            }
            SymbolicExpr::Variable(symbol) => ExprKind::Variable(*symbol),
            SymbolicExpr::Unary { op, expr } => ExprKind::Unary {
                op: *op,
                expr: Box::new(self.symbolic_to_expression(expr)),
            },
            SymbolicExpr::Binary { left, op, right } => ExprKind::Binary {
                left: Box::new(self.symbolic_to_expression(left)),
                op: *op,
                right: Box::new(self.symbolic_to_expression(right)),
                implicit: false,
            },
            SymbolicExpr::Call { function, args } => {
                let name = function.name();
                let symbol = self.symbols.get(name).unwrap_or(0);
                ExprKind::Call {
                    callee: Box::new(Expr::new(span, ExprKind::Variable(symbol))),
                    args: args
                        .iter()
                        .map(|argument| self.symbolic_to_expression(argument))
                        .collect::<Vec<_>>(),
                }
            }
        };
        Expr::new(span, kind)
    }

    /// Whether a symbolic tree is statically an integer under the active
    /// assumptions: integer literals, integer-typed equality facts, and
    /// even integer powers of known-integer bases compose.
    fn symbolic_is_integer(&self, expression: &SymbolicExpr) -> bool {
        match expression {
            SymbolicExpr::Number(Number::Integer(_)) => true,
            SymbolicExpr::Variable(symbol) => self.integer_symbols.borrow().contains(symbol),
            SymbolicExpr::Unary { op, expr } => {
                matches!(op, UnaryOp::Neg) && self.symbolic_is_integer(expr)
            }
            SymbolicExpr::Binary { left, op, right } => match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
                    self.symbolic_is_integer(left) && self.symbolic_is_integer(right)
                }
                BinaryOp::Pow => {
                    matches!(right.as_ref(), SymbolicExpr::Number(Number::Integer(_)))
                        && self.symbolic_is_integer(left)
                }
                _ => false,
            },
            SymbolicExpr::Call { function, args } => {
                matches!(
                    function,
                    Builtin::Floor | Builtin::Ceil | Builtin::Round | Builtin::Trunc
                ) && args.len() == 1
            }
            _ => false,
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
            Value::Derivation { result, .. } => self.stringify(result),
        }
    }

    fn format_symbolic(&self, expression: &SymbolicExpr, parent_precedence: u8) -> String {
        match expression {
            SymbolicExpr::Number(number) => number.to_string(),
            SymbolicExpr::Constant(constant) => constant.name().to_string(),
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
                if *op == UnaryOp::Transpose {
                    let text = format!("{}'", self.format_expr(expr, 13));
                    return if 13 < parent_precedence {
                        format!("({text})")
                    } else {
                        text
                    };
                }
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
            UnaryOp::Transpose => "'",
        }
    }
}
