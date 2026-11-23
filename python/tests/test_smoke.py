"""Tests for the opera-rs Python bindings and bundled CLI/MCP entrypoint."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import opera_rs


def test_version_and_product_catalog() -> None:
    assert isinstance(opera_rs.__version__, str)
    assert opera_rs.__version__ == opera_rs.version()

    all_products = opera_rs.products()
    assert len(all_products) == 9
    short_names = {p["short_name"] for p in all_products}
    assert "OPERA_L3_DSWX-HLS_V1" in short_names
    assert "OPERA_L3_DSWX-S1_V1" in short_names
    assert "OPERA_L3_DISP-S1_V1" in short_names

    dswx = opera_rs.find_product("dswx-s1")
    assert dswx["short_name"] == "OPERA_L3_DSWX-S1_V1"

    presets = opera_rs.expression_presets("OPERA_L2_RTC-S1_V1", "VV")
    assert len(presets) == 1
    assert "log10" in presets[0]["expression"]


def test_offline_tile_urls_and_vector_io(tmp_path: Path) -> None:
    urls = opera_rs.tile_urls(
        "C2617126679-POCLOUD",
        product="OPERA_L3_DSWX-HLS_V1",
        bands=["B01_WTR"],
        water_only=True,
    )
    assert "WebMercatorQuad/tilejson.json" in urls["tilejson_url"]
    assert "{z}/{x}/{y}.png" in urls["tile_url_template"]

    fc = {
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [
                        [
                            [-119.9, 35.8],
                            [-119.4, 35.8],
                            [-119.4, 36.3],
                            [-119.9, 36.3],
                            [-119.9, 35.8],
                        ]
                    ],
                },
                "properties": {"name": "tulare"},
            }
        ],
    }
    gpkg = tmp_path / "aoi.gpkg"
    count = opera_rs.write_vector(fc, gpkg, layer_name="aoi")
    assert count == 1
    bbox = opera_rs.read_aoi_bbox(gpkg)
    assert abs(bbox[0] - (-119.9)) < 1e-6
    assert abs(bbox[3] - 36.3) < 1e-6


def test_bundled_cli_and_mcp_server_via_python_module() -> None:
    usage_proc = subprocess.run(
        [sys.executable, "-m", "opera_rs", "--json-usage"],
        capture_output=True,
        text=True,
        check=True,
    )
    doc = json.loads(usage_proc.stdout)
    assert doc["program"] == "opera"

    mcp_proc = subprocess.Popen(
        [sys.executable, "-m", "opera_rs", "mcp"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    assert mcp_proc.stdin is not None and mcp_proc.stdout is not None
    mcp_proc.stdin.write(
        json.dumps(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": "products", "arguments": {"args": ["--json"]}},
            }
        )
        + "\n"
    )
    mcp_proc.stdin.flush()
    reply_line = mcp_proc.stdout.readline()
    mcp_proc.stdin.close()
    mcp_proc.wait(timeout=15)
    assert mcp_proc.returncode == 0
    reply = json.loads(reply_line)
    assert reply["result"]["isError"] is False
    payload = json.loads(reply["result"]["content"][0]["text"])
    assert len(payload["products"]) == 9
