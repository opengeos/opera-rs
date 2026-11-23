# CLI Reference

The `opera` command-line tool provides 13 structured NASA OPERA subcommands (each supporting `--json` output and `--json-usage` schema introspection) plus a built-in Model Context Protocol server (`opera mcp`).

## Spatial AOI Selection

Every spatial command (`search`, `stats`, `change`, `timeseries`, `flood-extent`, `preview`, `animate`, `export-html`) accepts an Area of Interest (AOI) in three ways:

- `--bbox W,S,E,N`: WGS 84 bounding box (negative western and southern coordinates are supported directly, e.g. `--bbox -119.95,35.85,-119.45,36.25`).
- `--place "Place Name"`: Geocoded via OpenStreetMap Nominatim with administrative-boundary preference.
- `--aoi-file path.{geojson,gpkg,shp}`: Read via pure-Rust `terrio` and automatically reprojected to `EPSG:4326`.

## Command Summary

| Command | Purpose |
| --- | --- |
| `opera products [NAME]` | List all 9 OPERA products, aliases, band defaults, and band-math expression presets |
| `opera geocode <PLACE>` | Resolve a place name to a WGS 84 bounding box |
| `opera search` | Search NASA CMR for granules, footprints, dates, and bands; optionally write `.geojson`/`.gpkg`/`.shp` |
| `opera tilejson` | Build (`--url-only`) or fetch `rasterio`/`xarray` TileJSON and XYZ tile URL templates |
| `opera point` | Sample pixel/variable values at `--lon` `--lat` with categorical class label decoding |
| `opera stats` | Compute zonal AOI statistics and 30 m categorical class areas (km^2) |
| `opera change` | Two-date before/after change detection, Markdown report, and optional side-by-side PNG preview |
| `opera timeseries` | Multi-date trend analysis (min, max, mean, net change, OLS slope/day) and optional CSV export |
| `opera flood-extent` | Vectorize observed DSWx-HLS + DSWx-S1 water-only tiles into GeoJSON/GeoPackage/Shapefile polygons |
| `opera preview` | Render a PNG map preview with basemap, metric scale bar, legend, and title banner |
| `opera animate` | Render a multi-date animated GIF across OPERA acquisitions |
| `opera export-html` | Export a self-contained MapLibre GL JS interactive HTML viewer |
| `opera inspect <FILE>` | Inspect a local OPERA GeoTIFF/COG on disk via `terrio` (CRS, stats, class areas) |
| `opera mcp` | Start the JSON-RPC 2.0 Model Context Protocol (MCP) server over stdio |

## Common Workflows

### Catalog, Geocoding, and CMR Search

```sh
# List all supported products and aliases
opera products

# Inspect details and expression presets for RTC-S1
opera products rtc-s1 --json

# Resolve a place name to a WGS 84 bounding box
opera geocode "Tulare Lake, California" --json

# Search DSWx-HLS granules and write footprints to GeoPackage
opera search --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30 -o tulare_footprints.gpkg --json
```

### Point Sampling, Zonal Statistics, and Tile URLs

```sh
# Sample a pixel at (lon, lat) with categorical class labels
opera point --product dswx-hls --lon -119.72 --lat 36.05 \
  --temporal 2023-04-20/2023-04-30 --json

# Compute 30 m surface-water class areas (km^2) over an AOI
opera stats --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30

# Build offline titiler-cmr URLs for rasterio (DSWx-HLS) or xarray (DISP-S1)
opera tilejson --product dswx-hls --water-only --url-only --json
opera tilejson --product disp-s1 --backend xarray --variable displacement --group / --url-only --json
```

### Change Detection and Time Series

```sh
# Two-date change detection with Markdown report and side-by-side comparison PNG
opera change --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --before 2023-01-15 --after 2023-04-30 \
  --preview-out tulare_change.png --markdown

# Multi-date trend analysis exported to CSV
opera timeseries --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-01-01 --end 2023-05-15 --interval-days 15 \
  --csv-out tulare_ts.csv
```

### Flood-Extent Vectorization and Visualization

```sh
# Union DSWx-HLS + cloud-penetrating DSWx-S1 water-only tiles into polygons
opera flood-extent --place "Valencia Region, Spain" \
  --start 2024-10-27 --end 2024-11-05 -o valencia_flood.geojson

# Render a static PNG map preview
opera preview --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30 --water-only -o tulare.png

# Render a multi-date animated GIF
opera animate --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-01-01 --end 2023-05-15 --water-only -o tulare.gif

# Export an interactive MapLibre GL JS HTML viewer
opera export-html --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30 --water-only -o tulare.html
```

### Local Raster Inspection

```sh
# Inspect a local OPERA GeoTIFF/COG on disk (CRS, band stats, categorical class areas)
opera inspect OPERA_L3_DSWx-HLS_B01_WTR.tif --product dswx-hls --json
```

## Machine-Readable Usage Catalog

Every data subcommand (excluding `mcp`) supports `--json` for structured output and `--json-usage` for schema introspection by AI agents and scripts:

```sh
opera --json-usage
opera change --json-usage
```
