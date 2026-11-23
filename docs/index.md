# opera-rs

Pure-Rust, agent-centric toolkit for searching, visualizing, and analyzing [NASA OPERA](https://www.jpl.nasa.gov/go/opera) satellite products (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1) via NASA CMR and `titiler-cmr`.

- **Zero GDAL or C/C++ dependencies:** Pure Rust across the core crate, CLI, MCP server, and Python wheels.
- **No Earthdata login required:** Public NASA CMR search and `titiler-cmr` server-side authentication for tiles, point queries, and AOI statistics.
- **Agent-centric by design:** Structured `--json` output, machine-readable `--json-usage` schema, and a built-in Model Context Protocol server (`opera mcp`) with inline PNG map previews and curated analysis prompts.

## Next Steps

- [Installation](installation.md)
- [CLI Reference](cli.md)
- [AI Agents (MCP)](mcp.md)
- [Rust Library](rust.md)
- [Python API](python.md)
