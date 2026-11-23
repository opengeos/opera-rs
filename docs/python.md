# Python API

Install `opera-rs` from PyPI (Python 3.11+, zero mandatory Python dependencies, includes the bundled `opera` CLI and `opera mcp` server):

```sh
pip install opera-rs
```

## Examples

```python
import opera_rs

# 1. Inspect the 9 supported NASA OPERA products
for p in opera_rs.products():
    print(p["short_name"], "->", p["short_title"])

# 2. Geocode a place or pass a WGS 84 bbox [west, south, east, north]
bbox = [-119.95, 35.85, -119.45, 36.25]

# 3. Search NASA CMR for DSWx-HLS granules and write footprints to GeoPackage
res = opera_rs.search("dswx-hls", bbox=bbox, start="2023-04-20", end="2023-04-30", limit=10)
opera_rs.write_vector(res["feature_collection"], "tulare_footprints.gpkg", layer_name="footprints")

# 4. Zonal categorical water statistics (30 m class areas in km^2)
st = opera_rs.stats(bbox, product="dswx-hls", start="2023-04-20", end="2023-04-30")
for cls in st["bands"]["b1"]["class_breakdown"]:
    print(cls["label"], f"{cls['area_km2']:.2f} km^2")

# 5. Two-date change detection
chg = opera_rs.detect_change(bbox, "2023-01-15", "2023-04-30", product="dswx-hls")
print("Open water delta (km^2):", chg["open_water_delta_km2"])

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
