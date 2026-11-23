# opera-rs

[![CI](https://github.com/opengeos/opera-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/opengeos/opera-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/opera-rs.svg)](https://crates.io/crates/opera-rs)
[![PyPI](https://img.shields.io/pypi/v/opera-rs.svg)](https://pypi.org/project/opera-rs/)
[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/opengeos/opera-rs/blob/main/LICENSE)

Pure-Rust, agent-centric toolkit for searching, visualizing, and analyzing [NASA OPERA](https://www.jpl.nasa.gov/go/opera) satellite products (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1) via NASA's public CMR API, [`titiler-cmr`](https://github.com/developmentseed/titiler-cmr), and [`terrio`](https://github.com/opengeos/terrio), with **zero GDAL dependencies** (`--no-default-features` has zero C/C++ dependencies) and **no Earthdata login required**.

Try `opera-rs` right now with zero local setup in the [Google Colab notebook](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb), or browse the [Notebooks](notebooks.md) page.

## Key Capabilities

| Capability | Description | Guide |
| --- | --- | --- |
| **All 9 NASA OPERA Products** | Built-in catalog with short names, aliases (`dswx-hls`, `dswx-s1`, `dist-alert`, `rtc-s1`, `disp-s1`), categorical colormaps, band defaults, and `rio-tiler` band-math expression presets. | [CLI Reference](cli.md) |
| **Public CMR Search & Geocoding** | Paginated NASA CMR UMM-G discovery with reprocessed-version deduplication, newest-per-tile filtering, Nominatim place geocoding, and AOI input via `--bbox`, `--place`, or GeoJSON/GeoPackage/Shapefile. | [CLI Reference](cli.md) |
| **Backend-Aware `titiler-cmr` Analytics** | Supports both `rasterio` (COG/GeoTIFF) and `xarray` (NetCDF/HDF5/Zarr) backends without local Earthdata credentials, with 30 m categorical class areas in km^2. | [Python API](python.md) |
| **Change Detection & Time Series** | Two-date before/after class-area deltas (`change` / `detect_change`) with Markdown reports and side-by-side PNG previews, plus multi-date OLS temporal trends (`timeseries` / `analyze_time_series`). | [Notebooks](notebooks.md) |
| **Observed Flood-Extent Vectorization** | Union DSWx-HLS and cloud-penetrating DSWx-S1 water-only tiles, trace exterior rings with dry-land holes, and export RFC 7946 polygons to GeoJSON, GeoPackage, or Shapefile. | [Python API](python.md) |
| **Pure-Rust Visualization** | Render PNG previews with legends and metric scale bars (`preview`), multi-date animated GIFs (`animate`), and self-contained interactive MapLibre GL JS HTML viewers (`export-html`). | [CLI Reference](cli.md) |
| **AI Agents & MCP Server** | Pure-Rust Model Context Protocol server (`opera mcp` or `uvx --from opera-rs opera mcp`) for Claude Code, Claude Desktop, Cursor, and VS Code with automatic tool exposure, inline PNG previews, `--json-usage`, and 4 workflow prompts. | [AI Agents (MCP)](mcp.md) |
| **Pure-Rust Local Geo I/O** | Read AOI vectors, write GeoJSON/GeoPackage/Shapefile outputs, and inspect local OPERA GeoTIFFs/COGs on disk via `terrio` with zero GDAL dependencies. | [Rust Library](rust.md) |

## Supported NASA OPERA Products

| Short Name | Alias | Backend | Default Band(s) | Description |
| --- | --- | --- | --- | --- |
| `OPERA_L3_DSWX-HLS_V1` | `dswx-hls` | `rasterio` | `B01_WTR` | 30 m optical surface water extent (HLS) |
| `OPERA_L3_DSWX-S1_V1` | `dswx-s1` | `rasterio` | `B01_WTR` | 30 m cloud-penetrating SAR surface water (Sentinel-1) |
| `OPERA_L3_DIST-ALERT-HLS_V1` | `dist-alert` | `rasterio` | `VEG-DIST-STATUS` | Near real-time 30 m land disturbance alerts |
| `OPERA_L3_DIST-ANN-HLS_V1` | `dist-ann` | `rasterio` | `VEG-DIST-STATUS` | Annual 30 m land disturbance summary |
| `OPERA_L2_RTC-S1_V1` | `rtc-s1` | `rasterio` | `VV`, `HH` | Radiometric terrain-corrected SAR backscatter |
| `OPERA_L2_RTC-S1-STATIC_V1` | `rtc-s1-static` | `rasterio` | `local_incidence_angle` | Static incidence angle and mask layers |
| `OPERA_L2_CSLC-S1_V1` | `cslc-s1` | `xarray` | - | Coregistered Single-Look Complex SAR bursts |
| `OPERA_L2_CSLC-S1-STATIC_V1` | `cslc-s1-static` | `xarray` | - | Static line-of-sight geometry layers |
| `OPERA_L3_DISP-S1_V1` | `disp-s1` | `xarray` | - | Line-of-sight surface displacement / subsidence |

## Quick Example

=== "CLI"

    ```sh
    # Search Tulare Lake surface water granules and save footprints as GeoPackage
    opera search --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
      --start 2023-04-20 --end 2023-04-30 -o tulare_footprints.gpkg --json

    # Compute 30 m categorical water class areas (km^2)
    opera stats --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
      --start 2023-04-20 --end 2023-04-30

    # Two-date change detection with Markdown report and comparison PNG
    opera change --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
      --before 2023-01-15 --after 2023-04-30 --preview-out tulare_change.png --markdown
    ```

=== "Python"

    ```python
    import opera_rs

    bbox = [-119.95, 35.85, -119.45, 36.25]

    # Search CMR granules and write footprints to GeoPackage
    res = opera_rs.search("dswx-hls", bbox=bbox, start="2023-04-20", end="2023-04-30")
    opera_rs.write_vector(res["feature_collection"], "tulare_footprints.gpkg")

    # Detect two-date surface-water expansion
    chg = opera_rs.detect_change(bbox, "2023-01-15", "2023-04-30", product="dswx-hls")
    print(f"Open water delta: {chg['open_water_delta_km2']:+.2f} km^2")
    ```

=== "Rust"

    ```rust,no_run
    use opera_rs::{BBox, CmrSearchParams, search_granules};

    fn main() -> opera_rs::Result<()> {
        let bbox = BBox::new(-119.95, 35.85, -119.45, 36.25)?;
        let res = search_granules(&CmrSearchParams {
            product: "dswx-hls".into(),
            bbox: Some(bbox),
            start: Some("2023-04-20".into()),
            end: Some("2023-04-30".into()),
            limit: 10,
            ..Default::default()
        })?;
        println!("Found {} hits across {} dates", res.hits, res.dates.len());
        Ok(())
    }
    ```

=== "MCP (AI Agents)"

    ```sh
    # Register with Claude Code (standalone binary)
    claude mcp add opera -- opera mcp

    # Or zero-install via uvx
    claude mcp add opera -- uvx --from opera-rs opera mcp
    ```

## Three Interfaces, One Pure-Rust Core

- **[Standalone CLI & MCP Server (`opera-cli`)](cli.md):** Single prebuilt binary for Linux, macOS, and Windows with 13 NASA OPERA subcommands, `--json` / `--json-usage`, and the `opera mcp` server.
- **[Rust Library (`opera-rs`)](rust.md):** Core crate on crates.io with modular `http`, `geo`, and `parallel` feature flags (`--no-default-features` compiles with zero C/C++ dependencies).
- **[Python Package (`opera-rs`)](python.md):** Prebuilt PyO3 ABI3 wheels on PyPI (Python 3.11+, zero mandatory Python dependencies) bundling both the Python API and the `opera` CLI/MCP server.
