# Python API

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb)
[![PyPI](https://img.shields.io/pypi/v/opera-rs.svg)](https://pypi.org/project/opera-rs/)

Install `opera-rs` from PyPI (Python 3.11+, zero mandatory Python dependencies, prebuilt ABI3 wheels including the bundled `opera` CLI and `opera mcp` server):

```sh
pip install opera-rs
```

To run all examples interactively without installing anything locally, open the [Google Colab demo notebook](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb).

## Function Reference

| Function | Purpose |
| --- | --- |
| `opera_rs.version()` / `__version__` | Return the installed `opera-rs` package version |
| `opera_rs.products()` | List metadata for all 9 supported NASA OPERA products |
| `opera_rs.find_product(query)` | Resolve a product by CMR short name or alias (e.g. `"dswx-hls"`) |
| `opera_rs.expression_presets(product, band)` | Return ready-made `rio-tiler` band-math expression presets |
| `opera_rs.geocode(place)` | Geocode a place name to a WGS 84 bounding box via Nominatim |
| `opera_rs.search(product, ...)` | Query NASA CMR granules, footprints, dates, and bands (`bbox` or `place`) |
| `opera_rs.tile_urls(concept_id, ...)` | Build `titiler-cmr` TileJSON and XYZ tile URL templates offline |
| `opera_rs.tilejson(product, ...)` | Fetch a live TileJSON document from `titiler-cmr` |
| `opera_rs.point(lon, lat, ...)` | Sample pixel values and categorical class labels at `(lon, lat)` |
| `opera_rs.stats(bbox, ...)` | Compute zonal AOI statistics and 30 m categorical class area breakdowns |
| `opera_rs.detect_change(bbox, before_date, after_date, ...)` | Run two-date before/after change detection with class area deltas |
| `opera_rs.analyze_time_series(bbox, start, end, ...)` | Run multi-date temporal trend analysis (OLS slope, min/max/mean) |
| `opera_rs.derive_flood_extent(bbox, start, end, ...)` | Vectorize observed DSWx-HLS + DSWx-S1 water-only tiles into polygons |
| `opera_rs.preview(bbox, ...)` | Render a PNG map preview and return PNG `bytes` (optionally write to disk) |
| `opera_rs.read_aoi_bbox(path)` | Read a GeoJSON, GeoPackage, or Shapefile AOI and return its WGS 84 bbox |
| `opera_rs.write_vector(feature_collection, out_path, ...)` | Write a GeoJSON `FeatureCollection` to `.geojson`, `.gpkg`, or `.shp` |
| `opera_rs.inspect_raster(path, ...)` | Inspect a local OPERA GeoTIFF/COG on disk via `terrio` |

## End-to-End Example

```python
import opera_rs

# 1. Inspect the 9 supported NASA OPERA products
for p in opera_rs.products():
    print(p["short_name"], "->", p["short_title"])

# 2. Geocode a place name via Nominatim, or pass an explicit WGS 84 bbox [W, S, E, N]
place = opera_rs.geocode("Tulare Lake, California")
print(place["display_name"], place["bbox"])
bbox = [-119.95, 35.85, -119.45, 36.25]

# 3. Search NASA CMR for DSWx-HLS granules and write footprints to GeoPackage
res = opera_rs.search("dswx-hls", bbox=bbox, start="2023-04-20", end="2023-04-30", limit=10)
opera_rs.write_vector(res["feature_collection"], "tulare_footprints.gpkg", layer_name="footprints")
print("Footprint bounds from GeoPackage:", opera_rs.read_aoi_bbox("tulare_footprints.gpkg"))

# 4. Sample a pixel and compute zonal categorical water statistics (30 m class areas in km^2)
pt = opera_rs.point(-119.72, 36.05, product="dswx-hls", temporal="2023-04-20/2023-04-30")
print("Point class labels:", pt["assets"][0]["class_labels"])

st = opera_rs.stats(bbox, product="dswx-hls", start="2023-04-20", end="2023-04-30")
first_band = next(iter(st["bands"].values()))
for cls in first_band["class_breakdown"]:
    print(cls["label"], f"{cls['area_km2']:.2f} km^2")

# 5. Two-date change detection and multi-date time series
chg = opera_rs.detect_change(bbox, "2023-01-15", "2023-04-30", product="dswx-hls")
delta = chg["open_water_delta_km2"]
print(f"Open water delta: {delta:+.2f} km^2" if delta is not None else "Open water delta: unavailable")

ts = opera_rs.analyze_time_series(
    bbox, "2023-01-01", "2023-05-15", product="dswx-hls", interval_days=20, max_observations=6
)
print("Time-series trend:", ts["trend"])

# 6. Derive observed flood polygons (DSWx-HLS + cloud-penetrating DSWx-S1)
flood = opera_rs.derive_flood_extent(bbox, "2023-04-20", "2023-04-30")
opera_rs.write_vector(flood["feature_collection"], "tulare_flood.geojson")

# 7. Render a PNG preview (returns bytes and optionally writes to disk)
png_bytes = opera_rs.preview(
    bbox,
    product="dswx-hls",
    start="2023-04-20",
    end="2023-04-30",
    water_only=True,
    output_path="tulare.png",
)
```

## Offline Tile URLs (Rasterio and Xarray)

```python
import opera_rs

# Rasterio COG product (defaults to B01_WTR and categorical colormap)
dswx = opera_rs.tile_urls("C2617126679-POCLOUD", product="dswx-hls", water_only=True)
print(dswx["tilejson_url"])
print(dswx["tile_url_template"])

# Xarray NetCDF/HDF5 product (DISP-S1 displacement)
disp = opera_rs.tile_urls(
    "C3294057315-ASF",
    product="disp-s1",
    backend="xarray",
    variables=["displacement"],
    group="/",
)
print(disp["tilejson_url"])
```

## Bundled CLI and MCP Server

The Python package exposes the bundled `opera` binary both as a console script and via `python -m opera_rs`:

```sh
opera --version
python -m opera_rs products
python -m opera_rs mcp
```
