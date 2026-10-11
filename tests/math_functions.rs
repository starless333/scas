use scas::{execute, execute_with_options, ExecutionError, ExecutionOptions, Limits};
use std::num::NonZeroU64;

fn output(source: &str) -> String {
    let mut bytes = Vec::new();
    execute(source, &mut bytes).expect("program should execute successfully");
    String::from_utf8(bytes).expect("language output should be UTF-8")
}

fn output_at_precision(source: &str, precision: u64) -> String {
    let mut bytes = Vec::new();
    execute_with_options(
        source,
        &mut bytes,
        ExecutionOptions {
            limits: Limits::default(),
            decimal_precision: NonZeroU64::new(precision).expect("non-zero precision"),
        },
    )
    .expect("program should execute successfully");
    String::from_utf8(bytes).expect("language output should be UTF-8")
}

fn runtime_error(source: &str) -> String {
    match execute(source, Vec::new()).expect_err("program should fail") {
        ExecutionError::Runtime(error) => error.message,
        other => panic!("expected a runtime error, got {other:?}"),
    }
}

#[test]
fn constants_and_common_angles_stay_exact_until_evalf() {
    assert_eq!(
        output(
            r#"
            print(pi);
            print(e);
            print(tau);
            print(sin(pi / 6));
            print(cos(pi / 3));
            print(tan(pi / 4));
            print(exp(i * pi));
            print(sqrt(2));
            print(evalf(sqrt(2)));
            "#,
        ),
        concat!(
            "pi\ne\ntau\n1/2\n1/2\n1\n-1\nsqrt(2)\n",
            "1.41421356237309504880168872420969807856967187537694807317667973799",
            "0732478462107038850387534327641573\n",
        ),
    );
}

#[test]
fn golden_ratio_is_a_reserved_exact_constant() {
    assert_eq!(
        output("print(phi); print(φ); print(phi == φ); print(phi^2); print(1/phi);"),
        "phi\nphi\ntrue\nphi + 1\nphi - 1\n"
    );
    let approximation = output_at_precision("print(evalf(phi));", 30);
    assert!(approximation.starts_with("1.618033988749894848204586834"));
}

#[test]
fn unicode_math_aliases_are_executable_source() {
    assert_eq!(
        output("print(sin(π / 6)); print(Γ(5)); print(π == pi); print(τ == tau);"),
        "1/2\n24\ntrue\ntrue\n"
    );
}

#[test]
fn principal_complex_branches_and_arithmetic_are_first_class() {
    assert_eq!(
        output_at_precision(
            r#"
            print(sqrt(-1));
            print(sqrt(-4));
            print(sqrt(-2));
            print((-1) ^ 0.5);
            print(i ^ 2);
            print((1 + i) * (1 - i));
            print(evalf(ln(-1)));
            "#,
            50,
        ),
        concat!(
            "i\n2 * i\ni * sqrt(2)\ni\n-1\n2\n",
            "3.1415926535897932384626433832795028841971693993751i\n",
        ),
    );
}

#[test]
fn special_functions_match_high_precision_reference_vectors() {
    // Reference values were independently generated with mpmath at 80 digits.
    assert_eq!(
        output_at_precision(
            r#"
            print(gamma(0.5));
            print(gamma(-0.5));
            print(lgamma(0.5));
            print(erf(0.5));
            print(erfc(3.0));
            "#,
            50,
        ),
        concat!(
            "1.7724538509055160272981674833411451827975494561224\n",
            "-3.5449077018110320545963349666822903655950989122448\n",
            "0.57236494292470008707171367567652935582364740645766\n",
            "0.52049987781304653768274665389196452873645157575796\n",
            "0.00002209049699858544137277612958232037984770708739925\n",
        ),
    );
}

#[test]
fn complex_erf_series_covers_moderate_complex_arguments() {
    assert_eq!(
        output_at_precision("print(erf(evalf(i))); print(erfc(evalf(i)));", 30),
        "1.65042575879754287602533772956i\n1 - 1.65042575879754287602533772956i\n"
    );
    assert!(output_at_precision("print(erf(evalf(3*i)));", 30)
        .starts_with("1629.99462260156565106164795208i"));
    let large = output_at_precision("print(erf(evalf(20*i))); print(erfc(evalf(20*i)));", 30);
    assert!(
        large.starts_with(
            "147479753962878620244773315313e+143i\n1 - 147479753962878620244773315313e+143i\n"
        ),
        "{large}"
    );
}

#[test]
fn complex_gamma_and_log_gamma_use_the_principal_branch() {
    let gamma = output_at_precision("print(gamma(evalf(1+i)));", 30);
    assert!(
        gamma.starts_with("0.498015668118356042713691117462 - 0.154949828301810685124955130484i"),
        "{gamma}"
    );
    let log_gamma = output_at_precision("print(lgamma(evalf(1+i)));", 30);
    assert!(
        log_gamma
            .starts_with("-0.650923199301856338885216831504 - 0.301640320467533197887531657797i"),
        "{log_gamma}"
    );
}

#[test]
fn simplification_requires_the_assumptions_that_make_it_valid() {
    assert_eq!(
        output(
            r#"
            print(sqrt(x^2));
            print(x/x);
            print(exp(ln(x)));
            print(ln(exp(x)));
            assume_real(x);
            print(sqrt(x^2));
            print(ln(exp(x)));
            assume_positive(x);
            print(sqrt(x^2));
            print(x/x);
            print(exp(ln(x)));
            "#,
        ),
        concat!(
            "sqrt(x ^ 2)\nx / x\nexp(ln(x))\nln(exp(x))\n",
            "abs(x)\nx\nx\n1\nx\n",
        ),
    );
}

#[test]
fn negative_assumptions_simplify_abs_and_reject_contradictions() {
    assert_eq!(
        output(
            r#"
            assume_negative(x);
            print(abs(x));
            print(sqrt(x^2));
            "#,
        ),
        "-x\n-x\n",
    );
    assert_eq!(
        runtime_error("assume_positive(x); assume_negative(x);"),
        "assume_negative contradicts an existing assumption for 'x'",
    );
}

#[test]
fn general_assume_accepts_equality_and_comparison_facts() {
    assert_eq!(
        output(
            r#"
            assume(x == 3);
            print(x + 1);
            print(x * x);
            forget_assumptions();
            print(x + 1);
            "#,
        ),
        "4\n9\nx + 1\n",
    );
}

#[test]
fn sequential_assume_calls_accumulate_and_consistency_checks_fold() {
    // The second assume evaluates `2 < 10`, which folds to a boolean
    // through the recorded equality; `assume` must accept it as a
    // consistency check rather than reject it as a non-constraint.
    assert_eq!(
        output(
            r#"
            assume(x == 2);
            assume(x < 10);
            print(x);
            "#,
        ),
        "2\n",
    );
    assert_eq!(
        output(
            r#"
            assume(z == 2 and z < 10);
            print(z * z);
            "#,
        ),
        "4\n",
    );
    assert_eq!(
        output(
            r#"
            assume(q == 2 and q != 5);
            print(q * q);
            "#,
        ),
        "4\n",
    );
    // A folded constraint that fails is a contradiction.
    assert_eq!(
        runtime_error("assume(x == 2); assume(x < 10); assume(x > 5);"),
        "assume contradicts the current assumptions",
    );
    // Range facts drive simplifications exactly like the named tools.
    assert_eq!(
        output(
            r#"
            assume(y > 0);
            print(sqrt(y^2));
            forget_assumptions();
            print(sqrt(y^2));
            "#,
        ),
        // `y > 0` marks y positive, exactly like `assume_positive(y)`,
        // so the square root of the square collapses to y itself.
        "y\nsqrt(y ^ 2)\n",
    );
}

#[test]
fn contradiction_detection_covers_bound_directions_and_expression_assumptions() {
    // Consistent bound/equality combinations are accepted (a past bug
    // reported `a > 2` with `a == 3` as a contradiction).
    assert_eq!(
        output("assume(a > 2);\nassume(a == 3);\nprint(a);\n"),
        "3\n"
    );
    // Two numeric bounds conflict only when the interval is empty.
    assert_eq!(
        runtime_error("assume(b > 2);\nassume(b < 1);"),
        "assume contradicts an existing constraint",
    );
    assert_eq!(
        runtime_error("assume(c > 0);\nassume(c == 0);"),
        "assume contradicts an existing constraint",
    );
    assert_eq!(
        runtime_error("assume(g < 0);\nassume(g == 1);"),
        "assume contradicts an existing constraint",
    );
    // assume_* tools and assume() facts cross-check in both directions.
    assert_eq!(
        runtime_error("assume(d == 0);\nassume_positive(d);"),
        "assume_positive contradicts an existing assumption for 'd'",
    );
    assert_eq!(
        runtime_error("assume_positive(p);\nassume(p == -1);"),
        "assume contradicts an existing constraint",
    );
    assert_eq!(
        runtime_error("assume_positive(p);\nassume_negative(p);"),
        "assume_negative contradicts an existing assumption for 'p'",
    );
    // Claims consistent with the substituted concrete value pass.
    assert_eq!(
        output("assume(h == 3);\nassume_positive(h);\nprint(h);\n"),
        "3\n",
    );
    // Inequality assumptions on linear expressions normalize to plain
    // variable bounds: `w + 1 > 0` records `w > -1`, `2 * x < 6` records
    // `x < 3`.
    assert_eq!(
        output("assume(w + 1 > 0);\nassume(w == 0);\nprint(w);\n"),
        "0\n",
    );
    assert_eq!(
        runtime_error("assume(f + 1 > 0);\nassume(f == -5);"),
        "assume contradicts an existing constraint",
    );
    assert_eq!(
        runtime_error("assume(t - 2 > 0);\nassume(t < 1);"),
        "assume contradicts an existing constraint",
    );
    assert_eq!(
        output("assume(2 * x < 6);\nassume(x == 0);\nprint(x);\n"),
        "0\n",
    );
    assert_eq!(
        runtime_error("assume(2 * x < 6);\nassume(x == 4);"),
        "assume contradicts an existing constraint",
    );
}

#[test]
fn assume_rejects_non_constraints_and_unsupported_forms() {
    assert!(runtime_error("assume(true);").contains("expects a constraint"));
    assert!(runtime_error("assume(x or y);").contains("expects a comparison"));
    assert!(runtime_error("assume(x + 1);").contains("expects a comparison"));
}

#[test]
fn calculus_covers_new_elementary_and_special_rules() {
    assert_eq!(
        output(
            r#"
            print(derivative(erf(x), x));
            print(derivative(log10(x), x));
            print(integral(sqrt(x), x));
            print(integral(cbrt(2x + 1), x));
            print(integral(sinh(3x), x));
            print(integral(sec(x)^2, x));
            "#,
        ),
        concat!(
            "2 * exp(-(x ^ 2)) / sqrt(pi)\n",
            "1 / (ln(10) * x)\n",
            "2 * sqrt(x) ^ 3 / 3\n",
            "3 * cbrt(2 * x + 1) ^ 4 / 4 / 2\n",
            "cosh(3 * x) / 3\n",
            "tan(x)\n",
        ),
    );
}

#[test]
fn transcendental_work_has_an_independent_precision_budget() {
    let options = ExecutionOptions {
        limits: Limits {
            max_decimal_precision: 100,
            max_transcendental_precision: 20,
            ..Limits::default()
        },
        decimal_precision: NonZeroU64::new(30).unwrap(),
    };
    let error = execute_with_options("print(evalf(pi));", Vec::new(), options).unwrap_err();
    let ExecutionError::Runtime(error) = error else {
        panic!("expected a runtime limit error")
    };
    assert!(error.is_limit);
    assert_eq!(
        error.message,
        "transcendental precision exceeds the 20 digit limit"
    );
}

#[test]
fn complex_acosh_outside_the_real_domain_terminates_on_the_principal_branch() {
    // Regression: the complex acosh path used to hang forever for values
    // below one, freezing the REPL and any evaluation that reached it.
    // acosh(-1/2) = 2*pi/3 * i on the principal branch.
    let text = output("print(evalf(acosh(-0.5)));");
    assert!(text.contains('i'), "expected a complex result, got {text}");
    assert!(text.contains("2.094"), "expected ~2.094i, got {text}");
    // Values inside the domain stay real and unchanged.
    let text = output("print(evalf(acosh(2)));");
    assert!(!text.contains('i'), "expected a real result, got {text}");
    assert!(text.contains("1.316"), "expected ~1.316, got {text}");
}

#[test]
fn singular_numeric_boundaries_return_errors_instead_of_panicking() {
    let cases = [
        ("print(csc(0.0));", "csc failed: csc has a pole at zero"),
        ("print(cot(0.0));", "cot failed: cot has a pole at zero"),
        (
            "print(atanh(1.0));",
            "atanh failed: atanh has poles at -1 and 1",
        ),
        (
            "print(atan2(0, 0));",
            "atan2 failed: atan2 is undefined when both arguments are zero",
        ),
        ("print(evalf(ln(0)));", "ln failed: ln has a pole at zero"),
    ];
    for (source, expected) in cases {
        assert_eq!(runtime_error(source), expected, "source: {source}");
    }
}
