# opera-rs (Python)

Python bindings and bundled `opera` CLI / MCP server for [`opengeos/opera-rs`](https://github.com/opengeos/opera-rs), a pure-Rust toolkit for searching, visualizing, and analyzing NASA OPERA satellite products via NASA CMR and `titiler-cmr`.

## Install

```sh
pip install opera-rs
```

Run the MCP server directly without a permanent install:

```sh
uvx --from opera-rs opera mcp
```

## Quick Start

```python
import opera_rs

print(opera_rs.__version__)
```
