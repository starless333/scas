use crate::interpreter::value::Value;
use crate::lexer::symbol::Symbol;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Shared, mutable handle to a scope. `Rc<RefCell<..>>` because closures
/// need to capture their defining scope by reference and keep it alive
/// past the point where the block/function that created it returns.
pub type Env = Rc<RefCell<Environment>>;

/// A single lexical scope: its own bindings, plus a link to the enclosing
/// scope for names it doesn't define itself.
///
/// Keyed by `Symbol` (the interned `u32` id from the lexer) rather than
/// `String` -- variable lookup is one of the hottest paths in a
/// tree-walking interpreter (every variable reference and every
/// assignment goes through it), and hashing/comparing a `u32` is
/// meaningfully cheaper than hashing/comparing a `String` on every single
/// lookup, with no heap traffic at all.
#[derive(Debug, Default)]
pub struct Environment {
    values: HashMap<Symbol, Value>,
    parent: Option<Env>,
}

impl Environment {
    pub fn new() -> Env {
        Rc::new(RefCell::new(Environment {
            values: HashMap::new(),
            parent: None,
        }))
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(RefCell::new(Environment {
            values: HashMap::new(),
            parent: Some(Rc::clone(parent)),
        }))
    }

    pub fn define(&mut self, name: Symbol, value: Value) {
        self.values.insert(name, value);
    }

    /// Walk outward through enclosing scopes looking for `name`.
    pub fn get(env: &Env, name: Symbol) -> Option<Value> {
        let scope = env.borrow();
        if let Some(v) = scope.values.get(&name) {
            return Some(v.clone());
        }
        match &scope.parent {
            Some(parent) => Environment::get(parent, name),
            None => None,
        }
    }

    /// Assigns to the *nearest* existing binding of `name`, walking
    /// outward through enclosing scopes. Returns `false` (and assigns
    /// nothing) if `name` isn't defined anywhere in the chain --
    /// assignment, unlike `let`, must not silently create a new
    /// global.
    pub fn assign(env: &Env, name: Symbol, value: Value) -> bool {
        let mut scope = env.borrow_mut();
        if let std::collections::hash_map::Entry::Occupied(mut entry) = scope.values.entry(name) {
            entry.insert(value);
            return true;
        }
        let parent = scope.parent.clone();
        drop(scope); // release this scope's borrow before recursing
        match parent {
            Some(parent) => Environment::assign(&parent, name, value),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::Value;
    use crate::lexer::SymbolTable;

    #[test]
    fn lookup_and_assignment_follow_lexical_scope() {
        let mut symbols = SymbolTable::new();
        let outer_name = symbols.intern("outer");
        let local_name = symbols.intern("local");
        let root = Environment::new();
        root.borrow_mut().define(outer_name, Value::Bool(false));
        let child = Environment::child(&root);
        child.borrow_mut().define(local_name, Value::Nil);

        assert!(matches!(
            Environment::get(&child, outer_name),
            Some(Value::Bool(false))
        ));
        assert!(Environment::assign(&child, outer_name, Value::Bool(true)));
        assert!(matches!(
            Environment::get(&root, outer_name),
            Some(Value::Bool(true))
        ));
        assert!(!Environment::assign(
            &child,
            symbols.intern("missing"),
            Value::Nil
        ));
    }
}
