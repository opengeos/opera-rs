# opera-rs contributor instructions

## Scope and architecture

`opera-rs` is a pure-Rust, agent-centric library, CLI, MCP server, and Python package for searching, visualizing, and analyzing [NASA OPERA](https://www.jpl.nasa.gov/go/opera) satellite products (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1) via NASA CMR and `titiler-cmr`, with zero GDAL or C/C++ dependencies.

- Put all core catalog, CMR search, geocoding, `titiler-cmr` client, analysis, vectorization, and rendering logic in `crates/opera-rs` (published to crates.io as `opera-rs`). Keep `crates/opera-cli` and `python/` thin wrappers.
- Keep default builds 100% pure Rust with zero GDAL or C/C++ dependencies. Network transport (`ureq` with `rustls`) lives behind the `http` feature (enabled by default) so offline/WASM builds (`--no-default-features`) compile cleanly without HTTP dependencies.
- Expose CLI subcommands consistently across `--json`, `--json-usage`, and `opera mcp` (which automatically registers every CLI subcommand except `mcp` and `help`).
- Keep `python/LICENSE` and `python/NOTICE` synchronized with root `LICENSE` and `NOTICE` so wheel and sdist builds pass license verification.
- `python/` is excluded from the root Cargo workspace so pure-Rust builds never require `libpython`. Minimum supported Python is **3.11** (`requires-python = ">=3.11"`, `abi3-py311`).

## Validation

Run the full gate matching `.github/workflows/ci.yml` before pushing:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p opera-rs --no-default-features --lib
cargo test --workspace --all-features
```

When touching `python/`, rebuild and test the Python bindings inside a virtualenv:

```sh
cd python && .venv/bin/maturin develop --release && .venv/bin/pytest -v
```

## GitHub workflow

- Submit feature and bugfix changes through feature branches and pull requests rather than pushing directly to `main`.
