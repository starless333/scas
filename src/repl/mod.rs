use scas::interpreter::Interpreter;
use scas::lexer::{Scanner, SymbolTable};
use scas::parser::Parser;
use std::fs;
use std::io::{self, BufRead, Write};
use std::num::NonZeroU64;

const HELP: &str = "Commands:\n  :help              show this help\n  :precision N       set decimal division precision\n  :load PATH         execute a source file\n  :definitions       list current names\n  :history           show session inputs\n  :reset             clear session definitions\n  :quit, :exit       leave the REPL\n  :clear             clear the screen\n  :plot EXPR         draw a graph; see :plot with no arguments\n\nTry:\n  f(x) = x^3 - 3x + 1\n  derivative(f(x), x)\n  solve(x^2 - 5x + 6, x)\n  factor(x^3 - y^3)\n  let A = [1, 2; 3, 4];\n  A[1, 0]";

/// Width (columns) and height (rows) of the terminal plot drawing. Sized for
/// an 80×24 terminal; the drawing plus its labels fits inside 24 rows.
#[allow(dead_code)]
const PLOT_ASCII_WIDTH: usize = 64;
#[allow(dead_code)]
const PLOT_ASCII_HEIGHT: usize = 17;

const PLOT_HELP: &str = "usage: :plot EXPRESSION[, EXPRESSION…] [from A to B] [> FILE]\n\n\
  The independent variable is x. Multiple expressions draw together in\n\
  distinct characters. With no range the window is -10 to 10.\n\
  Without a redirect the graph is drawn in the terminal; a file ending\n\
  in .svg receives a vector image, and any other file receives the\n\
  same drawing shown here.\n\n\
examples:\n\
  :plot sin(x)\n\
  :plot x^2 - 2x from -5 to 5\n\
  :plot sin(x), cos(x) from -2*pi to 2*pi\n\
  :plot sin(x), cos(x) > graph.svg\n\
  :plot f(t) = 2t + 1 from 0 to 10\n\n\
  Plots use the session's definitions and restore any existing value\n\
  of x (or the plotted parameter) afterwards.";

/// Run an interactive session. Definitions and variables persist between
/// submissions, including closures created by earlier submissions.
pub fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_with_io(stdin.lock(), stdout.lock(), 64, 17)
}

fn run_with_io<R, W>(
    mut input: R,
    mut output: W,
    plot_width: usize,
    plot_height: usize,
) -> io::Result<()>
where
    R: BufRead + Sized,
    W: Write,
{
    let mut symbols = SymbolTable::new();
    let mut interpreter = Interpreter::new(symbols.clone(), Vec::new());
    let mut source = String::new();
    let mut history: Vec<String> = Vec::new();

    writeln!(output, "scas 0.1.0 — type :help for help, :quit to exit")?;
    loop {
        write!(output, "{}", if source.is_empty() { "> " } else { "... " })?;
        output.flush()?;

        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            break;
        }

        if source.is_empty() {
            let command = line.trim().to_owned();
            if let Some(path) = command.strip_prefix(":load ") {
                match fs::read_to_string(path.trim()) {
                    Ok(loaded) => line = loaded,
                    Err(error) => {
                        writeln!(output, "error: failed to load `{}`: {error}", path.trim())?;
                        continue;
                    }
                }
            }
            if let Some(rest) = command.strip_prefix(":plot") {
                if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                    writeln!(output, "unknown command `{command}`; type :help for help")?;
                    continue;
                }
                if rest.trim().is_empty() {
                    writeln!(output, "{PLOT_HELP}")?;
                    continue;
                }
                let rest = rest.trim().to_owned();
                match scas::plot::execute_plot_command(
                    &mut interpreter,
                    &mut symbols,
                    &rest,
                    plot_width,
                    plot_height,
                    &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                ) {
                    Ok(outcome) => {
                        if let Some(path) = &outcome.wrote_file {
                            writeln!(output, "wrote {}", path.display())?;
                        } else {
                            writeln!(output, "{}", outcome.ascii)?;
                        }
                    }
                    Err(message) => writeln!(output, "error: {message}")?,
                }
                continue;
            }
            if let Some(argument) = command.strip_prefix(":precision") {
                match argument
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .and_then(NonZeroU64::new)
                {
                    Some(precision) => match interpreter.set_decimal_precision(precision) {
                        Ok(()) => writeln!(
                            output,
                            "decimal precision: {} digits",
                            interpreter.decimal_precision()
                        )?,
                        Err(error) => writeln!(output, "limit error: {error}")?,
                    },
                    None => writeln!(output, "usage: :precision N (N must be greater than zero)")?,
                }
                continue;
            }
            match command.as_str() {
                ":quit" | ":exit" | "quit" | "exit" => break,
                ":help" | "help" => {
                    writeln!(output, "{HELP}")?;
                    continue;
                }
                ":clear" | "clear" => {
                    write!(output, "\x1b[2J\x1b[H")?;
                    output.flush()?;
                    continue;
                }
                ":definitions" => {
                    writeln!(output, "{}", interpreter.definition_names().join("\n"))?;
                    continue;
                }
                ":history" => {
                    for (index, entry) in history.iter().enumerate() {
                        writeln!(output, "{}  {}", index + 1, entry.trim_end())?;
                    }
                    continue;
                }
                ":reset" => {
                    symbols = SymbolTable::new();
                    interpreter = Interpreter::new(symbols.clone(), Vec::new());
                    writeln!(output, "session reset")?;
                    continue;
                }
                "" => continue,
                _ => {}
            }
        }

        source.push_str(&line);
        if is_incomplete(&source) {
            continue;
        }

        let submission = terminate_if_needed(&source);
        history.push(submission.clone());
        let mut scanner = Scanner::with_symbols(&submission, symbols);
        let tokens = scanner.scan_tokens().clone();
        if scanner.had_error() {
            for error in scanner.errors() {
                writeln!(output, "{}", error.render(&submission, "<repl>"))?;
            }
            symbols = scanner.into_symbols();
            interpreter.set_symbols(symbols.clone());
            source.clear();
            continue;
        }
        symbols = scanner.into_symbols();
        interpreter.set_symbols(symbols.clone());

        match Parser::new(tokens).parse() {
            Ok(program) => {
                let result = interpreter.run_interactive(&program);
                output.write_all(&std::mem::take(interpreter.output_mut()))?;
                match result {
                    Ok(Some(value)) => writeln!(output, "{value}")?,
                    Ok(None) => {}
                    Err(error) => writeln!(output, "runtime error: {error}")?,
                }
            }
            Err(error) => writeln!(output, "{}", error.render(&submission, "<repl>"))?,
        }
        source.clear();
    }
    Ok(())
}

fn terminate_if_needed(source: &str) -> String {
    let trimmed = source.trim_end();
    if trimmed.ends_with(';') || trimmed.ends_with('}') {
        source.to_owned()
    } else {
        format!("{source};")
    }
}

/// Determines whether the REPL should show a continuation prompt.
fn is_incomplete(source: &str) -> bool {
    let mut delimiters = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c == '"' {
            in_string = true;
        } else if c == '/' && chars.peek() == Some(&'/') {
            while chars.next().is_some_and(|next| next != '\n') {}
        } else if matches!(c, '(' | '{' | '[') {
            delimiters.push(c);
        } else if matches!(c, ')' | '}' | ']') {
            delimiters.pop();
        }
    }
    in_string || !delimiters.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn repl_keeps_state_and_runs_block_functions() {
        let input = Cursor::new("let offset = 2;\nfn add(x) { let y = x + offset; return y; }\nadd(5)\nfn factorial(n) { if n <= 1 { return 1; } return n * factorial(n - 1); }\nfactorial(6)\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains('7'));
        assert!(output.contains("720"));
    }

    #[test]
    fn repl_recovers_after_a_parse_error() {
        let input = Cursor::new("let = 3;\n2 + 3\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("error: expected a variable name"));
        assert!(output.contains("--> <repl>:1:5"));
        assert!(output.contains("let = 3;"));
        assert!(output.contains("^ unexpected `=`"));
        assert!(output.contains('5'));
    }

    #[test]
    fn integers_and_decimals_do_not_lose_precision() {
        let input = Cursor::new("999999999999999999999999999999 + 1\n0.1 + 0.2\n1.000000000000000000000000000001 ^ 2\n1e+30 + 1\n1 / 7\n:precision 12\n1.0 / 7\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("1000000000000000000000000000000"));
        assert!(output.contains("0.3"));
        assert!(output.contains("1.000000000000000000000000000002000000000000000000000000000001"));
        assert!(output.contains("1000000000000000000000000000001"));
        assert!(output.contains("1/7"));
        assert!(output.contains("0.142857142857"));
    }

    #[test]
    fn rational_arithmetic_and_arrays_are_first_class_values() {
        let input = Cursor::new("1 / 2 + 1 / 3\n2 ^ -3\n1 / 2 == 0.5\nlet values = [1, 1 / 2, [3, 4],];\nvalues[1]\nvalues[2][0]\nvalues + [5, 6]\nfn second(xs) = xs[1];\nsecond([10, 20])\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("5/6"));
        assert!(output.contains("1/8"));
        assert!(output.contains("true"));
        assert!(output.contains("[1, 1/2, [3, 4], 5, 6]"));
        assert!(output.contains("20"));
    }

    #[test]
    fn mathematical_functions_support_implicit_multiplication_and_symbolic_arguments() {
        let input = Cursor::new("f(x, y) = 2x + 3y\nf\nf(x ^ 2, y)\nf(2, y)\nf(2, 5)\n2(x + 1)\nh(x, y, z) = x^2 + y^2 + z^2\ng(x, y) = h(x^2, y, 3)\ng\nx + x\n2x + 3x\nx - x\nx * x\nx^2 * x^3\n(x^2)^3\nx / x\n-(-x)\n0x + y\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("f(x, y) = 2 * x + 3 * y"));
        assert!(output.contains("2 * x ^ 2 + 3 * y"));
        assert!(output.contains("3 * y + 4"));
        assert!(output.contains("19"));
        assert!(output.contains("2 * (x + 1)"));
        assert!(output.contains("g(x, y) = x ^ 4 + y ^ 2 + 9"));
        assert!(output.contains("2 * x"));
        assert!(output.contains("5 * x"));
        assert!(output.contains("x ^ 5"));
        assert!(output.contains("x ^ 6"));
    }

    #[test]
    fn standard_math_library_is_available_across_repl_submissions() {
        // Each line is scanned independently, which grows the REPL symbol table
        // after the interpreter has already been created.
        let input = Cursor::new(
            "sqrt(4)\nsin(0)\ncos(0)\nln(e)\nlog(8, 2)\ngamma(5)\nerfc(0)\npi\n:quit\n",
        );
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("runtime error"), "{output}");
        assert!(output.contains("> 2\n> 0\n> 1\n> 1\n> 3\n> 24\n> 1\n> pi\n> "));
    }

    #[test]
    fn symbolic_algebra_functions_are_available_in_the_repl() {
        let input = Cursor::new(
            "sqrt(x^2)\nsin(-x)\nexpand((x+y)^2)\nfactor(x^2-y^2)\napart(1/(x^2-1))\n:quit\n",
        );
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("runtime error"), "{output}");
        assert!(output.contains("sqrt(x ^ 2)"));
        assert!(output.contains("-sin(x)"));
        assert!(output.contains("x ^ 2 + 2 * x * y + y ^ 2"));
        assert!(output.contains("(x - y) * (x + y)"));
        assert!(output.contains("-(1/2 / (x + 1)) + 1/2 / (x - 1)"));
    }

    #[test]
    fn repl_draws_ascii_plots_from_session_definitions() {
        let input = Cursor::new(
            "f(x) = x^2 - 2\n:plot f(x) from -3 to 3\n:plot sin(x)\n:plot nope(x)\n:quit\n",
        );
        let mut output = Vec::new();
        run_with_io(input, &mut output, 40, 10).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.contains('*'), "{output}");
        assert!(output.contains('-') && output.contains('|'), "{output}");
        assert!(output.contains('3'), "{output}");
        // The unbound-function case reports a clean error.
        assert!(output.contains("error:"), "{output}");
        assert!(output.contains("nope"), "{output}");
    }

    #[test]
    fn repl_plot_redirect_writes_svg_and_text_files() {
        let directory = std::env::temp_dir().join(format!(
            "scas-repl-plot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let absolute = directory.join("graph.svg");
        let text_target = directory.join("graph.txt");
        let input = Cursor::new(format!(
            ":plot sin(x), cos(x) from -6.5 to 6.5 > {}\n:plot x^2 > {}\n:quit\n",
            absolute.display(),
            text_target.display()
        ));
        let mut output = Vec::new();
        run_with_io(input, &mut output, 40, 10).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("error:"), "{output}");
        assert!(
            output.contains(&format!("wrote {}", absolute.display())),
            "{output}"
        );
        let svg = std::fs::read_to_string(&absolute).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("sin(x)"));
        assert!(svg.contains("cos(x)"));
        let text = std::fs::read_to_string(&text_target).unwrap();
        assert!(text.contains('*'));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn repl_plot_restores_bound_variables() {
        let input = Cursor::new("let x = 41;\n:plot x^2 from 1 to 3\nx\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 40, 10).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("runtime error"), "{output}");
        assert!(output.contains("41"), "{output}");
    }

    #[test]
    fn refreshing_repl_symbols_does_not_replace_user_shadowing() {
        let input = Cursor::new("let sin = 7;\nsin\ncos(0)\nsin\n:quit\n");
        let mut output = Vec::new();
        run_with_io(input, &mut output, 64, 17).unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(!output.contains("runtime error"), "{output}");
        assert!(output.contains("> > 7\n> 1\n> 7\n> "));
    }
}
