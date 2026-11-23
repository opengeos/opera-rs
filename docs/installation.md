# Installation

## Standalone CLI

Install the prebuilt `opera` binary:

```sh
# Linux and macOS
curl -LsSf https://opera-rs.opengeos.org/install.sh | sh

# Windows (PowerShell)
powershell -ExecutionPolicy ByPass -c "irm https://opera-rs.opengeos.org/install.ps1 | iex"
```

Or install from crates.io:

```sh
cargo install opera-cli
```

## Python

```sh
pip install opera-rs
```

Run the CLI or MCP server without installing globally using `uvx`:

```sh
uvx --from opera-rs opera --help
uvx --from opera-rs opera mcp
```

## Rust Library

Add `opera-rs` to your `Cargo.toml`:

```toml
[dependencies]
opera-rs = "0.1"
```
