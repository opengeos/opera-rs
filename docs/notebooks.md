# Notebooks

Interactive Jupyter notebooks demonstrating `opera-rs` on real public NASA OPERA satellite data (no Earthdata account or GDAL installation required). Open a notebook directly in Google Colab or run it locally with Jupyter.

## Interactive Demo Notebook

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb)
[View on GitHub](https://github.com/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb)

A complete tour of `opera-rs` in Python using the spring 2023 re-emergence of Tulare Lake, California:

1. **Product catalog & presets:** Inspect all 9 supported NASA OPERA collections (`opera_rs.products()`), resolve aliases (`opera_rs.find_product("dswx-hls")`), and view `rio-tiler` band-math expression presets (`opera_rs.expression_presets("rtc-s1", "VV")`).
2. **Geocoding & CMR search:** Geocode a place name via Nominatim (`opera_rs.geocode()`), query NASA CMR for `DSWx-HLS` granules (`opera_rs.search()`), write footprints to GeoPackage with `opera_rs.write_vector()`, and read AOI bounds back with `opera_rs.read_aoi_bbox()`.
3. **Point sampling & zonal statistics:** Sample categorical pixel classes at `(lon, lat)` (`opera_rs.point()`), compute 30 m surface-water class areas in km^2 (`opera_rs.stats()`), and plot the class breakdown.
4. **Two-date change detection:** Quantify winter-to-spring surface-water expansion (`opera_rs.detect_change()`) with per-class area deltas.
5. **Time-series trend analysis:** Track open-water and total-water evolution across multiple acquisitions (`opera_rs.analyze_time_series()`) and plot the trend.
6. **Flood-extent vectorization:** Union DSWx-HLS and DSWx-S1 water-only tiles into RFC 7946 GeoJSON polygons (`opera_rs.derive_flood_extent()`), save to disk, and plot the vectorized flood boundaries.
7. **Pure-Rust map preview & offline tile URLs:** Render an inline PNG map preview (`opera_rs.preview()`) and build offline `titiler-cmr` URL templates (`opera_rs.tile_urls()`) for both `rasterio` (`dswx-hls`) and `xarray` (`disp-s1`) products.
8. **Bundled CLI & MCP schema:** Run the bundled `opera` CLI via `python -m opera_rs` and inspect the machine-readable `--json-usage` catalog.

## Sentinel-1 Bursts, DISP-S1 Frames & Missing-Data Notebook

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_utils_demo.ipynb)
[View on GitHub](https://github.com/opengeos/opera-rs/blob/main/examples/opera_utils_demo.ipynb)

A focused walkthrough of the pure-Rust `opera-utils` integration in `opera-rs` (requires `opera-rs >= 0.2.0` or a local build from `main`):

1. **Sentinel-1 burst IDs & dates:** Normalize burst IDs (`opera_rs.normalize_burst_id()`), extract burst IDs and `YYYY-MM-DD` dates (`opera_rs.get_burst_id()`, `opera_rs.get_dates()`), and group CSLC-S1 / COMPRESSED-CSLC-S1 files (`opera_rs.group_by_burst()`).
2. **DISP-S1 filename parsing:** Extract frame ID, polarization, reference/secondary timestamps, and product version with `opera_rs.parse_disp_filename()`.
3. **Official `opera-adt/burst_db` v0.9.0 queries:** Look up frame metadata, UTM/WGS 84 bounds, and frame-to-burst mappings (`get_frame_to_burst_mapping()`, `get_frame_bbox()`, `get_frame_ids_for_burst()`).
4. **Spatial frame intersection & GeoPackage export:** Find all DISP-S1 frames intersecting Tulare Lake across 46,986 global frames (`get_intersecting_frames()`), plot them with Matplotlib, and export to `.gpkg` via `terrio`.
5. **Missing-data subset optimization:** Rank optimal rectangular `(burst_ids x dates)` CSLC subsets (`get_missing_data_options()`) and plot a 2D availability heatmap.
6. **Public NASA CMR DISP-S1 search:** Query `OPERA_L3_DISP-S1_V1` frame stacks with automatic version deduplication (`disp_search()`) and build offline `xarray` `titiler-cmr` TileJSON URLs.
7. **Bundled `opera` CLI:** Run `burst-id`, `frame`, and `frames-intersect` directly via `python -m opera_rs`.

## Running Locally

Install `opera-rs` alongside the plotting and notebook dependencies in a Python 3.11+ environment:

```sh
pip install "opera-rs>=0.2.0" matplotlib pandas pillow jupyterlab
git clone https://github.com/opengeos/opera-rs.git
jupyter lab opera-rs/examples/
```
