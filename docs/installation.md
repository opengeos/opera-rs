# Installation

## Prebuilt Standalone CLI

Install the standalone `opera` binary (no Python or Rust toolchain required):

```sh
# Linux and macOS
curl -LsSf https://opera-rs.opengeos.org/install.sh | sh

# Windows (PowerShell)
powershell -ExecutionPolicy ByPass -c "irm https://opera-rs.opengeos.org/install.ps1 | iex"
```

## Python (PyPI)

Install `opera-rs` (Python 3.11+, prebuilt ABI3 wheels include the `opera` CLI and `opera mcp` server):

```sh
pip install opera-rs
```

Or run without installing globally via `uvx`:

```sh
uvx --from opera-rs opera --help
uvx --from opera-rs opera mcp
```

## Rust (crates.io)

Standalone CLI:

```sh
cargo install opera-cli
```

Core library in your `Cargo.toml`:

```toml
[dependencies]
opera-rs = "0.1"
```

## Custom `titiler-cmr` Endpoint

By default, `opera-rs` uses `https://titiler-cmr.opengeos.org`. Override it with `--endpoint` or the `OPERA_TITILER_ENDPOINT` environment variable:

```sh
export OPERA_TITILER_ENDPOINT="https://your-titiler-cmr.example"
```
