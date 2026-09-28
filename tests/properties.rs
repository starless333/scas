use num_bigint::BigInt;
use num_rational::BigRational;
use scas::execute;

fn output(source: &str) -> String {
    let mut bytes = Vec::new();
    execute(source, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn randomized_exact_arithmetic_matches_num_rational() {
    let mut state = 0x51ca_5eed_u64;
    let mut source = String::new();
    let mut expected = String::new();
    for _ in 0..1_000 {
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (state as i64 % 20_001) - 10_000
        };
        let a = next();
        let b = next().max(1);
        let c = next();
        let d = next().max(1);
        source.push_str(&format!("print(({a} / {b}) + ({c} / {d}));\n"));
        let value = BigRational::new(BigInt::from(a), BigInt::from(b))
            + BigRational::new(BigInt::from(c), BigInt::from(d));
        if value.denom() == &BigInt::from(1) {
            expected.push_str(&format!("{}\n", value.numer()));
        } else {
            expected.push_str(&format!("{}/{}\n", value.numer(), value.denom()));
        }
    }
    assert_eq!(output(&source), expected);
}

#[test]
fn randomized_simplification_is_idempotent() {
    let mut state = 0x1d3a_907e_u64;
    let mut source = String::new();
    for index in 0..400 {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let a = state % 31;
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let b = state % 31;
        source.push_str(&format!(
            "let p{index} = simplify(({a} * x + {b} * x) + 0);\nprint(p{index} == simplify(p{index}));\n"
        ));
    }
    assert_eq!(output(&source), "true\n".repeat(400));
}
