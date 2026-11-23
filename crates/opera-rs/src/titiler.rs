//! Backend-aware `titiler-cmr` client (`rasterio` and `xarray`).
//!
//! `titiler-cmr` handles NASA Earthdata authentication and Cloud-Optimized GeoTIFF reads
//! server-side, enabling tile URLs, TileJSON, point pixel queries, zonal AOI statistics,
//! and time-series TileJSON without local Earthdata credentials.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::DEFAULT_TITILER_ENDPOINT;
use crate::error::{OperaError, Result};
use crate::products::{LayerStyle, OPERA_PIXEL_AREA_KM2, class_label};
use crate::types::{BBox, TitilerBackend, regex_escape, url_encode};

/// TileMatrixSet identifier used by `titiler-cmr` XYZ tile endpoints.
pub const TILE_MATRIX_SET: &str = "WebMercatorQuad";

/// Resolves the `titiler-cmr` endpoint from an explicit override, the `OPERA_TITILER_ENDPOINT`
/// environment variable, or [`DEFAULT_TITILER_ENDPOINT`], validating an `http://` or `https://`
/// URL scheme.
pub fn resolve_titiler_endpoint(explicit: Option<&str>) -> Result<String> {
    let raw = explicit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            std::env::var("OPERA_TITILER_ENDPOINT")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_TITILER_ENDPOINT.to_string());
    if !(raw.starts_with("https://") || raw.starts_with("http://")) {
        return Err(OperaError::InvalidInput(format!(
            "titiler endpoint must start with http:// or https://, got '{raw}'"
        )));
    }
    if raw.contains('?') || raw.contains('#') {
        return Err(OperaError::InvalidInput(format!(
            "titiler endpoint must not contain query or fragment components, got '{raw}'"
        )));
    }
    Ok(raw)
}

/// Common parameters accepted across `titiler-cmr` endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitilerQuery {
    /// Base `titiler-cmr` endpoint (defaults to [`DEFAULT_TITILER_ENDPOINT`]).
    pub endpoint: String,
    /// Reader backend (`rasterio` for COG/GeoTIFF, `xarray` for NetCDF/HDF5/Zarr).
    pub backend: TitilerBackend,
    /// CMR collection concept-id (e.g. `C2617126679-POCLOUD`).
    pub concept_id: String,
    /// Optional product short name (used locally for class labels and defaults).
    pub product: Option<String>,
    /// Optional exact CMR GranuleUR to pin the request to one granule.
    pub granule_ur: Option<String>,
    /// Optional temporal filter (`RFC3339` instant or `start/end` range).
    pub temporal: Option<String>,
    /// Rasterio asset/band tokens (`assets=...`).
    pub assets: Vec<String>,
    /// Optional `assets_regex` pattern.
    pub assets_regex: Option<String>,
    /// Xarray variable names (`variables=...`).
    pub variables: Vec<String>,
    /// Xarray group path.
    pub group: Option<String>,
    /// Xarray dimensional selection string/JSON.
    pub sel: Option<String>,
    /// Optional stretch (`"min,max"`).
    pub rescale: Vec<String>,
    /// Optional named colormap (e.g. `gray`, `terrain`, `viridis`).
    pub colormap_name: Option<String>,
    /// Optional explicit categorical colormap JSON (`{"class":[r,g,b,a]}`).
    pub colormap: Option<String>,
    /// Optional `rio-tiler` expression (assets exposed as `b1`, `b2`, ...).
    pub expression: Option<String>,
    /// Optional minimum zoom for TileJSON.
    pub minzoom: Option<u8>,
    /// Optional maximum zoom for TileJSON.
    pub maxzoom: Option<u8>,
}

impl TitilerQuery {
    /// Creates a new `rasterio` query against the default `titiler-cmr` endpoint.
    pub fn rasterio(concept_id: impl Into<String>) -> Self {
        Self {
            endpoint: DEFAULT_TITILER_ENDPOINT.to_string(),
            backend: TitilerBackend::Rasterio,
            concept_id: concept_id.into(),
            product: None,
            granule_ur: None,
            temporal: None,
            assets: Vec::new(),
            assets_regex: None,
            variables: Vec::new(),
            group: None,
            sel: None,
            rescale: Vec::new(),
            colormap_name: None,
            colormap: None,
            expression: None,
            minzoom: None,
            maxzoom: None,
        }
    }

    /// Applies a resolved [`LayerStyle`] (colormap JSON, colormap name, and rescale stretch).
    pub fn apply_style(&mut self, style: &LayerStyle) -> &mut Self {
        self.colormap = style.colormap.clone();
        self.colormap_name = style.colormap_name.clone();
        self.rescale = style.rescale.iter().cloned().collect();
        self
    }

    /// Builds a styled `rasterio` query pinned to a single granule UR and band.
    ///
    /// Rejects empty `granule_ur` or `band` values so the granule filter is never silently dropped.
    pub fn for_granule_band(
        endpoint: impl Into<String>,
        concept_id: impl Into<String>,
        product: impl Into<String>,
        granule_ur: impl Into<String>,
        band: impl Into<String>,
        expression: Option<String>,
        style: Option<&LayerStyle>,
    ) -> Result<Self> {
        let ur = granule_ur.into();
        if ur.trim().is_empty() {
            return Err(OperaError::InvalidInput(
                "granule_ur must not be empty".into(),
            ));
        }
        let b = band.into();
        if b.trim().is_empty() {
            return Err(OperaError::InvalidInput("band must not be empty".into()));
        }
        let mut q = Self::rasterio(concept_id);
        q.endpoint = endpoint.into();
        q.product = Some(product.into());
        q.granule_ur = Some(ur);
        q.assets = vec![b];
        q.expression = expression;
        if let Some(s) = style {
            q.apply_style(s);
        }
        Ok(q)
    }

    fn base(&self) -> &str {
        self.endpoint.trim_end_matches('/')
    }

    fn push_common_params(&self, pairs: &mut Vec<(String, String)>) {
        pairs.push(("collection_concept_id".into(), self.concept_id.clone()));
        if let Some(ur) = self.granule_ur.as_deref().filter(|s| !s.is_empty()) {
            pairs.push(("granule_ur".into(), ur.to_string()));
        }
        if let Some(t) = self.temporal.as_deref().filter(|s| !s.is_empty()) {
            pairs.push(("temporal".into(), t.to_string()));
        }
        for asset in &self.assets {
            pairs.push(("assets".into(), asset.clone()));
        }
        if let Some(rx) = self.assets_regex.as_deref().filter(|s| !s.is_empty()) {
            pairs.push(("assets_regex".into(), rx.to_string()));
        } else if self.assets.len() == 1 && self.backend == TitilerBackend::Rasterio {
            pairs.push(("assets_regex".into(), regex_escape(&self.assets[0])));
        }
        for var in &self.variables {
            pairs.push(("variables".into(), var.clone()));
        }
        if let Some(g) = self.group.as_deref().filter(|s| !s.is_empty()) {
            pairs.push(("group".into(), g.to_string()));
        }
        if let Some(s) = self.sel.as_deref().filter(|s| !s.is_empty()) {
            pairs.push(("sel".into(), s.to_string()));
        }
        if let Some(expr) = self
            .expression
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            pairs.push(("expression".into(), expr.to_string()));
            pairs.push(("asset_as_band".into(), "true".to_string()));
        }
        if let Some(cmap) = self.colormap.as_deref().filter(|s| !s.is_empty()) {
            pairs.push(("colormap".into(), cmap.to_string()));
        } else {
            for r in &self.rescale {
                if !r.trim().is_empty() {
                    pairs.push(("rescale".into(), r.trim().to_string()));
                }
            }
            if let Some(name) = self.colormap_name.as_deref().filter(|s| !s.is_empty()) {
                pairs.push(("colormap_name".into(), name.to_string()));
            }
        }
        if let Some(z) = self.minzoom {
            pairs.push(("minzoom".into(), z.to_string()));
        }
        if let Some(z) = self.maxzoom {
            pairs.push(("maxzoom".into(), z.to_string()));
        }
    }
}

fn encode_query(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Builds a `titiler-cmr` TileJSON URL (`/{backend}/WebMercatorQuad/tilejson.json?...`).
pub fn build_tilejson_url(query: &TitilerQuery) -> String {
    let mut pairs = Vec::new();
    query.push_common_params(&mut pairs);
    format!(
        "{}/{}/{TILE_MATRIX_SET}/tilejson.json?{}",
        query.base(),
        query.backend.as_str(),
        encode_query(&pairs)
    )
}

/// Builds a direct XYZ PNG tile URL template (`/{backend}/tiles/WebMercatorQuad/{z}/{x}/{y}.png?...`).
pub fn build_tile_url_template(query: &TitilerQuery, tile_size: u32) -> String {
    let mut pairs = Vec::new();
    query.push_common_params(&mut pairs);
    pairs.push(("tilesize".into(), tile_size.to_string()));
    format!(
        "{}/{}/tiles/{TILE_MATRIX_SET}/{{z}}/{{x}}/{{y}}.png?{}",
        query.base(),
        query.backend.as_str(),
        encode_query(&pairs)
    )
}

/// Builds a `titiler-cmr` `/point/{lon},{lat}` query URL.
pub fn build_point_url(query: &TitilerQuery, lon: f64, lat: f64) -> String {
    let mut pairs = Vec::new();
    query.push_common_params(&mut pairs);
    format!(
        "{}/{}/point/{lon},{lat}?{}",
        query.base(),
        query.backend.as_str(),
        encode_query(&pairs)
    )
}

/// Builds a `titiler-cmr` `/statistics` URL (target for GeoJSON AOI POST).
pub fn build_statistics_url(
    query: &TitilerQuery,
    categorical: bool,
    histogram_bins: Option<u32>,
) -> String {
    let mut pairs = Vec::new();
    query.push_common_params(&mut pairs);
    if categorical {
        pairs.push(("categorical".into(), "true".into()));
    } else if let Some(bins) = histogram_bins {
        pairs.push(("histogram_bins".into(), bins.max(1).to_string()));
    }
    format!(
        "{}/{}/statistics?{}",
        query.base(),
        query.backend.as_str(),
        encode_query(&pairs)
    )
}

/// Builds a `titiler-cmr` timeseries TileJSON URL.
pub fn build_timeseries_tilejson_url(
    query: &TitilerQuery,
    step: Option<&str>,
    temporal_mode: Option<&str>,
) -> String {
    let mut pairs = Vec::new();
    query.push_common_params(&mut pairs);
    if let Some(s) = step.filter(|s| !s.is_empty()) {
        pairs.push(("step".into(), s.to_string()));
    }
    if let Some(m) = temporal_mode.filter(|s| !s.is_empty()) {
        pairs.push(("temporal_mode".into(), m.to_string()));
    }
    format!(
        "{}/{}/timeseries/{TILE_MATRIX_SET}/tilejson.json?{}",
        query.base(),
        query.backend.as_str(),
        encode_query(&pairs)
    )
}

/// Parsed TileJSON document returned by `titiler-cmr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileJson {
    pub tiles: Vec<String>,
    pub bounds: Option<[f64; 4]>,
    pub minzoom: Option<u8>,
    pub maxzoom: Option<u8>,
    pub center: Option<Vec<f64>>,
}

/// Pixel values returned for one granule asset at a queried point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointAssetValue {
    pub name: String,
    pub values: Vec<Option<f64>>,
    pub band_names: Vec<String>,
    pub class_labels: Vec<Option<String>>,
}

/// Result of a `titiler-cmr` `/point` pixel query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PointResult {
    pub coordinates: [f64; 2],
    pub assets: Vec<PointAssetValue>,
}

/// Area and pixel breakdown for one discrete class value in a categorical histogram.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassBreakdown {
    pub class_value: i64,
    pub label: Option<String>,
    pub pixel_count: f64,
    pub percent: f64,
    pub area_km2: f64,
}

/// Per-band statistics returned by `titiler-cmr` `/statistics`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandStatistics {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub std: Option<f64>,
    pub median: Option<f64>,
    pub count: Option<f64>,
    pub valid_pixels: Option<f64>,
    pub valid_percent: Option<f64>,
    pub percentile_2: Option<f64>,
    pub percentile_98: Option<f64>,
    pub histogram: Option<(Vec<f64>, Vec<f64>)>,
    /// Decoded per-class pixel counts, percentages, and 30 m surface areas (when categorical).
    pub class_breakdown: Vec<ClassBreakdown>,
}

/// Result of a `titiler-cmr` `/statistics` zonal-stats request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatisticsResult {
    pub bands: BTreeMap<String, BandStatistics>,
}

/// Parses a `titiler-cmr` `/point` JSON response and enriches discrete values with OPERA class labels.
pub fn parse_point_response(
    json_str: &str,
    product: Option<&str>,
    band: Option<&str>,
) -> Result<PointResult> {
    #[derive(Deserialize)]
    struct RawAsset {
        name: String,
        values: Vec<Option<f64>>,
        #[serde(default)]
        band_names: Vec<String>,
    }
    #[derive(Deserialize)]
    struct RawPoint {
        coordinates: [f64; 2],
        #[serde(default)]
        assets: Vec<RawAsset>,
    }
    let raw: RawPoint = serde_json::from_str(json_str)?;
    let assets = raw
        .assets
        .into_iter()
        .map(|a| {
            let labels = a
                .values
                .iter()
                .map(|val| {
                    let v = (*val)?;
                    if (v.fract()).abs() < 1e-6 {
                        product
                            .and_then(|p| class_label(p, band, v.round() as i64))
                            .map(ToString::to_string)
                    } else {
                        None
                    }
                })
                .collect();
            PointAssetValue {
                name: a.name,
                values: a.values,
                band_names: a.band_names,
                class_labels: labels,
            }
        })
        .collect();
    Ok(PointResult {
        coordinates: raw.coordinates,
        assets,
    })
}

/// Parses a `titiler-cmr` `/statistics` JSON response and computes 30 m class areas when categorical.
pub fn parse_statistics_response(
    json_str: &str,
    product: Option<&str>,
    band: Option<&str>,
    categorical: bool,
) -> Result<StatisticsResult> {
    let root: Value = serde_json::from_str(json_str)?;
    let stats_obj = root
        .get("properties")
        .and_then(|p| p.get("statistics"))
        .and_then(Value::as_object)
        .or_else(|| {
            // FeatureCollection response: read first feature's properties.statistics
            root.get("features")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(|f| f.get("properties"))
                .and_then(|p| p.get("statistics"))
                .and_then(Value::as_object)
        });

    let mut bands = BTreeMap::new();
    if let Some(obj) = stats_obj {
        for (name, val) in obj {
            let Some(m) = val.as_object() else {
                continue;
            };
            let opt_f64 = |key: &str| m.get(key).and_then(Value::as_f64).filter(|v| v.is_finite());
            let histogram = m
                .get("histogram")
                .and_then(Value::as_array)
                .and_then(|arr| {
                    if arr.len() != 2 {
                        return None;
                    }
                    let counts: Vec<f64> = arr[0]
                        .as_array()?
                        .iter()
                        .filter_map(Value::as_f64)
                        .collect();
                    let edges: Vec<f64> = arr[1]
                        .as_array()?
                        .iter()
                        .filter_map(Value::as_f64)
                        .collect();
                    Some((counts, edges))
                });

            let mut class_breakdown = Vec::new();
            if categorical && let Some((counts, edges)) = &histogram {
                let total: f64 = counts.iter().copied().sum();
                for (&count, &edge) in counts.iter().zip(edges.iter()) {
                    let cls = edge.round() as i64;
                    let pct = if total > 0.0 {
                        (count / total) * 100.0
                    } else {
                        0.0
                    };
                    class_breakdown.push(ClassBreakdown {
                        class_value: cls,
                        label: product
                            .and_then(|p| class_label(p, band, cls))
                            .map(ToString::to_string),
                        pixel_count: count,
                        percent: pct,
                        area_km2: count * OPERA_PIXEL_AREA_KM2,
                    });
                }
            }

            bands.insert(
                name.clone(),
                BandStatistics {
                    min: opt_f64("min"),
                    max: opt_f64("max"),
                    mean: opt_f64("mean"),
                    std: opt_f64("std"),
                    median: opt_f64("median"),
                    count: opt_f64("count"),
                    valid_pixels: opt_f64("valid_pixels"),
                    valid_percent: opt_f64("valid_percent"),
                    percentile_2: opt_f64("percentile_2"),
                    percentile_98: opt_f64("percentile_98"),
                    histogram,
                    class_breakdown,
                },
            );
        }
    }

    Ok(StatisticsResult { bands })
}

/// Fetches a TileJSON document from `titiler-cmr`.
#[cfg(feature = "http")]
pub fn fetch_tilejson(query: &TitilerQuery) -> Result<TileJson> {
    let url = build_tilejson_url(query);
    let body = crate::cmr::http_get_text(&url)?;
    let tj: TileJson = serde_json::from_str(&body)?;
    if tj.tiles.is_empty() {
        return Err(OperaError::NoResults(
            "titiler-cmr returned no tiles for this query".into(),
        ));
    }
    Ok(tj)
}

#[cfg(not(feature = "http"))]
pub fn fetch_tilejson(_query: &TitilerQuery) -> Result<TileJson> {
    Err(OperaError::HttpDisabled)
}

/// Queries pixel/variable values at `(lon, lat)` via `titiler-cmr`.
#[cfg(feature = "http")]
pub fn fetch_point(query: &TitilerQuery, lon: f64, lat: f64) -> Result<PointResult> {
    if !lon.is_finite()
        || !lat.is_finite()
        || !(-180.0..=180.0).contains(&lon)
        || !(-90.0..=90.0).contains(&lat)
    {
        return Err(OperaError::InvalidInput(format!(
            "invalid lon/lat coordinates: ({lon}, {lat})"
        )));
    }
    let url = build_point_url(query, lon, lat);
    let body = crate::cmr::http_get_text(&url)?;
    parse_point_response(
        &body,
        query.product.as_deref(),
        query.assets.first().map(String::as_str),
    )
}

#[cfg(not(feature = "http"))]
pub fn fetch_point(_query: &TitilerQuery, _lon: f64, _lat: f64) -> Result<PointResult> {
    Err(OperaError::HttpDisabled)
}

/// Computes zonal AOI statistics over a GeoJSON `Feature` (or `FeatureCollection`) via `titiler-cmr`.
#[cfg(feature = "http")]
pub fn fetch_statistics(
    query: &TitilerQuery,
    aoi_geojson: &Value,
    categorical: bool,
    histogram_bins: Option<u32>,
) -> Result<StatisticsResult> {
    let url = build_statistics_url(query, categorical, histogram_bins);
    let body = crate::cmr::http_post_json(&url, aoi_geojson)?;
    parse_statistics_response(
        &body,
        query.product.as_deref(),
        query.assets.first().map(String::as_str),
        categorical,
    )
}

#[cfg(not(feature = "http"))]
pub fn fetch_statistics(
    _query: &TitilerQuery,
    _aoi_geojson: &Value,
    _categorical: bool,
    _histogram_bins: Option<u32>,
) -> Result<StatisticsResult> {
    Err(OperaError::HttpDisabled)
}

/// Convenience wrapper for [`fetch_statistics`] over a [`BBox`].
pub fn fetch_bbox_statistics(
    query: &TitilerQuery,
    bbox: BBox,
    categorical: bool,
    histogram_bins: Option<u32>,
) -> Result<StatisticsResult> {
    fetch_statistics(
        query,
        &bbox.to_geojson_feature(),
        categorical,
        histogram_bins,
    )
}

/// Fetches a time-indexed TileJSON map from `titiler-cmr`.
#[cfg(feature = "http")]
pub fn fetch_timeseries_tilejson(
    query: &TitilerQuery,
    step: Option<&str>,
    temporal_mode: Option<&str>,
) -> Result<BTreeMap<String, TileJson>> {
    let url = build_timeseries_tilejson_url(query, step, temporal_mode);
    let body = crate::cmr::http_get_text(&url)?;
    let map: BTreeMap<String, TileJson> = serde_json::from_str(&body)?;
    Ok(map)
}

#[cfg(not(feature = "http"))]
pub fn fetch_timeseries_tilejson(
    _query: &TitilerQuery,
    _step: Option<&str>,
    _temporal_mode: Option<&str>,
) -> Result<BTreeMap<String, TileJson>> {
    Err(OperaError::HttpDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::DSWX_WTR_WATER_ONLY_COLORMAP_JSON;

    #[test]
    fn builds_rasterio_and_xarray_urls() {
        let mut q = TitilerQuery::rasterio("C2617126679-POCLOUD");
        q.granule_ur = Some("OPERA_L3_DSWx-HLS_T11SKA_v1.1".into());
        q.assets = vec!["B01_WTR".into()];
        q.colormap = Some(DSWX_WTR_WATER_ONLY_COLORMAP_JSON.into());

        let tj = build_tilejson_url(&q);
        assert!(tj.starts_with(
            "https://titiler-cmr.opengeos.org/rasterio/WebMercatorQuad/tilejson.json?"
        ));
        assert!(tj.contains("collection_concept_id=C2617126679-POCLOUD"));
        assert!(tj.contains("assets=B01_WTR"));
        assert!(tj.contains("assets_regex=B01_WTR"));
        assert!(tj.contains("colormap="));

        let pt = build_point_url(&q, -119.7, 36.1);
        assert!(pt.contains("/rasterio/point/-119.7,36.1?"));

        let st = build_statistics_url(&q, true, None);
        assert!(st.contains("/rasterio/statistics?"));
        assert!(st.contains("categorical=true"));

        let mut xq = TitilerQuery::rasterio("C999-ASF");
        xq.backend = TitilerBackend::Xarray;
        xq.variables = vec!["displacement".into()];
        xq.group = Some("/science/grids".into());
        let ts = build_timeseries_tilejson_url(&xq, Some("P1M"), Some("point"));
        assert!(ts.contains("/xarray/timeseries/WebMercatorQuad/tilejson.json?"));
        assert!(ts.contains("variables=displacement"));
        assert!(ts.contains("step=P1M"));
    }

    #[test]
    fn parses_categorical_statistics_with_area_km2() {
        let payload = r#"{
            "type": "Feature",
            "properties": {
                "statistics": {
                    "b1": {
                        "min": 0.0,
                        "max": 255.0,
                        "mean": 1.2,
                        "std": 0.4,
                        "count": 1000.0,
                        "valid_pixels": 1000.0,
                        "valid_percent": 100.0,
                        "histogram": [[700.0, 200.0, 100.0], [0.0, 1.0, 2.0]]
                    }
                }
            }
        }"#;
        let res =
            parse_statistics_response(payload, Some("OPERA_L3_DSWX-HLS_V1"), Some("B01_WTR"), true)
                .unwrap();
        let b1 = &res.bands["b1"];
        assert_eq!(b1.class_breakdown.len(), 3);
        let open_water = b1
            .class_breakdown
            .iter()
            .find(|c| c.class_value == 1)
            .unwrap();
        assert_eq!(open_water.label.as_deref(), Some("Open water"));
        assert!((open_water.area_km2 - 200.0 * 0.0009).abs() < 1e-9);
        assert!((open_water.percent - 20.0).abs() < 1e-9);
    }

    #[test]
    fn parses_point_query_with_class_labels() {
        let payload = r#"{
            "coordinates": [-119.7, 36.1],
            "assets": [{
                "name": "granule_B01_WTR.tif",
                "values": [1.0],
                "band_names": ["b1"]
            }]
        }"#;
        let pt =
            parse_point_response(payload, Some("OPERA_L3_DSWX-HLS_V1"), Some("B01_WTR")).unwrap();
        assert_eq!(pt.assets[0].class_labels[0].as_deref(), Some("Open water"));
    }
}
