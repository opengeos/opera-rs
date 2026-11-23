# opera-rs

Pure-Rust, agent-centric toolkit for searching, visualizing, and analyzing [NASA OPERA](https://www.jpl.nasa.gov/go/opera) satellite products (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1) via NASA CMR, `titiler-cmr`, and [`terrio`](https://github.com/opengeos/terrio).

- **Zero GDAL or C/C++ dependencies:** Pure Rust across the `opera-rs` crate, `opera` CLI, `opera mcp` server, and `opera-rs` Python wheels.
- **No Earthdata login required:** Public NASA CMR granule discovery and server-side authenticated `titiler-cmr` tile rendering, pixel sampling, and AOI statistics.
- **Agent-centric by design:** Structured `--json` output on every subcommand, machine-readable `--json-usage` catalog, and a pure-Rust Model Context Protocol (MCP) server (`opera mcp`) with inline PNG map previews and curated remote-sensing workflow prompts.

## Quick Start

```sh
# Install via pip or run directly with uvx
pip install opera-rs

# Search Tulare Lake surface water granules
opera search --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 --start 2023-04-20 --end 2023-04-30

# Start the MCP server for Claude Code, Cursor, or VS Code
opera mcp
```

## Documentation Sections

- [Installation](installation.md)
- [CLI Reference](cli.md)
- [AI Agents (MCP)](mcp.md)
- [Rust Library](rust.md)
- [Python API](python.md)
