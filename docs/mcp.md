# AI Agents (MCP)

`opera mcp` is a pure-Rust [Model Context Protocol](https://modelcontextprotocol.io) server over standard input and output, modeled after `terrio mcp` and `lidar mcp`. AI agents such as Claude Code, Claude Desktop, Cursor, and VS Code Copilot can discover NASA OPERA granules, compute surface-water and disturbance area statistics, run two-date change detection and time-series analyses, vectorize observed flood polygons, and visually inspect rendered map previews.

Every [CLI subcommand](cli.md) except `mcp` and `help` is exposed automatically as an MCP tool whose `args` parameter is the CLI argument list and whose description is the command's full `--help`.

## Client Setup

=== "Claude Code"

    ```sh
    # Using the standalone opera binary
    claude mcp add opera -- opera mcp

    # Or zero-install via uvx
    claude mcp add opera -- uvx --from opera-rs opera mcp
    ```

=== "Claude Desktop / Cursor"

    Add to `claude_desktop_config.json` or `.cursor/mcp.json`:

    ```json
    {
      "mcpServers": {
        "opera": {
          "command": "opera",
          "args": ["mcp"]
        }
      }
    }
    ```

    Or zero-install with `uvx`:

    ```json
    {
      "mcpServers": {
        "opera": {
          "command": "uvx",
          "args": ["--from", "opera-rs", "opera", "mcp"]
        }
      }
    }
    ```

=== "VS Code"

    Add to `.vscode/mcp.json`:

    ```json
    {
      "servers": {
        "opera": {
          "command": "opera",
          "args": ["mcp"]
        }
      }
    }
    ```

## Example Agent Prompts

Once connected, you can ask your AI assistant questions such as:

- *"Search NASA OPERA DSWx-HLS granules over Tulare Lake, California in April 2023 and show me a water-only map preview."*
- *"Compare open-water area over Tulare Lake between January 15, 2023 and April 30, 2023, and save a before/after PNG."*
- *"Run a monthly DSWx-HLS surface-water time series for Tulare Lake from January to June 2023 and export the table to CSV."*
- *"Vectorize observed flood extent over Valencia, Spain between October 27 and November 5, 2024 using DSWx-HLS and DSWx-S1."*
- *"Check what NASA OPERA products cover surface displacement and build an xarray TileJSON URL for DISP-S1."*

## Agent-Centric Capabilities

- **Catalog & geocoding (`products`, `geocode`):** Inspect all 9 OPERA collections, aliases, band defaults, and resolve named places to WGS 84 bounding boxes.
- **No Earthdata credentials required:** Public CMR search (`search`) and server-side authenticated `titiler-cmr` analytics (`stats`, `point`, `change`, `timeseries`, `flood-extent`, `preview`).
- **Inline multimodal visual inspection (`preview`):** When `-o` is omitted, `opera mcp` writes the PNG into a private temporary directory, attaches it directly as MCP `image` content (`image/png`), and removes the temporary file automatically.
- **Built-in MCP prompts (`prompts/list`, `prompts/get`):**
  - `map-flood-event` (`place_or_bbox`, `start`, `end`, optional `out_dir`)
  - `detect-surface-change` (`product`, `place_or_bbox`, `before_date`, `after_date`, optional `band`)
  - `analyze-water-timeseries` (`place_or_bbox`, `start`, `end`, optional `product`, optional `out_dir`)
  - `explore-opera-area` (`place_or_bbox`, optional `product`)
- **Machine-readable schema (`--json-usage`):** `opera --json-usage` or `opera <subcommand> --json-usage` emits the versioned JSON schema of every flag, type, and default.

## Permissions, Annotations, and Limits

`opera mcp` does not sandbox filesystem access; configure your MCP client's tool approval policy and review output paths before approving file writes.

| Safeguard | Value / Behavior |
| --- | --- |
| Read-only tool hints (`readOnlyHint: true`, `destructiveHint: false`, `idempotentHint: true`) | `products`, `geocode`, `tilejson`, `point`, `stats`, `inspect`, `burst-id`, `disp-search` |
| Local-only tools (`openWorldHint: false`) | `products`, `inspect`, `burst-id`, `missing-data` |
| Network tools (`openWorldHint: true`) | `geocode`, `search`, `tilejson`, `point`, `stats`, `change`, `timeseries`, `flood-extent`, `preview`, `animate`, `export-html`, `frame`, `frames-intersect`, `disp-search` |
| Combined stdout/stderr cap | 100,000 bytes (stderr preserved first) |
| Inline PNG preview cap | 8 MiB |
| Concurrency & cancellation | Up to 8 concurrent isolated child processes with `notifications/cancelled` and graceful EOF drain |
