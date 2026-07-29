mod error_handling;
pub mod lexer {
        pub mod literal;
        pub mod token;
        pub mod token_type;
        pub mod scanner;
        pub mod symbol;
}
pub mod parser {
        pub mod parser;
}
pub mod ast {
        pub mod expr;
        pub mod statement;
        pub mod assign_op;
        pub mod binary_op;
        pub mod unary_op;
}
pub mod interpreter {
        pub mod environment;
        pub mod flow;
        pub mod interpreter;
        pub mod value;
}
use crate::interpreter::interpreter::Interpreter;
use crate::lexer::scanner::Scanner;
use crate::parser::parser::Parser;
use std::io::{Read};
fn main() {
        // Read all of stdin into a String.
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        // Borrow it as a &str.
        let source: &str = &input;

        let mut sc = Scanner::new(source);
        let tokens = sc.scan_tokens().clone();
        if sc.had_error() {
                eprintln! ("lexing failed, aborting.");
                return;
        }
        // Interpreter needs the interned strings behind each Symbol id to
        // report readable error messages (e.g. "undefined variable 'x'"),
        // so it takes ownership of the table the scanner built. `tokens`
        // is cloned out first since `into_symbols` consumes `sc`.
        let symbols = sc.into_symbols ();

        let mut parser = Parser::new (tokens);
        let ast = parser.parse ();

        // for st in &ast {
        //         println! ("{:?}", st);
        //         println! ();
        // }

        let mut interpreter = Interpreter::new (symbols);
        if let Err (e) = interpreter.run (&ast) {
                eprintln! ("runtime error: {e}");
        }
}
