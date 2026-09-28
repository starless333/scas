#![no_main]

use libfuzzer_sys::fuzz_target;
use scas::lexer::Scanner;
use scas::parser::Parser;

fuzz_target!(|input: &str| {
    let mut scanner = Scanner::new(input);
    let tokens = scanner.scan_tokens().clone();
    if !scanner.had_error() {
        let _ = Parser::new(tokens.clone()).parse();
        let _ = Parser::new(tokens).parse_recovering();
    }
});
