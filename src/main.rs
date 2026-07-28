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

use crate::lexer::scanner::Scanner;
use crate::parser::parser::Parser;
use std::io::{self, Read};

fn main() {
        // Read all of stdin into a String.
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();

        // Borrow it as a &str.
        let source: &str = &input;
        
        let mut sc = Scanner::new(source);
        let tokens = sc.scan_tokens();

        let mut parser = Parser::new (tokens.clone ());
        let ast = parser.parse ();
        
        // for t in tokens {
        //         println!("{:?}", t);
        // }

        for st in ast {
                println! ("{:?}", st);
                println! ();
        }
        
}
