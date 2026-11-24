//! OPERA DISP-S1 surface displacement filename parsing, stack deduplication, dataset enums,
//! and public NASA CMR frame search.
//!
//! Adapted in pure Rust from `opera-adt/opera-utils` (`disp/_product.py`, `disp/_search.py`,
//! `disp/_enums.py`) with zero GDAL, h5py, or Earthdata login requirements for CMR discovery.

use std::collections::BTreeMap;
#[cfg(feature = "http")]
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "http")]
use crate::DEFAULT_CMR_ENDPOINT;
use crate::bursts::{basename_for_parsing, parse_compact_datetime_iso};
use crate::error::{OperaError, Result};
#[cfg(feature = "http")]
use crate::types::url_encode;
use crate::types::{BBox, CalendarDate};

/// CMR short name for OPERA L3 Sentinel-1 Surface Displacement products.
pub const DISP_S1_SHORT_NAME: &str = "OPERA_L3_DISP-S1_V1";

/// Protocol preference for DISP-S1 download URLs extracted from NASA CMR UMM-G metadata.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UrlType {
    /// Direct HTTPS download URL (e.g. `https://datapool.asf.alaska.edu/...`).
    #[default]
    Https,
    /// In-region AWS S3 URL (e.g. `s3://asf-cumulus-prod-protected/...`).
    S3,
}

impl UrlType {
    /// Canonical lowercase string (`https` or `s3`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Https => "https",
            Self::S3 => "s3",
        }
    }
}

impl std::fmt::Display for UrlType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for UrlType {
    type Err = OperaError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "https" | "http" => Ok(Self::Https),
            "s3" => Ok(Self::S3),
            _ => Err(OperaError::InvalidInput(format!(
                "invalid url_type '{s}' (expected 'https' or 's3')"
            ))),
        }
    }
}

/// Spatial referencing method for DISP-S1 time-series rebasing (matches `opera_utils.disp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceMethod {
    None,
    Point,
    Median,
    Border,
    HighCoherence,
}

/// Primary displacement variable names in an OPERA DISP-S1 NetCDF/HDF5 product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplacementDataset {
    Displacement,
    ShortWavelengthDisplacement,
}

impl DisplacementDataset {
    /// NetCDF variable path inside a DISP-S1 granule.
    pub const fn dataset_name(self) -> &'static str {
        match self {
            Self::Displacement => "displacement",
            Self::ShortWavelengthDisplacement => "short_wavelength_displacement",
        }
    }
}

/// Correction layer paths inside an OPERA DISP-S1 NetCDF/HDF5 product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionDataset {
    SolidEarthTide,
    IonosphericDelay,
}

impl CorrectionDataset {
    /// HDF5/NetCDF path inside a DISP-S1 granule.
    pub const fn dataset_path(self) -> &'static str {
        match self {
            Self::SolidEarthTide => "/corrections/solid_earth_tide",
            Self::IonosphericDelay => "/corrections/ionospheric_delay",
        }
    }
}

/// Quality and mask variable names inside an OPERA DISP-S1 NetCDF/HDF5 product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityDataset {
    TemporalCoherence,
    PhaseSimilarity,
    PersistentScattererMask,
    TimeseriesInversionResiduals,
    ConnectedComponentLabels,
    RecommendedMask,
    EstimatedPhaseQuality,
    ShpCounts,
    WaterMask,
}

impl QualityDataset {
    /// NetCDF variable name inside a DISP-S1 granule.
    pub const fn dataset_name(self) -> &'static str {
        match self {
            Self::TemporalCoherence => "temporal_coherence",
            Self::PhaseSimilarity => "phase_similarity",
            Self::PersistentScattererMask => "persistent_scatterer_mask",
            Self::TimeseriesInversionResiduals => "timeseries_inversion_residuals",
            Self::ConnectedComponentLabels => "connected_component_labels",
            Self::RecommendedMask => "recommended_mask",
            Self::EstimatedPhaseQuality => "estimated_phase_quality",
            Self::ShpCounts => "shp_counts",
            Self::WaterMask => "water_mask",
        }
    }
}

/// Parsed metadata for one OPERA `DISP-S1` (or `DISP-NI`) product filename or CMR granule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispProduct {
    /// Original filename, local path, or selected download URL.
    pub filename: String,
    /// Sensor token (`S1` or `NI`).
    pub sensor: String,
    /// Acquisition mode (`IW`).
    pub acquisition_mode: String,
    /// Numeric DISP frame identifier (e.g. `11116` from `F11116`).
    pub frame_id: u32,
    /// Radar polarization (`VV` or `HH`).
    pub polarization: String,
    /// Reference (baseline) acquisition timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
    pub reference_datetime: String,
    /// Secondary (target) acquisition timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
    pub secondary_datetime: String,
    /// Product version string (e.g. `1.0`).
    pub version: String,
    /// Production generation timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
    pub generation_datetime: String,
    /// Optional granule size in bytes (from CMR UMM-G metadata or local filesystem).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_in_bytes: Option<u64>,
    /// Optional HTTPS download URL (populated from CMR UMM-G).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub https_url: Option<String>,
    /// Optional AWS S3 download URL (populated from CMR UMM-G).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3_url: Option<String>,
}

impl DispProduct {
    /// Parses a DISP-S1 filename, path, or URL:
    /// `OPERA_L3_DISP-S1_IW_F11116_VV_20160705T140755Z_20160729T140756Z_v1.0_20241219T231545Z.nc`
    pub fn from_filename(input: &str) -> Result<Self> {
        let name = basename_for_parsing(input);
        let stem = name
            .strip_suffix(".nc")
            .or_else(|| name.strip_suffix(".h5"))
            .unwrap_or(name);
        let parts: Vec<&str> = stem.split('_').collect();
        // OPERA _ L3 _ DISP-S1 _ IW _ F11116 _ VV _ <ref> _ <sec> _ v1.0 _ <gen>
        if parts.len() != 10 || parts[0] != "OPERA" || parts[1] != "L3" {
            return Err(OperaError::InvalidInput(format!(
                "invalid OPERA DISP filename format: '{input}'"
            )));
        }
        let sensor = parts[2]
            .strip_prefix("DISP-")
            .filter(|s| matches!(*s, "S1" | "NI"))
            .ok_or_else(|| {
                OperaError::InvalidInput(format!(
                    "expected DISP-S1 or DISP-NI product token in '{input}'"
                ))
            })?
            .to_string();
        let acquisition_mode = parts[3].to_string();
        let frame_str = parts[4].strip_prefix('F').ok_or_else(|| {
            OperaError::InvalidInput(format!("missing F<frame_id> token in '{input}'"))
        })?;
        if frame_str.len() != 5 || !frame_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(OperaError::InvalidInput(format!(
                "invalid DISP frame token '{}' in '{input}'",
                parts[4]
            )));
        }
        let frame_id: u32 = frame_str
            .parse()
            .map_err(|_| OperaError::InvalidInput(format!("invalid DISP frame id in '{input}'")))?;
        let polarization = match parts[5] {
            "VV" | "HH" | "VH" | "HV" => parts[5].to_string(),
            other => {
                return Err(OperaError::InvalidInput(format!(
                    "unexpected DISP polarization '{other}' in '{input}'"
                )));
            }
        };
        let reference_datetime = parse_compact_datetime_iso(parts[6]).ok_or_else(|| {
            OperaError::InvalidInput(format!("invalid DISP reference datetime in '{input}'"))
        })?;
        let secondary_datetime = parse_compact_datetime_iso(parts[7]).ok_or_else(|| {
            OperaError::InvalidInput(format!("invalid DISP secondary datetime in '{input}'"))
        })?;
        let version = parts[8]
            .strip_prefix('v')
            .filter(|v| {
                !v.is_empty()
                    && v.split('.')
                        .all(|comp| !comp.is_empty() && comp.parse::<u32>().is_ok())
            })
            .ok_or_else(|| {
                OperaError::InvalidInput(format!("invalid DISP version token in '{input}'"))
            })?
            .to_string();
        let generation_datetime = parse_compact_datetime_iso(parts[9]).ok_or_else(|| {
            OperaError::InvalidInput(format!("invalid DISP generation datetime in '{input}'"))
        })?;

        Ok(Self {
            filename: input.to_string(),
            sensor,
            acquisition_mode,
            frame_id,
            polarization,
            reference_datetime,
            secondary_datetime,
            version,
            generation_datetime,
            size_in_bytes: None,
            https_url: input.starts_with("https://").then(|| input.to_string()),
            s3_url: input.starts_with("s3://").then(|| input.to_string()),
        })
    }

    /// Constructs a [`DispProduct`] from a CMR UMM-G `umm` JSON object.
    pub fn from_umm(umm: &Value, url_type: UrlType) -> Result<Self> {
        let mut https_primary: Vec<String> = Vec::new();
        let mut https_fallback: Vec<String> = Vec::new();
        let mut s3_urls: Vec<String> = Vec::new();

        if let Some(urls) = umm.get("RelatedUrls").and_then(Value::as_array) {
            for entry in urls {
                let u = match entry.get("URL").and_then(Value::as_str) {
                    Some(u) => u,
                    None => continue,
                };
                let rtype = entry.get("Type").and_then(Value::as_str).unwrap_or("");
                let base = basename_for_parsing(u);
                if !base.ends_with(".nc") && !base.ends_with(".h5") {
                    continue;
                }
                if u.starts_with("s3://") {
                    s3_urls.push(u.to_string());
                } else if u.starts_with("https://") {
                    if rtype == "GET DATA" || rtype.is_empty() {
                        https_primary.push(u.to_string());
                    } else {
                        https_fallback.push(u.to_string());
                    }
                }
            }
        }

        let valid_https = https_primary
            .iter()
            .chain(https_fallback.iter())
            .find(|u| Self::from_filename(u).is_ok())
            .cloned();
        let valid_s3 = s3_urls
            .iter()
            .find(|u| Self::from_filename(u).is_ok())
            .cloned();

        let granule_ur = umm
            .get("GranuleUR")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let chosen = match url_type {
            UrlType::Https => valid_https
                .clone()
                .or_else(|| valid_s3.clone())
                .or(granule_ur),
            UrlType::S3 => valid_s3
                .clone()
                .or_else(|| valid_https.clone())
                .or(granule_ur),
        }
        .ok_or_else(|| OperaError::InvalidInput("CMR DISP-S1 item missing download URL".into()))?;

        let mut prod = Self::from_filename(&chosen)?;
        prod.https_url = valid_https;
        prod.s3_url = valid_s3;
        prod.size_in_bytes = umm
            .get("DataGranule")
            .and_then(|dg| dg.get("ArchiveAndDistributionInformation"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|info| info.get("SizeInBytes"))
            .and_then(Value::as_u64);
        Ok(prod)
    }

    /// Parses dotted version string (e.g. `"1.10"`) into numeric components for accurate comparison.
    fn version_key(&self) -> Vec<u32> {
        self.version
            .split('.')
            .filter_map(|p| p.parse::<u32>().ok())
            .collect()
    }
}

/// Deduplicated, chronologically sorted stack of [`DispProduct`] acquisitions for a single frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispProductStack {
    /// Common DISP-S1 frame identifier shared by all products in the stack.
    pub frame_id: u32,
    /// Number of superseded duplicate `(reference, secondary)` pairs removed during construction.
    pub duplicates_removed: usize,
    /// Chronologically sorted, deduplicated DISP products.
    pub products: Vec<DispProduct>,
}

impl DispProductStack {
    /// Builds a validated, deduplicated stack for a single `frame_id`.
    ///
    /// When multiple products share the same `(reference_datetime, secondary_datetime)` pair,
    /// keeps the product with the highest semantic version and newest `generation_datetime`.
    pub fn new(products: Vec<DispProduct>) -> Result<Self> {
        if products.is_empty() {
            return Err(OperaError::InvalidInput(
                "at least one DispProduct is required".into(),
            ));
        }
        let frame_id = products[0].frame_id;
        if products.iter().any(|p| p.frame_id != frame_id) {
            return Err(OperaError::InvalidInput(
                "all products in a DispProductStack must share the same frame_id".into(),
            ));
        }
        let original_len = products.len();
        let deduped = deduplicate_disp_products(products);
        let duplicates_removed = original_len.saturating_sub(deduped.len());
        Ok(Self {
            frame_id,
            duplicates_removed,
            products: deduped,
        })
    }

    /// Parses a list of DISP-S1 filenames or URLs into a [`DispProductStack`].
    pub fn from_file_list(files: &[String]) -> Result<Self> {
        let products = files
            .iter()
            .map(|f| DispProduct::from_filename(f))
            .collect::<Result<Vec<_>>>()?;
        Self::new(products)
    }

    /// Ordered list of reference timestamps (`YYYY-MM-DDTHH:MM:SSZ`).
    pub fn reference_dates(&self) -> Vec<&str> {
        self.products
            .iter()
            .map(|p| p.reference_datetime.as_str())
            .collect()
    }

    /// Ordered list of secondary timestamps (`YYYY-MM-DDTHH:MM:SSZ`).
    pub fn secondary_dates(&self) -> Vec<&str> {
        self.products
            .iter()
            .map(|p| p.secondary_datetime.as_str())
            .collect()
    }

    /// Ordered list of `(reference_datetime, secondary_datetime)` interferogram date pairs.
    pub fn ifg_date_pairs(&self) -> Vec<(&str, &str)> {
        self.products
            .iter()
            .map(|p| (p.reference_datetime.as_str(), p.secondary_datetime.as_str()))
            .collect()
    }

    /// Ordered list of product filenames / URLs.
    pub fn filenames(&self) -> Vec<&str> {
        self.products.iter().map(|p| p.filename.as_str()).collect()
    }
}

/// Deduplicates DISP products sharing `(frame_id, reference_datetime, secondary_datetime)` by
/// keeping the highest version and newest `generation_datetime`, returning them sorted by
/// `(frame_id, reference_datetime, secondary_datetime)`.
pub fn deduplicate_disp_products(products: Vec<DispProduct>) -> Vec<DispProduct> {
    let mut best: BTreeMap<(u32, String, String), DispProduct> = BTreeMap::new();
    for prod in products {
        let key = (
            prod.frame_id,
            prod.reference_datetime.clone(),
            prod.secondary_datetime.clone(),
        );
        match best.get(&key) {
            Some(existing)
                if (
                    existing.version_key(),
                    existing.generation_datetime.as_str(),
                ) >= (prod.version_key(), prod.generation_datetime.as_str()) => {}
            _ => {
                best.insert(key, prod);
            }
        }
    }
    best.into_values().collect()
}

/// Query parameters for searching NASA CMR for `OPERA_L3_DISP-S1_V1` frame products.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispSearchParams {
    /// Optional DISP-S1 frame ID (`attribute[]=int,FRAME_NUMBER,<id>`).
    pub frame_id: Option<u32>,
    /// Optional WGS 84 bounding box filter.
    pub bbox: Option<BBox>,
    /// Optional product version filter (default: `Some("1.0")`).
    pub product_version: Option<String>,
    /// Optional inclusive secondary-acquisition start date (`YYYY-MM-DD`).
    pub start: Option<String>,
    /// Optional inclusive secondary-acquisition end date (`YYYY-MM-DD`).
    pub end: Option<String>,
    /// Preferred URL protocol (`https` or `s3`).
    pub url_type: UrlType,
    /// Deduplicate superseded versions for identical `(frame_id, reference, secondary)` pairs.
    pub deduplicate: bool,
    /// Maximum number of raw CMR items to fetch (default: 500, capped at 2000).
    pub limit: usize,
    /// Optional override for the CMR search base URL.
    pub cmr_endpoint: Option<String>,
}

impl Default for DispSearchParams {
    fn default() -> Self {
        Self {
            frame_id: None,
            bbox: None,
            product_version: Some("1.0".to_string()),
            start: None,
            end: None,
            url_type: UrlType::Https,
            deduplicate: true,
            limit: 500,
            cmr_endpoint: None,
        }
    }
}

/// Structured output from [`search_disp_products`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispSearchResult {
    /// Distinct DISP-S1 frame IDs present in the returned products.
    pub frame_ids: Vec<u32>,
    /// Total matching granules reported by CMR (`cmr-hits`).
    pub total_hits: usize,
    /// Number of duplicate `(frame_id, reference, secondary)` entries removed when deduplicating.
    pub duplicates_removed: usize,
    /// Matching DISP-S1 products sorted by `(frame_id, secondary_datetime)`.
    pub products: Vec<DispProduct>,
}

/// Validates and normalizes an optional `YYYY-MM-DD` date boundary into an ISO UTC instant string.
fn normalize_date_bound(raw: Option<&str>, end_of_day: bool) -> Result<Option<String>> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let date = CalendarDate::parse(s)?;
    let suffix = if end_of_day {
        "T23:59:59Z"
    } else {
        "T00:00:00Z"
    };
    Ok(Some(format!("{date}{suffix}")))
}

/// Builds the CMR `granules.umm_json` query URL for [`DispSearchParams`] (pure offline helper).
#[cfg(feature = "http")]
fn build_disp_cmr_url(params: &DispSearchParams, page_size: usize) -> Result<String> {
    let base = params
        .cmr_endpoint
        .as_deref()
        .unwrap_or(DEFAULT_CMR_ENDPOINT)
        .trim_end_matches('/');
    let mut pairs: Vec<(String, String)> = vec![
        ("short_name".into(), DISP_S1_SHORT_NAME.into()),
        ("page_size".into(), page_size.to_string()),
    ];
    if let Some(ver) = params
        .product_version
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        pairs.push(("attribute[]".into(), format!("float,PRODUCT_VERSION,{ver}")));
    }
    if let Some(fid) = params.frame_id {
        pairs.push(("attribute[]".into(), format!("int,FRAME_NUMBER,{fid}")));
    }
    if let Some(bbox) = params.bbox {
        pairs.push(("bounding_box".into(), bbox.to_string()));
    }
    let start_iso = normalize_date_bound(params.start.as_deref(), false)?;
    let end_iso = normalize_date_bound(params.end.as_deref(), true)?;
    if start_iso.is_some() || end_iso.is_some() {
        pairs.push((
            "temporal".into(),
            format!(
                "{},{}",
                start_iso.as_deref().unwrap_or(""),
                end_iso.as_deref().unwrap_or("")
            ),
        ));
    }
    let qs = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    Ok(format!("{base}/granules.umm_json?{qs}"))
}

/// Queries public NASA CMR for `OPERA_L3_DISP-S1_V1` granules matching `params` (no Earthdata login required).
#[cfg(feature = "http")]
pub fn search_disp_products(params: &DispSearchParams) -> Result<DispSearchResult> {
    if params.frame_id.is_none() && params.bbox.is_none() {
        return Err(OperaError::InvalidInput(
            "specify at least one of frame_id or bbox for DISP-S1 search".into(),
        ));
    }
    let limit = params.limit.clamp(1, 2000);
    let page_size = limit.min(500);
    let start_iso = normalize_date_bound(params.start.as_deref(), false)?;
    let end_iso = normalize_date_bound(params.end.as_deref(), true)?;
    if let (Some(s), Some(e)) = (&start_iso, &end_iso)
        && s > e
    {
        return Err(OperaError::InvalidInput(format!(
            "start date '{}' must not be after end date '{}'",
            params.start.as_deref().unwrap_or(""),
            params.end.as_deref().unwrap_or("")
        )));
    }

    let url = build_disp_cmr_url(params, page_size)?;
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(45)))
        .build();
    let agent: ureq::Agent = config.into();

    let mut parsed: Vec<DispProduct> = Vec::new();
    let mut fetched_items: usize = 0;
    let mut total_hits: usize = 0;
    let mut search_after: Option<String> = None;

    while fetched_items < limit {
        let mut req = agent.get(&url).header(
            "User-Agent",
            format!(
                "opera-rs/{} (https://github.com/opengeos/opera-rs)",
                crate::VERSION
            ),
        );
        if let Some(token) = &search_after {
            req = req.header("CMR-Search-After", token);
        }
        let resp = req
            .call()
            .map_err(|e| OperaError::Http(format!("CMR DISP-S1 search failed: {e}")))?;

        if total_hits == 0
            && let Some(h) = resp.headers().get("cmr-hits").and_then(|v| v.to_str().ok())
            && let Ok(n) = h.parse::<usize>()
        {
            total_hits = n;
        }
        let next_after = resp
            .headers()
            .get("cmr-search-after")
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string);

        let text = resp
            .into_body()
            .read_to_string()
            .map_err(|e| OperaError::Http(format!("failed reading CMR DISP-S1 body: {e}")))?;
        let body: Value = serde_json::from_str(&text)?;
        let items = body
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| OperaError::Http("CMR DISP-S1 response missing 'items' array".into()))?;
        if items.is_empty() {
            break;
        }
        let got_count = items.len();
        for item in items {
            if fetched_items >= limit {
                break;
            }
            fetched_items += 1;
            if let Some(umm) = item.get("umm")
                && let Ok(prod) = DispProduct::from_umm(umm, params.url_type)
            {
                // CMR temporal filters match both reference and secondary times; filter strictly
                // on secondary_datetime to match opera_utils.disp.search behavior.
                if let Some(s) = &start_iso
                    && prod.secondary_datetime.as_str() < s.as_str()
                {
                    continue;
                }
                if let Some(e) = &end_iso
                    && prod.secondary_datetime.as_str() > e.as_str()
                {
                    continue;
                }
                parsed.push(prod);
            }
        }
        search_after = next_after;
        if search_after.is_none() || got_count < page_size {
            break;
        }
    }

    let raw_count = parsed.len();
    let mut products = if params.deduplicate {
        deduplicate_disp_products(parsed)
    } else {
        parsed
    };
    let duplicates_removed = raw_count.saturating_sub(products.len());
    products.sort_by(|a, b| {
        a.frame_id
            .cmp(&b.frame_id)
            .then_with(|| a.secondary_datetime.cmp(&b.secondary_datetime))
            .then_with(|| a.reference_datetime.cmp(&b.reference_datetime))
    });

    let frame_ids: Vec<u32> = products
        .iter()
        .map(|p| p.frame_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    Ok(DispSearchResult {
        frame_ids,
        total_hits: total_hits.max(products.len()),
        duplicates_removed,
        products,
    })
}

/// Stub when the `http` feature is disabled.
#[cfg(not(feature = "http"))]
pub fn search_disp_products(_params: &DispSearchParams) -> Result<DispSearchResult> {
    let _ = normalize_date_bound(None, false);
    Err(OperaError::HttpDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_disp_filename_and_builds_deduplicated_stack() {
        let f1 = "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11116_VV_20160705T140755Z_20160729T140756Z_v1.0_20241219T231545Z.nc";
        let f1_newer = "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11116_VV_20160705T140755Z_20160729T140756Z_v1.0_20250318T223016Z.nc";
        let f2 = "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11116_VV_20160705T140755Z_20160810T140756Z_v1.0_20250318T223016Z.nc";

        let p = DispProduct::from_filename(f1).unwrap();
        assert_eq!(p.sensor, "S1");
        assert_eq!(p.acquisition_mode, "IW");
        assert_eq!(p.frame_id, 11116);
        assert_eq!(p.polarization, "VV");
        assert_eq!(p.reference_datetime, "2016-07-05T14:07:55Z");
        assert_eq!(p.secondary_datetime, "2016-07-29T14:07:56Z");
        assert_eq!(p.version, "1.0");
        assert_eq!(p.generation_datetime, "2024-12-19T23:15:45Z");

        let stack = DispProductStack::from_file_list(&[
            f2.to_string(),
            f1.to_string(),
            f1_newer.to_string(),
        ])
        .unwrap();
        assert_eq!(stack.frame_id, 11116);
        assert_eq!(stack.duplicates_removed, 1);
        assert_eq!(stack.products.len(), 2);
        assert_eq!(
            stack.products[0].generation_datetime,
            "2025-03-18T22:30:16Z"
        );
        assert_eq!(
            stack.secondary_dates(),
            vec!["2016-07-29T14:07:56Z", "2016-08-10T14:07:56Z"]
        );
    }

    #[test]
    fn parses_disp_product_from_umm_and_rejects_mixed_frames() {
        let umm = json!({
            "GranuleUR": "OPERA_L3_DISP-S1_IW_F11115_VV_20160810T140735Z_20160903T140736Z_v1.0_20250318T223016Z",
            "RelatedUrls": [
                { "Type": "GET DATA", "URL": "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11115_VV_20160810T140735Z_20160903T140736Z_v1.0_20250318T223016Z.nc" },
                { "Type": "GET DATA VIA DIRECT ACCESS", "URL": "s3://asf-cumulus-prod-protected/OPERA_L3_DISP-S1_IW_F11115_VV_20160810T140735Z_20160903T140736Z_v1.0_20250318T223016Z.nc" }
            ],
            "DataGranule": {
                "ArchiveAndDistributionInformation": [{ "SizeInBytes": 123456789 }]
            }
        });
        let p_https = DispProduct::from_umm(&umm, UrlType::Https).unwrap();
        assert!(p_https.filename.starts_with("https://"));
        assert_eq!(p_https.size_in_bytes, Some(123_456_789));
        let p_s3 = DispProduct::from_umm(&umm, UrlType::S3).unwrap();
        assert!(p_s3.filename.starts_with("s3://"));

        let other = DispProduct::from_filename(
            "OPERA_L3_DISP-S1_IW_F11116_VV_20160705T140755Z_20160729T140756Z_v1.0_20241219T231545Z.nc",
        )
        .unwrap();
        assert!(DispProductStack::new(vec![p_https, other]).is_err());

        #[cfg(feature = "http")]
        assert!(search_disp_products(&DispSearchParams::default()).is_err());

        // Accept VH polarization, reject malformed version components, and skip metadata.nc / keep query strings.
        let vh = DispProduct::from_filename(
            "OPERA_L3_DISP-S1_IW_F11116_VH_20160705T140755Z_20160729T140756Z_v1.0_20241219T231545Z.nc",
        )
        .unwrap();
        assert_eq!(vh.polarization, "VH");
        assert!(
            DispProduct::from_filename(
                "OPERA_L3_DISP-S1_IW_F11116_VV_20160705T140755Z_20160729T140756Z_v1..10_20241219T231545Z.nc"
            )
            .is_err()
        );
        let umm_query = json!({
            "RelatedUrls": [
                { "Type": "GET DATA", "URL": "https://example.org/metadata.nc" },
                { "Type": "GET DATA", "URL": "https://datapool.asf.alaska.edu/OPERA_L3_DISP-S1_IW_F11115_VV_20160810T140735Z_20160903T140736Z_v1.0_20250318T223016Z.nc?download=1" }
            ]
        });
        let p_q = DispProduct::from_umm(&umm_query, UrlType::Https).unwrap();
        assert!(p_q.filename.ends_with("?download=1"));
        assert_eq!(p_q.frame_id, 11115);
    }
}
