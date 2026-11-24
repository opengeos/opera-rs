# opera-rs

[![CI](https://github.com/opengeos/opera-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/opengeos/opera-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/opera-rs.svg)](https://crates.io/crates/opera-rs)
[![PyPI](https://img.shields.io/pypi/v/opera-rs.svg)](https://pypi.org/project/opera-rs/)
[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb)
[![Docs](https://img.shields.io/badge/docs-opera--rs.opengeos.org-teal.svg)](https://opera-rs.opengeos.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Pure-Rust, agent-centric toolkit for searching, visualizing, and analyzing [NASA OPERA](https://www.jpl.nasa.gov/go/opera) satellite products (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1), powered by [`terrio`](https://github.com/opengeos/terrio), NASA's public CMR API, and [`titiler-cmr`](https://github.com/developmentseed/titiler-cmr) with **zero GDAL dependencies** (`--no-default-features` has zero C/C++ dependencies) and **no Earthdata login required**.

**Documentation:** [Installation](https://opera-rs.opengeos.org/installation/) | [Notebooks](https://opera-rs.opengeos.org/notebooks/) | [CLI](https://opera-rs.opengeos.org/cli/) | [AI Agents (MCP)](https://opera-rs.opengeos.org/mcp/) | [Rust](https://opera-rs.opengeos.org/rust/) | [Python](https://opera-rs.opengeos.org/python/) | [Benchmarks](https://opera-rs.opengeos.org/benchmarks/)

## Highlights

- **All 9 NASA OPERA products:** `DSWx-HLS`, `DSWx-S1` (cloud-penetrating SAR water), `DIST-ALERT`, `DIST-ANN`, `RTC-S1`, `RTC-S1-STATIC`, `CSLC-S1`, `CSLC-S1-STATIC`, and `DISP-S1`, with built-in categorical colormaps (`B01_WTR`, `--water-only`, `VEG-DIST-STATUS`), band defaults, and `rio-tiler` band-math expression presets (`10*log10(b1)` for SAR dB, water masks).
- **Public CMR search & place geocoding (`search`, `geocode`):** Paginated NASA CMR UMM-G granule discovery with reprocessed-version deduplication, newest-per-tile filtering, Nominatim place geocoding, and AOI input from `--bbox`, `--place`, or any GeoJSON / GeoPackage / Shapefile (`--aoi-file`, auto-reprojected to WGS 84 via `terrio`).
- **Backend-aware `titiler-cmr` analytics (`stats`, `point`, `tilejson`):** Supports both `rasterio` (COG/GeoTIFF) and `xarray` (NetCDF/HDF5/Zarr) backends without local Earthdata credentials. Categorical statistics automatically report 30 m class pixel counts, percentages, and surface area in km^2.
- **Change detection & time-series trends (`change`, `timeseries`):** Find nearest before/after scenes within a search window, compute class area deltas (km^2) or continuous band shifts, export Markdown/CSV reports, and compute OLS temporal trend slopes.
- **Observed flood-extent vectorization (`flood-extent`):** Union DSWx-HLS and cloud-penetrating DSWx-S1 water-only tiles, stitch into a binary mask, trace boundary rings with nested dry-land holes and RFC 7946 winding, and write polygons to GeoJSON, GeoPackage (`.gpkg`), or Shapefile (`.shp`).
- **Pure-Rust visualization (`preview`, `animate`, `export-html`):** Render static PNG previews (with CARTO Voyager basemap, categorical legend, metric scale bar, and title banner), side-by-side Before/After comparison PNGs, multi-date animated GIFs, and self-contained interactive MapLibre GL JS HTML viewers.
- **Sentinel-1 burst/frame utilities & DISP-S1 stacks (`opera-utils` integration):** Pure-Rust Sentinel-1 burst ID/date parsing (`burst-id`), `opera-adt/burst_db` v0.9.0 DISP-S1 frame lookups and spatial intersection (`frame`, `frames-intersect`), optimal missing-data subset selection (`missing-data`), and DISP-S1 CMR frame search with version deduplication (`disp-search`).
- **AI Agents & MCP server (`opera mcp`):** Pure-Rust Model Context Protocol (MCP) server over stdio modeled after `terrio mcp` and `lidar mcp` (`claude mcp add opera -- opera mcp` or zero-install via `uvx --from opera-rs opera mcp`), with automatic Clap tool exposure, inline PNG `preview` image attachments, `--json-usage` command schema, and 4 curated workflow prompts.
- **Three interfaces, one pure-Rust core:** Rust crate (`opera-rs` on crates.io), standalone CLI & MCP server (`opera-cli` -> `opera` binary), and Python package (`opera-rs` on PyPI, PyO3 ABI3 wheels for Python 3.11+ with bundled `opera` CLI/MCP).

## Supported NASA OPERA Products

| Short Name | Alias | Backend | Default Band(s) | Description |
|---|---|---|---|---|
| `OPERA_L3_DSWX-HLS_V1` | `dswx-hls` | `rasterio` | `B01_WTR` | 30 m optical surface water extent (HLS) |
| `OPERA_L3_DSWX-S1_V1` | `dswx-s1` | `rasterio` | `B01_WTR` | 30 m cloud-penetrating SAR surface water (Sentinel-1) |
| `OPERA_L3_DIST-ALERT-HLS_V1` | `dist-alert` | `rasterio` | `VEG-DIST-STATUS` | Near real-time 30 m land disturbance alerts |
| `OPERA_L3_DIST-ANN-HLS_V1` | `dist-ann` | `rasterio` | `VEG-DIST-STATUS` | Annual 30 m land disturbance summary |
| `OPERA_L2_RTC-S1_V1` | `rtc-s1` | `rasterio` | `VV`, `HH` | Radiometric terrain-corrected SAR backscatter |
| `OPERA_L2_RTC-S1-STATIC_V1` | `rtc-s1-static` | `rasterio` | `local_incidence_angle` | Static incidence angle and mask layers |
| `OPERA_L2_CSLC-S1_V1` | `cslc-s1` | `xarray` | - | Coregistered Single-Look Complex SAR bursts |
| `OPERA_L2_CSLC-S1-STATIC_V1` | `cslc-s1-static` | `xarray` | - | Static line-of-sight geometry layers |
| `OPERA_L3_DISP-S1_V1` | `disp-s1` | `xarray` | - | Line-of-sight surface displacement / subsidence |

## Installation

```sh
# Prebuilt standalone CLI (Linux / macOS)
curl -LsSf https://opera-rs.opengeos.org/install.sh | sh

# Prebuilt standalone CLI (Windows PowerShell)
powershell -ExecutionPolicy ByPass -c "irm https://opera-rs.opengeos.org/install.ps1 | iex"

# Python package (includes bundled `opera` CLI and `opera mcp`)
pip install opera-rs

# Rust CLI from crates.io
cargo install opera-cli
```

## AI Agents (MCP)

Connect `opera mcp` to Claude Code, Claude Desktop, Cursor, or VS Code:

```sh
# Standalone binary
claude mcp add opera -- opera mcp

# Zero-install via uvx
claude mcp add opera -- uvx --from opera-rs opera mcp
```

## CLI Quick Start

```sh
# Search DSWx-HLS over Tulare Lake, April 2023 and save footprints as GeoPackage
opera search --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30 -o tulare_footprints.gpkg --json

# Compute 30 m categorical water class areas (km^2)
opera stats --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30

# Detect two-date surface water expansion with side-by-side comparison PNG
opera change --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --before 2023-01-15 --after 2023-04-30 --preview-out tulare_change.png --markdown

# Vectorize observed flood extent (union of DSWx-HLS + DSWx-S1)
opera flood-extent --place "Valencia Region, Spain" \
  --start 2024-10-27 --end 2024-11-05 -o valencia_flood.geojson

# Render a PNG map preview or interactive MapLibre HTML viewer
opera preview --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30 --water-only -o tulare.png
opera export-html --product dswx-hls --bbox -119.95,35.85,-119.45,36.25 \
  --start 2023-04-20 --end 2023-04-30 --water-only -o tulare.html
```

## Rust Usage

```toml
[dependencies]
opera-rs = "0.1"
```

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

## Python Usage

```python
import opera_rs

bbox = [-119.95, 35.85, -119.45, 36.25]
res = opera_rs.search("dswx-hls", bbox=bbox, start="2023-04-20", end="2023-04-30")
change = opera_rs.detect_change(bbox, "2023-01-15", "2023-04-30", product="dswx-hls")
print("Open water delta (km^2):", change["open_water_delta_km2"])
```

## Benchmarks (`opera-rs` vs Python `opera-utils` + `geopandas`)

Measured on Linux x86_64 (Python 3.12, release build) against identical local cached `opera-adt/burst_db` v0.9.0 archives. See the full [Benchmarks](https://opera-rs.opengeos.org/benchmarks/) page and [`benchmarks/benchmark_opera_utils.py`](benchmarks/benchmark_opera_utils.py) to reproduce.

| Benchmark | `opera-rs` | Python (`opera-utils` + `geopandas`) | Speedup / Savings |
| --- | --- | --- | --- |
| Installed footprint (wheel / `site-packages`) | **1 pkg / 38 MB** (0 GDAL/C deps) | 56 pkgs / 446 MB | **11.7x smaller** |
| Python `import` (fresh process) | **27.6 ms** | 523.9 ms | **19.0x faster** |
| CLI cold start (standalone `opera` vs `opera-utils`) | **3.5 ms** | 828.8 ms | **237x faster** |
| `get_dates` x 10,000 CSLC filenames (Python API) | **5.3 ms** | 92.7 ms | **17.5x faster** |
| `get_missing_data_options` (1,276 CSLCs, 27x48, Python API) | **48.4 ms** | 2,859.7 ms | **59.1x faster** |
| End-to-end CLI `missing-data` (1,276 CSLCs, native binary) | **11.7 ms** | 3,438.5 ms | **293.9x faster** |
| `get_intersecting_frames` (46,986 frames, zip) | **174.1 ms** | 406.4 ms | **2.3x faster** |
| Read AOI bbox from GeoPackage (500 polygons) | **2.0 ms** | 3.5 ms (`pyogrio`/GDAL) | **1.7x faster** |

## Interactive Notebooks

- **Core OPERA Search, Analytics & Visualization:** [![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_rs_demo.ipynb) ([`examples/opera_rs_demo.ipynb`](examples/opera_rs_demo.ipynb))
- **Sentinel-1 Bursts, DISP-S1 Frames & Missing-Data Optimization:** [![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/opengeos/opera-rs/blob/main/examples/opera_utils_demo.ipynb) ([`examples/opera_utils_demo.ipynb`](examples/opera_utils_demo.ipynb))

## Workspace Layout

| Path | Description |
|---|---|
| `crates/opera-rs` | Pure-Rust core library (published to crates.io as `opera-rs`) |
| `crates/opera-cli` | Standalone `opera` CLI and MCP server (published to crates.io as `opera-cli`) |
| `python` | Python bindings (`opera-rs` on PyPI, PyO3) with bundled `opera` CLI and MCP server |
| `benchmarks` | Reproducible benchmark script comparing `opera-rs` against `opera-utils` and `geopandas` |
| `examples` | Interactive Jupyter / Google Colab demo notebooks (`opera_rs_demo.ipynb`, `opera_utils_demo.ipynb`) |
| `docs` | Zensical documentation site (`opera-rs.opengeos.org`) and installer scripts |

## License

`opera-rs` is licensed under the [MIT License](LICENSE). See [NOTICE](NOTICE) for third-party acknowledgments and license attributions.
