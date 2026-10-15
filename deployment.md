# SCAS deployment

## Recommended release channels

### Browser IDE

Deploy the `web/` directory as a static site through GitHub Pages. The existing
`.github/workflows/pages.yml` workflow builds the Wasm package, copies the user
documentation into `web/docs/`, uploads the static artifact, and deploys it.

In the repository's GitHub settings, set **Pages → Build and deployment →
Source** to **GitHub Actions**. A push to `main`, or a manual run of the
**Browser IDE** workflow, will publish the site.

Only `pages.yml` deploys the `github-pages` environment. The `docs.yml`
workflow checks Rust API documentation and uploads it as a separate Actions
artifact; it cannot overwrite the IDE. User documentation is included at
`web/docs/`. Pages deployments are serialized without canceling an in-flight
publish.

The build uses Cargo's `web-release` profile (release optimization, thin LTO,
one codegen unit, stripped debug information). `scripts/check-web.py` validates
local HTML links, JavaScript module paths, the Wasm header, and `.nojekyll`
before upload. The workflow gates deployment on a Chromium launch check against
`/web/`. That check verifies project-subdirectory URLs and boots the IDE, Wasm
worker, graph engine, and language service without depending on an external
CDN. The broader REPL, editor, and MathJax regression suite still runs, but is
non-blocking so a third-party CDN failure cannot leave Pages stuck on an older
deployment.

Reproduce the deployment checks locally:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked
bash scripts/build-web.sh
python3 -m http.server 8080 --bind 127.0.0.1
```

In a second terminal:

```sh
npm install --prefix /tmp/scas-browser playwright@1.64.1
/tmp/scas-browser/node_modules/.bin/playwright install chromium
NODE_PATH=/tmp/scas-browser/node_modules node scripts/check-web-browser.cjs
```

Use `CHROMIUM_PATH=/usr/bin/chromium` to run an installed Chromium instead.
Math typesetting uses pinned MathJax 4.0.0 from jsDelivr when preview or print
is first opened, so that check needs internet access. The IDE, REPL, and graph
engine run locally in Wasm.

After pushing, check the **Browser IDE** workflow's build and deploy jobs and
open the URL reported by its deploy job. Repository Pages settings and a hosted
publish require GitHub access; local validation alone does not verify them.

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
| Browser Wasm before web profile | 7.79 MB raw / 2.66 MB gzip |
| Browser Wasm with web profile | approximately 7.16 MB raw / 2.57 MB gzip |
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
2. Profile which crates and functions contribute most to the Wasm artifact.
3. Consider `wasm-opt`, comparing download size and CAS performance before
   keeping any additional optimization.
4. Only after profiling, consider consolidating the numeric backends or
   splitting the evaluation worker from the browser UI module.

The duplicate Pages publisher and browser release-profile tuning are resolved.
The native release profile remains independent of the browser profile.

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
