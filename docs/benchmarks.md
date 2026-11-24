# Benchmarks: `opera-rs` vs Python (`opera-utils` + `geopandas`)

`opera-rs` implements NASA OPERA Sentinel-1 burst/frame utilities, `opera-adt/burst_db` queries, missing-data subset optimization, DISP-S1 filename parsing, and GeoPackage vector I/O in pure Rust with zero GDAL, GEOS, PROJ, or HDF5 C/C++ dependencies.

This page compares `opera-rs` against the upstream Python [`opera-utils`](https://github.com/opera-adt/opera-utils) and [`geopandas`](https://geopandas.org) / [`pyogrio`](https://pyogrio.readthedocs.io) stack across package footprint, cold startup, Python API workloads, official `burst_db` v0.9.0 queries, end-to-end CLI commands, and local GeoPackage I/O.

## Methodology & Hardware

- **Machine:** Linux x86_64, Intel Xeon w7-3445 (40 logical cores)
- **Software:** Python 3.12.13, `opera-rs` 0.1.0 (release build), `opera-utils` 0.25.9, `geopandas` 1.2.0, `shapely` 2.2.0, `pyogrio` 0.13.0
- **Fairness controls:**
  - Both implementations read the exact same local cached `opera-adt/burst_db` v0.9.0 `.zip` archives on disk (zero network latency).
  - For archive queries, `opera_utils`'s in-process `@functools.cache` is cleared before each timed run so both tools measure actual ZIP decompression, JSON parsing, and query execution.
  - Every benchmark asserts output parity between `opera-rs` and `opera-utils` (identical burst counts, frame IDs, and optimal missing-data subset totals).
  - Timings report the **median** across 5 to 7 timed runs after warmup.
- **Reproduce locally:** Run [`benchmarks/benchmark_opera_utils.py`](https://github.com/opengeos/opera-rs/blob/main/benchmarks/benchmark_opera_utils.py):

```sh
pip install opera-rs "opera-utils[disp]" geopandas shapely
python benchmarks/benchmark_opera_utils.py
```

## 1. Installation & Dependency Footprint

Measured in clean Python 3.12 virtual environments installed from wheels (non-editable):

| Metric | `opera-rs` | `opera-utils[disp]` + `geopandas` | Difference |
| --- | --- | --- | --- |
| Installed packages | **1** | 56 | **56x fewer** |
| Installed `site-packages` size | **38 MB** (12 MB wheel, incl. `opera` CLI) | 446 MB | **11.7x smaller** |
| Native C geospatial dependencies | **None** (zero GDAL/GEOS/PROJ/HDF5) | GDAL, GEOS, PROJ, HDF5, libcurl, etc. | Zero system libs |

## 2. Cold Startup & CLI Overhead

Startup latency directly affects shell pipelines, serverless functions, and AI agent MCP tool calls where commands run as fresh subprocesses.

| Task | `opera-rs` | Python (`opera-utils`) | Speedup |
| --- | --- | --- | --- |
| Python `import` (fresh process) | **27.6 ms** | 523.9 ms (`opera_utils`) | **19.0x** |
| Python `import` + GeoPandas (fresh process) | **27.6 ms** | 514.7 ms (`opera_utils, geopandas`) | **18.6x** |
| CLI startup (`opera --version` vs `opera-utils --help`, PyPI shim) | **24.3 ms** | 828.8 ms | **34.1x** |
| CLI startup (standalone Rust `opera` binary vs `opera-utils`) | **3.5 ms** | 828.8 ms | **237x** |

## 3. Burst Parsing, DISP-S1 Filenames & Missing-Data Optimization (Python API)

In-process Python calls comparing `opera_rs` (PyO3 bindings) against `opera_utils`:

| Task | `opera-rs` (Python API) | `opera-utils` (Python) | Speedup |
| --- | --- | --- | --- |
| `get_burst_id` x 10,000 filenames (Python loop) | **3.2 ms** | 8.3 ms | **2.6x** |
| `group_by_burst` (10,000 CSLC filenames) | **7.2 ms** | 19.1 ms | **2.7x** |
| `parse_disp_filename` x 10,000 filenames (Python loop) | **33.2 ms** | 177.8 ms | **5.4x** |
| `get_dates` x 10,000 filenames (Python loop) | **5.3 ms** | 92.7 ms | **17.5x** |
| `get_missing_data_options` (636 CSLCs, 27 bursts x 24 dates) | **4.0 ms** | 149.9 ms | **37.3x** |
| `get_missing_data_options` (1,276 CSLCs, 27 bursts x 48 dates) | **48.4 ms** | 2,859.7 ms | **59.1x** |

## 4. Official `opera-adt/burst_db` v0.9.0 Archive Queries (Python API)

Single-call uncached lookups against the official `opera-adt/burst_db` v0.9.0 release ZIP archives (46,986 DISP-S1 frames):

| Task | `opera-rs` (Python API) | `opera-utils` (Python) | Speedup |
| --- | --- | --- | --- |
| `get_frame_to_burst_mapping(11114)` (`frame-to-burst.json.zip`, 4.3 MB) | **184.6 ms** | 215.5 ms | **1.2x** |
| `get_frame_ids_for_burst` (`burst-to-frame.json.zip`, 3.4 MB) | **683.3 ms** | 956.2 ms | **1.4x** |
| `get_frame_geojson` (3 of 46,986 frames, `frame-geometries-simple.geojson.zip`) | **138.1 ms** | 188.8 ms | **1.4x** |
| `get_intersecting_frames` (Tulare Lake bbox across all 46,986 frames) | **174.1 ms** | 406.4 ms | **2.3x** |

## 5. End-to-End CLI Workflows (Scripts & AI Agent MCP Calls)

Wall-clock latency of complete CLI invocations from cold process start to JSON/text output:

| Task | `opera` (PyPI shim) | `opera` (native Rust binary) | Python (`opera-utils`) | Speedup (native) |
| --- | --- | --- | --- | --- |
| Frame bbox lookup (`frame 11114 --latlon`) | **284.8 ms** | 337.5 ms | 1,150.2 ms | **3.4x to 4.0x** |
| Intersecting frames over AOI (46,986 frames) | **261.6 ms** | **255.5 ms** | 944.1 ms | **3.7x** |
| Missing-data optimization (1,276 CSLCs, 27x48) | **33.9 ms** | **11.7 ms** | 3,438.5 ms | **293.9x** |

## 6. Pure-Rust Vector I/O (`terrio`) vs GeoPandas / pyogrio (GDAL)

Writing and reading 500 DISP-S1 frame polygons to/from a local GeoPackage (`.gpkg`):

| Task | `opera-rs` (`terrio`, zero GDAL) | GeoPandas + `pyogrio` (GDAL) | Speedup |
| --- | --- | --- | --- |
| Read AOI bounding box from GeoPackage (500 polygons) | **2.0 ms** | 3.5 ms | **1.7x** |
| Write 500 frame polygons to GeoPackage (`.gpkg`) | 5.8 ms | **5.3 ms** | 0.9x (parity) |

## Key Takeaways

- **Combinatorial & CLI workloads see the biggest gains (19x to 294x):** Pure-Rust subset optimization (`missing-data`) avoids Python object allocation and NumPy per-iteration overhead, running **37x to 59x** faster from Python and **294x** faster via the standalone `opera` CLI. Cold CLI and MCP tool calls also skip Python's ~520 to 830 ms import overhead.
- **ZIP/JSON-bound `burst_db` queries are 1.2x to 2.3x faster uncached:** Reading a 3 to 4 MB compressed archive that inflates into tens of megabytes of JSON is largely bound by DEFLATE decompression and JSON parsing, where Rust's `flate2` + `serde_json` still beats Python's `zipfile` + `json` + `shapely` (especially on spatial intersection across all 46,986 polygons: 174 ms vs 406 ms). Note that for repeated in-process Python lookups against the same archive, `opera_utils` caches the parsed dictionary in memory via `@functools.cache`.
- **GeoPackage I/O matches C GDAL with zero GDAL dependencies:** Through `terrio`, `opera-rs` reads GeoPackage AOI bounds **1.7x faster** than GeoPandas/`pyogrio` and writes 500 polygons in **5.8 ms** (matching GDAL's 5.3 ms) while shipping a single 12 MB wheel with zero external dependencies.
