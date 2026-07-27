mod error_handling;
mod literal;
mod token;
mod token_type;
mod scanner;
use scanner::Scanner;

fn main() {
        let source = r#"
                let x = 10.5e-3
                let name = "hello world"
                fn add(a, b) { return a + b }
                if x >= 5 { print("big") } else { print("small") }
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
