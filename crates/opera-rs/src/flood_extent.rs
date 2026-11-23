//! Observed flood-extent polygon vectorization from OPERA DSWx water-only tiles.
//!
//! Pure-Rust port of `opengeos/geolibre-nasa-opera/src/lib/opera/flood-extent.ts`:
//! 1. Select the highest Web Mercator zoom staying within `max_tiles_across`.
//! 2. Fetch DSWx water-only PNG tiles (open water + partial surface water opaque;
//!    not-water, cloud, snow, ocean, and fill transparent) from `titiler-cmr` across
//!    DSWx-HLS and/or cloud-penetrating DSWx-S1 scenes, and union into a binary mask.
//! 3. Trace mask boundaries with a clockwise edge walk, split pinched vertices into
//!    simple rings, drop collinear nodes, project from Web Mercator pixels to WGS 84
//!    lon/lat, nest dry-land holes via even-odd containment, enforce RFC 7946 winding,
//!    and filter out tiny polygons below `min_area_km2`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(feature = "http")]
use crate::DEFAULT_TITILER_ENDPOINT;
#[cfg(feature = "http")]
use crate::cmr::{CmrSearchParams, search_granules};
use crate::error::{OperaError, Result};
#[cfg(feature = "http")]
use crate::products::{DSWX_WTR_WATER_ONLY_COLORMAP_JSON, find_product};
#[cfg(feature = "http")]
use crate::titiler::{TitilerQuery, build_tile_url_template};
use crate::types::BBox;

/// Tile edge length in pixels for Web Mercator XYZ grids.
pub const TILE_SIZE: usize = 256;
const MAX_TILES_ACROSS_CAP: usize = 32;

/// Decoded RGBA raster image.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    pub width: usize,
    pub height: usize,
    /// Packed `RGBA` bytes (`width * height * 4`).
    pub data: Vec<u8>,
}

/// Maximum supported Web Mercator zoom level (guards against bit-shift overflow).
pub const MAX_MERCATOR_ZOOM: u8 = 24;

/// Converts WGS 84 `(lon, lat)` to global Web Mercator pixel coordinates at `zoom`.
pub fn lon_lat_to_pixel(lon: f64, lat: f64, zoom: u8) -> (f64, f64) {
    let z = zoom.min(MAX_MERCATOR_ZOOM);
    let scale = (TILE_SIZE as f64) * (1u64 << z) as f64;
    let x = ((lon + 180.0) / 360.0) * scale;
    let sin_lat = lat.to_radians().sin().clamp(-0.999_999_9, 0.999_999_9);
    let y = (0.5 - ((1.0 + sin_lat) / (1.0 - sin_lat)).ln() / (4.0 * std::f64::consts::PI)) * scale;
    (x, y)
}

/// Inverse of [`lon_lat_to_pixel`]: global Web Mercator pixel to WGS 84 `[lon, lat]`.
pub fn pixel_to_lon_lat(px: f64, py: f64, zoom: u8) -> [f64; 2] {
    let z = zoom.min(MAX_MERCATOR_ZOOM);
    let scale = (TILE_SIZE as f64) * (1u64 << z) as f64;
    let lon = (px / scale) * 360.0 - 180.0;
    let n = std::f64::consts::PI - 2.0 * std::f64::consts::PI * (py / scale);
    let lat = (0.5 * (n.exp() - (-n).exp())).atan().to_degrees();
    [lon, lat]
}

/// Picks the highest zoom level whose actual tile grid (columns and rows) stays within
/// `max_tiles_across` tiles along both axes.
pub fn choose_zoom(bbox: BBox, max_tiles_across: usize) -> Result<u8> {
    let safe = bbox.clamp_mercator()?;
    let budget = max_tiles_across.clamp(1, MAX_TILES_ACROSS_CAP) as u32;
    for z in (0..=16u8).rev() {
        let grid = tile_grid_for_bbox(safe, z)?;
        let cols = grid.tx1.saturating_sub(grid.tx0) + 1;
        let rows = grid.ty1.saturating_sub(grid.ty0) + 1;
        if cols <= budget && rows <= budget {
            return Ok(z);
        }
    }
    Ok(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Edge {
    sx: i32,
    sy: i32,
    ex: i32,
    ey: i32,
}

fn cw_index(dx: i32, dy: i32) -> i32 {
    match (dx, dy) {
        (1, 0) => 0,
        (0, 1) => 1,
        (-1, 0) => 2,
        _ => 3,
    }
}

/// Traces boundary edges of truthy cells in a `width x height` binary mask into closed integer
/// node rings `(0..=width, 0..=height)`.
pub fn trace_mask_rings(mask: &[u8], width: usize, height: usize) -> Vec<Vec<[i32; 2]>> {
    let at = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let (ux, uy) = (x as usize, y as usize);
        ux < width && uy < height && mask[uy * width + ux] != 0
    };

    let mut edges_by_start: HashMap<(i32, i32), Vec<Edge>> = HashMap::new();
    let mut add_edge = |sx: i32, sy: i32, ex: i32, ey: i32| {
        edges_by_start
            .entry((sx, sy))
            .or_default()
            .push(Edge { sx, sy, ex, ey });
    };

    for y in 0..height as i32 {
        for x in 0..width as i32 {
            if !at(x, y) {
                continue;
            }
            if !at(x, y - 1) {
                add_edge(x, y, x + 1, y);
            }
            if !at(x + 1, y) {
                add_edge(x + 1, y, x + 1, y + 1);
            }
            if !at(x, y + 1) {
                add_edge(x + 1, y + 1, x, y + 1);
            }
            if !at(x - 1, y) {
                add_edge(x, y + 1, x, y);
            }
        }
    }

    // Deterministic iteration order across platforms.
    let mut start_keys: Vec<(i32, i32)> = edges_by_start.keys().copied().collect();
    start_keys.sort_unstable();

    let mut used: HashSet<Edge> = HashSet::new();
    let mut rings: Vec<Vec<[i32; 2]>> = Vec::new();

    for key in start_keys {
        let starts = edges_by_start[&key].clone();
        for start in starts {
            if used.contains(&start) {
                continue;
            }
            let mut ring = vec![[start.sx, start.sy]];
            let mut edge = start;
            loop {
                used.insert(edge);
                ring.push([edge.ex, edge.ey]);
                let candidates: Vec<Edge> = edges_by_start
                    .get(&(edge.ex, edge.ey))
                    .map(|list| list.iter().copied().filter(|c| !used.contains(c)).collect())
                    .unwrap_or_default();
                if candidates.is_empty() {
                    break;
                }
                let next = if candidates.len() == 1 {
                    candidates[0]
                } else {
                    let in_dir = cw_index(edge.ex - edge.sx, edge.ey - edge.sy);
                    *candidates
                        .iter()
                        .min_by_key(|c| (cw_index(c.ex - c.sx, c.ey - c.sy) - in_dir + 4) % 4)
                        .expect("non-empty candidates")
                };
                edge = next;
                if edge == start || used.contains(&edge) {
                    break;
                }
            }
            if edge == start && ring.last() != Some(&[start.sx, start.sy]) {
                ring.push([start.sx, start.sy]);
            }
            if ring.len() >= 4 {
                let simplified = drop_collinear(&ring);
                rings.extend(split_repeated_ring_vertices(&simplified));
            }
        }
    }
    rings
}

/// Splits a closed ring at any repeated interior vertex into simple closed loops.
pub fn split_repeated_ring_vertices(ring: &[[i32; 2]]) -> Vec<Vec<[i32; 2]>> {
    if ring.len() < 4 {
        return Vec::new();
    }
    let mut closed = ring.to_vec();
    if closed.first() != closed.last() {
        closed.push(closed[0]);
    }
    let mut seen: HashMap<[i32; 2], usize> = HashMap::new();
    for idx in 0..closed.len() - 1 {
        let v = closed[idx];
        if let Some(&prev) = seen.get(&v) {
            let first = closed[prev..=idx].to_vec();
            let mut second = Vec::with_capacity(closed.len() - (idx - prev));
            second.extend_from_slice(&closed[..=prev]);
            second.extend_from_slice(&closed[idx + 1..]);
            let mut out = Vec::new();
            if first.len() >= 4 {
                out.extend(split_repeated_ring_vertices(&first));
            }
            if second.len() >= 4 {
                out.extend(split_repeated_ring_vertices(&second));
            }
            return out;
        }
        seen.insert(v, idx);
    }
    vec![closed]
}

fn drop_collinear(ring: &[[i32; 2]]) -> Vec<[i32; 2]> {
    let mut pts = ring.to_vec();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n + 1);
    for i in 0..n {
        let a = pts[(i + n - 1) % n];
        let b = pts[i];
        let c = pts[(i + 1) % n];
        let cross = i64::from(b[0] - a[0]) * i64::from(c[1] - a[1])
            - i64::from(b[1] - a[1]) * i64::from(c[0] - a[0]);
        if cross != 0 {
            out.push(b);
        }
    }
    if let Some(&first) = out.first() {
        out.push(first);
    }
    out
}

fn signed_area(ring: &[[f64; 2]]) -> f64 {
    let mut a = 0.0;
    for w in ring.windows(2) {
        a += w[0][0] * w[1][1] - w[1][0] * w[0][1];
    }
    a * 0.5
}

fn ensure_winding(ring: &[[f64; 2]], ccw: bool) -> Vec<[f64; 2]> {
    let is_ccw = signed_area(ring) > 0.0;
    if is_ccw == ccw {
        ring.to_vec()
    } else {
        ring.iter().copied().rev().collect()
    }
}

fn point_in_ring(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    if ring.len() < 4 {
        return false;
    }
    let mut inside = false;
    let m = ring.len() - 1;
    let mut j = m - 1;
    for i in 0..m {
        let [xi, yi] = ring[i];
        let [xj, yj] = ring[j];
        let intersects =
            ((yi > p[1]) != (yj > p[1])) && (p[0] < ((xj - xi) * (p[1] - yi)) / (yj - yi) + xi);
        if intersects {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Approximate area of a closed lon/lat ring in square kilometers (equirectangular at mean lat).
pub fn ring_area_km2(ring: &[[f64; 2]]) -> f64 {
    if ring.len() < 4 {
        return 0.0;
    }
    let mean_lat = ring.iter().map(|p| p[1]).sum::<f64>() / (ring.len() as f64);
    let m_per_deg_lat = 111_320.0;
    let m_per_deg_lon = 111_320.0 * mean_lat.to_radians().cos();
    let mut a = 0.0;
    for w in ring.windows(2) {
        let x1 = w[0][0] * m_per_deg_lon;
        let y1 = w[0][1] * m_per_deg_lat;
        let x2 = w[1][0] * m_per_deg_lon;
        let y2 = w[1][1] * m_per_deg_lat;
        a += x1 * y2 - x2 * y1;
    }
    (a.abs() * 0.5) / 1_000_000.0
}

/// Downsamples a binary water mask by `scale`, retaining water if any pixel in the block is wet.
pub fn downsample_binary_mask(
    mask: &[u8],
    width: usize,
    height: usize,
    scale: usize,
) -> (Vec<u8>, usize, usize, usize) {
    let s = scale.max(1);
    if s == 1 {
        return (mask.to_vec(), width, height, 1);
    }
    let sw = width.div_ceil(s);
    let sh = height.div_ceil(s);
    let mut out = vec![0u8; sw * sh];
    for y in 0..height {
        for x in 0..width {
            if mask[y * width + x] != 0 {
                out[(y / s) * sw + (x / s)] = 1;
            }
        }
    }
    (out, sw, sh, s)
}

/// Options for converting a binary mask into a WGS 84 GeoJSON `FeatureCollection`.
#[derive(Debug, Clone, Copy)]
pub struct MaskVectorizeOptions {
    pub zoom: u8,
    pub origin_px: f64,
    pub origin_py: f64,
    pub scale: usize,
    pub min_area_km2: f64,
    /// Optional original (un-downsampled) raster width in pixels to clip partial boundary blocks.
    pub source_width: Option<usize>,
    /// Optional original (un-downsampled) raster height in pixels to clip partial boundary blocks.
    pub source_height: Option<usize>,
}

struct RingHierarchyItem {
    ring: Vec<[f64; 2]>,
    bbox: [f64; 4],
    abs_area: f64,
    parent: Option<usize>,
    depth: usize,
}

fn ring_bbox(ring: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for &[x, y] in ring {
        b[0] = b[0].min(x);
        b[1] = b[1].min(y);
        b[2] = b[2].max(x);
        b[3] = b[3].max(y);
    }
    b
}

fn bbox_contains(outer: [f64; 4], inner: [f64; 4]) -> bool {
    outer[0] <= inner[0] + 1e-12
        && outer[1] <= inner[1] + 1e-12
        && outer[2] >= inner[2] - 1e-12
        && outer[3] >= inner[3] - 1e-12
}

/// True when `p` lies strictly on a segment `[a, b]` (within floating tolerance).
fn point_on_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> bool {
    let eps = 1e-11;
    let cross = (p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0]);
    if cross.abs() > eps {
        return false;
    }
    p[0] >= a[0].min(b[0]) - eps
        && p[0] <= a[0].max(b[0]) + eps
        && p[1] >= a[1].min(b[1]) - eps
        && p[1] <= a[1].max(b[1]) + eps
}

fn point_on_ring_boundary(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    ring.windows(2).any(|w| point_on_segment(p, w[0], w[1]))
}

/// True when `inner` lies strictly inside `outer` (corner-touching polygons do not count as nested).
fn ring_strictly_inside(inner: &[[f64; 2]], outer: &[[f64; 2]]) -> bool {
    // Check vertices of `inner` that do not lie on `outer`'s boundary.
    for &p in inner.iter().take(inner.len().saturating_sub(1)) {
        if !point_on_ring_boundary(p, outer) {
            return point_in_ring(p, outer);
        }
    }
    // Check edge midpoints of `inner` not lying on `outer`'s boundary.
    for w in inner.windows(2) {
        let mid = [0.5 * (w[0][0] + w[1][0]), 0.5 * (w[0][1] + w[1][1])];
        if !point_on_ring_boundary(mid, outer) {
            return point_in_ring(mid, outer);
        }
    }
    false
}

/// Converts a binary water mask into a GeoJSON `FeatureCollection` of RFC 7946 polygons with
/// dry-land holes properly nested and islands separated. Returns `(feature_collection, total_area_km2)`.
pub fn mask_to_feature_collection(
    mask: &[u8],
    width: usize,
    height: usize,
    opts: MaskVectorizeOptions,
) -> (Value, f64) {
    let scale = opts.scale.max(1) as f64;
    let max_px_x = opts.source_width.map(|w| w as f64);
    let max_px_y = opts.source_height.map(|h| h as f64);
    let raw_rings = trace_mask_rings(mask, width, height);
    let mut items: Vec<RingHierarchyItem> = raw_rings
        .into_iter()
        .filter_map(|r| {
            let projected: Vec<[f64; 2]> = r
                .into_iter()
                .map(|[nx, ny]| {
                    let rx = (nx as f64) * scale;
                    let ry = (ny as f64) * scale;
                    let rx = max_px_x.map_or(rx, |lim| rx.min(lim));
                    let ry = max_px_y.map_or(ry, |lim| ry.min(lim));
                    let [lon, lat] =
                        pixel_to_lon_lat(opts.origin_px + rx, opts.origin_py + ry, opts.zoom);
                    [lon.clamp(-180.0, 180.0), lat.clamp(-90.0, 90.0)]
                })
                .collect();
            if projected.len() < 4 {
                return None;
            }
            let abs_area = signed_area(&projected).abs();
            let bbox = ring_bbox(&projected);
            Some(RingHierarchyItem {
                ring: projected,
                bbox,
                abs_area,
                parent: None,
                depth: 0,
            })
        })
        .collect();

    // Largest rings first so parents always precede children.
    items.sort_by(|a, b| b.abs_area.total_cmp(&a.abs_area));

    for idx in 0..items.len() {
        let mut parent_area = f64::INFINITY;
        let mut parent_idx = None;
        let child_bbox = items[idx].bbox;
        for cand in 0..idx {
            if items[cand].abs_area < parent_area
                && bbox_contains(items[cand].bbox, child_bbox)
                && ring_strictly_inside(&items[idx].ring, &items[cand].ring)
            {
                parent_idx = Some(cand);
                parent_area = items[cand].abs_area;
            }
        }
        items[idx].parent = parent_idx;
        if let Some(p) = parent_idx {
            items[idx].depth = items[p].depth + 1;
        }
    }

    let mut poly_rings: HashMap<usize, Vec<Vec<[f64; 2]>>> = HashMap::new();
    let mut shell_order: Vec<usize> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        if item.depth % 2 == 1 {
            if let Some(p) = item.parent
                && let Some(coords) = poly_rings.get_mut(&p)
            {
                coords.push(ensure_winding(&item.ring, false));
            }
            continue;
        }
        poly_rings.insert(idx, vec![ensure_winding(&item.ring, true)]);
        shell_order.push(idx);
    }

    let mut total_area_km2 = 0.0;
    let mut features = Vec::with_capacity(shell_order.len());
    for idx in shell_order {
        if let Some(coords) = poly_rings.remove(&idx) {
            let outer_area = ring_area_km2(&coords[0]);
            let hole_area: f64 = coords[1..].iter().map(|h| ring_area_km2(h)).sum();
            let net_area = (outer_area - hole_area).max(0.0);
            if net_area < opts.min_area_km2 {
                continue;
            }
            total_area_km2 += net_area;
            features.push(json!({
                "type": "Feature",
                "geometry": {
                    "type": "Polygon",
                    "coordinates": coords
                },
                "properties": {
                    "source": "OPERA DSWx (observed water, not human-QAed)",
                    "area_km2": (net_area * 10_000.0).round() / 10_000.0
                }
            }));
        }
    }

    (
        json!({
            "type": "FeatureCollection",
            "features": features
        }),
        total_area_km2,
    )
}

/// Options for deriving an observed flood-extent polygon from OPERA DSWx scenes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloodExtentOptions {
    /// AOI bounding box `[west, south, east, north]`.
    pub bbox: BBox,
    /// Inclusive event window start (`YYYY-MM-DD`).
    pub start: String,
    /// Inclusive event window end (`YYYY-MM-DD`).
    pub end: String,
    /// DSWx products to union (defaults to both `OPERA_L3_DSWX-HLS_V1` and cloud-penetrating `OPERA_L3_DSWX-S1_V1`).
    pub products: Vec<String>,
    /// Maximum granules per product to union (default: 6).
    pub max_granules_per_product: usize,
    /// Maximum tiles across the longer bbox side (default: 4).
    pub max_tiles_across: usize,
    /// Alpha threshold (0..=255) above which a water-only tile pixel counts as water (default: 8).
    pub alpha_threshold: u8,
    /// Optional binary mask downsampling factor (default: 1).
    pub scale: usize,
    /// Minimum polygon area in km^2 to retain (default: 0.02).
    pub min_area_km2: f64,
    /// Optional `titiler-cmr` endpoint override.
    pub titiler_endpoint: Option<String>,
    /// Optional CMR endpoint override.
    pub cmr_endpoint: Option<String>,
}

impl FloodExtentOptions {
    /// Creates default options uniting optical DSWx-HLS and radar DSWx-S1 over `bbox` and dates.
    pub fn new(bbox: BBox, start: impl Into<String>, end: impl Into<String>) -> Self {
        Self {
            bbox,
            start: start.into(),
            end: end.into(),
            products: vec![
                "OPERA_L3_DSWX-HLS_V1".to_string(),
                "OPERA_L3_DSWX-S1_V1".to_string(),
            ],
            max_granules_per_product: 6,
            max_tiles_across: 4,
            alpha_threshold: 8,
            scale: 1,
            min_area_km2: 0.02,
            titiler_endpoint: None,
            cmr_endpoint: None,
        }
    }
}

/// Output of [`derive_flood_extent`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloodExtentResult {
    pub bbox: BBox,
    pub start: String,
    pub end: String,
    pub products_used: Vec<String>,
    pub granules_used: Vec<String>,
    pub zoom: u8,
    pub polygon_count: usize,
    pub total_area_km2: f64,
    /// Number of tile requests that failed or could not be decoded (0 when coverage was complete).
    #[serde(default)]
    pub failed_tiles: usize,
    /// Explicitly marks whether this extent is OPERA-observed (not human-QAed).
    pub provenance: String,
    pub feature_collection: Value,
}

/// Decodes a PNG byte slice into 8-bit [`RgbaImage`].
pub fn decode_png_rgba(bytes: &[u8]) -> Result<RgbaImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::ALPHA | png::Transformations::STRIP_16,
    );
    let mut reader = decoder
        .read_info()
        .map_err(|e| OperaError::Image(format!("PNG header decode failed: {e}")))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| OperaError::Image(format!("PNG frame decode failed: {e}")))?;
    let raw = &buf[..info.buffer_size()];
    let width = info.width as usize;
    let height = info.height as usize;

    let data = match info.color_type {
        png::ColorType::Rgba => raw.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(width * height * 4);
            for &[r, g, b] in raw.as_chunks::<3>().0 {
                out.extend_from_slice(&[r, g, b, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(width * height * 4);
            for &[g, a] in raw.as_chunks::<2>().0 {
                out.extend_from_slice(&[g, g, g, a]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(width * height * 4);
            for &g in raw {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        other => {
            return Err(OperaError::Image(format!(
                "unsupported PNG color type: {other:?}"
            )));
        }
    };

    Ok(RgbaImage {
        width,
        height,
        data,
    })
}

/// Tile-grid bounds for a bbox at a given Web Mercator zoom.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TileGridBounds {
    pub tx0: u32,
    pub ty0: u32,
    pub tx1: u32,
    pub ty1: u32,
    #[cfg(feature = "http")]
    pub gx0: f64,
    #[cfg(feature = "http")]
    pub gy0: f64,
    #[cfg(feature = "http")]
    pub gx1: f64,
    #[cfg(feature = "http")]
    pub gy1: f64,
}

/// Computes tile grid bounds for `bbox` at `zoom`.
pub(crate) fn tile_grid_for_bbox(bbox: BBox, zoom: u8) -> Result<TileGridBounds> {
    let safe = bbox.clamp_mercator()?;
    let z = zoom.min(MAX_MERCATOR_ZOOM);
    let (gx0, gy0) = lon_lat_to_pixel(safe.west, safe.north, z);
    let (gx1, gy1) = lon_lat_to_pixel(safe.east, safe.south, z);
    let max_idx = (1u32 << z).saturating_sub(1);
    let clamp_idx = |v: f64| -> u32 { v.clamp(0.0, f64::from(max_idx)) as u32 };
    let tx0 = clamp_idx((gx0 / TILE_SIZE as f64).floor());
    let ty0 = clamp_idx((gy0 / TILE_SIZE as f64).floor());
    let tx1 = clamp_idx((gx1 / TILE_SIZE as f64).ceil() - 1.0);
    let ty1 = clamp_idx((gy1 / TILE_SIZE as f64).ceil() - 1.0);
    Ok(TileGridBounds {
        tx0,
        ty0,
        tx1,
        ty1,
        #[cfg(feature = "http")]
        gx0,
        #[cfg(feature = "http")]
        gy0,
        #[cfg(feature = "http")]
        gx1,
        #[cfg(feature = "http")]
        gy1,
    })
}

/// Derives an observed flood-water polygon `FeatureCollection` by searching OPERA DSWx
/// granules over the AOI + event dates, stitching their water-only tiles from `titiler-cmr`,
/// and vectorizing the observed surface-water mask.
#[cfg(feature = "http")]
pub fn derive_flood_extent(opts: &FloodExtentOptions) -> Result<FloodExtentResult> {
    let endpoint = opts
        .titiler_endpoint
        .as_deref()
        .unwrap_or(DEFAULT_TITILER_ENDPOINT);
    let zoom = choose_zoom(opts.bbox, opts.max_tiles_across)?;
    let grid = tile_grid_for_bbox(opts.bbox, zoom)?;
    let (tx0, ty0, tx1, ty1) = (grid.tx0, grid.ty0, grid.tx1, grid.ty1);
    let (gx0, gy0, gx1, gy1) = (grid.gx0, grid.gy0, grid.gx1, grid.gy1);

    let mut tile_templates = Vec::new();
    let mut products_used = Vec::new();
    let mut granules_used = Vec::new();

    for prod_name in &opts.products {
        let prod = find_product(prod_name)?;
        if !prod.short_name.to_ascii_uppercase().contains("DSWX") {
            return Err(OperaError::InvalidInput(format!(
                "flood_extent requires DSWx products (DSWx-HLS or DSWx-S1), got '{}'",
                prod.short_name
            )));
        }
        let short_name = prod.short_name;
        let search = search_granules(&CmrSearchParams {
            product: short_name.clone(),
            bbox: Some(opts.bbox),
            start: Some(opts.start.clone()),
            end: Some(opts.end.clone()),
            limit: opts.max_granules_per_product.clamp(1, 25),
            deduplicate_versions: true,
            newest_per_tile: false,
            cmr_endpoint: opts.cmr_endpoint.clone(),
        })?;
        if search.granules.is_empty() {
            continue;
        }
        products_used.push(short_name.clone());
        for g in search
            .granules
            .into_iter()
            .take(opts.max_granules_per_product)
        {
            let Some(cid) = g.concept_id.clone().or_else(|| search.concept_id.clone()) else {
                continue;
            };
            let mut q = TitilerQuery::rasterio(cid);
            q.endpoint = endpoint.to_string();
            q.product = Some(short_name.clone());
            q.granule_ur = Some(g.id.clone());
            q.assets = vec!["B01_WTR".to_string()];
            q.colormap = Some(DSWX_WTR_WATER_ONLY_COLORMAP_JSON.to_string());
            tile_templates.push(build_tile_url_template(&q, TILE_SIZE as u32));
            granules_used.push(g.id);
        }
    }

    if tile_templates.is_empty() {
        return Err(OperaError::NoResults(format!(
            "no OPERA DSWx granules found for {} to {} over {}",
            opts.start, opts.end, opts.bbox
        )));
    }

    let cols = (tx1 - tx0 + 1) as usize;
    let rows = (ty1 - ty0 + 1) as usize;
    let stitch_w = cols * TILE_SIZE;
    let stitch_h = rows * TILE_SIZE;
    let origin_px = (tx0 as usize * TILE_SIZE) as f64;
    let origin_py = (ty0 as usize * TILE_SIZE) as f64;

    let mut urls = Vec::new();
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            for tpl in &tile_templates {
                let url = tpl
                    .replace("{z}", &zoom.to_string())
                    .replace("{x}", &tx.to_string())
                    .replace("{y}", &ty.to_string());
                urls.push((tx, ty, url));
            }
        }
    }

    // Fetch and merge tiles in bounded batches so peak memory stays bounded even for large grids.
    const BATCH_SIZE: usize = 16;
    let mut mask = vec![0u8; stitch_w * stitch_h];
    let mut succeeded_tiles = 0usize;
    let mut failed_tiles = 0usize;
    let mut first_error: Option<String> = None;

    for chunk in urls.chunks(BATCH_SIZE) {
        #[cfg(feature = "parallel")]
        let batch_results: Vec<(u32, u32, Result<RgbaImage>)> = {
            use rayon::prelude::*;
            chunk
                .par_iter()
                .map(|(tx, ty, url)| {
                    let res = crate::cmr::http_get_bytes(url).and_then(|b| decode_png_rgba(&b));
                    (*tx, *ty, res)
                })
                .collect()
        };

        #[cfg(not(feature = "parallel"))]
        let batch_results: Vec<(u32, u32, Result<RgbaImage>)> = chunk
            .iter()
            .map(|(tx, ty, url)| {
                let res = crate::cmr::http_get_bytes(url).and_then(|b| decode_png_rgba(&b));
                (*tx, *ty, res)
            })
            .collect();

        for (tx, ty, res) in batch_results {
            let img = match res {
                Ok(img) => {
                    succeeded_tiles += 1;
                    img
                }
                Err(e) => {
                    failed_tiles += 1;
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                    }
                    continue;
                }
            };
            let ox = (tx - tx0) as usize * TILE_SIZE;
            let oy = (ty - ty0) as usize * TILE_SIZE;
            let copy_h = TILE_SIZE.min(img.height);
            let copy_w = TILE_SIZE.min(img.width);
            for py in 0..copy_h {
                for px in 0..copy_w {
                    let alpha = img.data[(py * img.width + px) * 4 + 3];
                    let sx = ox + px;
                    let sy = oy + py;
                    let global_x = origin_px + (sx as f64) + 0.5;
                    let global_y = origin_py + (sy as f64) + 0.5;
                    if alpha > opts.alpha_threshold
                        && global_x >= gx0
                        && global_x <= gx1
                        && global_y >= gy0
                        && global_y <= gy1
                    {
                        mask[sy * stitch_w + sx] = 1;
                    }
                }
            }
        }
    }

    if succeeded_tiles == 0 {
        return Err(OperaError::Http(format!(
            "all {failed_tiles} DSWx tile request(s) failed{}",
            first_error
                .map(|e| format!(" (first error: {e})"))
                .unwrap_or_default()
        )));
    }

    let (sampled, sw, sh, used_scale) =
        downsample_binary_mask(&mask, stitch_w, stitch_h, opts.scale);
    let (feature_collection, total_area_km2) = mask_to_feature_collection(
        &sampled,
        sw,
        sh,
        MaskVectorizeOptions {
            zoom,
            origin_px,
            origin_py,
            scale: used_scale,
            min_area_km2: opts.min_area_km2,
            source_width: Some(stitch_w),
            source_height: Some(stitch_h),
        },
    );
    let polygon_count = feature_collection["features"]
        .as_array()
        .map_or(0, Vec::len);

    Ok(FloodExtentResult {
        bbox: opts.bbox,
        start: opts.start.clone(),
        end: opts.end.clone(),
        products_used,
        granules_used,
        zoom,
        polygon_count,
        total_area_km2,
        failed_tiles,
        provenance: "OPERA-observed water extent (not human-QAed)".to_string(),
        feature_collection,
    })
}

#[cfg(not(feature = "http"))]
pub fn derive_flood_extent(_opts: &FloodExtentOptions) -> Result<FloodExtentResult> {
    Err(OperaError::HttpDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_ring_with_hole_and_nests_correctly() {
        // 5x5 grid: outer 3x3 ring of water with dry hole in the center (2,2).
        let mut mask = vec![0u8; 25];
        for y in 1..=3 {
            for x in 1..=3 {
                if !(x == 2 && y == 2) {
                    mask[y * 5 + x] = 1;
                }
            }
        }
        let rings = trace_mask_rings(&mask, 5, 5);
        assert_eq!(rings.len(), 2, "outer ring and interior hole ring");

        let (fc, area) = mask_to_feature_collection(
            &mask,
            5,
            5,
            MaskVectorizeOptions {
                zoom: 8,
                origin_px: 32000.0,
                origin_py: 24000.0,
                scale: 1,
                min_area_km2: 0.0,
                source_width: Some(5),
                source_height: Some(5),
            },
        );
        let feats = fc["features"].as_array().unwrap();
        assert_eq!(feats.len(), 1, "hole nested into single Polygon feature");
        let coords = feats[0]["geometry"]["coordinates"].as_array().unwrap();
        assert_eq!(coords.len(), 2, "exterior ring + 1 hole ring");
        assert!(area > 0.0);
    }

    #[test]
    fn corner_touching_water_components_stay_separate_shells() {
        // Row 0: [0, 1, 1], Row 1: [1, 0, 0] -> two wet components touching at one corner.
        let mask = [0u8, 1, 1, 1, 0, 0];
        let (fc, area) = mask_to_feature_collection(
            &mask,
            3,
            2,
            MaskVectorizeOptions {
                zoom: 8,
                origin_px: 32000.0,
                origin_py: 24000.0,
                scale: 1,
                min_area_km2: 0.0,
                source_width: Some(3),
                source_height: Some(2),
            },
        );
        let feats = fc["features"].as_array().unwrap();
        assert_eq!(
            feats.len(),
            2,
            "corner-touching components must not nest as holes"
        );
        assert!(area > 0.0);
    }

    #[test]
    fn mercator_projection_roundtrips() {
        let (px, py) = lon_lat_to_pixel(-119.7, 36.1, 10);
        let [lon, lat] = pixel_to_lon_lat(px, py, 10);
        assert!((lon - (-119.7)).abs() < 1e-6);
        assert!((lat - 36.1).abs() < 1e-6);
    }

    #[test]
    fn chooses_zoom_within_tile_budget() {
        let bbox = BBox::new(-120.0, 35.8, -119.4, 36.3).unwrap();
        let z = choose_zoom(bbox, 4).unwrap();
        assert!((8..=13).contains(&z), "unexpected zoom {z}");
    }
}
