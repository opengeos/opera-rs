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


def normalize_burst_id(burst_id: str) -> str:
    """Normalize a Sentinel-1 burst ID to lowercase underscores (e.g. ``'t087_185683_iw2'``)."""
    return str(_opera_rs.normalize_burst_id_py(burst_id))


def get_burst_id(filename: str | Path) -> str:
    """Extract the normalized Sentinel-1 burst ID from a filename, granule UR, or URL."""
    return str(_opera_rs.get_burst_id_py(str(filename)))


def group_by_burst(files: list[str | Path]) -> dict[str, list[str]]:
    """Group Sentinel-1 CSLC filenames or URLs by normalized burst ID."""
    return json.loads(_opera_rs.group_by_burst_json([str(f) for f in files]))


def get_dates(filename: str | Path) -> list[str]:
    """Extract all ``YYYY-MM-DD`` dates from a filename stem."""
    return list(_opera_rs.get_dates_py(str(filename)))


def parse_disp_filename(filename: str | Path) -> dict[str, Any]:
    """Parse an ``OPERA_L3_DISP-S1`` filename or URL into structured metadata."""
    return json.loads(_opera_rs.parse_disp_filename_json(str(filename)))


def get_frame_to_burst_mapping(
    frame_id: int,
    *,
    json_file: str | Path | None = None,
) -> dict[str, Any]:
    """Look up DISP-S1 frame metadata (EPSG, UTM/WGS 84 bounds, burst IDs) from ``burst_db``."""
    path_str = str(json_file) if json_file is not None else None
    return json.loads(_opera_rs.frame_to_burst_mapping_json(int(frame_id), path_str))


def get_burst_ids_for_frame(
    frame_id: int,
    *,
    json_file: str | Path | None = None,
) -> list[str]:
    """Return normalized Sentinel-1 burst IDs belonging to a DISP-S1 frame."""
    path_str = str(json_file) if json_file is not None else None
    return list(_opera_rs.burst_ids_for_frame_py(int(frame_id), path_str))


def get_frame_ids_for_burst(
    burst_id: str,
    *,
    json_file: str | Path | None = None,
) -> list[int]:
    """Return DISP-S1 frame ID(s) containing a Sentinel-1 burst ID."""
    path_str = str(json_file) if json_file is not None else None
    return list(_opera_rs.frame_ids_for_burst_py(burst_id, path_str))


def get_frame_bbox(
    frame_id: int,
    *,
    latlon: bool = False,
    json_file: str | Path | None = None,
) -> dict[str, Any]:
    """Return ``{'frame_id', 'epsg', 'bbox'}`` for a DISP-S1 frame (UTM or WGS 84)."""
    path_str = str(json_file) if json_file is not None else None
    return json.loads(_opera_rs.frame_bbox_json(int(frame_id), bool(latlon), path_str))


def get_frame_geojson(
    frame_ids: list[int] | None = None,
    *,
    json_file: str | Path | None = None,
) -> dict[str, Any]:
    """Return a GeoJSON FeatureCollection of DISP-S1 frame geometries."""
    path_str = str(json_file) if json_file is not None else None
    ids = [int(i) for i in frame_ids] if frame_ids is not None else None
    return json.loads(_opera_rs.frame_geojson_json(ids, path_str))


def get_burst_id_geojson(
    burst_ids: list[str] | None = None,
    *,
    json_file: str | Path | None = None,
) -> dict[str, Any]:
    """Return a GeoJSON FeatureCollection of Sentinel-1 burst geometries."""
    path_str = str(json_file) if json_file is not None else None
    return json.loads(_opera_rs.burst_id_geojson_json(burst_ids, path_str))


def get_intersecting_frames(
    bbox: BBoxLike,
    *,
    json_file: str | Path | None = None,
) -> dict[str, Any]:
    """Return a GeoJSON FeatureCollection of DISP-S1 frames intersecting ``bbox``."""
    b = _require_bbox(bbox)
    path_str = str(json_file) if json_file is not None else None
    return json.loads(_opera_rs.intersecting_frames_json(b, path_str))


def get_missing_data_options(slc_files: list[str | Path]) -> list[dict[str, Any]]:
    """Compute ranked optimal Sentinel-1 burst/date subsets for missing CSLC data."""
    return json.loads(_opera_rs.missing_data_options_json([str(f) for f in slc_files]))


def disp_search(
    *,
    frame_id: int | None = None,
    bbox: BBoxLike | None = None,
    place: str | None = None,
    start: str | None = None,
    end: str | None = None,
    product_version: str | None = "1.0",
    url_type: str = "https",
    deduplicate: bool = True,
    limit: int = 500,
    cmr_endpoint: str | None = None,
    nominatim_endpoint: str | None = None,
) -> dict[str, Any]:
    """Search public NASA CMR for ``OPERA_L3_DISP-S1_V1`` products by frame ID or AOI."""
    if bbox is not None and place is not None:
        raise ValueError("specify either bbox or place, not both")
    resolved_bbox = _bbox_tuple(bbox)
    if resolved_bbox is None and place is not None:
        geo = geocode(place, endpoint=nominatim_endpoint)["bbox"]
        resolved_bbox = (geo["west"], geo["south"], geo["east"], geo["north"])
    return json.loads(
        _opera_rs.disp_search_json(
            int(frame_id) if frame_id is not None else None,
            resolved_bbox,
            product_version,
            start,
            end,
            url_type,
            deduplicate,
            limit,
            cmr_endpoint,
        )
    )


__all__ = [
    "__version__",
    "analyze_time_series",
    "derive_flood_extent",
    "detect_change",
    "disp_search",
    "expression_presets",
    "find_product",
    "geocode",
    "get_burst_id",
    "get_burst_id_geojson",
    "get_burst_ids_for_frame",
    "get_dates",
    "get_frame_bbox",
    "get_frame_geojson",
    "get_frame_ids_for_burst",
    "get_frame_to_burst_mapping",
    "get_intersecting_frames",
    "get_missing_data_options",
    "group_by_burst",
    "inspect_raster",
    "normalize_burst_id",
    "parse_disp_filename",
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
