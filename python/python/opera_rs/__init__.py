"""Python bindings for opera-rs (NASA OPERA search, visualization, analysis, and MCP server)."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from opera_rs import _opera_rs
from opera_rs._opera_rs import __version__, version

BBoxLike = tuple[float, float, float, float] | list[float]


def _bbox_tuple(bbox: BBoxLike | None) -> tuple[float, float, float, float] | None:
    if bbox is None:
        return None
    vals = [float(x) for x in bbox]
    if len(vals) != 4:
        raise ValueError("bbox must contain 4 numbers: (west, south, east, north)")
    return (vals[0], vals[1], vals[2], vals[3])


def _require_bbox(bbox: BBoxLike) -> tuple[float, float, float, float]:
    resolved = _bbox_tuple(bbox)
    if resolved is None:
        raise ValueError("bbox is required")
    return resolved


def products() -> list[dict[str, Any]]:
    """Return metadata for all 9 supported NASA OPERA products."""
    return json.loads(_opera_rs.catalog_json())


def find_product(query: str) -> dict[str, Any]:
    """Look up a NASA OPERA product by CMR short name or alias (e.g. ``'dswx-hls'``)."""
    return json.loads(_opera_rs.find_product_json(query))


def expression_presets(product: str, band: str) -> list[dict[str, Any]]:
    """Return ready-made ``rio-tiler`` band-math expression presets for a product and band."""
    return json.loads(_opera_rs.expression_presets_json(product, band))


def geocode(place: str, *, endpoint: str | None = None) -> dict[str, Any]:
    """Geocode a place name to a WGS 84 bounding box via OpenStreetMap Nominatim."""
    return json.loads(_opera_rs.geocode_json(place, endpoint))


def search(
    product: str = "OPERA_L3_DSWX-HLS_V1",
    *,
    bbox: BBoxLike | None = None,
    place: str | None = None,
    start: str | None = None,
    end: str | None = None,
    limit: int = 50,
    deduplicate_versions: bool = True,
    newest_per_tile: bool = False,
    cmr_endpoint: str | None = None,
    nominatim_endpoint: str | None = None,
) -> dict[str, Any]:
    """Search public NASA CMR for OPERA granules, footprints, dates, and bands."""
    if bbox is not None and place is not None:
        raise ValueError("specify either bbox or place, not both")
    resolved_bbox = _bbox_tuple(bbox)
    if resolved_bbox is None and place is not None:
        geo = geocode(place, endpoint=nominatim_endpoint)["bbox"]
        resolved_bbox = (geo["west"], geo["south"], geo["east"], geo["north"])
    return json.loads(
        _opera_rs.search_json(
            product,
            resolved_bbox,
            start,
            end,
            limit,
            deduplicate_versions,
            newest_per_tile,
            cmr_endpoint,
        )
    )


def tile_urls(
    concept_id: str,
    *,
    product: str | None = None,
    backend: str = "rasterio",
    granule_ur: str | None = None,
    temporal: str | None = None,
    bands: list[str] | None = None,
    variables: list[str] | None = None,
    group: str | None = None,
    rescale: str | None = None,
    colormap: str | None = None,
    water_only: bool = False,
    expression: str | None = None,
    endpoint: str | None = None,
    tile_size: int = 512,
) -> dict[str, str]:
    """Build ``titiler-cmr`` TileJSON and XYZ tile URL templates offline."""
    return json.loads(
        _opera_rs.tile_urls_json(
            concept_id,
            product,
            backend,
            granule_ur,
            temporal,
            bands,
            variables,
            group,
            rescale,
            colormap,
            water_only,
            expression,
            endpoint,
            tile_size,
        )
    )


def tilejson(
    product: str = "OPERA_L3_DSWX-HLS_V1",
    *,
    concept_id: str | None = None,
    granule_ur: str | None = None,
    band: str | None = None,
    water_only: bool = False,
    endpoint: str | None = None,
) -> dict[str, Any]:
    """Fetch a TileJSON document from ``titiler-cmr``."""
    return json.loads(
        _opera_rs.fetch_tilejson_json(
            product, concept_id, granule_ur, band, water_only, endpoint
        )
    )


def point(
    lon: float,
    lat: float,
    *,
    product: str = "OPERA_L3_DSWX-HLS_V1",
    concept_id: str | None = None,
    granule_ur: str | None = None,
    temporal: str | None = None,
    band: str | None = None,
    expression: str | None = None,
    endpoint: str | None = None,
) -> dict[str, Any]:
    """Sample pixel values (with categorical class labels) at ``(lon, lat)``."""
    return json.loads(
        _opera_rs.point_json(
            product,
            float(lon),
            float(lat),
            concept_id,
            granule_ur,
            temporal,
            band,
            expression,
            endpoint,
        )
    )


def stats(
    bbox: BBoxLike,
    *,
    product: str = "OPERA_L3_DSWX-HLS_V1",
    concept_id: str | None = None,
    granule_ur: str | None = None,
    start: str | None = None,
    end: str | None = None,
    band: str | None = None,
    expression: str | None = None,
    categorical: bool | None = None,
    histogram_bins: int | None = None,
    endpoint: str | None = None,
) -> dict[str, Any]:
    """Compute zonal AOI statistics and 30 m categorical class area breakdowns.

    When ``granule_ur`` is provided, ``start`` and ``end`` are ignored because the request
    is pinned directly to that granule.
    """
    b = _require_bbox(bbox)
    return json.loads(
        _opera_rs.stats_json(
            product,
            b,
            concept_id,
            granule_ur,
            start,
            end,
            band,
            expression,
            categorical,
            histogram_bins,
            endpoint,
        )
    )


def detect_change(
    bbox: BBoxLike,
    before_date: str,
    after_date: str,
    *,
    product: str = "OPERA_L3_DSWX-HLS_V1",
    window_days: int = 10,
    band: str | None = None,
    expression: str | None = None,
    endpoint: str | None = None,
) -> dict[str, Any]:
    """Run two-date OPERA change detection over an AOI."""
    b = _require_bbox(bbox)
    return json.loads(
        _opera_rs.detect_change_json(
            product, b, before_date, after_date, window_days, band, expression, endpoint
        )
    )


def analyze_time_series(
    bbox: BBoxLike,
    start: str,
    end: str,
    *,
    product: str = "OPERA_L3_DSWX-HLS_V1",
    interval_days: int | None = None,
    max_observations: int = 12,
    band: str | None = None,
    expression: str | None = None,
    endpoint: str | None = None,
) -> dict[str, Any]:
    """Run multi-date OPERA time-series trend analysis over an AOI."""
    b = _require_bbox(bbox)
    return json.loads(
        _opera_rs.analyze_time_series_json(
            product,
            b,
            start,
            end,
            interval_days,
            max_observations,
            band,
            expression,
            endpoint,
        )
    )


def derive_flood_extent(
    bbox: BBoxLike,
    start: str,
    end: str,
    *,
    products: list[str] | None = None,
    max_granules: int = 6,
    max_tiles_across: int = 4,
    min_area_km2: float = 0.02,
    endpoint: str | None = None,
) -> dict[str, Any]:
    """Vectorize observed OPERA DSWx surface-water polygons (OPERA-observed, not QAed)."""
    b = _require_bbox(bbox)
    return json.loads(
        _opera_rs.derive_flood_extent_json(
            b,
            start,
            end,
            products,
            max_granules,
            max_tiles_across,
            min_area_km2,
            endpoint,
        )
    )


def preview(
    bbox: BBoxLike,
    *,
    product: str = "OPERA_L3_DSWX-HLS_V1",
    start: str | None = None,
    end: str | None = None,
    band: str | None = None,
    water_only: bool = False,
    max_dim: int = 1024,
    basemap: bool = True,
    title: str | None = None,
    output_path: str | Path | None = None,
    endpoint: str | None = None,
) -> bytes:
    """Render a PNG map preview of an OPERA layer over ``bbox`` and return PNG bytes."""
    b = _require_bbox(bbox)
    out_str = str(output_path) if output_path is not None else None
    return _opera_rs.preview_png(
        product,
        b,
        start,
        end,
        band,
        water_only,
        max_dim,
        basemap,
        title,
        out_str,
        endpoint,
    )


def read_aoi_bbox(path: str | Path) -> list[float]:
    """Read a GeoJSON, GeoPackage, or Shapefile AOI and return its WGS 84 bbox."""
    return list(_opera_rs.read_aoi_bbox_py(str(path)))


def write_vector(
    feature_collection: dict[str, Any],
    out_path: str | Path,
    *,
    layer_name: str = "opera",
) -> int:
    """Write a GeoJSON FeatureCollection to GeoJSON, GeoPackage (.gpkg), or Shapefile (.shp)."""
    return int(
        _opera_rs.write_vector_py(json.dumps(feature_collection), str(out_path), layer_name)
    )


def inspect_raster(
    path: str | Path,
    *,
    product: str | None = None,
    band: str | None = None,
) -> dict[str, Any]:
    """Inspect a local OPERA GeoTIFF/COG on disk via pure-Rust ``terrio``."""
    return json.loads(_opera_rs.inspect_raster_json(str(path), product, band))


__all__ = [
    "__version__",
    "analyze_time_series",
    "derive_flood_extent",
    "detect_change",
    "expression_presets",
    "find_product",
    "geocode",
    "inspect_raster",
    "point",
    "preview",
    "products",
    "read_aoi_bbox",
    "search",
    "stats",
    "tile_urls",
    "tilejson",
    "version",
    "write_vector",
]
