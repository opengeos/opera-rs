#!/usr/bin/env python3
"""Reproducible benchmarks comparing opera-rs against Python opera-utils + GeoPandas.

Usage:
    python benchmarks/benchmark_opera_utils.py
    python benchmarks/benchmark_opera_utils.py --repeats 7 --json-out results.json

Requirements:
    pip install opera-rs "opera-utils[disp]" geopandas shapely
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable

import geopandas as gpd
import opera_rs
import opera_utils
from opera_utils import burst_frame_db, datasets, disp, missing_data


def bench(fn: Callable[[], Any], repeats: int = 7, warmup: int = 2) -> tuple[float, Any]:
    """Return median runtime in milliseconds across ``repeats`` after ``warmup`` runs."""
    out = None
    for _ in range(warmup):
        out = fn()
    times_ms: list[float] = []
    for _ in range(repeats):
        t0 = time.perf_counter()
        out = fn()
        times_ms.append((time.perf_counter() - t0) * 1000.0)
    return statistics.median(times_ms), out


def bench_proc(cmd: list[str], repeats: int = 7, warmup: int = 2) -> float:
    """Return median wall-clock runtime in milliseconds for a subprocess command."""
    for _ in range(warmup):
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    times_ms: list[float] = []
    for _ in range(repeats):
        t0 = time.perf_counter()
        subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        times_ms.append((time.perf_counter() - t0) * 1000.0)
    return statistics.median(times_ms)


def clear_opera_utils_caches() -> None:
    """Clear @functools.cache entries so archive decompression and parsing are measured fairly."""
    for fn in (
        burst_frame_db.read_zipped_json,
        burst_frame_db.get_frame_to_burst_mapping,
        burst_frame_db.get_burst_to_frame_mapping,
    ):
        if hasattr(fn, "cache_clear"):
            fn.cache_clear()


def build_cslc_stack(num_dates: int, gaps: dict[int, set[int]]) -> list[str]:
    """Generate realistic OPERA_L2_CSLC-S1 filenames for a 27-burst frame with missing dates."""
    burst_ids = [f"t042_{88900 + i:06d}_iw{(i % 3) + 1}" for i in range(27)]
    dates = [
        f"{2023 + (d // 24)}{((d % 24) // 2) + 1:02d}{((d % 2) * 12) + 1:02d}"
        for d in range(num_dates)
    ]
    files: list[str] = []
    for bi, burst in enumerate(burst_ids):
        for di, dt in enumerate(dates):
            if di in gaps.get(bi, ()):
                continue
            files.append(
                f"OPERA_L2_CSLC-S1_{burst.upper().replace('_', '-')}_"
                f"{dt}T140000Z_20240504T185235Z_S1A_VV_v1.1.h5"
            )
    return files


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeats", type=int, default=7, help="Timed repetitions per benchmark")
    parser.add_argument("--warmup", type=int, default=2, help="Warmup runs before timing")
    parser.add_argument(
        "--opera-bin",
        default=shutil.which("opera") or str(Path(sys.executable).parent / "opera"),
        help="Path to the opera CLI executable",
    )
    parser.add_argument(
        "--opera-utils-bin",
        default=shutil.which("opera-utils") or str(Path(sys.executable).parent / "opera-utils"),
        help="Path to the opera-utils CLI executable",
    )
    parser.add_argument("--json-out", type=Path, default=None, help="Optional JSON output path")
    args = parser.parse_args()

    py_exe = sys.executable
    repeats = max(1, args.repeats)
    warmup = max(0, args.warmup)
    heavy_repeats = min(repeats, 5)
    heavy_warmup = min(warmup, 1)

    print(f"Host: {platform.system()} {platform.machine()}, Python {platform.python_version()}")
    print(f"CPU: {os.cpu_count()} logical cores")
    print(f"opera-rs: {opera_rs.__version__} | opera-utils: {opera_utils.__version__}")
    print()

    # Pre-fetch official burst_db v0.9.0 archives once so both sides read identical local files.
    f2b_path = str(datasets.fetch_frame_to_burst_mapping_file())
    b2f_path = str(datasets.fetch_burst_to_frame_mapping_file())
    frame_geom_path = str(datasets.fetch_frame_geometries_simple())

    results: list[dict[str, Any]] = []

    def record(category: str, task: str, rs_ms: float, py_ms: float, note: str = "") -> None:
        speedup = py_ms / rs_ms if rs_ms > 0 else float("inf")
        results.append(
            {
                "category": category,
                "task": task,
                "opera_rs_ms": round(rs_ms, 2),
                "python_ms": round(py_ms, 2),
                "speedup": round(speedup, 1),
                "note": note,
            }
        )
        print(
            f"[{category:12s}] {task:48s} | "
            f"opera-rs: {rs_ms:8.2f} ms | python: {py_ms:8.2f} ms | {speedup:6.1f}x {note}"
        )

    # 1. Cold startup (fresh process, warm OS page cache)
    rs_imp = bench_proc([py_exe, "-c", "import opera_rs"], repeats, warmup)
    py_imp = bench_proc([py_exe, "-c", "import opera_utils"], repeats, warmup)
    py_imp_geo = bench_proc([py_exe, "-c", "import opera_utils, geopandas"], repeats, warmup)
    record("Startup", "Python `import` (fresh process)", rs_imp, py_imp)
    record("Startup", "Python `import` + GeoPandas (fresh process)", rs_imp, py_imp_geo)

    if Path(args.opera_bin).exists() and Path(args.opera_utils_bin).exists():
        rs_cli = bench_proc([args.opera_bin, "--version"], repeats, warmup)
        py_cli = bench_proc([args.opera_utils_bin, "--help"], repeats, warmup)
        record("Startup", "CLI startup (`--version` / `--help`)", rs_cli, py_cli)

    # 2. Burst ID & date parsing (10,000 realistic CSLC-S1 filenames)
    cslc_files = [
        f"OPERA_L2_CSLC-S1_T{(i % 175) + 1:03d}-{185000 + (i % 900):06d}-IW{(i % 3) + 1}_"
        f"2023{(i % 12) + 1:02d}{(i % 28) + 1:02d}T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
        for i in range(10_000)
    ]
    rs_grp, rs_grp_out = bench(lambda: opera_rs.group_by_burst(cslc_files), repeats, warmup)
    py_grp, py_grp_out = bench(lambda: opera_utils.group_by_burst(cslc_files), repeats, warmup)
    assert len(rs_grp_out) == len(py_grp_out)
    record("Bursts", "group_by_burst (10,000 CSLC filenames)", rs_grp, py_grp)

    rs_bid, _ = bench(lambda: [opera_rs.get_burst_id(f) for f in cslc_files], repeats, warmup)
    py_bid, _ = bench(lambda: [opera_utils.get_burst_id(f) for f in cslc_files], repeats, warmup)
    record("Bursts", "get_burst_id x 10,000 (Python loop)", rs_bid, py_bid)

    rs_dates, _ = bench(lambda: [opera_rs.get_dates(f) for f in cslc_files], repeats, warmup)
    py_dates, _ = bench(lambda: [opera_utils.get_dates(f) for f in cslc_files], repeats, warmup)
    record("Bursts", "get_dates x 10,000 (Python loop)", rs_dates, py_dates)

    # 3. DISP-S1 filename parsing (10,000 filenames)
    disp_files = [
        f"OPERA_L3_DISP-S1_IW_F{11114 + (i % 50):05d}_VV_"
        f"20160705T140755Z_2023{(i % 12) + 1:02d}{(i % 28) + 1:02d}T140812Z_"
        f"v1.0_20241219T231545Z.nc"
        for i in range(10_000)
    ]
    rs_disp, _ = bench(
        lambda: [opera_rs.parse_disp_filename(f) for f in disp_files], heavy_repeats, heavy_warmup
    )
    py_disp, _ = bench(
        lambda: [disp.DispProduct.from_filename(f) for f in disp_files], heavy_repeats, heavy_warmup
    )
    record("DISP-S1", "parse_disp_filename x 10,000 (Python loop)", rs_disp, py_disp)

    # 4. Missing-data subset optimization
    stack_24 = build_cslc_stack(
        24,
        {0: {2, 7, 15}, 5: {4, 15}, 11: {7, 19}, 18: {11, 22}, 24: {2, 11, 22}},
    )
    rs_md1, rs_md1_out = bench(lambda: opera_rs.get_missing_data_options(stack_24), repeats, warmup)
    py_md1, py_md1_out = bench(
        lambda: missing_data.get_missing_data_options(stack_24), repeats, warmup
    )
    assert rs_md1_out[0]["total_num_bursts"] == py_md1_out[0].total_num_bursts
    record("Missing Data", f"get_missing_data_options ({len(stack_24)} CSLCs, 27x24)", rs_md1, py_md1)

    stack_48 = build_cslc_stack(
        48,
        {
            0: {2, 11, 25},
            3: {5, 19},
            7: {8, 25, 41},
            11: {11, 30},
            15: {14, 33, 45},
            19: {19, 38},
            22: {22, 41},
            25: {2, 30, 45},
        },
    )
    rs_md2, rs_md2_out = bench(
        lambda: opera_rs.get_missing_data_options(stack_48), heavy_repeats, heavy_warmup
    )
    py_md2, py_md2_out = bench(
        lambda: missing_data.get_missing_data_options(stack_48), heavy_repeats, heavy_warmup
    )
    assert rs_md2_out[0]["total_num_bursts"] == py_md2_out[0].total_num_bursts
    record("Missing Data", f"get_missing_data_options ({len(stack_48)} CSLCs, 27x48)", rs_md2, py_md2)

    # 5. Official opera-adt/burst_db v0.9.0 local archive queries (uncached)
    rs_f2b, rs_f2b_out = bench(
        lambda: opera_rs.get_frame_to_burst_mapping(11114, json_file=f2b_path),
        heavy_repeats,
        heavy_warmup,
    )
    py_f2b, py_f2b_out = bench(
        lambda: (clear_opera_utils_caches(), burst_frame_db.get_frame_to_burst_mapping(11114, json_file=f2b_path))[1],
        heavy_repeats,
        heavy_warmup,
    )
    assert len(rs_f2b_out["burst_ids"]) == len(py_f2b_out["burst_ids"])
    record("burst_db", "get_frame_to_burst_mapping(11114) [zip]", rs_f2b, py_f2b)

    rs_b2f, rs_b2f_out = bench(
        lambda: opera_rs.get_frame_ids_for_burst("t042_088905_iw1", json_file=b2f_path),
        heavy_repeats,
        heavy_warmup,
    )
    py_b2f, py_b2f_out = bench(
        lambda: (clear_opera_utils_caches(), burst_frame_db.get_frame_ids_for_burst("t042_088905_iw1", json_file=b2f_path))[1],
        heavy_repeats,
        heavy_warmup,
    )
    assert sorted(rs_b2f_out) == sorted(py_b2f_out)
    record("burst_db", "get_frame_ids_for_burst [zip]", rs_b2f, py_b2f)

    rs_fg, rs_fg_out = bench(
        lambda: opera_rs.get_frame_geojson([11114, 11115, 11116], json_file=frame_geom_path),
        heavy_repeats,
        heavy_warmup,
    )
    py_fg, py_fg_out = bench(
        lambda: (clear_opera_utils_caches(), burst_frame_db.get_frame_geojson([11114, 11115, 11116], json_file=frame_geom_path))[1],
        heavy_repeats,
        heavy_warmup,
    )
    assert len(rs_fg_out["features"]) == len(py_fg_out["features"]) == 3
    record("burst_db", "get_frame_geojson (3 of 46,986 frames) [zip]", rs_fg, py_fg)

    tulare_bbox = (-119.95, 35.85, -119.45, 36.25)
    rs_int, rs_int_out = bench(
        lambda: opera_rs.get_intersecting_frames(tulare_bbox, json_file=frame_geom_path),
        heavy_repeats,
        heavy_warmup,
    )
    py_int, py_int_out = bench(
        lambda: (clear_opera_utils_caches(), burst_frame_db.get_intersecting_frames(tulare_bbox))[1],
        heavy_repeats,
        heavy_warmup,
    )
    assert sorted(int(f["id"]) for f in rs_int_out["features"]) == sorted(
        int(f["id"]) for f in py_int_out["features"]
    )
    record("burst_db", "get_intersecting_frames (46,986 frames) [zip]", rs_int, py_int)

    # 6. End-to-end CLI workflows and GeoPackage Vector I/O
    with tempfile.TemporaryDirectory() as td:
        if Path(args.opera_bin).exists() and Path(args.opera_utils_bin).exists():
            rs_cli_frame = bench_proc(
                [args.opera_bin, "frame", "11114", "--latlon", "--db-file", f2b_path, "--json"],
                heavy_repeats,
                heavy_warmup,
            )
            py_cli_frame = bench_proc(
                [args.opera_utils_bin, "disp-s1-frame-bbox", "11114", "--latlon"],
                heavy_repeats,
                heavy_warmup,
            )
            record("CLI (E2E)", "Frame bbox lookup (frame 11114, --latlon)", rs_cli_frame, py_cli_frame)

            rs_cli_int = bench_proc(
                [
                    args.opera_bin,
                    "frames-intersect",
                    "--bbox",
                    "-119.95,35.85,-119.45,36.25",
                    "--ids-only",
                    "--db-file",
                    frame_geom_path,
                ],
                heavy_repeats,
                heavy_warmup,
            )
            py_cli_int = bench_proc(
                [
                    py_exe,
                    "-c",
                    "import opera_utils; opera_utils.get_intersecting_frames((-119.95, 35.85, -119.45, 36.25))",
                ],
                heavy_repeats,
                heavy_warmup,
            )
            record("CLI (E2E)", "Intersecting frames over AOI (46,986 frames)", rs_cli_int, py_cli_int)

            list_txt = Path(td) / "cslc_list.txt"
            list_txt.write_text("\n".join(stack_48) + "\n")
            rs_cli_md = bench_proc(
                [args.opera_bin, "missing-data", "--file", str(list_txt), "--json"],
                heavy_repeats,
                heavy_warmup,
            )
            py_cli_md = bench_proc(
                [
                    py_exe,
                    "-c",
                    f"from pathlib import Path; from opera_utils.missing_data import get_missing_data_options; "
                    f"get_missing_data_options(Path({str(list_txt)!r}).read_text().splitlines())",
                ],
                heavy_repeats,
                heavy_warmup,
            )
            record("CLI (E2E)", f"Missing-data optimization ({len(stack_48)} CSLCs)", rs_cli_md, py_cli_md)

        fc_500 = opera_rs.get_frame_geojson(list(range(10000, 10500)), json_file=frame_geom_path)
        gdf_500 = gpd.GeoDataFrame.from_features(fc_500["features"], crs="EPSG:4326")
        gpkg_rs = str(Path(td) / "rs.gpkg")
        gpkg_py = str(Path(td) / "py.gpkg")

        rs_w, _ = bench(
            lambda: opera_rs.write_vector(fc_500, gpkg_rs, layer_name="frames"),
            heavy_repeats,
            heavy_warmup,
        )
        py_w, _ = bench(
            lambda: gdf_500.to_file(gpkg_py, driver="GPKG", layer="frames"),
            heavy_repeats,
            heavy_warmup,
        )
        record("Vector I/O", "Write 500 frame polygons to GeoPackage (.gpkg)", rs_w, py_w)

        rs_r, _ = bench(lambda: opera_rs.read_aoi_bbox(gpkg_rs), heavy_repeats, heavy_warmup)
        py_r, _ = bench(lambda: list(gpd.read_file(gpkg_py).total_bounds), heavy_repeats, heavy_warmup)
        record("Vector I/O", "Read AOI bbox from GeoPackage (500 polys)", rs_r, py_r)

    if args.json_out is not None:
        args.json_out.write_text(json.dumps(results, indent=2) + "\n")
        print(f"\nWrote JSON results to {args.json_out}")


if __name__ == "__main__":
    main()
