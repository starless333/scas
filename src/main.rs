mod repl;
use scas::{execute_with_options, ExecutionOptions};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::num::NonZeroU64;
use std::process::ExitCode;

const HELP: &str = "scas 0.1.0
Symbolic computer algebra system

USAGE:
    scas [OPTIONS] [FILE]
    scas build FILE [OUTPUT.tex]

OPTIONS:
    -h, --help                 Show this help
    -V, --version              Show the version
    --precision N              Decimal significant digits
    --max-transcendental-precision N
                               Transcendental-function digit budget
    --max-steps N              Evaluation step budget
    --max-call-depth N         Function recursion limit
    --max-exponent N           Exponent magnitude limit
    --max-shift N              Bit-shift limit
    --max-nesting N            Parser delimiter nesting limit

With no FILE, SCAS starts the REPL on a terminal or reads a program from stdin.";

fn parse_number<T: std::str::FromStr>(flag: &str, value: Option<String>) -> Result<T, String> {
    value
        .ok_or_else(|| format!("{flag} requires a value"))?
        .parse()
        .map_err(|_| format!("invalid value for {flag}"))
}

fn main() -> ExitCode {
    let raw_args = env::args().skip(1).collect::<Vec<_>>();
    if raw_args.first().is_some_and(|argument| argument == "build") {
        return build_document(&raw_args[1..]);
    }
    let mut options = ExecutionOptions::default();
    let mut source_path = None;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        let result =
            match argument.as_str() {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return ExitCode::SUCCESS;
                }
                "-V" | "--version" => {
                    println!("scas {}", env!("CARGO_PKG_VERSION"));
                    return ExitCode::SUCCESS;
                }
                "--precision" => parse_number::<u64>(&argument, args.next()).and_then(|value| {
                    NonZeroU64::new(value)
                        .map(|value| options.decimal_precision = value)
                        .ok_or_else(|| "--precision must be greater than zero".to_string())
                }),
                "--max-steps" => parse_number(&argument, args.next())
                    .map(|value| options.limits.max_steps = value),
                "--max-transcendental-precision" => parse_number(&argument, args.next())
                    .map(|value| options.limits.max_transcendental_precision = value),
                "--max-call-depth" => parse_number(&argument, args.next())
                    .map(|value| options.limits.max_call_depth = value),
                "--max-exponent" => parse_number(&argument, args.next())
                    .map(|value| options.limits.max_exponent = value),
                "--max-shift" => parse_number(&argument, args.next())
                    .map(|value| options.limits.max_shift = value),
                "--max-nesting" => parse_number(&argument, args.next())
                    .map(|value| options.limits.max_nesting = value),
                _ if argument.starts_with('-') => Err(format!("unknown option `{argument}`")),
                _ if source_path.is_none() => {
                    source_path = Some(argument);
                    Ok(())
                }
                _ => Err("only one source file may be supplied".to_string()),
            };
        if let Err(message) = result {
            eprintln!("error: {message}\n\nTry `scas --help` for usage.");
            return ExitCode::FAILURE;
        }
    }

    if source_path.is_none() && io::stdin().is_terminal() {
        if let Err(error) = repl::run() {
            eprintln!("REPL error: {error}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    let (input, source_name) = if let Some(path) = source_path {
        match fs::read_to_string(&path) {
            Ok(input) => (input, path),
            Err(error) => {
                eprintln!("error: failed to read `{path}`: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let mut input = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut input) {
            eprintln!("error: failed to read standard input: {error}");
            return ExitCode::FAILURE;
        }
        (input, "<stdin>".to_string())
    };
    let stdout = io::stdout();
    if let Err(error) = execute_with_options(&input, stdout.lock(), options) {
        eprintln!("{}", error.render(&input, &source_name));
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn build_document(arguments: &[String]) -> ExitCode {
    if arguments.is_empty() || arguments.len() > 2 {
        eprintln!("error: usage: scas build FILE [OUTPUT.tex]");
        return ExitCode::FAILURE;
    }
    let input_path = &arguments[0];
    let source = match fs::read_to_string(input_path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("error: failed to read `{input_path}`: {error}");
            return ExitCode::FAILURE;
        }
    };
    let document = match scas::document::build(&source) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("{}", error.render(&source, input_path));
            return ExitCode::FAILURE;
        }
    };
    let output_path = arguments.get(1).cloned().unwrap_or_else(|| {
        let mut path = std::path::PathBuf::from(input_path);
        path.set_extension("tex");
        path.to_string_lossy().into_owned()
    });
    if let Err(error) = fs::write(&output_path, document.latex()) {
        eprintln!("error: failed to write `{output_path}`: {error}");
        return ExitCode::FAILURE;
    }
    for diagnostic in &document.diagnostics {
        eprintln!("{:?}: {}", diagnostic.severity, diagnostic.message);
    }
    println!("Built {output_path}");
    ExitCode::SUCCESS
}
