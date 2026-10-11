# SCAS deployment

## Recommended release channels

### Browser IDE

Deploy the `web/` directory as a static site through GitHub Pages. The existing
`.github/workflows/pages.yml` workflow builds the Wasm package, copies the user
documentation into `web/docs/`, uploads the static artifact, and deploys it.

In the repository's GitHub settings, set **Pages → Build and deployment →
Source** to **GitHub Actions**. A push to `main`, or a manual run of the
**Browser IDE** workflow, will publish the site.

Only one workflow should deploy the `github-pages` environment. The current
`pages.yml` and `docs.yml` workflows both deploy there and can overwrite each
other. Keep `pages.yml` as the deployment workflow; the browser build already
includes the documentation under `web/docs/`.

GitHub documents the custom Pages workflow and artifact requirements here:
<https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages>.

### Command-line application

Publish the `scas` executable through GitHub Releases. The existing release
workflow builds platform archives when a version tag is pushed:

```sh
git tag v0.1.0
git push origin v0.1.0
```

### Desktop IDE

The current release workflow does not package `scas-gui`. Build it explicitly
with:

```sh
cargo build --locked --release --features gui --bin scas-gui
```

Start with `.zip`/`.tar.gz` archives. Add an MSI, AppImage, and signed macOS
application later if desktop distribution becomes a priority.

## Size: source, build cache, and shipped artifacts

The local working directory is approximately 17 GB because `target/` contains
Rust compiler output from many development and test builds. It is not part of
the application and is ignored by Git through `/target` in `.gitignore.
Cloning the repository or downloading a release does **not** download this
directory.

Measured artifacts at the time of this document:

| Item | Approximate size |
| --- | ---: |
| Local root `target/` cache | 17 GB |
| Browser deployment directory | 8.96 MB raw |
| Browser Wasm | 7.34 MB raw |
| Browser Wasm with gzip | 2.55 MB |
| Browser Wasm with Brotli | 2.12 MB |
| Release CLI | 5.34 MB |
| Stripped release CLI | 4.73 MB |

The repository currently tracks `fuzz/target/`, however. That directory holds
about 87.8 MB of generated fuzz-build files and should be removed from version
control. The checked-in `.github/skills/` tooling contributes another 17.3 MB,
mostly from a bundled executable. Neither directory is uploaded by the Pages
workflow, but both make source checkouts larger than necessary.

## Is deleting `target/` safe?

Yes. From the repository root, either of these is safe when no Cargo process is
currently building:

```sh
cargo clean
```

or:

```sh
rm -rf target/
```

Both remove disposable compiler output. Source code, tests, Git history, and
the generated browser package in `web/pkg/` remain intact. The next Cargo build
will take longer because dependencies must be compiled again.

Deleting the root `target/` does not remove the separately tracked
`fuzz/target/` directory. Remove that from Git in a deliberate repository
cleanup and change `.gitignore` from `/target` to `target/` so nested Cargo
build directories are ignored too.

## Why the binary is larger than its feature list suggests

Compiled size is driven by runtime foundations, not only visible CAS features.
SCAS contains the `eframe`/`egui` UI and renderer, font processing, WebGL or
native window integration, an editor, plotting, parsing, an interpreter, and
several arbitrary-precision number implementations.

The numeric dependency graph currently contains two big-integer families:

- `num-bigint`, used through `num-rational` and `bigdecimal`;
- `dashu-int`, used through `dashu-float` and `dashu-cmplx`.

The browser also instantiates the Wasm module in the page and in a Web Worker.
The browser should cache the network download, but the two instances increase
runtime initialization and memory costs. These are reasonable future profiling
targets, but they are not reasons to delay an initial static deployment.

## Deployment priority order

1. Stop tracking `fuzz/target/` and ignore nested `target/` directories.
2. Remove the duplicate GitHub Pages deployment so only `pages.yml` publishes.
3. Enable release stripping and link-time optimization.
4. Add `wasm-opt`, then compare download size and CAS performance before
   keeping the optimization.
5. Profile which crates and functions contribute most to the Wasm artifact.
6. Only after profiling, consider consolidating the numeric backends or
   splitting the evaluation worker from the browser UI module.

Cargo supports release symbol stripping, LTO, code-generation-unit controls,
size optimization, and abort-on-panic through release profiles. Measure each
combination because size-oriented optimization is not guaranteed to be smaller
or faster for every program:
<https://doc.rust-lang.org/cargo/reference/profiles.html>.

A reasonable starting profile is:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

Use a separate web release profile when experimenting with `opt-level = "s"`
or `"z"`, so native CAS performance is not reduced accidentally.
