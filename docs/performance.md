# Performance baseline

Baseline captured on 2026-09-28 with Rust 1.79.0, Linux x86-64, and an Intel
Core i5-8265U. Run `cargo bench --bench baselines` to reproduce it.

| Workload | Mean time |
| --- | ---: |
| Scan 1,000 declarations | 551 µs |
| Parse 1,000 declarations | 826 µs |
| Large exact arithmetic expression | 89 µs |
| Expand `(x + y + z)^8` | 505 µs |
| 1,000 interpreted function calls | 815 µs |
| Algebra example, end to end | 169 µs |
| LaTeX expression render (reused buffer) | 0.18 µs |

The complete benchmark process took 0.43 seconds and reached approximately
21,120 KiB maximum resident memory. A `perf stat` run recorded roughly 2.7
instructions per cycle and a 0.4% branch-miss rate. These figures are reference
points, not cross-machine guarantees; investigate a sustained regression above
20% on the same host before changing an implementation.

The initial profile shows scanning/parsing and interpreted calls dominate the
small representative workloads. Exact arithmetic and the current polynomial
expansion are below those costs. Optimization should therefore target measured
parser allocation or interpreter dispatch costs before changing algebra rules.
