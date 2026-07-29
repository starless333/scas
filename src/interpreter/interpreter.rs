use crate::ast::assign_op::AssignOp;
use crate::ast::binary_op::BinaryOp;
use crate::ast::expr::Expr;
use crate::ast::statement::Statement;
use crate::ast::unary_op::UnaryOp;
use crate::interpreter::environment::{Env, Environment};
use crate::interpreter::flow::{ExecResult, Flow};
use crate::interpreter::value::{UserFunction, Value};
use crate::lexer::symbol::{Symbol, SymbolTable};
use std::rc::Rc;

pub struct Interpreter {
        globals: Env,
        /// Owned so error messages can resolve a `Symbol` back to its
        /// source text (`"undefined variable 'x'"`). Evaluation itself
        /// never needs this -- every name reference in the AST is already
        /// the interned id, not text.
        symbols: SymbolTable,
}

impl Interpreter {
        pub fn new(symbols: SymbolTable) -> Self {
                Interpreter { globals: Environment::new(), symbols }
        }

        /// Runs a whole program (as produced by `Parser::parse`) in the
        /// global scope. Returns a human-readable message on the first
        /// unhandled runtime error.
        pub fn run(&mut self, program: &[Statement]) -> Result<(), String> {
                let env = Rc::clone(&self.globals);
                for stmt in program {
                        self.execute(&env, stmt).map_err(|flow| self.describe_top_level(flow))?;
                }
                Ok(())
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
                self.error(format!("undefined variable '{}'", self.symbols.resolve(name)))
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
                                let f = Value::Function(Rc::new(UserFunction {
                                        params: params.clone(),
                                        body: body.clone(),
                                        closure: Rc::clone(env),
                                }));
                                env.borrow_mut().define(*name, f);
                                Ok(())
                        }

                        Statement::If { condition, then_branch, else_branch } => {
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

                        Statement::For { init, condition, update, body } => {
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
                                println!("{}", self.stringify(&v));
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

        // ---- expressions ------------------------------------------------

        fn evaluate(&mut self, env: &Env, expr: &Expr) -> ExecResult<Value> {
                match expr {
                        Expr::Bool(b) => Ok(Value::Bool(*b)),
                        Expr::Nil => Ok(Value::Nil),
                        Expr::Number(text) => self.parse_number(text),
                        Expr::String(s) => Ok(Value::Str(s.clone())),
                        Expr::Variable(sym) => {
                                Environment::get(env, *sym).ok_or_else(|| self.undefined_variable(*sym))
                        }
                        Expr::Group(inner) => self.evaluate(env, inner),

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
                // Function bodies are a single expression (see the parser's
                // `function_declaration`), so calling one is just evaluating
                // it in the freshly-bound parameter scope -- there's no
                // `Statement::Return` reachable from inside a bare `Expr`.
                self.evaluate(&call_env, &f.body)
        }

        // ---- number parsing ----------------------------------------------

        fn parse_number(&self, text: &str) -> ExecResult<Value> {
                let parsed: Option<f64> = if let Some(hex) = text.strip_prefix("0x") {
                        i64::from_str_radix(hex, 16).ok().map(|n| n as f64)
                } else if let Some(bin) = text.strip_prefix("0b") {
                        i64::from_str_radix(bin, 2).ok().map(|n| n as f64)
                } else if let Some(oct) = text.strip_prefix("0o") {
                        i64::from_str_radix(oct, 8).ok().map(|n| n as f64)
                } else {
                        text.parse::<f64>().ok()
                };
                parsed
                        .map(Value::Number)
                        .ok_or_else(|| self.error(format!("invalid number literal '{text}'")))
        }

        // ---- operators -----------------------------------------------------

        fn apply_unary(&self, op: UnaryOp, v: Value) -> ExecResult<Value> {
                match (op, &v) {
                        (UnaryOp::Neg, Value::Number(n)) => Ok(Value::Number(-n)),
                        (UnaryOp::Not, _) => Ok(Value::Bool(!v.is_truthy())),
                        (UnaryOp::BitNot, Value::Number(n)) => {
                                let i = self.as_int(*n)?;
                                Ok(Value::Number(!i as f64))
                        }
                        (op, _) => Err(self.error(format!("cannot apply {op:?} to a {}", v.type_name()))),
                }
        }

        fn apply_binary(&self, op: BinaryOp, l: Value, r: Value) -> ExecResult<Value> {
                use BinaryOp::*;
                match op {
                        Add => match (&l, &r) {
                                (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
                                (Value::Str(a), Value::Str(b)) => Ok(Value::Str(format!("{a}{b}"))),
                                _ => Err(self.type_error("+", &l, &r)),
                        },
                        Sub => self.numeric(l, r, "-", |a, b| a - b),
                        Mul => self.numeric(l, r, "*", |a, b| a * b),
                        Div => self.numeric_checked(l, r, "/", |a, b| (b != 0.0).then(|| a / b)),
                        Mod => self.numeric_checked(l, r, "%", |a, b| (b != 0.0).then(|| a % b)),
                        Pow => self.numeric(l, r, "^", |a, b| a.powf(b)),

                        Eq => Ok(Value::Bool(l.equals(&r))),
                        Ne => Ok(Value::Bool(!l.equals(&r))),

                        Lt => self.compare(l, r, "<", |o| o.is_lt()),
                        Le => self.compare(l, r, "<=", |o| !o.is_gt()),
                        Gt => self.compare(l, r, ">", |o| o.is_gt()),
                        Ge => self.compare(l, r, ">=", |o| !o.is_lt()),

                        BitAnd => self.bitwise(l, r, "&", |a, b| a & b),
                        BitOr => self.bitwise(l, r, "|", |a, b| a | b),
                        BitXor => self.bitwise(l, r, "xor", |a, b| a ^ b),
                        Shl => self.bitwise(l, r, "<<", |a, b| a << b),
                        Shr => self.bitwise(l, r, ">>", |a, b| a >> b),

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
                        AssignOp::Assign => unreachable!("plain '=' is handled without reading the current value"),
                };
                self.apply_binary(bin_op, current, rhs)
        }

        fn numeric(&self, l: Value, r: Value, sym: &str, f: impl Fn(f64, f64) -> f64) -> ExecResult<Value> {
                match (&l, &r) {
                        (Value::Number(a), Value::Number(b)) => Ok(Value::Number(f(*a, *b))),
                        _ => Err(self.type_error(sym, &l, &r)),
                }
        }

        fn numeric_checked(
                &self,
                l: Value,
                r: Value,
                sym: &str,
                f: impl Fn(f64, f64) -> Option<f64>,
        ) -> ExecResult<Value> {
                match (&l, &r) {
                        (Value::Number(a), Value::Number(b)) => f(*a, *b)
                                .map(Value::Number)
                                .ok_or_else(|| self.error(format!("division by zero in '{sym}'"))),
                        _ => Err(self.type_error(sym, &l, &r)),
                }
        }

        fn compare(&self, l: Value, r: Value, sym: &str, f: impl Fn(std::cmp::Ordering) -> bool) -> ExecResult<Value> {
                match (&l, &r) {
                        (Value::Number(a), Value::Number(b)) => a
                                .partial_cmp(b)
                                .map(|o| Value::Bool(f(o)))
                                .ok_or_else(|| self.error(format!("cannot compare NaN with '{sym}'"))),
                        (Value::Str(a), Value::Str(b)) => Ok(Value::Bool(f(a.cmp(b)))),
                        _ => Err(self.type_error(sym, &l, &r)),
                }
        }

        fn bitwise(&self, l: Value, r: Value, sym: &str, f: impl Fn(i64, i64) -> i64) -> ExecResult<Value> {
                match (&l, &r) {
                        (Value::Number(a), Value::Number(b)) => {
                                let (a, b) = (self.as_int(*a)?, self.as_int(*b)?);
                                Ok(Value::Number(f(a, b) as f64))
                        }
                        _ => Err(self.type_error(sym, &l, &r)),
                }
        }

        fn as_int(&self, n: f64) -> ExecResult<i64> {
                if n.is_finite() && n.fract() == 0.0 {
                        Ok(n as i64)
                } else {
                        Err(self.error(format!("expected an integer, got {n}")))
                }
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
                        Value::Function(_) => "<function>".to_string(),
                }
        }
}
