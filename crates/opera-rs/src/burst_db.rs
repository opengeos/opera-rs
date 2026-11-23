//! Pure-Rust client and query engine for the official NASA OPERA Sentinel-1 burst & DISP-S1
//! frame database (`opera-adt/burst_db` v0.9.0).
//!
//! Adapted from `opera-adt/opera-utils` (`datasets.py`, `burst_frame_db.py`) with zero GDAL,
//! Shapely, or GeoPandas dependencies. Archives are fetched lazily on demand (when the `http`
//! feature is enabled), SHA-256 verified before caching, and queried directly in Rust.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::DeflateDecoder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bursts::normalize_burst_id;
use crate::error::{OperaError, Result};
use crate::types::BBox;

/// Release version of `opera-adt/burst_db` matching `opera-utils`.
pub const BURST_DB_VERSION: &str = "0.9.0";

/// Base download URL for `opera-adt/burst_db` release archives.
pub const BURST_DB_BASE_URL: &str =
    "https://github.com/opera-adt/burst_db/releases/download/v0.9.0/";

/// Assets published in `opera-adt/burst_db` v0.9.0 with pinned SHA-256 digests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BurstDbAsset {
    /// Mapping from DISP-S1 frame ID (`1..46986`) to EPSG, UTM bounds, and constituent burst IDs.
    FrameToBurst,
    /// Mapping from normalized Sentinel-1 burst ID (`t001_000001_iw1`) to DISP-S1 frame IDs.
    BurstToFrame,
    /// Simplified WGS 84 GeoJSON polygons and orbit pass metadata for all DISP-S1 frames.
    FrameGeometries,
    /// Simplified WGS 84 GeoJSON polygons for all Sentinel-1 burst IDs.
    BurstGeometries,
}

impl BurstDbAsset {
    /// Archive filename in `opera-adt/burst_db` v0.9.0.
    pub const fn filename(self) -> &'static str {
        match self {
            Self::FrameToBurst => "opera-s1-disp-0.9.0-frame-to-burst.json.zip",
            Self::BurstToFrame => "opera-s1-disp-0.9.0-burst-to-frame.json.zip",
            Self::FrameGeometries => "frame-geometries-simple-0.9.0.geojson.zip",
            Self::BurstGeometries => "burst-id-geometries-simple-0.9.0.geojson.zip",
        }
    }

    /// Expected lowercase hex SHA-256 checksum of the release archive.
    pub const fn sha256(self) -> &'static str {
        match self {
            Self::FrameToBurst => {
                "0a0662d47f10e49dc20f1809407916b87565d1ed33c988ba86a3d6547bb4a28f"
            }
            Self::BurstToFrame => {
                "93c458a6324970366d65e3639554d05e9ed46947dee2d32e2728e098336b8c9a"
            }
            Self::FrameGeometries => {
                "115182c420a0446f3f015c16e623fe9535337679796e44a65cf6154392f66eb2"
            }
            Self::BurstGeometries => {
                "4d9a046029e0dbe03a0746f5e7df645d4746e134676751085f55a341eb23f466"
            }
        }
    }

    /// Full release download URL for this asset.
    pub fn url(self) -> String {
        format!("{BURST_DB_BASE_URL}{}", self.filename())
    }
}

/// Sentinel-1 orbit pass direction for a DISP-S1 frame or burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrbitPass {
    /// South-to-north ascending pass.
    Ascending,
    /// North-to-south descending pass.
    Descending,
}

impl OrbitPass {
    /// Canonical uppercase string (`ASCENDING` or `DESCENDING`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ascending => "ASCENDING",
            Self::Descending => "DESCENDING",
        }
    }
}

impl std::fmt::Display for OrbitPass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OrbitPass {
    type Err = OperaError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "ASCENDING" | "ASC" => Ok(Self::Ascending),
            "DESCENDING" | "DESC" => Ok(Self::Descending),
            _ => Err(OperaError::InvalidInput(format!(
                "invalid orbit pass '{s}' (expected ASCENDING or DESCENDING)"
            ))),
        }
    }
}

/// Metadata for one OPERA DISP-S1 frame from `opera-s1-disp-0.9.0-frame-to-burst.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameInfo {
    /// Numeric DISP-S1 frame identifier (e.g. `11114`).
    pub frame_id: u32,
    /// Native UTM EPSG code (e.g. `32610`).
    pub epsg: u32,
    /// Whether the frame intersects land.
    pub is_land: bool,
    /// Whether the frame is in the North America priority processing domain.
    pub is_north_america: bool,
    /// Native projected minimum X (meters).
    pub xmin: f64,
    /// Native projected minimum Y (meters).
    pub ymin: f64,
    /// Native projected maximum X (meters).
    pub xmax: f64,
    /// Native projected maximum Y (meters).
    pub ymax: f64,
    /// Optional WGS 84 (`EPSG:4326`) bounding box computed via `terrio` when the `geo` feature is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox_wgs84: Option<BBox>,
    /// Optional Sentinel-1 orbit pass direction when queried alongside frame geometries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orbit_pass: Option<OrbitPass>,
    /// Normalized Sentinel-1 burst IDs belonging to this frame.
    pub burst_ids: Vec<String>,
}

/// Frame bounding box paired with its EPSG code (matches `opera-utils disp-s1-frame-bbox`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameBBox {
    /// Numeric DISP-S1 frame identifier.
    pub frame_id: u32,
    /// Coordinate reference system EPSG code (`4326` when `latlon = true`, otherwise native UTM).
    pub epsg: u32,
    /// `[xmin, ymin, xmax, ymax]` in the reported EPSG CRS.
    pub bbox: [f64; 4],
}

/// Resolves the local cache directory for `opera-adt/burst_db` archives.
///
/// Honors `OPERA_UTILS_DATA_DIR` (matching `opera-utils`), then `XDG_CACHE_HOME`,
/// `LOCALAPPDATA` (Windows), `$HOME/.cache`, and finally `std::env::temp_dir()`.
pub fn default_cache_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("OPERA_UTILS_DATA_DIR")
        && !explicit.trim().is_empty()
    {
        return PathBuf::from(explicit);
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME")
        && !xdg.trim().is_empty()
    {
        return PathBuf::from(xdg)
            .join("opera-rs")
            .join("burst_db")
            .join(BURST_DB_VERSION);
    }
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA")
        && !local_app_data.trim().is_empty()
    {
        return PathBuf::from(local_app_data)
            .join("opera-rs")
            .join("burst_db")
            .join(BURST_DB_VERSION);
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home)
            .join(".cache")
            .join("opera-rs")
            .join("burst_db")
            .join(BURST_DB_VERSION);
    }
    std::env::temp_dir()
        .join("opera-rs")
        .join("burst_db")
        .join(BURST_DB_VERSION)
}

/// Returns the local path to a cached `opera-adt/burst_db` v0.9.0 archive, downloading and
/// SHA-256 verifying it on first use.
#[cfg(feature = "http")]
pub fn fetch_burst_db_file(asset: BurstDbAsset, cache_dir: Option<&Path>) -> Result<PathBuf> {
    let dir = cache_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(default_cache_dir);
    let target = dir.join(asset.filename());
    if target.is_file() {
        let existing = std::fs::read(&target)?;
        if sha256_hex(&existing) == asset.sha256() {
            return Ok(target);
        }
    }

    std::fs::create_dir_all(&dir)?;
    let url = asset.url();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(120)))
        .build();
    let agent: ureq::Agent = config.into();
    let resp = agent
        .get(&url)
        .header(
            "User-Agent",
            format!(
                "opera-rs/{} (https://github.com/opengeos/opera-rs)",
                crate::VERSION
            ),
        )
        .call()
        .map_err(|e| OperaError::Http(format!("failed to download {url}: {e}")))?;
    let mut reader = resp.into_body().into_reader();
    let mut bytes = Vec::new();
    // Cap at 128 MiB as a safety guard against runaway responses.
    let limit: u64 = 128 * 1024 * 1024;
    reader
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| OperaError::Http(format!("failed reading {url}: {e}")))?;
    if bytes.len() as u64 > limit {
        return Err(OperaError::Http(format!(
            "burst_db asset {} exceeded {} MiB safety limit",
            asset.filename(),
            limit / (1024 * 1024)
        )));
    }

    let got = sha256_hex(&bytes);
    if got != asset.sha256() {
        return Err(OperaError::Http(format!(
            "SHA-256 mismatch for {}: expected {}, got {}",
            asset.filename(),
            asset.sha256(),
            got
        )));
    }

    let pid = std::process::id();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut tmp_path = None;
    let mut tmp_file = None;
    for attempt in 0u32..32 {
        let candidate = dir.join(format!(
            ".{}.{}.{}.{}.tmp",
            asset.filename(),
            pid,
            nonce,
            attempt
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(f) => {
                tmp_path = Some(candidate);
                tmp_file = Some(f);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(OperaError::Io(e)),
        }
    }
    let tmp_path = tmp_path.ok_or_else(|| {
        OperaError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate unique temp file for burst_db cache",
        ))
    })?;
    let mut tmp_file = tmp_file.expect("file opened alongside path");
    if let Err(err) = std::io::Write::write_all(&mut tmp_file, &bytes) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(OperaError::Io(err));
    }
    drop(tmp_file);
    if let Err(err) = std::fs::rename(&tmp_path, &target) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(OperaError::Io(err));
    }
    Ok(target)
}

/// Stub when the `http` feature is disabled.
#[cfg(not(feature = "http"))]
pub fn fetch_burst_db_file(asset: BurstDbAsset, cache_dir: Option<&Path>) -> Result<PathBuf> {
    let dir = cache_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(default_cache_dir);
    let target = dir.join(asset.filename());
    if target.is_file() {
        return Ok(target);
    }
    Err(OperaError::HttpDisabled)
}

fn resolve_asset_path(asset: BurstDbAsset, explicit: Option<&Path>) -> Result<PathBuf> {
    match explicit {
        Some(path) => Ok(path.to_path_buf()),
        None => fetch_burst_db_file(asset, None),
    }
}

/// Reads either a `.zip` archive containing a JSON/GeoJSON file or a plain JSON/GeoJSON file
/// into a [`serde_json::Value`].
pub fn read_zipped_json(path: impl AsRef<Path>) -> Result<Value> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let is_zip = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
        || bytes.starts_with(b"PK\x03\x04");
    let json_bytes = if is_zip {
        extract_first_zip_entry(&bytes)?
    } else {
        bytes
    };
    Ok(serde_json::from_slice(&json_bytes)?)
}

/// Reprojects a bounding box `[xmin, ymin, xmax, ymax]` from `src_epsg` to `EPSG:4326`
/// using pure-Rust `terrio::crs::Transformer`.
#[cfg(feature = "geo")]
pub fn reproject_bbox_to_wgs84(src_epsg: u32, bounds: [f64; 4]) -> Result<BBox> {
    if src_epsg == 4326 {
        return BBox::from_array(bounds);
    }
    let src = terrio::crs::SpatialRef::parse(&format!("EPSG:{src_epsg}"))?;
    let dst = terrio::crs::SpatialRef::parse("EPSG:4326")?;
    let t = terrio::crs::Transformer::new(&src, &dst)?;
    let (w, s, e, n) = t.transform_bounds((bounds[0], bounds[1], bounds[2], bounds[3]))?;
    BBox::new(
        w.clamp(-180.0, 180.0),
        s.clamp(-90.0, 90.0),
        e.clamp(-180.0, 180.0),
        n.clamp(-90.0, 90.0),
    )
}

#[cfg(not(feature = "geo"))]
pub fn reproject_bbox_to_wgs84(src_epsg: u32, bounds: [f64; 4]) -> Result<BBox> {
    if src_epsg == 4326 {
        return BBox::from_array(bounds);
    }
    Err(OperaError::InvalidInput(
        "reprojecting UTM frame bounds to WGS 84 requires the `geo` feature".into(),
    ))
}

/// Looks up one DISP-S1 frame's EPSG, UTM bounds, optional WGS 84 bounds, and constituent burst IDs.
pub fn get_frame_to_burst_mapping(frame_id: u32, json_file: Option<&Path>) -> Result<FrameInfo> {
    let path = resolve_asset_path(BurstDbAsset::FrameToBurst, json_file)?;
    let root = read_zipped_json(&path)?;
    let key = frame_id.to_string();
    let entry = root
        .get("data")
        .and_then(|d| d.get(&key))
        .ok_or_else(|| OperaError::NoResults(format!("DISP-S1 frame {frame_id} not found")))?;

    let epsg = entry
        .get("epsg")
        .and_then(Value::as_u64)
        .ok_or_else(|| OperaError::InvalidInput(format!("frame {frame_id} missing epsg")))?
        as u32;
    let is_land = value_as_bool(entry.get("is_land")).unwrap_or(false);
    let is_north_america = value_as_bool(entry.get("is_north_america")).unwrap_or(false);
    let xmin = entry
        .get("xmin")
        .and_then(Value::as_f64)
        .ok_or_else(|| OperaError::InvalidInput(format!("frame {frame_id} missing xmin")))?;
    let ymin = entry
        .get("ymin")
        .and_then(Value::as_f64)
        .ok_or_else(|| OperaError::InvalidInput(format!("frame {frame_id} missing ymin")))?;
    let xmax = entry
        .get("xmax")
        .and_then(Value::as_f64)
        .ok_or_else(|| OperaError::InvalidInput(format!("frame {frame_id} missing xmax")))?;
    let ymax = entry
        .get("ymax")
        .and_then(Value::as_f64)
        .ok_or_else(|| OperaError::InvalidInput(format!("frame {frame_id} missing ymax")))?;
    let burst_ids: Vec<String> = entry
        .get("burst_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(normalize_burst_id)
                .collect()
        })
        .unwrap_or_default();

    let bbox_wgs84 = reproject_bbox_to_wgs84(epsg, [xmin, ymin, xmax, ymax]).ok();

    Ok(FrameInfo {
        frame_id,
        epsg,
        is_land,
        is_north_america,
        xmin,
        ymin,
        xmax,
        ymax,
        bbox_wgs84,
        orbit_pass: None,
        burst_ids,
    })
}

/// Returns the normalized Sentinel-1 burst IDs belonging to one DISP-S1 frame ID.
pub fn get_burst_ids_for_frame(frame_id: u32, json_file: Option<&Path>) -> Result<Vec<String>> {
    Ok(get_frame_to_burst_mapping(frame_id, json_file)?.burst_ids)
}

/// Returns the DISP-S1 frame ID(s) containing a Sentinel-1 burst ID
/// (most bursts belong to 1 frame; bursts in frame overlaps belong to 2 frames).
pub fn get_frame_ids_for_burst(burst_id: &str, json_file: Option<&Path>) -> Result<Vec<u32>> {
    let norm = normalize_burst_id(burst_id);
    let path = resolve_asset_path(BurstDbAsset::BurstToFrame, json_file)?;
    let root = read_zipped_json(&path)?;
    let entry = root
        .get("data")
        .and_then(|d| d.get(&norm))
        .ok_or_else(|| OperaError::NoResults(format!("burst ID '{norm}' not found in burst_db")))?;
    let ids: Vec<u32> = entry
        .get("frame_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| OperaError::InvalidInput(format!("burst '{norm}' missing frame_ids")))?
        .iter()
        .filter_map(Value::as_u64)
        .map(|n| n as u32)
        .collect();
    Ok(ids)
}

/// Returns the bounding box of a DISP-S1 frame in either native UTM (`latlon = false`) or
/// WGS 84 `EPSG:4326` (`latlon = true`).
pub fn get_frame_bbox(frame_id: u32, latlon: bool, json_file: Option<&Path>) -> Result<FrameBBox> {
    let info = get_frame_to_burst_mapping(frame_id, json_file)?;
    if latlon {
        let wgs = reproject_bbox_to_wgs84(info.epsg, [info.xmin, info.ymin, info.xmax, info.ymax])?;
        Ok(FrameBBox {
            frame_id,
            epsg: 4326,
            bbox: wgs.to_array(),
        })
    } else {
        Ok(FrameBBox {
            frame_id,
            epsg: info.epsg,
            bbox: [info.xmin, info.ymin, info.xmax, info.ymax],
        })
    }
}

/// Returns a GeoJSON `FeatureCollection` of DISP-S1 frame geometries, optionally filtered to `frame_ids`.
pub fn get_frame_geojson(frame_ids: Option<&[u32]>, json_file: Option<&Path>) -> Result<Value> {
    let path = resolve_asset_path(BurstDbAsset::FrameGeometries, json_file)?;
    let mut fc = read_zipped_json(&path)?;
    if let Some(ids) = frame_ids {
        let want: BTreeSet<u32> = ids.iter().copied().collect();
        let filtered: Vec<Value> = fc
            .get("features")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| {
                feature_frame_id(f)
                    .map(|id| want.contains(&id))
                    .unwrap_or(false)
            })
            .collect();
        if let Some(obj) = fc.as_object_mut() {
            obj.insert("features".into(), Value::Array(filtered));
        }
    }
    Ok(fc)
}

/// Returns a GeoJSON `FeatureCollection` of Sentinel-1 burst geometries, optionally filtered to `burst_ids`.
pub fn get_burst_id_geojson(
    burst_ids: Option<&[String]>,
    json_file: Option<&Path>,
) -> Result<Value> {
    let path = resolve_asset_path(BurstDbAsset::BurstGeometries, json_file)?;
    let mut fc = read_zipped_json(&path)?;
    if let Some(ids) = burst_ids {
        let want: BTreeSet<String> = ids.iter().map(|b| normalize_burst_id(b)).collect();
        let filtered: Vec<Value> = fc
            .get("features")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|f| {
                f.get("properties")
                    .and_then(|p| p.get("burst_id_jpl"))
                    .and_then(Value::as_str)
                    .map(|b| want.contains(&normalize_burst_id(b)))
                    .unwrap_or(false)
            })
            .collect();
        if let Some(obj) = fc.as_object_mut() {
            obj.insert("features".into(), Value::Array(filtered));
        }
    }
    Ok(fc)
}

/// Returns the Sentinel-1 orbit pass direction (`ASCENDING` or `DESCENDING`) for a DISP-S1 frame.
pub fn get_frame_orbit_pass(frame_id: u32, json_file: Option<&Path>) -> Result<OrbitPass> {
    let fc = get_frame_geojson(Some(&[frame_id]), json_file)?;
    let feat = fc
        .get("features")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .ok_or_else(|| OperaError::NoResults(format!("DISP-S1 frame {frame_id} not found")))?;
    let raw = feat
        .get("properties")
        .and_then(|p| p.get("orbit_pass"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            OperaError::InvalidInput(format!("frame {frame_id} missing orbit_pass property"))
        })?;
    raw.parse()
}

/// Returns a GeoJSON `FeatureCollection` of all DISP-S1 frames whose simplified WGS 84 geometry
/// intersects `bbox` (pure-Rust polygon-bbox intersection, no GeoPandas or Shapely required).
pub fn get_intersecting_frames(bbox: BBox, json_file: Option<&Path>) -> Result<Value> {
    let path = resolve_asset_path(BurstDbAsset::FrameGeometries, json_file)?;
    let fc = read_zipped_json(&path)?;
    let mut matched = Vec::new();
    if let Some(features) = fc.get("features").and_then(Value::as_array) {
        for feat in features {
            if let Some(geom) = feat.get("geometry")
                && geojson_geometry_intersects_bbox(geom, bbox)
            {
                let mut cloned = feat.clone();
                // Ensure `frame_id` is also present inside `properties` so vector writers
                // (GeoPackage / Shapefile) preserve the frame identifier as an attribute column.
                if let Some(fid) = feature_frame_id(&cloned)
                    && let Some(props) = cloned.get_mut("properties").and_then(Value::as_object_mut)
                {
                    props.entry("frame_id").or_insert_with(|| json!(fid));
                }
                matched.push(cloned);
            }
        }
    }
    Ok(json!({
        "type": "FeatureCollection",
        "features": matched,
    }))
}

/// Returns a GeoJSON `FeatureCollection` of all DISP-S1 frames whose simplified WGS 84 geometry
/// contains the point `(lon, lat)` (exact point-in-polygon with hole handling).
pub fn get_intersecting_frames_for_point(
    lon: f64,
    lat: f64,
    json_file: Option<&Path>,
) -> Result<Value> {
    if !lon.is_finite()
        || !lat.is_finite()
        || !(-180.0..=180.0).contains(&lon)
        || !(-90.0..=90.0).contains(&lat)
    {
        return Err(OperaError::InvalidInput(format!(
            "invalid WGS 84 coordinates ({lon}, {lat})"
        )));
    }
    let path = resolve_asset_path(BurstDbAsset::FrameGeometries, json_file)?;
    let fc = read_zipped_json(&path)?;
    let mut matched = Vec::new();
    if let Some(features) = fc.get("features").and_then(Value::as_array) {
        for feat in features {
            if let Some(geom) = feat.get("geometry")
                && geojson_geometry_contains_point(geom, [lon, lat])
            {
                let mut cloned = feat.clone();
                if let Some(fid) = feature_frame_id(&cloned)
                    && let Some(props) = cloned.get_mut("properties").and_then(Value::as_object_mut)
                {
                    props.entry("frame_id").or_insert_with(|| json!(fid));
                }
                matched.push(cloned);
            }
        }
    }
    Ok(json!({
        "type": "FeatureCollection",
        "features": matched,
    }))
}

/// Extracts integer frame IDs from a `get_intersecting_frames` or `get_frame_geojson` FeatureCollection.
pub fn frame_ids_from_geojson(fc: &Value) -> Vec<u32> {
    fc.get("features")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(feature_frame_id).collect())
        .unwrap_or_default()
}

fn feature_frame_id(feat: &Value) -> Option<u32> {
    if let Some(n) = feat.get("id").and_then(Value::as_u64) {
        return Some(n as u32);
    }
    if let Some(s) = feat.get("id").and_then(Value::as_str)
        && let Ok(n) = s.parse::<u32>()
    {
        return Some(n);
    }
    feat.get("properties")
        .and_then(|p| p.get("frame_id"))
        .and_then(Value::as_u64)
        .map(|n| n as u32)
}

fn value_as_bool(v: Option<&Value>) -> Option<bool> {
    match v {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust polygon / bbox intersection (Polygon & MultiPolygon with holes)
// ---------------------------------------------------------------------------

fn geojson_geometry_contains_point(geom: &Value, pt: [f64; 2]) -> bool {
    let gtype = geom.get("type").and_then(Value::as_str).unwrap_or("");
    let coords = match geom.get("coordinates") {
        Some(c) => c,
        None => return false,
    };
    let contains_poly = |rings: &[Vec<[f64; 2]>]| -> bool {
        (point_in_ring(pt, &rings[0]) || point_on_ring_boundary(pt, &rings[0]))
            && !rings[1..].iter().any(|h| point_in_ring(pt, h))
    };
    match gtype {
        "Polygon" => parse_polygon_rings(coords)
            .map(|rings| contains_poly(&rings))
            .unwrap_or(false),
        "MultiPolygon" => coords
            .as_array()
            .map(|polys| {
                polys.iter().any(|p| {
                    parse_polygon_rings(p)
                        .map(|rings| contains_poly(&rings))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn point_on_ring_boundary(pt: [f64; 2], ring: &[[f64; 2]]) -> bool {
    ring.windows(2)
        .any(|w| orient(w[0], w[1], pt).abs() <= 1e-12 && on_segment(w[0], w[1], pt))
}

fn geojson_geometry_intersects_bbox(geom: &Value, bbox: BBox) -> bool {
    let gtype = geom.get("type").and_then(Value::as_str).unwrap_or("");
    let coords = match geom.get("coordinates") {
        Some(c) => c,
        None => return false,
    };
    match gtype {
        "Polygon" => parse_polygon_rings(coords)
            .map(|rings| polygon_intersects_bbox(&rings, bbox))
            .unwrap_or(false),
        "MultiPolygon" => coords
            .as_array()
            .map(|polys| {
                polys.iter().any(|p| {
                    parse_polygon_rings(p)
                        .map(|rings| polygon_intersects_bbox(&rings, bbox))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false),
        _ => false,
    }
}

fn parse_polygon_rings(val: &Value) -> Option<Vec<Vec<[f64; 2]>>> {
    let rings_json = val.as_array()?;
    let mut rings = Vec::with_capacity(rings_json.len());
    for ring_val in rings_json {
        let pts_json = ring_val.as_array()?;
        let mut ring = Vec::with_capacity(pts_json.len());
        for pt in pts_json {
            let arr = pt.as_array()?;
            if arr.len() < 2 {
                continue;
            }
            let x = arr[0].as_f64()?;
            let y = arr[1].as_f64()?;
            ring.push([x, y]);
        }
        if ring.len() >= 3 {
            if ring.first() != ring.last() {
                ring.push(ring[0]);
            }
            rings.push(ring);
        }
    }
    (!rings.is_empty()).then_some(rings)
}

fn polygon_intersects_bbox(rings: &[Vec<[f64; 2]>], bbox: BBox) -> bool {
    let exterior = &rings[0];
    // 1. Fast bounding-box rejection on exterior ring.
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &[x, y] in exterior {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if max_x < bbox.west || min_x > bbox.east || max_y < bbox.south || min_y > bbox.north {
        return false;
    }

    // 2. Any exterior vertex inside bbox -> intersects.
    if exterior
        .iter()
        .any(|&[x, y]| x >= bbox.west && x <= bbox.east && y >= bbox.south && y <= bbox.north)
    {
        return true;
    }

    // 3. Any bbox corner inside the polygon (inside exterior and not inside any hole) -> intersects.
    let corners = [
        [bbox.west, bbox.south],
        [bbox.east, bbox.south],
        [bbox.east, bbox.north],
        [bbox.west, bbox.north],
    ];
    for c in corners {
        if point_in_ring(c, exterior) && !rings[1..].iter().any(|h| point_in_ring(c, h)) {
            return true;
        }
    }

    // 4. Any exterior edge intersects any bbox edge -> intersects.
    let bbox_edges = [
        ([bbox.west, bbox.south], [bbox.east, bbox.south]),
        ([bbox.east, bbox.south], [bbox.east, bbox.north]),
        ([bbox.east, bbox.north], [bbox.west, bbox.north]),
        ([bbox.west, bbox.north], [bbox.west, bbox.south]),
    ];
    for w in exterior.windows(2) {
        for &(b0, b1) in &bbox_edges {
            if segments_intersect(w[0], w[1], b0, b1) {
                return true;
            }
        }
    }

    false
}

fn point_in_ring(pt: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let [px, py] = pt;
    let mut inside = false;
    for w in ring.windows(2) {
        let [x1, y1] = w[0];
        let [x2, y2] = w[1];
        if ((y1 > py) != (y2 > py)) && (px < (x2 - x1) * (py - y1) / (y2 - y1) + x1) {
            inside = !inside;
        }
    }
    inside
}

fn orient(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    p[0] >= a[0].min(b[0]) - 1e-12
        && p[0] <= a[0].max(b[0]) + 1e-12
        && p[1] >= a[1].min(b[1]) - 1e-12
        && p[1] <= a[1].max(b[1]) + 1e-12
}

fn segments_intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    if (o1 > 0.0 && o2 < 0.0 || o1 < 0.0 && o2 > 0.0)
        && (o3 > 0.0 && o4 < 0.0 || o3 < 0.0 && o4 > 0.0)
    {
        return true;
    }
    if o1.abs() <= 1e-12 && on_segment(a, b, c) {
        return true;
    }
    if o2.abs() <= 1e-12 && on_segment(a, b, d) {
        return true;
    }
    if o3.abs() <= 1e-12 && on_segment(c, d, a) {
        return true;
    }
    if o4.abs() <= 1e-12 && on_segment(c, d, b) {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Minimal pure-Rust ZIP central-directory extractor (Stored + Deflate)
// ---------------------------------------------------------------------------

fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(buf.get(off..off + 2)?.try_into().ok()?))
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(off..off + 4)?.try_into().ok()?))
}

/// Extracts the first non-directory entry from a standard ZIP archive using Central Directory
/// metadata (so entries with data descriptors are handled accurately).
pub(crate) fn extract_first_zip_entry(zip_bytes: &[u8]) -> Result<Vec<u8>> {
    if zip_bytes.len() < 22 {
        return Err(OperaError::InvalidInput("truncated ZIP archive".into()));
    }
    // Locate End of Central Directory (EOCD) signature `PK\x05\x06` scanning backwards.
    let search_start = zip_bytes.len().saturating_sub(65_557);
    let mut eocd_off = None;
    for pos in (search_start..=zip_bytes.len() - 22).rev() {
        if &zip_bytes[pos..pos + 4] == b"PK\x05\x06" {
            eocd_off = Some(pos);
            break;
        }
    }
    let eocd = eocd_off.ok_or_else(|| {
        OperaError::InvalidInput("invalid ZIP archive: missing EOCD record".into())
    })?;
    let cd_entries = read_u16_le(zip_bytes, eocd + 10)
        .ok_or_else(|| OperaError::InvalidInput("truncated ZIP EOCD".into()))?;
    let cd_offset = read_u32_le(zip_bytes, eocd + 16)
        .ok_or_else(|| OperaError::InvalidInput("truncated ZIP EOCD".into()))?
        as usize;

    let mut pos = cd_offset;
    for _ in 0..cd_entries {
        if zip_bytes.get(pos..pos + 4) != Some(b"PK\x01\x02") {
            return Err(OperaError::InvalidInput(
                "invalid ZIP central directory entry".into(),
            ));
        }
        let method = read_u16_le(zip_bytes, pos + 10).unwrap_or(0);
        let comp_size = read_u32_le(zip_bytes, pos + 20).unwrap_or(0) as usize;
        let name_len = read_u16_le(zip_bytes, pos + 28).unwrap_or(0) as usize;
        let extra_len = read_u16_le(zip_bytes, pos + 30).unwrap_or(0) as usize;
        let comment_len = read_u16_le(zip_bytes, pos + 32).unwrap_or(0) as usize;
        let local_hdr_off = read_u32_le(zip_bytes, pos + 42).unwrap_or(0) as usize;
        let name_bytes = zip_bytes
            .get(pos + 46..pos + 46 + name_len)
            .ok_or_else(|| OperaError::InvalidInput("truncated ZIP entry name".into()))?;
        pos += 46 + name_len + extra_len + comment_len;

        if name_bytes.ends_with(b"/") {
            continue;
        }

        if zip_bytes.get(local_hdr_off..local_hdr_off + 4) != Some(b"PK\x03\x04") {
            return Err(OperaError::InvalidInput("invalid ZIP local header".into()));
        }
        let lh_name_len = read_u16_le(zip_bytes, local_hdr_off + 26).unwrap_or(0) as usize;
        let lh_extra_len = read_u16_le(zip_bytes, local_hdr_off + 28).unwrap_or(0) as usize;
        let data_start = local_hdr_off + 30 + lh_name_len + lh_extra_len;
        let data_end = data_start
            .checked_add(comp_size)
            .ok_or_else(|| OperaError::InvalidInput("ZIP entry size overflow".into()))?;
        let comp = zip_bytes
            .get(data_start..data_end)
            .ok_or_else(|| OperaError::InvalidInput("truncated ZIP entry payload".into()))?;

        return match method {
            0 => Ok(comp.to_vec()),
            8 => {
                let mut decoder = DeflateDecoder::new(comp);
                let mut out = Vec::new();
                decoder.read_to_end(&mut out)?;
                Ok(out)
            }
            other => Err(OperaError::InvalidInput(format!(
                "unsupported ZIP compression method {other}"
            ))),
        };
    }

    Err(OperaError::InvalidInput(
        "ZIP archive contains no file entries".into(),
    ))
}

// ---------------------------------------------------------------------------
// Minimal pure-Rust FIPS 180-4 SHA-256 (zero external dependencies)
// ---------------------------------------------------------------------------

#[cfg_attr(not(feature = "http"), allow(dead_code))]
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((data.len() + 9).div_ceil(64) * 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(&mut out, "{word:08x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write;

    fn make_single_file_zip(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(payload).unwrap();
        let comp = encoder.finish().unwrap();

        let mut zip = Vec::new();
        let local_off = 0u32;
        // Local file header
        zip.extend_from_slice(b"PK\x03\x04");
        zip.extend_from_slice(&20u16.to_le_bytes()); // version needed
        zip.extend_from_slice(&0u16.to_le_bytes()); // flags
        zip.extend_from_slice(&8u16.to_le_bytes()); // deflate
        zip.extend_from_slice(&0u16.to_le_bytes()); // mod time
        zip.extend_from_slice(&0u16.to_le_bytes()); // mod date
        zip.extend_from_slice(&0u32.to_le_bytes()); // crc (unused by our reader)
        zip.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes()); // extra len
        zip.extend_from_slice(name.as_bytes());
        zip.extend_from_slice(&comp);

        let cd_off = zip.len() as u32;
        // Central directory header
        zip.extend_from_slice(b"PK\x01\x02");
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&20u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&8u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u32.to_le_bytes());
        zip.extend_from_slice(&local_off.to_le_bytes());
        zip.extend_from_slice(name.as_bytes());

        let cd_size = (zip.len() as u32) - cd_off;
        // EOCD
        zip.extend_from_slice(b"PK\x05\x06");
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&1u16.to_le_bytes());
        zip.extend_from_slice(&cd_size.to_le_bytes());
        zip.extend_from_slice(&cd_off.to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn reads_zipped_frame_and_burst_fixtures_and_intersects_bbox() {
        let dir = tempfile::tempdir().unwrap();

        let f2b_json = serde_json::to_vec(&json!({
            "data": {
                "11114": {
                    "epsg": 32610,
                    "is_land": true,
                    "is_north_america": true,
                    "xmin": 546450,
                    "ymin": 4204110,
                    "xmax": 833790,
                    "ymax": 4409070,
                    "burst_ids": ["t042_088905_iw1", "t042_088905_iw2"]
                }
            }
        }))
        .unwrap();
        let f2b_path = dir.path().join("frame-to-burst.json.zip");
        std::fs::write(
            &f2b_path,
            make_single_file_zip("frame-to-burst.json", &f2b_json),
        )
        .unwrap();

        let info = get_frame_to_burst_mapping(11114, Some(&f2b_path)).unwrap();
        assert_eq!(info.epsg, 32610);
        assert!(info.is_land);
        assert_eq!(info.burst_ids, vec!["t042_088905_iw1", "t042_088905_iw2"]);

        let bbox_utm = get_frame_bbox(11114, false, Some(&f2b_path)).unwrap();
        assert_eq!(bbox_utm.epsg, 32610);
        assert_eq!(bbox_utm.bbox, [546450.0, 4204110.0, 833790.0, 4409070.0]);

        #[cfg(feature = "geo")]
        {
            let bbox_ll = get_frame_bbox(11114, true, Some(&f2b_path)).unwrap();
            assert_eq!(bbox_ll.epsg, 4326);
            assert!(bbox_ll.bbox[0] < -121.0 && bbox_ll.bbox[2] > -120.0);
            assert!(bbox_ll.bbox[1] > 37.0 && bbox_ll.bbox[3] < 40.5);
        }

        let b2f_json = serde_json::to_vec(&json!({
            "data": {
                "t042_088905_iw1": { "frame_ids": [11114, 11115] }
            }
        }))
        .unwrap();
        let b2f_path = dir.path().join("burst-to-frame.json.zip");
        std::fs::write(
            &b2f_path,
            make_single_file_zip("burst-to-frame.json", &b2f_json),
        )
        .unwrap();
        assert_eq!(
            get_frame_ids_for_burst("T042-088905-IW1", Some(&b2f_path)).unwrap(),
            vec![11114, 11115]
        );

        let geoms_json = serde_json::to_vec(&json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "id": 11114,
                    "properties": { "is_land": 1, "is_north_america": true, "orbit_pass": "DESCENDING" },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[[-122.5, 37.9], [-119.1, 37.9], [-119.1, 39.8], [-122.5, 39.8], [-122.5, 37.9]]]
                    }
                },
                {
                    "type": "Feature",
                    "id": 20697,
                    "properties": { "is_land": 1, "is_north_america": true, "orbit_pass": "ASCENDING" },
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[[-104.0, 30.5], [-101.0, 30.5], [-101.0, 32.5], [-104.0, 32.5], [-104.0, 30.5]]]
                    }
                }
            ]
        }))
        .unwrap();
        let geoms_path = dir.path().join("frames.geojson.zip");
        std::fs::write(
            &geoms_path,
            make_single_file_zip("frames.geojson", &geoms_json),
        )
        .unwrap();

        assert_eq!(
            get_frame_orbit_pass(11114, Some(&geoms_path)).unwrap(),
            OrbitPass::Descending
        );
        let hits = get_intersecting_frames(
            BBox::new(-121.0, 38.0, -120.0, 39.0).unwrap(),
            Some(&geoms_path),
        )
        .unwrap();
        assert_eq!(frame_ids_from_geojson(&hits), vec![11114]);
    }
}
