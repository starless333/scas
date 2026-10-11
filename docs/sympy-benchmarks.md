# SCAS and SymPy fixed workload comparison

Captured 2026-10-11 on an Intel Core i5-8265U, Linux x86-64, Rust 1.79.0, Python 3.14.7, and SymPy 1.14.0. SCAS was built with `cargo build --release --bin scas` from commit `b570f6a` plus the changes described here. Run `python3 scripts/benchmark_sympy.py` to reproduce on your machine.

| Workload | SCAS median, ms | SymPy median, ms |
| --- | ---: | ---: |
| Polynomial GCD, degrees 8 and 6 | 0.044 | 0.194 |
| Resultant, degrees 5 and 4 | 0.155 | 0.164 |
| 8×8 determinant | 0.379 | 3.216 |
| Gamma at 50 digits, argument 0.5 | 1.377 | 0.023 |

Each entry is the median of five batches, divided by the number of evaluations in a batch. SCAS batches include parsing, evaluation, and `print` output; process startup is amortized. SymPy batches call its corresponding API on constructed expressions in an already-running Python process. These are task timings, not equal-layer engine microbenchmarks. In particular, SymPy simplifies the half-integer gamma argument very efficiently, while SCAS runs its numeric special-function path. Results vary with machine, build, and input size. The script defines every expression and batch size.
