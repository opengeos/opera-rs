# opera-rs (Python)

Python bindings and bundled `opera` CLI / MCP server for [`opengeos/opera-rs`](https://github.com/opengeos/opera-rs), a pure-Rust, agent-centric toolkit for searching, visualizing, and analyzing NASA OPERA satellite products via NASA CMR and `titiler-cmr` (zero GDAL dependencies, no Earthdata login required).

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

# List all 9 NASA OPERA collections
print([p["short_title"] for p in opera_rs.products()])

# Search DSWx-HLS surface water over Tulare Lake, April 2023
bbox = [-119.95, 35.85, -119.45, 36.25]
res = opera_rs.search("dswx-hls", bbox=bbox, start="2023-04-20", end="2023-04-30", limit=5)
print(res["hits"], res["dates"])

# Compute 30 m categorical water class areas (km^2)
st = opera_rs.stats(bbox, product="dswx-hls", start="2023-04-20", end="2023-04-30")
print(st["bands"]["b1"]["class_breakdown"])

# Render a PNG map preview with CARTO basemap, scale bar, and legend
opera_rs.preview(bbox, product="dswx-hls", start="2023-04-20", end="2023-04-30", water_only=True, output_path="tulare.png")
```
