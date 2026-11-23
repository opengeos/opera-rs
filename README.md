# opera-rs

[![CI](https://github.com/opengeos/opera-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/opengeos/opera-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/opera-rs.svg)](https://crates.io/crates/opera-rs)
[![PyPI](https://img.shields.io/pypi/v/opera-rs)](https://pypi.org/project/opera-rs/)
[![Docs](https://img.shields.io/badge/docs-opera--rs.opengeos.org-teal.svg)](https://opera-rs.opengeos.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Pure-Rust, agent-centric toolkit for searching, visualizing, and analyzing [NASA OPERA](https://www.jpl.nasa.gov/go/opera) satellite products (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1) with a standalone CLI, built-in **Model Context Protocol (MCP)** server (`opera mcp`), Rust library (`opera-rs`), and Python bindings (`opera-rs`). Zero GDAL or C/C++ dependencies, and no Earthdata login required for public CMR search and `titiler-cmr` visualization and statistics.

**Documentation:** [Installation](https://opera-rs.opengeos.org/installation/) | [CLI](https://opera-rs.opengeos.org/cli/) | [AI Agents (MCP)](https://opera-rs.opengeos.org/mcp/) | [Rust](https://opera-rs.opengeos.org/rust/) | [Python](https://opera-rs.opengeos.org/python/)

## Workspace Layout

| Path | Description |
|---|---|
| `crates/opera-rs` | Pure-Rust core library (published to crates.io as `opera-rs`) |
| `crates/opera-cli` | Standalone `opera` CLI and MCP server (`opera-cli`) |
| `python` | Python bindings (`opera-rs`, PyO3) with bundled `opera` CLI and MCP server |
| `docs` | Zensical documentation site and CLI installer scripts (`install.sh`, `install.ps1`) |

## License

`opera-rs` is licensed under the [MIT License](LICENSE). See [NOTICE](NOTICE) for acknowledgments and third-party notices.
