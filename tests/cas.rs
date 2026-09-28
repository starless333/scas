use scas::{execute, ExecutionError};

fn output(source: &str) -> String {
    let mut bytes = Vec::new();
    execute(source, &mut bytes).expect("program should succeed");
    String::from_utf8(bytes).unwrap()
}

fn runtime_error(source: &str) -> String {
    match execute(source, Vec::new()).expect_err("program should fail") {
        ExecutionError::Runtime(message) => message,
        error => panic!("expected runtime error, got {error:?}"),
    }
}

#[test]
fn canonical_arithmetic_collects_terms_powers_and_factors() {
    assert_eq!(
        output(
            r#"
            print(y + x + x);
            print(z * x * y);
            print(1/2*x + 1/3*x);
            print(0.5x + 0.25x);
            print(x^2 * y^2);
            print(x^5 / x^2);
            print((x*y)/(x*z));
            print(x / -2);
            "#,
        ),
        "2 * x + y\nx * y * z\n5/6 * x\n3/4 * x\n(x * y) ^ 2\nx ^ 3\ny / z\n-x / 2\n",
    );
}

#[test]
fn fractions_are_combined_reduced_and_checked() {
    assert_eq!(
        output(
            r#"
            print(a/b + c/d);
            print((x^2 - 1)/(x - 1));
            print((x^3 - x)/(x^2 - 1));
            print(apart(1/(x^2 - 1)));
            print(apart((3x + 5)/(x^2 + 3x + 2)));
            "#,
        ),
        concat!(
            "(a * d + b * c) / (b * d)\n",
            "x + 1\n",
            "x\n",
            "-(1/2 / (x + 1)) + 1/2 / (x - 1)\n",
            "1 / (x + 2) + 2 / (x + 1)\n",
        ),
    );
    assert_eq!(runtime_error("print(x/(x-x));"), "division by zero in '/'");
}

#[test]
fn expansion_and_factoring_cover_polynomial_identities() {
    assert_eq!(
        output(
            r#"
            print(expand(a*(b+c)));
            print(expand((a+b)*(c+d)));
            print(expand((x+y)^2));
            print(expand((x-y)^2));
            print(factor(x^2-y^2));
            print(factor(x^3-y^3));
            print(factor(x^3+y^3));
            print(factor(6x^2+9x));
            print(factor(x^2+2x+1));
            print(factor((x^2+1)^2));
            print(factor(x^3-6x^2+11x-6));
            print(horner(2x^3-6x^2+2x-1));
            "#,
        ),
        concat!(
            "a * b + a * c\n",
            "a * c + a * d + b * c + b * d\n",
            "x ^ 2 + 2 * x * y + y ^ 2\n",
            "x ^ 2 - 2 * x * y + y ^ 2\n",
            "(x - y) * (x + y)\n",
            "(x - y) * (x ^ 2 + x * y + y ^ 2)\n",
            "(x + y) * (x ^ 2 - x * y + y ^ 2)\n",
            "3 * x * (2 * x + 3)\n",
            "(x + 1) ^ 2\n",
            "(x ^ 2 + 1) ^ 2\n",
            "(x - 1) * (x - 2) * (x - 3)\n",
            "((2 * x - 6) * x + 2) * x - 1\n",
        ),
    );
}

#[test]
fn standard_math_library_has_numeric_and_symbolic_behavior() {
    assert_eq!(
        output(
            r#"
            print(pi);
            print(e);
            print(tau);
            print(abs(-3/2));
            print(sqrt(4));
            print(sqrt(9/16));
            print(sqrt(1000000000000000000000000000000));
            print(cbrt(27));
            print(exp(0));
            print(ln(e));
            print(log(8, 2));
            print(log10(100));
            print(log2(8));
            print(sin(0));
            print(cos(0));
            print(tan(0));
            print(sin(pi));
            print(cos(pi));
            print(asin(0));
            print(acos(1));
            print(atan2(0, 1));
            print(sinh(0));
            print(cosh(0));
            print(tanh(0));
            print(acosh(1));
            print(floor(1.9));
            print(ceil(1.1));
            print(round(1.6));
            print(trunc(-1.9));
            print(min(2, 3/2));
            print(max(2, 3/2));
            print(hypot(3, 4));
            print(gcd(84, 30));
            print(lcm(12, 18));
            print(gamma(5));
            print(erf(0));
            print(sec(0));
            print(sign(-9));
            print(degrees(pi));
            print(radians(180));
            print(sqrt(x^2));
            print(sin(-x));
            print(cos(-x));
            "#,
        ),
        concat!(
            "3.141592653589793\n2.718281828459045\n6.283185307179586\n",
            "3/2\n2\n3/4\n1000000000000000\n3\n1\n1\n3\n2\n3\n",
            "0\n1\n0\n0\n-1\n0\n0\n0\n0\n1\n0\n0\n",
            "1\n2\n2\n-1\n3/2\n2\n5\n6\n36\n24\n0\n1\n-1\n",
            "180\n3.141592653589793\nabs(x)\n-sin(x)\ncos(x)\n",
        ),
    );
}

#[test]
fn standard_math_library_reports_domains_and_arity() {
    assert_eq!(
        runtime_error("print(sqrt(-1));"),
        "sqrt argument -1 is outside the real-number domain",
    );
    assert_eq!(
        runtime_error("print(log(1, 1));"),
        "log domain requires value > 0, base > 0, and base != 1",
    );
    assert_eq!(
        runtime_error("print(asin(2));"),
        "asin argument 2 is outside the real-number domain",
    );
    assert_eq!(
        runtime_error("print(tan(pi/2));"),
        "tan argument 1.5707963267948965 is outside the real-number domain",
    );
    assert_eq!(
        runtime_error("print(sin());"),
        "sin expects 1 argument(s) but got 0",
    );
}
