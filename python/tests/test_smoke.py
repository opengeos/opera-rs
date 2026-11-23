"""Tests for the opera-rs Python bindings and bundled CLI/MCP entrypoint."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

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

    default_band_urls = opera_rs.tile_urls(
        "C2617126679-POCLOUD",
        product="dswx-hls",
    )
    assert "assets=B01_WTR" in default_band_urls["tilejson_url"]
    assert "colormap=" in default_band_urls["tilejson_url"]

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


def test_validation_errors(monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(ValueError, match="water_only requires product"):
        opera_rs.tile_urls("C2617126679-POCLOUD", water_only=True)

    with pytest.raises(ValueError, match="either bbox or place"):
        opera_rs.search(bbox=(-119.9, 35.8, -119.4, 36.3), place="Tulare Lake")

    with pytest.raises(ValueError, match="bbox is required"):
        opera_rs.stats(None)  # type: ignore[arg-type]

    with pytest.raises(ValueError, match="http:// or https://"):
        opera_rs.tile_urls("C2617126679-POCLOUD", endpoint="file:///etc/passwd")

    with pytest.raises(ValueError):
        opera_rs.tile_urls("C2617126679-POCLOUD", backend="invalid-backend")

    with pytest.raises(ValueError):
        opera_rs.tile_urls("C2617126679-POCLOUD", product="unknown-product")

    monkeypatch.setenv("OPERA_TITILER_ENDPOINT", "https://custom-titiler.example.org")
    custom_urls = opera_rs.tile_urls(
        "C2617126679-POCLOUD",
        product="rtc-s1",
        bands=["VV"],
        expression="10*log10(b1)",
    )
    assert custom_urls["tilejson_url"].startswith("https://custom-titiler.example.org/")
    assert "rescale=" not in custom_urls["tilejson_url"]

    xarray_urls = opera_rs.tile_urls(
        "C3294057315-ASF",
        backend="xarray",
        variables=["displacement"],
        group="/",
    )
    assert "/xarray/WebMercatorQuad/tilejson.json" in xarray_urls["tilejson_url"]
    assert "variables=displacement" in xarray_urls["tilejson_url"]
    with pytest.raises(ValueError, match="variables="):
        opera_rs.tile_urls("C3294057315-ASF", backend="xarray", bands=["VV"])


def test_bundled_cli_and_mcp_server_via_python_module(tmp_path: Path) -> None:
    usage_proc = subprocess.run(
        [sys.executable, "-P", "-m", "opera_rs", "--json-usage"],
        capture_output=True,
        text=True,
        check=True,
    )
    doc = json.loads(usage_proc.stdout)
    assert doc["program"] == "opera"

    # Plant a shadowing opera_rs package in cwd to verify `python -P -m opera_rs` ignores it
    # when `opera mcp` spawns child tool processes.
    decoy = tmp_path / "opera_rs"
    decoy.mkdir()
    (decoy / "__init__.py").write_text("raise RuntimeError('shadowed opera_rs imported')\n")

    mcp_proc = subprocess.Popen(
        [sys.executable, "-P", "-m", "opera_rs", "mcp"],
        cwd=tmp_path,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    request_line = (
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
    try:
        stdout, _stderr = mcp_proc.communicate(input=request_line, timeout=30)
    except subprocess.TimeoutExpired:
        mcp_proc.kill()
        mcp_proc.communicate()
        raise

    assert mcp_proc.returncode == 0
    reply_line = stdout.splitlines()[0]
    reply = json.loads(reply_line)
    assert reply["result"]["isError"] is False
    payload = json.loads(reply["result"]["content"][0]["text"])
    assert len(payload["products"]) == 9
