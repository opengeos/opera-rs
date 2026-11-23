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

## Running Locally

Install `opera-rs` alongside the plotting and notebook dependencies in a Python 3.11+ environment:

```sh
pip install opera-rs matplotlib pandas pillow jupyterlab
git clone https://github.com/opengeos/opera-rs.git
jupyter lab opera-rs/examples/opera_rs_demo.ipynb
```
