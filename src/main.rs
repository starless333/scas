mod repl;
use scas::execute;
use std::io::{self, IsTerminal, Read};
use std::process::ExitCode;

fn main() -> ExitCode {
    if io::stdin().is_terminal() {
        if let Err(error) = repl::run() {
            eprintln!("REPL error: {error}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("error: failed to read standard input: {error}");
        return ExitCode::FAILURE;
    }
    let stdout = io::stdout();
    if let Err(error) = execute(&input, stdout.lock()) {
        eprintln!("{}", error.render(&input, "<stdin>"));
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
