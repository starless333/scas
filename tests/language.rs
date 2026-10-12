use scas::interpreter::Interpreter;
use scas::lexer::Scanner;
use scas::parser::Parser;
use scas::runtime_io::{MemoryIo, RuntimeIo};
use scas::{execute, execute_with_limits, ExecutionError, Limits};
use std::num::NonZeroU64;
use std::rc::Rc;

#[test]
fn imports_share_scope_and_virtual_io_supports_text_and_input() {
    let io = Rc::new(MemoryIo::new());
    io.insert_file("math.scas", "double(x) = 2 * x;".to_owned())
        .unwrap();
    io.queue_input("Ada".to_owned());
    io.set_args(vec!["first".to_owned()]);
    let source = "import \"math.scas\"; import \"math.scas\"; write_text(\"out.txt\", format(\"{} {}\", input(\"name: \"), double(21))); print(read_text(\"out.txt\")); print(args()[0]);";
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    assert!(scanner.errors().is_empty());
    let program = Parser::new(tokens).parse().unwrap();
    let mut interpreter = Interpreter::with_limits_and_io(
        scanner.into_symbols(),
        Vec::new(),
        Limits::default(),
        io.clone(),
    );
    interpreter.run(&program).unwrap();
    let result = String::from_utf8(interpreter.output_mut().clone()).unwrap();
    assert!(result.contains("Ada 42"), "{result}");
    assert!(result.contains("first"), "{result}");
    assert_eq!(io.read_text("out.txt").unwrap(), "Ada 42");
}

#[test]
fn numeric_casts_convert_input_strings_and_numeric_values() {
    let io = Rc::new(MemoryIo::new());
    for value in ["42", "2.5", "7/9", "-3.8"] {
        io.queue_input(value.to_owned());
    }
    let source = r#"
        print(int(input("integer: ")));
        let decimal = double(input("decimal: "));
        print(decimal);
        print(type_of(decimal));
        print(rational(input("rational: ")));
        print(int(input("truncate: ")));
        print(rational(0.125));
        print(double(1/4));
        print(int(7/3));
    "#;
    let mut scanner = Scanner::new(source);
    let program = Parser::new(scanner.scan_tokens().clone()).parse().unwrap();
    let mut interpreter =
        Interpreter::with_limits_and_io(scanner.into_symbols(), Vec::new(), Limits::default(), io);
    interpreter.run(&program).unwrap();
    assert_eq!(
        String::from_utf8(interpreter.output_mut().clone()).unwrap(),
        "42\n2.5\ndecimal\n7/9\n-3\n1/8\n0.25\n2\n"
    );

    assert_eq!(
        runtime_error("print(rational(\"1/0\"));"),
        "rational cannot convert a rational with denominator zero"
    );
}

#[test]
fn bundled_scas_library_imports_in_native_execution() {
    assert_eq!(
        output("import \"stdlib/core.scas\"; print(square(7)); print(lerp(0, 10, 1/4));"),
        "49\n5/2\n"
    );
}

#[test]
fn virtual_csv_and_json_import_as_values() {
    let io = Rc::new(MemoryIo::new());
    io.insert_file("rows.csv", "name,value\n\"Ada, A.\",42\n".to_owned())
        .unwrap();
    io.insert_file(
        "data.json",
        r#"{"ok":true,"items":[1,2.5],"exact":12345678901234567890.12345678901234567890}"#
            .to_owned(),
    )
    .unwrap();
    let source =
        "print(read_csv(\"rows.csv\")[1][0]); print(read_json(\"data.json\")[\"items\"][1]); print(read_json(\"data.json\")[\"exact\"]);";
    let mut scanner = Scanner::new(source);
    let program = Parser::new(scanner.scan_tokens().clone()).parse().unwrap();
    let mut interpreter =
        Interpreter::with_limits_and_io(scanner.into_symbols(), Vec::new(), Limits::default(), io);
    interpreter.run(&program).unwrap();
    assert_eq!(
        String::from_utf8(interpreter.output_mut().clone()).unwrap(),
        "Ada, A.\n2.5\n12345678901234567890.1234567890123456789\n"
    );
}

fn output(source: &str) -> String {
    let mut bytes = Vec::new();
    execute(source, &mut bytes).expect("program should execute successfully");
    String::from_utf8(bytes).expect("language output should be UTF-8")
}

fn runtime_error(source: &str) -> String {
    match execute(source, Vec::new()).expect_err("program should fail") {
        ExecutionError::Runtime(error) => error.message,
        other => panic!("expected a runtime error, got {other:?}"),
    }
}

fn output_with_precision(source: &str, precision: u64) -> String {
    let mut scanner = Scanner::new(source);
    let tokens = scanner.scan_tokens().clone();
    assert!(scanner.errors().is_empty());
    let symbols = scanner.into_symbols();
    let program = Parser::new(tokens).parse().expect("program should parse");
    let mut interpreter = Interpreter::new(symbols, Vec::new());
    interpreter
        .set_decimal_precision(NonZeroU64::new(precision).unwrap())
        .unwrap();
    interpreter.run(&program).expect("program should execute");
    String::from_utf8(std::mem::take(interpreter.output_mut())).unwrap()
}

#[test]
fn arithmetic_precedence_associativity_and_implicit_multiplication() {
    assert_eq!(
        output(
            r#"
            print(1 + 2 * 3 ^ 2);
            print(20 / 5 / 2);
            print(2 ^ 3 ^ 2);
            print(-2 ^ 2);
            print(17 % 5);
            print(2(3 + 4));
            print("SC" + "AS");
            print([1, 2] + [3]);
            "#,
        ),
        "19\n2\n512\n-4\n2\n14\nSCAS\n[1, 2, 3]\n",
    );
}

#[test]
fn comparison_logical_and_bitwise_operators() {
    assert_eq!(
        output(
            r#"
            print(2 == 2.0);
            print(2 != 3);
            print(2 < 3);
            print(2 <= 2);
            print(3 > 2);
            print(3 >= 3);
            print(!false);
            print(false and (1 / 0));
            print(true or (1 / 0));
            print(6 & 3);
            print(4 | 1);
            print(6 xor 3);
            print(3 << 2);
            print(16 >> 2);
            print(~1);
            "#,
        ),
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\n2\n5\n5\n12\n4\n-2\n",
    );
}

#[test]
fn every_assignment_operator_updates_the_existing_binding() {
    assert_eq!(
        output(
            r#"
            let value = 10;
            value = 12;
            value += 3;
            value -= 5;
            value *= 4;
            value /= 8;
            value %= 3;
            value ^= 3;
            value &= 6;
            value |= 8;
            print(value);
            "#,
        ),
        "8\n",
    );
}

#[test]
fn control_flow_closures_scopes_arrays_and_function_arity_work() {
    assert_eq!(
        output(
            r#"
            fn make_adder(offset) {
                fn add(value) = value + offset;
                return add;
            }
            let add_four = make_adder(4);
            print(add_four(3));

            fn announce() { print(99); }
            announce();

            let scoped = 1;
            { let scoped = 2; print(scoped); }
            print(scoped);

            let total = 0;
            for let i = 0; i < 8; i += 1 {
                if i == 2 { continue; }
                if i == 6 { break; }
                total += i;
            }
            while total < 20 { total += 1; }
            print(total);

            let values = [10, [20, 30]];
            print(values[0]);
            print(values[1][1]);
            "#,
        ),
        "7\n99\n2\n1\n20\n10\n30\n",
    );

    assert_eq!(
        runtime_error("fn one(value) = value; one();"),
        "expected 1 argument(s) but got 0",
    );
    assert_eq!(
        runtime_error("fn one(value) = value; one(1, 2);"),
        "expected 1 argument(s) but got 2",
    );
}

#[test]
fn documenter_prerequisite_grammar_is_executable() {
    assert_eq!(
        output(
            r#"
            print(3x^2);
            let branch = 0;
            if false { branch = 1; } else if true { branch = 2; } else { branch = 3; }
            print(branch);
            let A = [1, 2; 3, 4];
            print(A[1, 0]);
            let x_1 = 7;
            print(x_1);
            "#,
        ),
        "3 * x ^ 2\n2\n3\n7\n",
    );
}

#[test]
fn documenter_markers_parse_and_execute_with_shared_scope() {
    assert_eq!(
        output(
            r#"
            /// A documented calculation with #{ a } interpolation text.
            //// This remains an ordinary comment.
            \{
                let a = 2;
                a + 1 =>;
            \}
            \[ a^2 => \]
            print(a + 1);
            "#,
        ),
        "3\n",
    );
}

#[test]
fn runtime_failures_have_stable_messages() {
    let cases = [
        ("print(1 / 0);", "division by zero in '/'"),
        ("print(1 % 0);", "division by zero in '%'"),
        (
            "print(1 << -1);",
            "shift amount must be a non-negative machine-sized integer",
        ),
        (
            "print(1 << 999999999999999999999999999999);",
            "shift amount must be a non-negative machine-sized integer",
        ),
        (
            "print(2 ^ 9223372036854775808);",
            "exponent magnitude exceeds the limit of 100000",
        ),
        (
            "print(0 ^ -1);",
            "zero cannot be raised to a negative power",
        ),
        (
            "print([1, 2][2]);",
            "array index 2 is out of bounds for length 2",
        ),
        ("print([1][1.0]);", "array index must be an integer"),
        ("print(1[0]);", "cannot index a integer"),
        (
            "print(\"a\" - 1);",
            "cannot apply '-' to string and integer",
        ),
        ("missing = 1;", "undefined variable 'missing'"),
        ("return 1;", "'return' used outside of a function"),
        ("break;", "'break' used outside of a loop"),
        ("continue;", "'continue' used outside of a loop"),
    ];

    for (source, expected) in cases {
        assert_eq!(runtime_error(source), expected, "source: {source}");
    }
}

#[test]
fn numeric_promotion_exactness_and_decimal_precision_are_stable() {
    assert_eq!(
        output(
            r#"
            print(999999999999999999999999999999 + 1);
            print(1 / 2 + 1 / 3);
            print(6 / 3);
            print(1 / 2 == 0.5);
            print(1 + 0.25);
            print(0.1 + 0.2);
            print(1.000000000000000000000000000001 ^ 2);
            "#,
        ),
        concat!(
            "1000000000000000000000000000000\n",
            "5/6\n",
            "2\n",
            "true\n",
            "1.25\n",
            "0.3\n",
            "1.000000000000000000000000000002000000000000000000000000000001\n",
        ),
    );
    assert_eq!(
        output_with_precision("print(1.0 / 7);", 12),
        "0.142857142857\n"
    );
}

#[test]
fn decimal_literals_may_omit_the_leading_zero() {
    assert_eq!(
        output("print(.8); print(.25e2); print(1 + .05);"),
        "0.8\n25\n1.05\n"
    );
}

#[test]
fn resource_limits_stop_runaway_and_expensive_programs() {
    let limits = Limits {
        max_steps: 30,
        max_call_depth: 8,
        max_exponent: 12,
        max_shift: 12,
        max_decimal_precision: 12,
        max_decimal_scale: 12,
        max_number_bits: 8,
        max_matrix_elements: 3,
        ..Limits::default()
    };

    let failures = [
        ("while true { }", "execution exceeded the 30 step limit"),
        (
            "fn recurse() = recurse(); recurse();",
            "function calls exceeded the depth limit of 8",
        ),
        (
            "print(2 ^ 13);",
            "exponent magnitude exceeds the limit of 12",
        ),
        ("print(1 << 13);", "shift amount exceeds the limit of 12"),
        ("print(1e-13);", "decimal scale exceeds the limit of 12"),
        (
            "print(255 * 255);",
            "numeric result exceeds the 8 bit limit",
        ),
        (
            "print((i + j)_(i,j,2));",
            "matrix rule exceeds the 3 element execution limit",
        ),
    ];
    for (source, expected) in failures {
        let error = execute_with_limits(source, Vec::new(), limits).unwrap_err();
        let ExecutionError::Runtime(error) = error else {
            panic!("expected limit error for {source}");
        };
        assert!(error.is_limit, "source: {source}");
        assert_eq!(error.message, expected);
        assert!(error.span.is_some());
    }

    let mut scanner = Scanner::new("print(1);");
    let tokens = scanner.scan_tokens().clone();
    let symbols = scanner.into_symbols();
    let mut interpreter = Interpreter::with_limits(symbols, Vec::new(), limits);
    assert!(
        interpreter
            .set_decimal_precision(NonZeroU64::new(13).unwrap())
            .unwrap_err()
            .is_limit
    );
    interpreter
        .run(&Parser::new(tokens).parse().unwrap())
        .unwrap();
}

#[test]
fn deeply_nested_input_is_rejected_before_recursive_parsing() {
    let limits = Limits {
        max_nesting: 4,
        ..Limits::default()
    };
    let error = execute_with_limits("print(((((1)))));", Vec::new(), limits).unwrap_err();
    let ExecutionError::Runtime(error) = error else {
        panic!("expected a limit error")
    };
    assert!(error.is_limit);
    assert_eq!(error.message, "input exceeds the nesting limit of 4");
}
