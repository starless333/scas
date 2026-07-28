mod error_handling;

pub mod lexer {
        pub mod literal;
        pub mod token;
        pub mod token_type;
        pub mod scanner;
        pub mod symbol;
}

use crate::lexer::scanner::Scanner;

fn main() {
        let source = r#"
                let x = 10.5e-3
                let name = "hello world"
                fn add(a, b) { return a + b }
                if x >= 5 { print("big") } else { print("small") }
                let y = 0;
                // a comment
                0xFF 0b101 0o17 0
                alpha_beta
        "#;

        let mut sc = Scanner::new(source);
        let tokens = sc.scan_tokens();

        for t in tokens {
                println!("{:?}", t);
        }

        if sc.had_error() {
                eprintln!("scanning finished with errors");
        }
}
