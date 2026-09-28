use scas::interpreter::Interpreter;
use scas::lexer::Scanner;
use scas::parser::Parser;
use scas::{execute, ExecutionError};
use std::num::NonZeroU64;

fn output(source: &str) -> String {
    let mut bytes = Vec::new();
    execute(source, &mut bytes).expect("program should execute successfully");
    String::from_utf8(bytes).expect("language output should be UTF-8")
}

fn runtime_error(source: &str) -> String {
    match execute(source, Vec::new()).expect_err("program should fail") {
        ExecutionError::Runtime(message) => message,
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
    interpreter.set_decimal_precision(NonZeroU64::new(precision).unwrap());
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
            "print(2 ^ 1.5);",
            "'^' requires a numeric base and an integer exponent",
        ),
        (
            "print(2 ^ 9223372036854775808);",
            "exponent is too large to evaluate (maximum magnitude is 2^63 - 1)",
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
