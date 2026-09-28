# Contributing

Open an issue before a large semantic change. Keep language behavior documented
in `docs/language.md` and `src/grammar.org`, add regression tests, and do not
mark algebraic identities safe without stating their domain requirements.

Before submitting a change, run:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Changes use the MIT license and should add an entry under `Unreleased` in
`CHANGELOG.md` when user-visible.
