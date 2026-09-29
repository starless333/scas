use scas::{execute, ExecutionError};

fn output(source: &str) -> String {
    let mut bytes = Vec::new();
    execute(source, &mut bytes).expect("program should succeed");
    String::from_utf8(bytes).unwrap()
}

fn runtime_error(source: &str) -> String {
    match execute(source, Vec::new()).expect_err("program should fail") {
        ExecutionError::Runtime(error) => error.message,
        error => panic!("expected runtime error, got {error:?}"),
    }
}

#[test]
fn randomized_safe_symbolic_collection_identity_holds() {
    let mut state = 0xcafe_babe_u64;
    let mut source = String::new();
    for _ in 0..250 {
        state = state
            .wrapping_mul(2_862_933_555_777_941_757)
            .wrapping_add(3_037_000_493);
        let a = (state % 51) as i64;
        source.push_str(&format!("print(({a} * x + 0) == {a} * x);\n"));
    }
    assert_eq!(output(&source), "true\n".repeat(250));
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
        concat!(
            "2 * x + y\nx * y * z\n5/6 * x\n3/4 * x\n(x * y) ^ 2\n",
            "x ^ 5 / x ^ 2\nx * y / (x * z)\n-x / 2\n",
        ),
    );
}

#[test]
fn exact_polynomial_invariants_and_euclidean_tools_are_available() {
    assert_eq!(
        output(
            r#"
            print(resultant(x^2 + 1, x + 1, x));
            print(resultant(x^2 - 1, x - 1, x));
            print(discriminant(x^3 - 3x + 2, x));
            print(degree(3x^4 - 1, x));
            print(coefficients(3x^2 + 2x - 1, x));
            print(polynomial_gcd(x^3-x, x^2-1, x));
            print(polynomial_divrem(x^3-1, x-1, x));
            "#,
        ),
        concat!(
            "2\n0\n0\n4\n[-1, 2, 3]\n",
            "x ^ 2 - 1\n[x ^ 2 + x + 1, 0]\n",
        ),
    );
    assert_eq!(
        runtime_error("print(polynomial_divrem(x, 0, x));"),
        "polynomial_divrem: polynomial_divrem divisor must be nonzero",
    );
}

#[test]
fn substitution_solving_and_calculus_have_safe_fallbacks() {
    assert_eq!(
        output(
            r#"
            print(collect(x*x + 2x + x^2, x));
            print(subs(x^2 + y, x, 3));
            print(solve(x^2 - 1, x));
            print(derivative(x^3 + 2x + sin(x), x));
            print(derivative(sin(x) + tan(x), x));
            print(integral(x^2 + 2, x));
            print(integral(sqrt(x), x));
            "#,
        ),
        concat!(
            "2 * x ^ 2 + 2 * x\n",
            "y + 9\n",
            "[-1, 1]\n",
            "2 + 3 * x ^ 2 + cos(x)\n",
            "cos(x) + sec(x) ^ 2\n",
            "(x ^ 3 + 6 * x) / 3\n",
            "2 * sqrt(x) ^ 3 / 3\n",
        ),
    );
}

#[test]
fn solver_handles_complex_polynomials_and_inverse_transcendentals() {
    assert_eq!(
        output(
            r#"
            print(solve(x^2 + 1, x));
            print(solve(x^2 - 2, x));
            print(solve(sin(x) - 1/2, x));
            print(solve(cos(2x + 1), x));
            print(solve(tan(3x) - 1, x));
            print(solve(exp(2x + 1) - 7, x));
            print(solve(log(x) - 2, x));
            print(solve(2^x - 8, x));
            "#,
        ),
        concat!(
            "[-i, i]\n",
            "[-sqrt(8) / 2, sqrt(8) / 2]\n",
            "[asin(1/2), pi - asin(1/2)]\n",
            "[(acos(0) - 1) / 2, (-acos(0) - 1) / 2]\n",
            "[atan(1) / 3]\n",
            "[(ln(7) - 1) / 2]\n",
            "[exp(2)]\n",
            "[log(8, 2)]\n",
        ),
    );

    assert_eq!(
        output("print(solve(x^3 - 2, x));"),
        "[-0.62996052494744 - 1.09112363597172i, -0.62996052494744 + 1.09112363597172i, 1.25992104989487]\n"
    );
}

#[test]
fn calculus_handles_general_derivatives_and_linear_substitution() {
    assert_eq!(
        output(
            r#"
            print(derivative(x^x, x));
            print(derivative(asin(2x), x));
            print(derivative(sec(x), x));
            print(integral(3 * x^2, x));
            print(integral((2x + 1)^3, x));
            print(integral(1 / (2x + 1), x));
            print(integral(sin(3x), x));
            print(integral(tan(2x), x));
            "#,
        ),
        concat!(
            "(1 / x * x + ln(x)) * x ^ x\n",
            "1 / sqrt(-(4 * x ^ 2) + 1) * 2\n",
            "sec(x) * tan(x)\n",
            "x ^ 3\n",
            "(2 * x + 1) ^ 4 / 4 / 2\n",
            "ln(2 * x + 1) / 2\n",
            "-cos(3 * x) / 3\n",
            "-ln(cos(2 * x)) / 2\n",
        ),
    );
}

#[test]
fn domain_documented_transcendental_and_absolute_identities_simplify() {
    assert_eq!(
        output(
            r#"
            print(abs(x)^2);
            print(log(exp(x)));
            print(exp(log(x)));
            print(exp(x) * exp(y));
            print(sin(t)^2 + cos(t)^2);
            print(1 + tan(t)^2);
            print(expand(exp(x + y)));
            print(expand(sin(x + y)));
            assume_nonnegative(x);
            print(sqrt(x^2));
            print(abs(x));
            "#,
        ),
        concat!(
            "abs(x) ^ 2\nlog(exp(x))\nexp(log(x))\nexp(x + y)\n1\nsec(t) ^ 2\n",
            "exp(x) * exp(y)\n",
            "cos(x) * sin(y) + cos(y) * sin(x)\n",
            "x\nx\n",
        ),
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
            "(x ^ 2 - 1) / (x - 1)\n",
            "(x ^ 3 - x) / (x ^ 2 - 1)\n",
            "-(1/2 / (x + 1)) + 1/2 / (x - 1)\n",
            "1 / (x + 2) + 2 / (x + 1)\n",
        ),
    );
    assert_eq!(runtime_error("print(x/(x-x));"), "division by zero in '/'");
}

#[test]
fn linear_algebra_library_computes_exact_results_and_step_variants() {
    assert_eq!(
        output(
            r#"
            let A = [1, 2; 3, 4];
            print(transpose(A));
            print(A');
            print(A' * [5, 11]);
            print([1, 2, 3; 4, 5, 6]');
            print(trace(A));
            print(det(A));
            print(rank([1, 2; 2, 4]));
            print(rref(A));
            print(inverse(A));
            print(matmul(A, A));
            print(A * A);
            print(matmul(A, [5, 11]));
            print(A * [5, 11]);
            print([1, 2, 3; 4, 5, 6] * [7, 8; 9, 10; 11, 12]);
            print([1, 2, 3; 4, 5, 6] * [1, 2, 3]);
            print(dot([1, 2, 3], [4, 5, 6]));
            print(cross([1, 2, 3], [4, 5, 6]));
            print(norm([3, 4]));
            print(solve_linear(A, [5, 11]));
            "#,
        ),
        concat!(
            "[[1, 3], [2, 4]]\n[[1, 3], [2, 4]]\n[38, 54]\n",
            "[[1, 4], [2, 5], [3, 6]]\n5\n-2\n1\n[[1, 0], [0, 1]]\n",
            "[[-2, 1], [3/2, -1/2]]\n[[7, 10], [15, 22]]\n",
            "[[7, 10], [15, 22]]\n[27, 59]\n[27, 59]\n",
            "[[58, 64], [139, 154]]\n[14, 32]\n",
            "32\n[-3, 6, -3]\n5\n[1, 2]\n",
        )
    );
    let steps = output("print(rref_steps([1, 2; 3, 4]));");
    assert_eq!(steps, "[[1, 0], [0, 1]]\n");
    assert_eq!(output("let A = [1, 2; 3, 4]; print(A'' == A);"), "true\n");
    assert_eq!(
        runtime_error("print([1, 2; 3, 4] * [1, 2, 3]);"),
        "* matrix and vector dimensions do not align",
    );
}

#[test]
fn square_matrices_support_integer_powers_by_squaring() {
    assert_eq!(
        output(
            r#"
            let A = [1, 1; 1, 0];
            print(A^0);
            print(A^1);
            print(A^2);
            print(A^10);
            print([1, 2; 3, 4]^-1);
            print([1, 2; 3, 4]^2 * [1, 0]);
            "#,
        ),
        concat!(
            "[[1, 0], [0, 1]]\n[[1, 1], [1, 0]]\n[[2, 1], [1, 1]]\n",
            "[[89, 55], [55, 34]]\n[[-2, 1], [3/2, -1/2]]\n[7, 15]\n",
        ),
    );
    assert_eq!(
        runtime_error("print([1, 2, 3; 4, 5, 6]^2);"),
        "matrix power expects a square matrix",
    );
    assert_eq!(
        runtime_error("print([1, 2; 3, 4]^(1/2));"),
        "matrix exponent must be an integer",
    );
    assert_eq!(
        runtime_error("print([1, 2; 3, 4]^n);"),
        "matrix exponent must be an integer",
    );
    assert_eq!(
        runtime_error("print([1, 2; 2, 4]^-1);"),
        "matrix is singular and has no inverse",
    );
}

#[test]
fn expanded_calculus_and_multivariable_tools_remain_exact() {
    assert_eq!(
        output(
            r#"
            print(derivative(phi*x, x));
            print(integral(ln(x), x));
            print(integral(asin(x), x));
            print(integral(sec(x), x));
            print(gradient(x^2+y^2, [x,y]));
            print(jacobian([x*y, x+y], [x,y]));
            print(hessian(x^2+x*y+y^2, [x,y]));
            print(definite_integral(x^2, x, 0, 2));
            "#,
        ),
        concat!(
            "phi\n",
            "ln(x) * x - x\n",
            "asin(x) * x + sqrt(-(x ^ 2) + 1)\n",
            "ln(sec(x) + tan(x))\n",
            "[2 * x, 2 * y]\n",
            "[[y, x], [1, 1]]\n",
            "[[2, 1], [1, 2]]\n",
            "8/3\n",
        )
    );
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
            print(atan(0));
            print(atan2(0, 1));
            print(sinh(0));
            print(cosh(0));
            print(tanh(0));
            print(asinh(0));
            print(acosh(1));
            print(atanh(0));
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
            print(erfc(0));
            print(lgamma(1));
            print(sec(0));
            print(csc(pi/2));
            print(cot(pi/4));
            print(sign(-9));
            print(degrees(pi));
            print(radians(180));
            print(sqrt(x^2));
            print(sin(-x));
            print(cos(-x));
            "#,
        ),
        concat!(
            "pi\ne\ntau\n",
            "3/2\n2\n3/4\n1000000000000000\n3\n1\n1\n3\n2\n3\n",
            "0\n1\n0\n0\n-1\n0\n0\n0\n0\n0\n1\n0\n0\n0\n0\n",
            "1\n2\n2\n-1\n3/2\n2\n5\n6\n36\n24\n0\n1\n0\n1\n1\n1\n-1\n",
            "180\npi\nsqrt(x ^ 2)\n-sin(x)\ncos(x)\n",
        ),
    );
}

#[test]
fn standard_math_library_reports_poles_and_arity() {
    assert_eq!(
        runtime_error("print(log(1, 1));"),
        "log base must be nonzero and different from 1",
    );
    assert_eq!(
        runtime_error("print(tan(pi/2));"),
        "tan is undefined at an odd multiple of pi/2",
    );
    assert_eq!(
        runtime_error("print(evalf(gamma(-1)));"),
        "gamma failed: gamma has a pole at non-positive integers",
    );
    assert_eq!(
        runtime_error("print(sin());"),
        "sin expects 1 argument(s) but got 0",
    );
}
