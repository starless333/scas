use std::collections::HashMap;

pub type Symbol = u32;

#[derive(Debug, Default)]
pub struct SymbolTable {
        ids: HashMap<String, Symbol>,
        names: Vec<String>,
}

impl SymbolTable {
        pub fn new() -> Self {
                Self::default()
        }

        pub fn intern(&mut self, name: &str) -> Symbol {
                if let Some(&sym) = self.ids.get(name) {
                        return sym;
                }

                let sym = self.names.len() as Symbol;

                self.names.push(name.to_owned());
                self.ids.insert(name.to_owned(), sym);

                sym
        }

        pub fn resolve(&self, sym: Symbol) -> &str {
                &self.names[sym as usize]
        }

        pub fn len(&self) -> usize {
                self.names.len()
        }

        pub fn is_empty(&self) -> bool {
                self.names.is_empty()
        }
}
