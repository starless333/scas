# Correctness evidence

SCAS checks exact algebra with property tests, integration fixtures, and independent differential tests. These establish confidence for the tested families; they are not a proof for every input or domain.

## SymPy differential run

Run `cargo build --release --bin scas` and `python3 scripts/compare_sympy.py`. The script uses seed `20261011`, generates 25 cases each for expansion, differentiation, polynomial GCD, and resultants, evaluates SCAS in one process, and checks symbolic equivalence with SymPy. A nonzero exit means at least one mismatch.

| Family | Passed | Cases |
| --- | ---: | ---: |
| Expansion | 25 | 25 |
| Differentiation | 25 | 25 |
| Polynomial GCD | 25 | 25 |
| Resultant | 25 | 25 |
| **Total** | **100** | **100** |

Recorded 2026-10-11 on commit `b570f6a` plus the changes described in this document, using SCAS's release binary and SymPy 1.14.0. The tested expressions are small integer polynomials in one variable. The result says nothing about multivariate inputs, singular domains, numerical accuracy, or unsupported expressions. Change `--seed` and `--per-family` for additional runs.

## Other checks

- `cargo test --all-targets --all-features` includes fixed output fixtures, algebra properties, runtime limits, and document rendering tests.
- `fuzz/` contains scanner and parser targets; [weekly fuzz CI](../.github/workflows/fuzz.yml) runs each for 30 seconds.
- [High precision function tests](../tests/math_functions.rs) compare special function outputs against independent `mpmath` reference vectors.
- [Release CI](../.github/workflows/release.yml) tests each supported platform, produces checksums, and creates build provenance attestations.
- [CI](../.github/workflows/ci.yml) enforces formatting, clippy, tests, and the seeded SymPy differential run.

The fixed performance comparison and its measurement method are in [SymPy benchmarks](sympy-benchmarks.md).
