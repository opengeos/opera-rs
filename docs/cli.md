# CLI Reference

The `opera` command-line tool exposes 13 structured NASA OPERA commands plus the `mcp` server and global `--json-usage` catalog flag.

## Spatial AOI Selection

Every spatial command (`search`, `stats`, `change`, `timeseries`, `flood-extent`, `preview`, `animate`, `export-html`) accepts an AOI in three ways:

- `--bbox W,S,E,N`: WGS 84 bounding box (negative western/southern coordinates supported directly).
- `--place "Place Name"`: Geocoded via OpenStreetMap Nominatim (with administrative-boundary preference).
- `--aoi-file path.{geojson,gpkg,shp}`: Read via pure-Rust `terrio` and automatically reprojected to `EPSG:4326`.

## Commands

| Command | Purpose |
|---|---|
| `opera products [NAME]` | List all 9 OPERA products, aliases, band defaults, and band-math expression presets |
| `opera geocode <PLACE>` | Resolve a place name to a WGS 84 bounding box |
| `opera search` | Search NASA CMR for granules, footprints, dates, and bands; optionally write `.geojson`/`.gpkg`/`.shp` |
| `opera tilejson` | Build (`--url-only`) or fetch `rasterio`/`xarray` TileJSON and XYZ tile URL templates |
| `opera point` | Sample pixel/variable values at `--lon` `--lat` with categorical class label decoding |
| `opera stats` | Compute zonal AOI statistics and 30 m categorical class areas (km^2) |
| `opera change` | Two-date before/after change detection, Markdown report, and optional side-by-side PNG preview |
| `opera timeseries` | Multi-date trend analysis (min, max, mean, net change, OLS slope/day) and optional CSV export |
| `opera flood-extent` | Vectorize observed DSWx-HLS + DSWx-S1 water-only tiles into GeoJSON/GeoPackage polygons |
| `opera preview` | Render a PNG map preview with CARTO Voyager basemap, metric scale bar, legend, and title banner |
| `opera animate` | Render a multi-date animated GIF across OPERA acquisitions |
| `opera export-html` | Export a self-contained MapLibre GL JS interactive HTML viewer |
| `opera inspect <FILE>` | Inspect a local OPERA GeoTIFF/COG on disk via `terrio` (CRS, stats, class areas) |
| `opera mcp` | Start the JSON-RPC 2.0 Model Context Protocol (MCP) server over stdio |

## Machine-Readable Usage Catalog

```sh
opera --json-usage
opera change --json-usage
```
