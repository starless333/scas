#![no_main]

use libfuzzer_sys::fuzz_target;
use scas::document::interpolations;
use scas::lexer::Scanner;

fuzz_target!(|input: &str| {
    let mut scanner = Scanner::new(input);
    let _ = scanner.scan_tokens();
    let _ = interpolations(input);
});
