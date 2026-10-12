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
fn symbolic_geometric_sum_closes_over_symbolic_bounds() {
    assert_eq!(output("print(sum(2^k, k, 0, n));"), "2 ^ (n + 1) - 1\n");
    assert_eq!(output("print(sum(1^k, k, 2, n));"), "n - 1\n");
}

#[test]
fn weighted_geometric_sum_is_a_closed_hypergeometric_family() {
    let result = output("print(sum(k*2^k, k, 0, n));");
    assert!(!result.contains("sum("), "{result}");
    assert!(result.contains("2 ^ (n + 1)"), "{result}");
    assert_eq!(output("print(sum(k*2^k, k, 0, 3));"), "34\n");
}

#[test]
fn assertions_introspection_and_paired_eigenvectors() {
    assert_eq!(
        output(r#"
            print(type_of([1, 2]));
            print(help(limit));
            print(assert(2 + 2 == 4));
            print(verify(true));
            print(format("lambda={} vector={{}} {}", 3, [0, 1]));
            print(eigenpairs([2,0;0,3]));
        "#),
        "array\nlimit: limit(expression, variable, point[, side])\ntrue\ntrue\nlambda=3 vector={} [0, 1]\n[[3, [0, 1]], [2, [1, 0]]]\n",
    );
    assert_eq!(
        runtime_error("assert(false, \"matrix identity is wrong\");"),
        "assertion failed: matrix identity is wrong"
    );
    assert_eq!(
        runtime_error("assert(x == 0);"),
        "assertion failed: claim evaluated to false or could not be proved"
    );
}

#[test]
fn equation_chains_require_proof_at_every_link() {
    assert_eq!(
        output("print(verify_chain([x^2 + 2*x + 1, (x + 1)^2, x*x + 2*x + 1]));"),
        "true\n"
    );
    assert_eq!(
        runtime_error("verify_chain([x^2, x^2 + 1]);"),
        "equation chain fails at step 1 -> 2"
    );
    assert_eq!(
        runtime_error("verify_chain([x/x, 1]);"),
        "equation chain fails at step 1 -> 2"
    );
}

#[test]
fn user_rewrite_rules_match_consistent_subexpressions_once() {
    assert_eq!(
        output("print(rewrite(sin(x)+sin(y), sin(t), t^2));"),
        "x ^ 2 + y ^ 2\n"
    );
    assert_eq!(
        runtime_error("rewrite(sin(x), sin(t), t+u);"),
        "rewrite replacement uses an unbound pattern variable 'u'"
    );
}

#[test]
fn stored_rewrite_rules_use_first_match_and_assumption_guards() {
    assert_eq!(
        output(
            r#"
            let rules = [[sin(t), t^2, ["positive", t]], [cos(t), t]];
            print(rewrite(sin(x)+cos(y), rules));
            assume_positive(x);
            print(rewrite(sin(x)+cos(y), rules));
        "#
        ),
        "y + sin(x)\ny + x ^ 2\n"
    );
    assert_eq!(
        runtime_error("rewrite(sin(x), [[sin(t), t, [\"unknown\", t]]]);"),
        "rewrite guard expects positive, negative, nonnegative, or nonzero"
    );
    assert_eq!(
        output("let rules=[[sin(t),t^2,[\"positive\",t]],[sin(t),t^3]]; print(rewrite(sin(x),rules)); assume_positive(x); print(rewrite(sin(x),rules));"),
        "x ^ 3\nx ^ 2\n"
    );
}

#[test]
fn partial_fractions_support_repeated_rational_poles() {
    let decomposition = output("print(apart((x+1)/(x-1)^2));");
    assert!(decomposition.contains("(x - 1) ^ 2"), "{decomposition}");
    assert!(decomposition.contains("x - 1"), "{decomposition}");
    assert_eq!(
        output("print(together(apart((x+1)/(x-1)^2)));"),
        "(x + 1) / (x ^ 2 - 2 * x + 1)\n"
    );
    assert_eq!(
        output("print(subs(apart(1/((x-1)*(x^2+1))), x, 2));"),
        "1/5\n"
    );
    assert_eq!(
        output("print(subs(apart((2*x+3)/((x-1)^2*(x^2+1))), x, 2));"),
        "7/5\n"
    );
}

#[test]
fn repeated_quadratic_poles_decompose_and_integrate() {
    let decomposed = output("print(apart((x^3+x+1)/(x^2+1)^2));");
    assert!(decomposed.contains("(x ^ 2 + 1) ^ 2"), "{decomposed}");
    assert!(decomposed.contains("x / (x ^ 2 + 1)"), "{decomposed}");
    assert_eq!(
        output("print(subs(together(apart((x^3+x+1)/(x^2+1)^2)), x, 2));"),
        "11/25\n"
    );
    let primitive = output("print(integral(1/(x^2+1)^2, x));");
    assert!(primitive.contains("atan(x)"), "{primitive}");
    assert!(!primitive.contains("integral("), "{primitive}");
    assert_eq!(
        output("print(integral((x+1)/(x^2+2*x+2)^2, x));"),
        "-((x ^ 2 + 2 * x + 2) ^ -1) / 2\n"
    );
    assert!(
        output("print(integral(1/(x^2-2)^2, x));").contains("integral("),
        "unsupported branch must remain explicit"
    );
}

#[test]
fn periodic_trigonometric_families_name_an_integer_parameter() {
    assert_eq!(
        output("print(solve_periodic(sin(x), x, n)); print(solve_periodic(tan(2*x), x, n));"),
        "{\"families\": [2 * n * pi, 2 * n * pi + pi], \"parameter\": n, \"domain\": Z}\n{\"families\": [1/2 * n * pi], \"parameter\": n, \"domain\": Z}\n"
    );
    assert_eq!(
        runtime_error("solve_periodic(sin(x^2), x, n);"),
        "solve_periodic expects an affine sin argument"
    );
    assert_eq!(
        output("print(solve_periodic(sin(x)-2, x, n)[\"families\"]);"),
        "[]\n"
    );
    let unit_sine = output("print(solve_periodic(sin(x)-1, x, n)[\"families\"]);");
    assert_eq!(unit_sine.matches(", ").count(), 0, "{unit_sine}");
}

#[test]
fn integer_quartics_factor_without_rational_roots() {
    let factored = output("print(factor(x^4+4));");
    assert!(
        factored.contains("x ^ 2") && factored.contains(" * "),
        "{factored}"
    );
    assert_eq!(output("print(expand(factor(x^4+4)));"), "x ^ 4 + 4\n");
}

#[test]
fn integer_sextics_can_split_into_irreducible_cubics() {
    let source = "(x^3+x+1)*(x^3+2*x+2)";
    let factored = output(&format!("print(factor(expand({source})));"));
    assert!(
        factored.contains("x ^ 3 + x + 1") && factored.contains("x ^ 3 + 2 * x + 2"),
        "{factored}"
    );
    assert_eq!(
        output(&format!(
            "print(expand(factor(expand({source}))) == expand({source}));"
        )),
        "true\n"
    );
}

#[test]
fn nonmonic_degree_eight_polynomials_can_split_into_quartics() {
    let source = "(2*x^4+x+1)*(x^4+2*x+2)";
    let factored = output(&format!("print(factor(expand({source})));"));
    assert!(
        factored.contains("2 * x ^ 4 + x + 1") && factored.contains("x ^ 4 + 2 * x + 2"),
        "{factored}"
    );
    assert_eq!(
        output(&format!(
            "print(expand(factor(expand({source}))) == expand({source}));"
        )),
        "true\n"
    );
}

#[test]
fn rational_laurent_series_handles_a_nontrivial_denominator() {
    let series = output("print(series(1/(x+x^2),x,0,3));");
    assert!(series.contains("/ x"), "{series}");
    assert_eq!(output("print(limit((x+x^2)/(x),x,0));"), "1\n");
    let with_remainder = output("print(series(1/(x+x^2),x,0,3,\"O\"));");
    assert!(with_remainder.contains("O(x ^ 4)"), "{with_remainder}");
}

#[test]
fn directional_absolute_value_limits_handle_composite_expressions() {
    assert_eq!(
        output(
            "print(limit((abs(x)+x)/x,x,0,\"left\")); print(limit((abs(x)+x)/x,x,0,\"right\"));"
        ),
        "0\n2\n"
    );
    assert_eq!(
        runtime_error("limit((abs(x)+x)/x,x,0);"),
        "left and right limits differ for absolute-value expression"
    );
}

#[test]
fn high_order_rational_poles_and_full_order_series_are_exact() {
    assert_eq!(output("print(limit(x^20/x^18,x,0)); print(limit((2*x^20)/(3*x^20),x,0)); print(limit(1/(x^10+x^11),x,0)); print(limit(1/x^11,x,0,\"left\"));"), "0\n2/3\ninfinity\n-infinity\n");
    let series = output("print(series(1/(x+x^2),x,0,64,\"O\"));");
    assert!(series.starts_with("1 / x - 1 + x - x ^ 2"), "{series}");
    assert!(series.ends_with("- x ^ 64 + O(x ^ 65)\n"), "{series}");
}

#[test]
fn repeated_diagonal_eigenvalues_keep_independent_exact_vectors() {
    assert_eq!(
        output("print(eigenpairs([2,0,0;0,2,0;0,0,3],\"ascending\"));"),
        "[[2, [1, 0, 0]], [2, [0, 1, 0]], [3, [0, 0, 1]]]\n"
    );
    assert_eq!(
        output("print(eigenpairs([2,1,0;0,3,1;0,0,4],\"ascending\"));"),
        "[[2, [1, 0, 0]], [3, [1, 1, 0]], [4, [1/2, 1, 1]]]\n"
    );
}

#[test]
fn nonsymmetric_complex_eigenpairs_and_complex_svd_reconstruct() {
    assert_eq!(output("let p=eigenpairs([0,-1,0;1,0,0;0,0,2]); print(p[0][0]); print(abs(p[1][0])); print(abs(p[2][0]));"), "2\n1\n1\n");
    assert_eq!(output("let a=[1+i,2;3*i,4;5,6-i]; let d=svd(a); let r=matmul(matmul(d[0],d[1]),d[2]); assert(abs(r[0][0]-(1+i))<1/1000000); assert(abs(r[1][0]-3*i)<1/1000000); assert(abs(r[2][1]-(6-i))<1/1000000); print(d[1][0][0]>d[1][1][1]);"), "true\n");
}

#[test]
fn higher_polynomial_sums_use_exact_closed_forms() {
    assert_eq!(
        output("print(subs(sum(k^4,k,1,n),n,5)); print(subs(sum(k^5,k,2,n),n,4));"),
        "979\n1299\n"
    );
}

#[test]
fn separable_ode_returns_an_implicit_solution_with_its_restriction() {
    let solved = output("print(dsolve_separable(x, y, x, y, C));");
    assert!(solved.contains("ln(abs(y))"), "{solved}");
    assert!(solved.contains("equilibrium solutions"), "{solved}");
    assert_eq!(
        runtime_error("dsolve_separable(x, y, x, x, C);"),
        "dsolve_separable requires f(x) and g(y) with distinct variables"
    );
}

#[test]
fn maps_and_string_formatting_are_available() {
    assert_eq!(
        output(
            r#"
        let settings = map([["theme", "dark"], ["zoom", 2], ["zoom", 3]]);
        print(type_of(settings));
        print(settings["zoom"]);
        print(keys(settings));
        print(values(put(settings, "zoom", 4)));
        print(format("{}: {}", "zoom", settings["zoom"]));
    "#
        ),
        "map\n3\n[\"theme\", \"zoom\"]\n[\"dark\", 4]\nzoom: 3\n"
    );
    assert_eq!(
        runtime_error("print(map([])[\"missing\"]);"),
        "map key 'missing' was not found"
    );
}

#[test]
fn rational_limits_at_infinity_are_exact() {
    assert_eq!(
        output(
            r#"
        print(limit((3*x^2 + 1)/(2*x^2 - x), x, "infinity"));
        print(limit(1/(x^2 + 1), x, "-infinity"));
        print(limit(7, x, "infinity"));
    "#
        ),
        "3/2\n0\n7\n"
    );
    assert_eq!(
        output("print(limit(x, x, \"infinity\")); print(limit(x^3, x, \"-infinity\"));"),
        "infinity\n-infinity\n"
    );
}

#[test]
fn matrix_limits_and_matrix_valued_functions_are_entrywise() {
    assert_eq!(
        output(
            r#"
            print(limit([x, sin(x)/x; x^2, 3], x, 0));
            fn M(t) = [t, t^2; sin(t)/t, cos(t)];
            print(limit(M(x), x, 0));
            print(limit(trace(M(x)), x, 0));
            print(limit([1/x, x^2; 2, 3], x, 0, "right"));
        "#
        ),
        "[[0, 1], [0, 3]]\n[[0, 0], [1, 1]]\n1\n[[infinity, 0], [2, 3]]\n"
    );
}

#[test]
fn exact_matrix_power_limits_cover_diagonal_and_nondiagonal_sequences() {
    assert_eq!(
        output(r#"
            let A = [3/4, 1/4; 1/4, 3/4];
            print(limit(A^n, n, "infinity"));
            print(limit(trace(A^n), n, "infinity"));
            print(limit(det(A^n), n, "infinity"));
            let P = [1,1;0,1];
            print(limit(P * [1,0;0,1/2]^n * inverse(P), n, "infinity"));
            print(limit([1,0,0;0,1/2,0;0,0,0]^n, n, "infinity"));
            print(limit([1/2,1;0,1/2]^n, n, "infinity"));
            print(limit([2,0;0,3]^n, n, "-infinity"));
        "#),
        "[[1/2, 1/2], [1/2, 1/2]]\n1\n0\n[[1, -1], [0, 0]]\n[[1, 0, 0], [0, 0, 0], [0, 0, 0]]\n[[0, 0], [0, 0]]\n[[0, 0], [0, 0]]\n"
    );
    assert!(runtime_error("print(limit([1,1;0,1]^n, n, \"infinity\"));").contains("Jordan block"));
}

#[test]
fn documented_matrix_limits_render_the_final_matrix() {
    let document = scas::document::build(
        "let A = [3/4, 1/4; 1/4, 3/4];\n\\[ limit(A^n, n, \"infinity\") => \\]\n\\[ limit([x, 2*x; 0, 1], x, 0) => \\]",
    )
    .unwrap();
    let latex = document.latex();
    assert!(latex.contains("\\begin{pmatrix}"), "{latex}");
    assert!(latex.contains("\\frac{1}{2}"), "{latex}");
}

#[test]
fn directional_poles_and_laurent_terms_are_explicit() {
    assert_eq!(output("print(limit(1/x, x, 0, \"left\")); print(limit(1/x, x, 0, \"right\")); print(limit(1/x^2, x, 0)); print(limit(x/(x^2), x, 0, \"right\"));"), "-infinity\ninfinity\ninfinity\ninfinity\n");
    assert_eq!(
        runtime_error("print(limit(1/x, x, 0));"),
        "left and right limits diverge with opposite signs"
    );
    assert_eq!(output("print(series(1/x, x, 0, 3));"), "1 / x\n");
    let laurent = output("print(series((1 + x)/(x^2), x, 0, 2));");
    assert!(
        laurent.contains("1 / x ^ 2") && laurent.contains("1 / x"),
        "{laurent}"
    );
}

#[test]
fn absolute_value_ratios_have_directional_real_limits() {
    assert_eq!(
        output("print(limit(abs(x)/x,x,0,\"left\")); print(limit(abs(x)/x,x,0,\"right\")); print(limit(abs(2*x-2)/(2*x-2),x,1,\"left\"));"),
        "-1\n1\n-1\n"
    );
    assert_eq!(
        runtime_error("limit(abs(x)/x,x,0);"),
        "left and right limits differ for absolute-value ratio"
    );
}

#[test]
fn eigenvalue_order_and_exact_small_svd_reconstruct_matrix() {
    assert_eq!(output("print(eigenvalues([2,0;0,3], \"ascending\")); print(eigenpairs([2,0;0,3], \"descending\"));"), "[2, 3]\n[[3, [0, 1]], [2, [1, 0]]]\n");
    assert_eq!(
        output("let d = svd([3,0;0,4]); print(d[1]); print(matmul(matmul(d[0], d[1]), d[2]));"),
        "[[4, 0], [0, 3]]\n[[3, 0], [0, 4]]\n"
    );
    assert_eq!(
        output(
            "let d = svd([0,0;0,0]); print(matmul(matmul(d[0], d[1]), d[2])); print(svd([[-5]])[1]);"
        ),
        "[[0, 0], [0, 0]]\n[[5]]\n"
    );
    assert_eq!(
        output("let d = svd([1,1;0,0]); let r = matmul(matmul(d[0], d[1]), d[2]); assert(abs(evalf(r[0][0]) - 1) < 1/1000000); assert(abs(evalf(r[0][1]) - 1) < 1/1000000); print(r[1]);"),
        "[0, 0]\n"
    );
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
fn limits_series_sums_products_and_multiple_integrals_are_exact() {
    assert_eq!(
        output(
            r#"
            print(limit(sin(x) / x, x, 0));
            print(limit((x^2 - 1) / (x - 1), x, 1, "left"));
            print(series(exp(x), x, 0, 3));
            print(sum(k^2, k, 1, 5));
            print(product(k, k, 1, 5));
            print(sum(k^2, k, 1, n));
            print(multiple_integral(x*y, [x, y]));
            print(dsolve([1, -3, 2], x));
            "#,
        ),
        concat!(
            "1\n2\n",
            "(2 * x ^ 3 + 6 * x ^ 2 + 12 * x + 12) / 12\n",
            "55\n120\n(2 * n + 1) * n * (n + 1) / 6\n",
            "1/4 * x ^ 2 * y ^ 2\n",
            "[exp(2 * x), exp(x)]\n",
        ),
    );
}

#[test]
fn spectral_decompositions_and_vector_spaces_are_available() {
    assert_eq!(
        output(
            r#"
            print(characteristic_polynomial([1,2;3,4], x));
            print(eigenvalues([2,0;0,3]));
            print(eigenvectors([2,0;0,3]));
            print(lu([4,3;6,3]));
            print(qr([1,0;0,1]));
            print(cholesky([4,2;2,3]));
            print(null_space([1,2,3;2,4,6]));
            print(column_space([1,2,3;2,4,7]));
            print(gram_schmidt([[1,0],[1,1]]));
            print(projection([1,2], [1,0]));
            print(change_basis([2,3], [[1,0],[0,1]]));
            print(matrix_exp([1,0;0,2]));
            print(jordan_form([2,1;0,2]));
            "#,
        ),
        concat!(
            "x ^ 2 - 5 * x - 2\n[3, 2]\n[[0, 1], [1, 0]]\n",
            "[[[1, 0], [3/2, 1]], [[4, 3], [0, -3/2]]]\n",
            "[[[1, 0], [0, 1]], [[1, 0], [0, 1]]]\n",
            "[[2, 0], [1, sqrt(2)]]\n[[-2, 1, 0], [-3, 0, 1]]\n",
            "[[1, 2], [3, 7]]\n[[1, 0], [0, 1]]\n[1, 0]\n[2, 3]\n",
            "[[e, 0], [0, exp(2)]]\n[[2, 1], [0, 2]]\n",
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
            print(integral(e^x, x));
            print(integral(e^(2x + 1), x));
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
            "e ^ x\n",
            "e ^ (2 * x + 1) / 2\n",
        ),
    );
}

#[test]
fn integrals_of_constant_base_exponentials_and_rational_powers() {
    assert_eq!(
        output(
            r#"
            print(integral(2^x, x));
            print(integral(2^(3x + 1), x));
            print(integral(10^(-x), x));
            print(integral((1/2)^x, x));
            print(integral(1^x, x));
            print(integral(1^(x^2), x));
            print(integral(x^(1/2), x));
            print(integral((2x + 1)^(1/2), x));
            print(integral(x^(-1/2), x));
            print(integral(0^x, x));
            print(integral((-2)^x, x));
            print(integral(2^(x^2), x));
            "#,
        ),
        concat!(
            "2 ^ x / ln(2)\n",
            "2 ^ (3 * x + 1) / (3 * ln(2))\n",
            "10 ^ -x / (-1 * ln(10))\n",
            "1/2 ^ x / ln(1/2)\n",
            "x\n",
            "x\n",
            "x ^ 3/2 / 3/2\n",
            "(2 * x + 1) ^ 3/2 / 3/2 / 2\n",
            "x ^ 1/2 / 1/2\n",
            "integral(0 ^ x, x)\n",
            "integral(-2 ^ x, x)\n",
            "integral(2 ^ x ^ 2, x)\n",
        ),
    );
}

#[test]
fn polynomial_times_constant_base_exponential_integrates_by_parts() {
    let result = output(
        r#"
        print(integral(x * 2^x, x));
        print(definite_integral(2^x, x, 0, 1));
        "#,
    );
    assert_eq!(
        result,
        "-1 * (2 ^ x / ln(2) / ln(2)) + 2 ^ x / ln(2) * x\nln(2) ^ -1\n"
    );
}

#[test]
fn new_integral_rules_differentiate_back_to_the_integrand() {
    assert_eq!(
        output(
            r#"
            print(assert(abs(evalf(subs(derivative(integral(2^x, x), x) - 2^x, x, 2))) < 1/1000000000));
            print(assert(abs(evalf(subs(derivative(integral(2^(3x + 1), x), x) - 2^(3x + 1), x, 1))) < 1/1000000000));
            print(assert(abs(evalf(subs(derivative(integral(x^(1/2), x), x) - x^(1/2), x, 4))) < 1/1000000000));
            "#,
        ),
        "true\ntrue\ntrue\n",
    );
}

#[test]
fn integration_by_parts_and_definite_symmetry() {
    assert_eq!(
        output(
            r#"
            print(integral(x*exp(x), x));
            print(integral(x*sin(x), x));
            print(integral(x^2*cos(x), x));
            print(integral(x*sinh(x), x));
            print(integral((x + 1)*exp(x), x));
            print(definite_integral(sin(x), x, -1, 1));
            print(definite_integral(x*exp(x), x, -1, 1));
            print(definite_integral(x + x^2, x, -1, 1));
            print(definite_integral(cos(x), x, -pi, pi));
            print(definite_integral(x^3*exp(x^2), x, -2, 2));
            print(definite_integral(x^2, x, -2, 2));
            print(definite_integral(x^x, x, 0, 1));
            print(sin(0));
            print(exp(0));
            print(ln(1));
            "#,
        ),
        concat!(
            "-1 * exp(x) + exp(x) * x\n",
            "-cos(x) * x + sin(x)\n",
            "-2 * -cos(x) * x + -2 * sin(x) + sin(x) * x ^ 2\n",
            "-1 * sinh(x) + cosh(x) * x\n",
            "-1 * exp(x) + exp(x) + exp(x) * x\n",
            "0\n",
            "2 * exp(-1)\n",
            "2/3\n",
            "0\n",
            "0\n",
            "16/3\n",
            "definite_integral(x ^ x, x, 0, 1)\n",
            "0\n",
            "1\n",
            "0\n",
        ),
    );
}

#[test]
fn rational_integration_through_partial_fractions() {
    assert_eq!(
        output(
            r#"
            print(integral((3x + 1)/((x - 1)*(x + 2)), x));
            print(integral(2/(x - 1), x));
            print(integral(1/x, x));
            print(integral(1/(2x + 1), x));
            print(integral((x + 1)/(x^2 - 1), x));
            print(integral((x^2 + 1)/(x^3 - x), x));
            print(integral(1/(x^2 + 1), x));
            print(integral(1/(x^2 + 4), x));
            print(integral(3/(x^2 + 9), x));
            print(integral(sin(x)^2, x));
            print(integral(cos(3x)^2, x));
            print(integral(x/(x + 1), x));
            print(apart((x^2 + 1)/(x^3 - x)));
            print(solve(x^3 - x, x));
            "#,
        ),
        concat!(
            "4/3 * ln(x - 1) + 5/3 * ln(x + 2)\n",
            "2 * ln(x - 1)\n",
            "ln(x)\n",
            "ln(2 * x + 1) / 2\n",
            "ln(x - 1)\n",
            "ln(x + 1) - ln(x) + ln(x - 1)\n",
            "atan(x)\n",
            "atan(x / 2) / 2\n",
            "atan(x / 3)\n",
            "(4 * x - 2 * sin(2 * x)) / 8\n",
            "(12 * x + 2 * sin(6 * x)) / 24\n",
            "-ln(x + 1) + x\n",
            "1 / (x + 1) - 1 / x + 1 / (x - 1)\n",
            "[-1, 0, 1]\n",
        ),
    );
}

#[test]
fn together_combines_fractions_and_apart_divides_improper_ones() {
    assert_eq!(
        output(
            r#"
            print(apart((x^2 + 1)/(x + 1)));
            print(apart(x^3/(x^2 - 1)));
            print(together(1/x + 1/(x + 1)));
            print(together(1/(x - 1) - 1/(x + 1)));
            print(together(1/x + 2/x^2));
            print(apart(together(1/(x - 1) + 1/(x + 1))));
            "#,
        ),
        concat!(
            "x - 1 + 2 / (x + 1)\n",
            "x + (1/2 / (x + 1) + 1/2 / (x - 1))\n",
            "(2 * x + 1) / (x ^ 2 + x)\n",
            "2 / (x ^ 2 - 1)\n",
            "(x + 2) / x ^ 2\n",
            "1 / (x + 1) + 1 / (x - 1)\n",
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
