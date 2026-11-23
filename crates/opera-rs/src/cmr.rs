//! Public NASA CMR (Common Metadata Repository) client and UMM-G parser.
//!
//! NASA CMR granule and collection search endpoints are public and require no Earthdata login.
//! This module builds CMR URLs, parses UMM-G responses (including antimeridian `MultiPolygon`
//! footprints and `BoundingRectangles`), extracts band/layer tokens, deduplicates reprocessed
//! granule versions, and exports GeoJSON `FeatureCollection` footprints.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(feature = "http")]
use crate::DEFAULT_CMR_ENDPOINT;
use crate::error::{OperaError, Result};
#[cfg(feature = "http")]
use crate::products::find_product;
use crate::types::{BBox, CalendarDate, url_encode};

/// Default CMR page size per request.
pub const CMR_PAGE_SIZE: usize = 50;

/// Parameters for searching NASA CMR for OPERA granules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CmrSearchParams {
    /// OPERA product short name or alias (e.g. `OPERA_L3_DSWX-HLS_V1` or `dswx-hls`).
    pub product: String,
    /// Optional WGS 84 bounding box filter `[west, south, east, north]`.
    pub bbox: Option<BBox>,
    /// Optional inclusive start date (`YYYY-MM-DD`).
    pub start: Option<String>,
    /// Optional inclusive end date (`YYYY-MM-DD`).
    pub end: Option<String>,
    /// Maximum number of granules to return (default: 50, capped at 500).
    pub limit: usize,
    /// Drop superseded reprocessed granule versions for the same tile and acquisition time.
    pub deduplicate_versions: bool,
    /// Keep only the newest acquisition per tile/burst id.
    pub newest_per_tile: bool,
    /// Optional override for the CMR base URL.
    pub cmr_endpoint: Option<String>,
}

impl Default for CmrSearchParams {
    fn default() -> Self {
        Self {
            product: "OPERA_L3_DSWX-HLS_V1".to_string(),
            bbox: None,
            start: None,
            end: None,
            limit: 50,
            deduplicate_versions: true,
            newest_per_tile: false,
            cmr_endpoint: None,
        }
    }
}

/// One parsed OPERA granule returned by NASA CMR.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperaGranule {
    /// CMR GranuleUR (unique granule identifier).
    pub id: String,
    /// Parent CMR collection concept-id (`C...`), required by `titiler-cmr`.
    pub concept_id: Option<String>,
    /// Product CMR `short_name`.
    pub product: String,
    /// Acquisition start timestamp (ISO-8601 / RFC3339).
    pub begin_date: Option<String>,
    /// Acquisition end timestamp (ISO-8601 / RFC3339).
    pub end_date: Option<String>,
    /// Acquisition date `YYYY-MM-DD`.
    pub date: String,
    /// Acquisition time `HH:MM`.
    pub time: String,
    /// MGRS tile or Sentinel-1 burst identifier parsed from the GranuleUR.
    pub tile: String,
    /// Combined WGS 84 bounding box of the granule footprint.
    pub bbox: Option<BBox>,
    /// Closed exterior coordinate rings (`[lon, lat]`) forming a Polygon (1 ring) or MultiPolygon (>1).
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Optional interior exclusion rings (holes) parallel to `rings`, from UMM-G `ExclusiveZone`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Vec<[f64; 2]>>>,
    /// Deduplicated GeoTIFF band/layer tokens (e.g. `B01_WTR`, `VV`, `VEG-DIST-STATUS`).
    pub bands: Vec<String>,
    /// HTTPS raster data links (`*.tif`, `*.tiff`, `*.h5`, `*.nc`).
    pub data_links: Vec<String>,
}

fn ring_signed_area(ring: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for w in ring.windows(2) {
        a += w[0][0] * w[1][1] - w[1][0] * w[0][1];
    }
    a * 0.5
}

fn normalize_ring_winding(ring: &[[f64; 2]], ccw: bool) -> Vec<[f64; 2]> {
    let is_ccw = ring_signed_area(ring) > 0.0;
    if is_ccw == ccw {
        ring.to_vec()
    } else {
        ring.iter().copied().rev().collect()
    }
}

impl OperaGranule {
    /// Converts this granule's footprint into an RFC 7946 GeoJSON `Feature`
    /// (CCW exterior rings, CW exclusion holes).
    pub fn to_geojson_feature(&self) -> Option<Value> {
        if self.rings.is_empty() {
            return None;
        }
        let build_poly = |idx: usize, ext: &Vec<[f64; 2]>| -> Vec<Vec<[f64; 2]>> {
            let mut poly = vec![normalize_ring_winding(ext, true)];
            if let Some(poly_holes) = self.holes.get(idx) {
                for hole in poly_holes {
                    poly.push(normalize_ring_winding(hole, false));
                }
            }
            poly
        };
        let geometry = if self.rings.len() == 1 {
            json!({
                "type": "Polygon",
                "coordinates": build_poly(0, &self.rings[0])
            })
        } else {
            let polys: Vec<Vec<Vec<[f64; 2]>>> = self
                .rings
                .iter()
                .enumerate()
                .map(|(i, r)| build_poly(i, r))
                .collect();
            json!({
                "type": "MultiPolygon",
                "coordinates": polys
            })
        };
        Some(json!({
            "type": "Feature",
            "geometry": geometry,
            "properties": {
                "id": self.id,
                "product": self.product,
                "concept_id": self.concept_id,
                "date": self.date,
                "time": self.time,
                "begin_date": self.begin_date,
                "end_date": self.end_date,
                "tile": self.tile,
                "bands": self.bands,
                "link_count": self.data_links.len(),
            }
        }))
    }

    /// Returns a `titiler-cmr` `temporal` filter (`start/end`) isolating this granule's timestamp.
    pub fn datetime_range(&self) -> Option<String> {
        match (self.begin_date.as_deref(), self.end_date.as_deref()) {
            (Some(b), Some(e)) => Some(format!("{b}/{e}")),
            (Some(b), None) => Some(format!("{b}/{b}")),
            (None, Some(e)) => Some(format!("{e}/{e}")),
            (None, None) => None,
        }
    }
}

/// Structured result of a NASA CMR OPERA granule search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GranuleSearchResult {
    /// Normalized CMR `short_name` of the searched OPERA product.
    pub product: String,
    /// Resolved CMR collection `concept_id`, if present on granules or collection lookup.
    pub concept_id: Option<String>,
    /// Total CMR hit count reported by the server.
    pub hits: u64,
    /// Parsed granules (ordered newest first).
    pub granules: Vec<OperaGranule>,
    /// Combined WGS 84 bounding box across all returned granules.
    pub bounds: Option<BBox>,
    /// Distinct acquisition dates (`YYYY-MM-DD`), newest first.
    pub dates: Vec<String>,
    /// GeoJSON `FeatureCollection` of granule footprints.
    pub feature_collection: Value,
}

/// Builds a CMR collection lookup URL for `short_name`.
pub fn build_collection_url(cmr_endpoint: &str, short_name: &str) -> String {
    let base = cmr_endpoint.trim_end_matches('/');
    format!(
        "{base}/collections.umm_json?short_name={}&page_size=1",
        url_encode(short_name)
    )
}

/// Builds a paginated CMR UMM-G granule search URL (sorted newest first).
pub fn build_granule_search_url(
    cmr_endpoint: &str,
    short_name: &str,
    bbox: Option<BBox>,
    start: Option<&str>,
    end: Option<&str>,
    page_size: usize,
    page_num: usize,
) -> Result<String> {
    build_granule_search_url_sorted(
        cmr_endpoint,
        short_name,
        bbox,
        start,
        end,
        page_size,
        page_num,
        false,
    )
}

/// Builds a paginated CMR UMM-G granule search URL with selectable chronological sort order.
#[allow(clippy::too_many_arguments)]
pub fn build_granule_search_url_sorted(
    cmr_endpoint: &str,
    short_name: &str,
    bbox: Option<BBox>,
    start: Option<&str>,
    end: Option<&str>,
    page_size: usize,
    page_num: usize,
    ascending: bool,
) -> Result<String> {
    let base = cmr_endpoint.trim_end_matches('/');
    let sort_key = if ascending {
        "start_date"
    } else {
        "-start_date"
    };
    let mut url = format!(
        "{base}/granules.umm_json?short_name={}&page_size={}&page_num={}&sort_key={sort_key}",
        url_encode(short_name),
        page_size.clamp(1, 200),
        page_num.max(1)
    );
    if let Some(b) = bbox {
        url.push_str(&format!(
            "&bounding_box={}",
            url_encode(&b.to_query_string())
        ));
    }
    match (
        start.filter(|s| !s.trim().is_empty()),
        end.filter(|s| !s.trim().is_empty()),
    ) {
        (Some(s), Some(e)) => {
            let sd = CalendarDate::parse(s)?;
            let ed = CalendarDate::parse(e)?;
            if sd > ed {
                return Err(OperaError::InvalidInput(format!(
                    "start date {sd} must be on or before end date {ed}"
                )));
            }
            let temporal = format!("{},{}", sd.start_of_day_rfc3339(), ed.end_of_day_rfc3339());
            url.push_str(&format!("&temporal={}", url_encode(&temporal)));
        }
        (Some(s), None) => {
            let sd = CalendarDate::parse(s)?;
            let temporal = format!("{},", sd.start_of_day_rfc3339());
            url.push_str(&format!("&temporal={}", url_encode(&temporal)));
        }
        (None, Some(e)) => {
            let ed = CalendarDate::parse(e)?;
            let temporal = format!(",{}", ed.end_of_day_rfc3339());
            url.push_str(&format!("&temporal={}", url_encode(&temporal)));
        }
        (None, None) => {}
    }
    Ok(url)
}

// --- UMM-G deserialization structs ---

#[cfg(feature = "http")]
#[derive(Debug, Default, Deserialize)]
struct UmmCollectionPage {
    #[serde(default)]
    items: Vec<UmmCollectionItem>,
}

#[cfg(feature = "http")]
#[derive(Debug, Default, Deserialize)]
struct UmmCollectionItem {
    #[serde(default)]
    meta: UmmCollectionMeta,
}

#[cfg(feature = "http")]
#[derive(Debug, Default, Deserialize)]
struct UmmCollectionMeta {
    #[serde(rename = "concept-id")]
    concept_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct UmmGranulePage {
    #[serde(default)]
    pub hits: u64,
    #[serde(default)]
    pub items: Vec<UmmGranuleItem>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct UmmGranuleItem {
    #[serde(default)]
    meta: UmmGranuleMeta,
    #[serde(default)]
    umm: UmmGranuleBody,
}

#[derive(Debug, Default, Deserialize)]
struct UmmGranuleMeta {
    #[serde(rename = "concept-id")]
    concept_id: Option<String>,
    #[serde(rename = "collection-concept-id")]
    collection_concept_id: Option<String>,
    #[serde(rename = "native-id")]
    native_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmGranuleBody {
    #[serde(rename = "GranuleUR")]
    granule_ur: Option<String>,
    #[serde(default)]
    temporal_extent: UmmTemporalExtent,
    #[serde(default)]
    spatial_extent: UmmSpatialExtent,
    #[serde(default)]
    related_urls: Vec<UmmRelatedUrl>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmTemporalExtent {
    range_date_time: Option<UmmRangeDateTime>,
    single_date_time: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmRangeDateTime {
    beginning_date_time: Option<String>,
    ending_date_time: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmSpatialExtent {
    #[serde(default)]
    horizontal_spatial_domain: UmmHorizontalSpatialDomain,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmHorizontalSpatialDomain {
    #[serde(default)]
    geometry: UmmGeometry,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmGeometry {
    #[serde(default, rename = "GPolygons")]
    gpolygons: Vec<UmmGPolygon>,
    #[serde(default)]
    bounding_rectangles: Vec<UmmBoundingRectangle>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmGPolygon {
    #[serde(default)]
    boundary: UmmBoundary,
    #[serde(default)]
    exclusive_zone: Option<UmmExclusiveZone>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmExclusiveZone {
    #[serde(default)]
    boundaries: Vec<UmmBoundary>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmBoundary {
    #[serde(default)]
    points: Vec<UmmPoint>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmPoint {
    longitude: f64,
    latitude: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UmmBoundingRectangle {
    west_bounding_coordinate: f64,
    south_bounding_coordinate: f64,
    east_bounding_coordinate: f64,
    north_bounding_coordinate: f64,
}

#[derive(Debug, Default, Deserialize)]
struct UmmRelatedUrl {
    #[serde(rename = "URL")]
    url: Option<String>,
}

fn url_path_without_query(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

fn is_raster_url(url: &str) -> bool {
    let lower = url_path_without_query(url).to_ascii_lowercase();
    lower.ends_with(".tif")
        || lower.ends_with(".tiff")
        || lower.ends_with(".h5")
        || lower.ends_with(".hdf5")
        || lower.ends_with(".hdf")
        || lower.ends_with(".nc")
}

/// Extracts the GeoTIFF band/layer token from an OPERA data URL (e.g. `B01_WTR`, `VV`,
/// `VEG-DIST-STATUS`, `local_incidence_angle`). Returns `None` for non-GeoTIFF URLs.
pub fn extract_band_token(url: &str, granule_ur: &str) -> Option<String> {
    let path = url_path_without_query(url);
    let filename = path.rsplit('/').next()?;
    let lower = filename.to_ascii_lowercase();
    let stem = if lower.ends_with(".tiff") {
        &filename[..filename.len() - 5]
    } else if lower.ends_with(".tif") {
        &filename[..filename.len() - 4]
    } else {
        return None;
    };
    let token = match stem
        .strip_prefix(granule_ur)
        .and_then(|r| r.strip_prefix('_'))
    {
        Some(t) => t,
        None => {
            // Fallback for common `..._B01_WTR` two-part tokens or single trailing token.
            let parts: Vec<&str> = stem.split('_').collect();
            if parts.len() >= 2
                && parts[parts.len() - 2].starts_with('B')
                && parts[parts.len() - 2].len() == 3
                && parts[parts.len() - 2][1..]
                    .chars()
                    .all(|c| c.is_ascii_digit())
            {
                let start =
                    stem.len() - parts[parts.len() - 2].len() - parts[parts.len() - 1].len() - 1;
                &stem[start..]
            } else {
                parts.last().copied().unwrap_or("")
            }
        }
    };
    (!token.is_empty()).then(|| token.to_string())
}

fn close_boundary(boundary: &UmmBoundary) -> Option<Vec<[f64; 2]>> {
    let mut ring: Vec<[f64; 2]> = boundary
        .points
        .iter()
        .map(|p| [p.longitude, p.latitude])
        .collect();
    if ring.len() < 3 {
        return None;
    }
    if ring.first() != ring.last() {
        ring.push(ring[0]);
    }
    Some(ring)
}

type RingCoords = Vec<[f64; 2]>;
type RingsAndHoles = (Vec<RingCoords>, Vec<Vec<RingCoords>>);

fn parse_rings_and_holes(geo: &UmmGeometry) -> RingsAndHoles {
    let mut rings = Vec::new();
    let mut holes = Vec::new();
    for poly in &geo.gpolygons {
        if let Some(ext) = close_boundary(&poly.boundary) {
            let poly_holes: Vec<Vec<[f64; 2]>> = poly
                .exclusive_zone
                .as_ref()
                .map(|ez| ez.boundaries.iter().filter_map(close_boundary).collect())
                .unwrap_or_default();
            rings.push(ext);
            holes.push(poly_holes);
        }
    }
    if rings.is_empty() {
        for r in &geo.bounding_rectangles {
            let (w, s) = (r.west_bounding_coordinate, r.south_bounding_coordinate);
            let (e, n) = (r.east_bounding_coordinate, r.north_bounding_coordinate);
            if w > e {
                // Antimeridian-crossing rectangle: split into [w..180] and [-180..e].
                rings.push(vec![[w, s], [180.0, s], [180.0, n], [w, n], [w, s]]);
                holes.push(Vec::new());
                rings.push(vec![[-180.0, s], [e, s], [e, n], [-180.0, n], [-180.0, s]]);
                holes.push(Vec::new());
            } else {
                rings.push(vec![[w, s], [e, s], [e, n], [w, n], [w, s]]);
                holes.push(Vec::new());
            }
        }
    }
    if holes.iter().all(Vec::is_empty) {
        holes.clear();
    }
    (rings, holes)
}

fn rings_bbox(rings: &[Vec<[f64; 2]>]) -> Option<BBox> {
    let mut west = f64::INFINITY;
    let mut south = f64::INFINITY;
    let mut east = f64::NEG_INFINITY;
    let mut north = f64::NEG_INFINITY;
    let mut any = false;
    for [lon, lat] in rings.iter().flatten() {
        any = true;
        west = west.min(*lon);
        south = south.min(*lat);
        east = east.max(*lon);
        north = north.max(*lat);
    }
    if !any {
        return None;
    }
    BBox::new(west, south, east, north).ok()
}

pub(crate) fn parse_umm_granule(item: &UmmGranuleItem, product: &str) -> Option<OperaGranule> {
    let id = item
        .umm
        .granule_ur
        .clone()
        .or_else(|| item.meta.native_id.clone())
        .or_else(|| item.meta.concept_id.clone())?;
    let begin_date = item
        .umm
        .temporal_extent
        .range_date_time
        .as_ref()
        .and_then(|r| r.beginning_date_time.clone())
        .or_else(|| item.umm.temporal_extent.single_date_time.clone());
    let end_date = item
        .umm
        .temporal_extent
        .range_date_time
        .as_ref()
        .and_then(|r| r.ending_date_time.clone())
        .or_else(|| begin_date.clone());

    let date_ref = begin_date.as_deref().unwrap_or("");
    let date = date_ref.get(..10).unwrap_or("").to_string();
    let time = date_ref.get(11..16).unwrap_or("").to_string();
    let parts: Vec<&str> = id.split('_').collect();
    // DISP-S1 URs take the form `OPERA_L3_DISP-S1_IW_F<frame>_...`, so frame ID is at index 4.
    let tile = if parts.get(3).copied() == Some("IW")
        && parts.get(4).is_some_and(|p| p.starts_with('F'))
    {
        parts[4]
    } else {
        parts.get(3).copied().unwrap_or("")
    }
    .to_string();

    let (rings, holes) =
        parse_rings_and_holes(&item.umm.spatial_extent.horizontal_spatial_domain.geometry);
    let bbox = rings_bbox(&rings);

    let mut data_links = Vec::new();
    let mut bands = Vec::new();
    for url in item
        .umm
        .related_urls
        .iter()
        .filter_map(|u| u.url.as_deref())
    {
        if is_raster_url(url) {
            if url.starts_with("https://") && !data_links.iter().any(|existing| existing == url) {
                data_links.push(url.to_string());
            }
            if url.starts_with("https://")
                && let Some(band) = extract_band_token(url, &id)
                && !bands.iter().any(|b| b == &band)
            {
                bands.push(band);
            }
        }
    }

    Some(OperaGranule {
        id,
        concept_id: item.meta.collection_concept_id.clone(),
        product: product.to_string(),
        begin_date,
        end_date,
        date,
        time,
        tile,
        bbox,
        rings,
        holes,
        bands,
        data_links,
    })
}

/// Parses a raw UMM-G JSON page returned by CMR into `(hits, raw_item_count, Vec<OperaGranule>)`.
pub(crate) fn parse_granule_page_with_raw_count(
    json_str: &str,
    product: &str,
) -> Result<(u64, usize, Vec<OperaGranule>)> {
    let page: UmmGranulePage = serde_json::from_str(json_str)?;
    let raw_count = page.items.len();
    let granules = page
        .items
        .iter()
        .filter_map(|item| parse_umm_granule(item, product))
        .collect();
    Ok((page.hits, raw_count, granules))
}

/// Parses a raw UMM-G JSON page returned by CMR into `(hits, Vec<OperaGranule>)`.
pub fn parse_granule_page_json(json_str: &str, product: &str) -> Result<(u64, Vec<OperaGranule>)> {
    let (hits, _, granules) = parse_granule_page_with_raw_count(json_str, product)?;
    Ok((hits, granules))
}

/// Chronological sort key for ISO-8601 UTC timestamps, handling fractional seconds consistently.
pub fn timestamp_sort_key(ts: &str) -> (String, String) {
    let trimmed = ts.trim_end_matches('Z');
    match trimmed.split_once('.') {
        Some((whole, frac)) => (whole.to_string(), frac.trim_end_matches('0').to_string()),
        None => (trimmed.to_string(), String::new()),
    }
}

/// Keeps only the latest reprocessed version per `(tile, begin_date)`.
///
/// CMR frequently lists reprocessed OPERA granules alongside the originals they replace;
/// their GranuleURs differ from the production timestamp onward, so the lexicographically
/// greatest UR is the newest production run.
pub fn deduplicate_latest_versions(granules: Vec<OperaGranule>) -> Vec<OperaGranule> {
    let mut out: Vec<OperaGranule> = Vec::with_capacity(granules.len());
    for g in granules {
        if let Some(existing) = out
            .iter_mut()
            .find(|o| !g.tile.is_empty() && o.tile == g.tile && o.begin_date == g.begin_date)
        {
            if g.id > existing.id {
                *existing = g;
            }
        } else {
            out.push(g);
        }
    }
    out
}

/// Keeps only the newest acquisition per tile/burst id, preserving first-seen tile order.
pub fn keep_newest_per_tile(granules: Vec<OperaGranule>) -> Vec<OperaGranule> {
    let mut out: Vec<(String, (String, String), OperaGranule)> = Vec::new();
    for g in granules {
        let key = timestamp_sort_key(g.begin_date.as_deref().unwrap_or(""));
        if !g.tile.is_empty()
            && let Some(existing) = out.iter_mut().find(|(tile, _, _)| tile == &g.tile)
        {
            if key > existing.1 {
                *existing = (g.tile.clone(), key, g);
            }
            continue;
        }
        out.push((g.tile.clone(), key, g));
    }
    out.into_iter().map(|(_, _, g)| g).collect()
}

/// Builds a [`GranuleSearchResult`] from parsed granules and hit count.
pub fn assemble_search_result(
    product: &str,
    hits: u64,
    granules: Vec<OperaGranule>,
) -> GranuleSearchResult {
    let concept_id = granules.iter().find_map(|g| g.concept_id.clone());
    let bounds = granules.iter().filter_map(|g| g.bbox).reduce(BBox::union);

    let mut date_set: BTreeSet<String> = BTreeSet::new();
    for g in &granules {
        if !g.date.is_empty() {
            date_set.insert(g.date.clone());
        }
    }
    let dates: Vec<String> = date_set.into_iter().rev().collect();

    let features: Vec<Value> = granules
        .iter()
        .filter_map(OperaGranule::to_geojson_feature)
        .collect();
    let feature_collection = json!({
        "type": "FeatureCollection",
        "features": features,
    });

    GranuleSearchResult {
        product: product.to_string(),
        concept_id,
        hits,
        granules,
        bounds,
        dates,
        feature_collection,
    }
}

/// Resolves a CMR collection `concept_id` (e.g. `C2021957657-LPCLOUD`) from a product short name
/// or alias via the public NASA CMR collections API.
#[cfg(feature = "http")]
pub fn resolve_concept_id(product: &str, cmr_endpoint: Option<&str>) -> Result<String> {
    let short_name = match find_product(product) {
        Ok(p) => p.short_name,
        Err(_) => product.trim().to_string(),
    };
    let endpoint = cmr_endpoint.unwrap_or(DEFAULT_CMR_ENDPOINT);
    let url = build_collection_url(endpoint, &short_name);
    let body = http_get_text(&url)?;
    let page: UmmCollectionPage = serde_json::from_str(&body)?;
    page.items
        .into_iter()
        .find_map(|i| i.meta.concept_id)
        .ok_or_else(|| OperaError::NoResults(format!("no CMR collection found for '{short_name}'")))
}

#[cfg(not(feature = "http"))]
pub fn resolve_concept_id(_product: &str, _cmr_endpoint: Option<&str>) -> Result<String> {
    Err(OperaError::HttpDisabled)
}

/// Searches public NASA CMR for OPERA granules matching `params`.
#[cfg(feature = "http")]
pub fn search_granules(params: &CmrSearchParams) -> Result<GranuleSearchResult> {
    search_granules_sorted(params, false)
}

/// Searches public NASA CMR for OPERA granules with selectable chronological sort order.
#[cfg(feature = "http")]
pub(crate) fn search_granules_sorted(
    params: &CmrSearchParams,
    ascending: bool,
) -> Result<GranuleSearchResult> {
    let short_name = match find_product(&params.product) {
        Ok(p) => p.short_name,
        Err(_) => params.product.trim().to_string(),
    };
    let endpoint = params
        .cmr_endpoint
        .as_deref()
        .unwrap_or(DEFAULT_CMR_ENDPOINT);
    let target_limit = params.limit.clamp(1, 500);
    let page_size = target_limit.min(CMR_PAGE_SIZE);
    let max_pages = target_limit.div_ceil(page_size).max(1) + 2;

    let mut all_granules = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let mut total_hits = 0u64;

    for page_num in 1..=max_pages {
        let url = build_granule_search_url_sorted(
            endpoint,
            &short_name,
            params.bbox,
            params.start.as_deref(),
            params.end.as_deref(),
            page_size,
            page_num,
            ascending,
        )?;
        let body = http_get_text(&url)?;
        let (hits, raw_count, page_granules) =
            parse_granule_page_with_raw_count(&body, &short_name)?;
        if page_num == 1 {
            total_hits = hits;
        }
        for g in page_granules {
            if seen_ids.insert(g.id.clone()) {
                all_granules.push(g);
            }
        }
        if raw_count < page_size || all_granules.len() >= target_limit {
            break;
        }
    }

    if params.deduplicate_versions {
        all_granules = deduplicate_latest_versions(all_granules);
    }
    if params.newest_per_tile {
        all_granules = keep_newest_per_tile(all_granules);
    }
    all_granules.truncate(target_limit);

    Ok(assemble_search_result(
        &short_name,
        total_hits,
        all_granules,
    ))
}

#[cfg(not(feature = "http"))]
pub fn search_granules(_params: &CmrSearchParams) -> Result<GranuleSearchResult> {
    Err(OperaError::HttpDisabled)
}

/// Resolved product metadata, collection concept-id, granules, and band for an AOI query.
#[derive(Debug, Clone)]
pub struct ResolvedAoiLayer {
    pub product: crate::products::OperaProduct,
    pub concept_id: String,
    pub granules: Vec<OperaGranule>,
    pub band: String,
    pub latest_date: Option<String>,
    pub feature_collection: Value,
}

/// Searches NASA CMR for the newest matching granules of `product` over `bbox`, resolving the
/// collection `concept_id` and selecting the best available band.
#[cfg(feature = "http")]
#[allow(clippy::too_many_arguments)]
pub fn resolve_aoi_layer(
    product: &str,
    bbox: BBox,
    start: Option<String>,
    end: Option<String>,
    max_granules: usize,
    band: Option<String>,
    concept_id: Option<String>,
    cmr_endpoint: Option<String>,
) -> Result<ResolvedAoiLayer> {
    let prod = find_product(product)?;
    let cap = max_granules.clamp(1, 15);
    let search = search_granules(&CmrSearchParams {
        product: prod.short_name.clone(),
        bbox: Some(bbox),
        start,
        end,
        limit: cap * 3,
        deduplicate_versions: true,
        newest_per_tile: true,
        cmr_endpoint,
    })?;
    let first = search.granules.first().ok_or_else(|| {
        OperaError::NoResults(format!("no {} granules found over {bbox}", prod.short_name))
    })?;
    let cid = concept_id
        .or_else(|| search.concept_id.clone())
        .or_else(|| first.concept_id.clone())
        .ok_or_else(|| OperaError::NoResults("missing concept_id".into()))?;
    let chosen_band = band
        .or_else(|| prod.choose_band(&first.bands))
        .ok_or_else(|| {
            OperaError::InvalidInput(format!(
                "no default band available for product '{}'; specify a band explicitly",
                prod.short_name
            ))
        })?;
    let granules: Vec<OperaGranule> = search.granules.into_iter().take(cap).collect();
    let capped = assemble_search_result(&prod.short_name, granules.len() as u64, granules);
    Ok(ResolvedAoiLayer {
        product: prod,
        concept_id: cid,
        granules: capped.granules,
        band: chosen_band,
        latest_date: capped.dates.first().cloned(),
        feature_collection: capped.feature_collection,
    })
}

#[cfg(not(feature = "http"))]
#[allow(clippy::too_many_arguments)]
pub fn resolve_aoi_layer(
    _product: &str,
    _bbox: BBox,
    _start: Option<String>,
    _end: Option<String>,
    _max_granules: usize,
    _band: Option<String>,
    _concept_id: Option<String>,
    _cmr_endpoint: Option<String>,
) -> Result<ResolvedAoiLayer> {
    Err(OperaError::HttpDisabled)
}

#[cfg(feature = "http")]
pub(crate) fn http_agent() -> ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    AGENT
        .get_or_init(|| {
            let config = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(60)))
                .user_agent(format!(
                    "opera-rs/{} (https://github.com/opengeos/opera-rs)",
                    crate::VERSION
                ))
                .build();
            config.into()
        })
        .clone()
}

#[cfg(feature = "http")]
fn validate_http_url(url: &str) -> Result<()> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(())
    } else {
        Err(OperaError::InvalidInput(format!(
            "HTTP request URL must start with http:// or https://, got '{url}'"
        )))
    }
}

#[cfg(feature = "http")]
pub(crate) fn http_get_text(url: &str) -> Result<String> {
    validate_http_url(url)?;
    let agent = http_agent();
    let mut resp = agent
        .get(url)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| OperaError::Http(format!("GET {url} failed: {e}")))?;
    resp.body_mut()
        .read_to_string()
        .map_err(|e| OperaError::Http(format!("reading response from {url} failed: {e}")))
}

#[cfg(feature = "http")]
pub(crate) fn http_get_bytes(url: &str) -> Result<Vec<u8>> {
    validate_http_url(url)?;
    let agent = http_agent();
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| OperaError::Http(format!("GET {url} failed: {e}")))?;
    resp.body_mut()
        .with_config()
        .limit(32 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| OperaError::Http(format!("reading bytes from {url} failed: {e}")))
}

#[cfg(feature = "http")]
pub(crate) fn http_post_json(url: &str, body: &Value) -> Result<String> {
    validate_http_url(url)?;
    let agent = http_agent();
    let payload = serde_json::to_string(body)?;
    let mut resp = agent
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send(payload.as_bytes())
        .map_err(|e| OperaError::Http(format!("POST {url} failed: {e}")))?;
    resp.body_mut()
        .read_to_string()
        .map_err(|e| OperaError::Http(format!("reading POST response from {url} failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_CMR_ENDPOINT;

    #[test]
    fn extracts_band_tokens_across_opera_products() {
        let ur = "OPERA_L3_DSWx-HLS_T11SKA_20230401T183929Z_20260315T235803Z_S2B_30_v1.1";
        let base = format!("https://archive.podaac.earthdata.nasa.gov/x/{ur}");
        assert_eq!(
            extract_band_token(&format!("{base}_B01_WTR.tif"), ur).as_deref(),
            Some("B01_WTR")
        );
        assert_eq!(
            extract_band_token(&format!("{base}_B05_WTR-1.tif"), ur).as_deref(),
            Some("B05_WTR-1")
        );
        assert_eq!(extract_band_token(&format!("{base}_BROWSE.png"), ur), None);

        let rtc = "OPERA_L2_RTC-S1-STATIC_T004-006637-IW3_20140403_S1A_30_v1.0";
        assert_eq!(
            extract_band_token(&format!("https://h/{rtc}_local_incidence_angle.tif"), rtc)
                .as_deref(),
            Some("local_incidence_angle")
        );
    }

    #[test]
    fn parses_umm_granule_page_and_deduplicates_versions() {
        let ur_old = "OPERA_L3_DSWx-HLS_T11SKA_20230430T183929Z_20230502T000000Z_S2B_30_v1.0";
        let ur_new = "OPERA_L3_DSWx-HLS_T11SKA_20230430T183929Z_20260315T235803Z_S2B_30_v1.1";
        let payload = json!({
            "hits": 2,
            "items": [
                {
                    "meta": {"collection-concept-id": "C2617126679-POCLOUD"},
                    "umm": {
                        "GranuleUR": ur_old,
                        "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": "2023-04-30T18:39:29Z", "EndingDateTime": "2023-04-30T18:39:29Z"}},
                        "SpatialExtent": {"HorizontalSpatialDomain": {"Geometry": {"GPolygons": [{"Boundary": {"Points": [
                            {"Longitude": -120.0, "Latitude": 35.8},
                            {"Longitude": -119.4, "Latitude": 35.8},
                            {"Longitude": -119.4, "Latitude": 36.3},
                            {"Longitude": -120.0, "Latitude": 36.3}
                        ]}}]}}},
                        "RelatedUrls": [
                            {"URL": format!("https://d/{ur_old}_B01_WTR.tif")},
                            {"URL": format!("s3://d/{ur_old}_B01_WTR.tif")}
                        ]
                    }
                },
                {
                    "meta": {"collection-concept-id": "C2617126679-POCLOUD"},
                    "umm": {
                        "GranuleUR": ur_new,
                        "TemporalExtent": {"RangeDateTime": {"BeginningDateTime": "2023-04-30T18:39:29Z", "EndingDateTime": "2023-04-30T18:39:29Z"}},
                        "SpatialExtent": {"HorizontalSpatialDomain": {"Geometry": {"BoundingRectangles": [{
                            "WestBoundingCoordinate": -120.0,
                            "SouthBoundingCoordinate": 35.8,
                            "EastBoundingCoordinate": -119.4,
                            "NorthBoundingCoordinate": 36.3
                        }]}}},
                        "RelatedUrls": [
                            {"URL": format!("https://d/{ur_new}_B01_WTR.tif")},
                            {"URL": format!("https://d/{ur_new}_B02_BWTR.tif")}
                        ]
                    }
                }
            ]
        });
        let (hits, parsed) =
            parse_granule_page_json(&payload.to_string(), "OPERA_L3_DSWX-HLS_V1").unwrap();
        assert_eq!(hits, 2);
        assert_eq!(parsed.len(), 2);

        let deduped = deduplicate_latest_versions(parsed);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].id, ur_new);
        assert_eq!(deduped[0].bands, vec!["B01_WTR", "B02_BWTR"]);

        let result = assemble_search_result("OPERA_L3_DSWX-HLS_V1", hits, deduped);
        assert_eq!(result.concept_id.as_deref(), Some("C2617126679-POCLOUD"));
        assert_eq!(result.dates, vec!["2023-04-30"]);
        assert_eq!(
            result.feature_collection["features"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn builds_granule_search_url_and_validates_dates() {
        let bbox = BBox::new(-119.9, 35.8, -119.4, 36.3).unwrap();
        let url = build_granule_search_url(
            DEFAULT_CMR_ENDPOINT,
            "OPERA_L3_DSWX-HLS_V1",
            Some(bbox),
            Some("2023-04-01"),
            Some("2023-04-30"),
            25,
            1,
        )
        .unwrap();
        assert!(url.contains("short_name=OPERA_L3_DSWX-HLS_V1"));
        assert!(url.contains("bounding_box=-119.9%2C35.8%2C-119.4%2C36.3"));
        assert!(url.contains("temporal=2023-04-01T00%3A00%3A00Z%2C2023-04-30T23%3A59%3A59.999Z"));

        assert!(
            build_granule_search_url(
                DEFAULT_CMR_ENDPOINT,
                "OPERA_L3_DSWX-HLS_V1",
                None,
                Some("2024-05-01"),
                Some("2024-04-01"),
                10,
                1,
            )
            .is_err()
        );
    }
}
