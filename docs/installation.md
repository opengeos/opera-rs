# Installation

`opera-rs` is distributed as a prebuilt standalone binary (`opera`), a Python package (`opera-rs` on PyPI, which also bundles the `opera` CLI and MCP server), and Rust crates (`opera-rs` and `opera-cli` on crates.io). None of these require GDAL or NASA Earthdata credentials.

## Prebuilt Standalone CLI

Install the standalone `opera` binary with no Python or Rust toolchain required:

=== "Linux / macOS"

    ```sh
    curl -LsSf https://opera-rs.opengeos.org/install.sh | sh
    ```

=== "Windows (PowerShell)"

    ```powershell
    powershell -ExecutionPolicy ByPass -c "irm https://opera-rs.opengeos.org/install.ps1 | iex"
    ```

Verify the installation:

```sh
opera --version
opera products
```

Prebuilt release archives (`tar.gz` / `zip` + SHA-256 checksums) are also attached to every [GitHub Release](https://github.com/opengeos/opera-rs/releases).

## Python (PyPI)

Install `opera-rs` (Python 3.11+, prebuilt ABI3 wheels with zero mandatory Python dependencies, bundling the `opera` CLI and `opera mcp` server):

```sh
pip install opera-rs
```

Verify the Python package and bundled CLI:

```sh
python -c "import opera_rs; print(opera_rs.__version__)"
opera --version
```

Or run the CLI / MCP server without installing globally via [`uvx`](https://docs.astral.sh/uv/guides/tools/):

```sh
uvx --from opera-rs opera --help
uvx --from opera-rs opera mcp
```

## Rust (crates.io)

Install the standalone CLI from crates.io:

```sh
cargo install opera-cli
```

Add the core library to your `Cargo.toml`:

```toml
[dependencies]
opera-rs = "0.1"
```

## Building from Source

Requires Rust 1.90 or newer (no GDAL required):

```sh
git clone https://github.com/opengeos/opera-rs.git
cd opera-rs
cargo build --release -p opera-cli
./target/release/opera --version
```

## Custom `titiler-cmr` Endpoint

By default, `opera-rs` connects to `https://titiler-cmr.opengeos.org`. Override the endpoint per command with `--endpoint`, or globally via the `OPERA_TITILER_ENDPOINT` environment variable:

```sh
export OPERA_TITILER_ENDPOINT="https://your-titiler-cmr.example"
```
