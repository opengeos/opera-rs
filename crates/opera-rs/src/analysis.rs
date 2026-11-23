//! High-level NASA OPERA analytical workflows: two-date change detection and time-series trends.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "http")]
use crate::DEFAULT_TITILER_ENDPOINT;
use crate::cmr::OperaGranule;
#[cfg(feature = "http")]
use crate::cmr::{CmrSearchParams, search_granules, search_granules_sorted};
use crate::error::{OperaError, Result};
use crate::products::{DSWX_OPEN_WATER_CLASS, DSWX_PARTIAL_WATER_CLASS};
#[cfg(feature = "http")]
use crate::products::{
    find_product, is_categorical_band, is_dswx_binary_water_band, is_dswx_water_band,
};
use crate::titiler::BandStatistics;
#[cfg(feature = "http")]
use crate::titiler::{StatisticsResult, TitilerQuery, fetch_statistics};
use crate::types::{BBox, CalendarDate};

/// Options for two-date OPERA change detection (`detect_change`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeDetectionOptions {
    /// OPERA product short name or alias (e.g. `dswx-hls`, `dswx-s1`, `dist-alert`, `rtc-s1`).
    pub product: String,
    /// Bounding box `[west, south, east, north]`.
    pub bbox: BBox,
    /// Optional explicit GeoJSON AOI polygon (defaults to `bbox.to_geojson_feature()`).
    pub aoi_geojson: Option<Value>,
    /// Target baseline date (`YYYY-MM-DD`).
    pub before_date: String,
    /// Target comparison date (`YYYY-MM-DD`).
    pub after_date: String,
    /// Search window in days on each side of `before_date` and `after_date` (required; recommended: 10).
    pub window_days: i64,
    /// Optional band override (defaults to product's preferred band, e.g. `B01_WTR` or `VV`).
    pub band: Option<String>,
    /// Optional `rio-tiler` band-math expression (selected band as `b1`).
    pub expression: Option<String>,
    /// Optional `titiler-cmr` endpoint override.
    pub titiler_endpoint: Option<String>,
    /// Optional CMR endpoint override.
    pub cmr_endpoint: Option<String>,
}

/// Summary of one date's granule and statistics inside a change or time-series analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationSummary {
    pub target_date: String,
    pub granule_id: String,
    pub acquired_date: String,
    pub begin_date: Option<String>,
    pub tile: String,
    pub band: String,
    pub stats: BandStatistics,
    /// Open water area (class 1) in km^2 when analyzing DSWx WTR bands.
    pub open_water_km2: Option<f64>,
    /// Combined open + partial surface water (classes 1 + 2) in km^2 when analyzing DSWx WTR bands.
    pub total_water_km2: Option<f64>,
}

/// Change in one discrete class between before and after observations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassDelta {
    pub class_value: i64,
    pub label: Option<String>,
    pub before_area_km2: f64,
    pub after_area_km2: f64,
    pub delta_area_km2: f64,
    pub percent_change: Option<f64>,
}

/// Quantified two-date change detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeDetectionResult {
    pub product: String,
    pub concept_id: String,
    pub band: String,
    pub bbox: BBox,
    pub categorical: bool,
    pub before: ObservationSummary,
    pub after: ObservationSummary,
    pub mean_delta: Option<f64>,
    pub open_water_delta_km2: Option<f64>,
    pub open_water_percent_change: Option<f64>,
    pub total_water_delta_km2: Option<f64>,
    pub total_water_percent_change: Option<f64>,
    pub class_deltas: Vec<ClassDelta>,
    /// Advisory warnings (e.g. when the AOI spans multiple tiles on a selected acquisition date).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl ChangeDetectionResult {
    /// Formats a concise Markdown change-detection report suitable for agents and publications.
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str(&format!(
            "# OPERA Change Detection Report ({})\n\n",
            self.product
        ));
        md.push_str(&format!("- **Band:** `{}`\n", self.band));
        md.push_str(&format!(
            "- **AOI bbox:** `{}` ({:.2} km^2)\n",
            self.bbox,
            self.bbox.area_km2()
        ));
        md.push_str(&format!(
            "- **Before:** {} (`{}`)\n",
            self.before.acquired_date, self.before.granule_id
        ));
        md.push_str(&format!(
            "- **After:** {} (`{}`)\n\n",
            self.after.acquired_date, self.after.granule_id
        ));

        let has_open_water = self.before.open_water_km2.is_some()
            && self.after.open_water_km2.is_some()
            && self.open_water_delta_km2.is_some();
        let has_total_water = self.before.total_water_km2.is_some()
            && self.after.total_water_km2.is_some()
            && self.total_water_delta_km2.is_some();

        if has_open_water || has_total_water {
            md.push_str("## Surface Water Summary\n\n");
            if let (Some(b_ow), Some(a_ow), Some(d_ow)) = (
                self.before.open_water_km2,
                self.after.open_water_km2,
                self.open_water_delta_km2,
            ) {
                md.push_str(&format!(
                    "- **Open water (class 1):** {:.3} km^2 -> {:.3} km^2 (delta: {:+.3} km^2{})\n",
                    b_ow,
                    a_ow,
                    d_ow,
                    self.open_water_percent_change
                        .map(|p| format!(", {:+.1}%", p))
                        .unwrap_or_default()
                ));
            }
            if let (Some(b_tw), Some(a_tw), Some(d_tw)) = (
                self.before.total_water_km2,
                self.after.total_water_km2,
                self.total_water_delta_km2,
            ) {
                md.push_str(&format!(
                    "- **Total surface water:** {:.3} km^2 -> {:.3} km^2 (delta: {:+.3} km^2{})\n",
                    b_tw,
                    a_tw,
                    d_tw,
                    self.total_water_percent_change
                        .map(|p| format!(", {:+.1}%", p))
                        .unwrap_or_default()
                ));
            }
            md.push('\n');
        }

        if !self.class_deltas.is_empty() {
            md.push_str("## Class Area Breakdown (30 m native pixels)\n\n");
            md.push_str("| Class | Label | Before (km^2) | After (km^2) | Delta (km^2) |\n");
            md.push_str("|---:|---|---:|---:|---:|\n");
            for c in &self.class_deltas {
                md.push_str(&format!(
                    "| {} | {} | {:.3} | {:.3} | {:+.3} |\n",
                    c.class_value,
                    c.label.as_deref().unwrap_or("-"),
                    c.before_area_km2,
                    c.after_area_km2,
                    c.delta_area_km2
                ));
            }
            md.push('\n');
        } else {
            md.push_str("## Continuous Band Summary\n\n");
            md.push_str("| Metric | Before | After | Delta |\n");
            md.push_str("|---|---:|---:|---:|\n");
            if let (Some(b), Some(a)) = (self.before.stats.mean, self.after.stats.mean) {
                md.push_str(&format!("| Mean | {:.4} | {:.4} | {:+.4} |\n", b, a, a - b));
            }
            if let (Some(b), Some(a)) = (self.before.stats.median, self.after.stats.median) {
                md.push_str(&format!(
                    "| Median | {:.4} | {:.4} | {:+.4} |\n",
                    b,
                    a,
                    a - b
                ));
            }
            if let (Some(b), Some(a)) = (self.before.stats.std, self.after.stats.std) {
                md.push_str(&format!(
                    "| Std Dev | {:.4} | {:.4} | {:+.4} |\n",
                    b,
                    a,
                    a - b
                ));
            }
        }
        if !self.warnings.is_empty() {
            md.push_str("\n## Warnings\n\n");
            for w in &self.warnings {
                md.push_str(&format!("- {w}\n"));
            }
        }
        md
    }
}

/// Extracts `(open_water_km2, total_water_km2)` from categorical DSWx class statistics.
///
/// - For binary water bands (`B02_BWTR`, where class 1 combines open and partial water),
///   pass `binary_water = true` so `open_water_km2` is left as `None` and `total_water_km2`
///   reports class 1 area.
/// - For DSWx-S1 `B01_WTR` (where class 3 is inundated vegetation), pass
///   `include_inundated_veg = true` so `total_water_km2` sums classes 1 + 2 + 3.
/// - Returns `(None, None)` when the histogram contains only masked/nodata classes (such as
///   cloud/shadow class 253 or snow/ice class 252) with zero observed land or water pixels.
pub fn extract_dswx_water_areas_for_band(
    stats: &BandStatistics,
    binary_water: bool,
    include_inundated_veg: bool,
) -> (Option<f64>, Option<f64>) {
    if stats.class_breakdown.is_empty() {
        return (None, None);
    }
    let has_observed_land_or_water = stats.class_breakdown.iter().any(|c| {
        let valid_class = if binary_water {
            matches!(c.class_value, 0 | 1)
        } else if include_inundated_veg {
            matches!(c.class_value, 0..=3)
        } else {
            matches!(c.class_value, 0..=2)
        };
        valid_class && c.pixel_count > 0.0
    });
    if !has_observed_land_or_water {
        return (None, None);
    }
    let class1 = stats
        .class_breakdown
        .iter()
        .find(|c| c.class_value == i64::from(DSWX_OPEN_WATER_CLASS))
        .map_or(0.0, |c| c.area_km2);
    if binary_water {
        return (None, Some(class1));
    }
    let partial = stats
        .class_breakdown
        .iter()
        .find(|c| c.class_value == i64::from(DSWX_PARTIAL_WATER_CLASS))
        .map_or(0.0, |c| c.area_km2);
    let inundated_veg = if include_inundated_veg {
        stats
            .class_breakdown
            .iter()
            .find(|c| c.class_value == 3)
            .map_or(0.0, |c| c.area_km2)
    } else {
        0.0
    };
    (Some(class1), Some(class1 + partial + inundated_veg))
}

/// Computes class-by-class deltas between two categorical [`BandStatistics`] snapshots.
///
/// Returns an empty vector if either snapshot lacks a categorical class breakdown, avoiding
/// fabricated zero-to-nonzero area changes when a histogram is missing.
pub fn compute_class_deltas(before: &BandStatistics, after: &BandStatistics) -> Vec<ClassDelta> {
    if before.class_breakdown.is_empty() || after.class_breakdown.is_empty() {
        return Vec::new();
    }
    let mut keys: Vec<i64> = before
        .class_breakdown
        .iter()
        .chain(after.class_breakdown.iter())
        .map(|c| c.class_value)
        .collect();
    keys.sort_unstable();
    keys.dedup();

    keys.into_iter()
        .map(|cls| {
            let b = before.class_breakdown.iter().find(|c| c.class_value == cls);
            let a = after.class_breakdown.iter().find(|c| c.class_value == cls);
            let b_area = b.map_or(0.0, |c| c.area_km2);
            let a_area = a.map_or(0.0, |c| c.area_km2);
            let delta = a_area - b_area;
            let pct = (b_area > 0.0).then_some((delta / b_area) * 100.0);
            let label = a
                .and_then(|c| c.label.clone())
                .or_else(|| b.and_then(|c| c.label.clone()));
            ClassDelta {
                class_value: cls,
                label,
                before_area_km2: b_area,
                after_area_km2: a_area,
                delta_area_km2: delta,
                percent_change: pct,
            }
        })
        .collect()
}

/// Picks the dated granule whose acquisition date is closest to `target`.
///
/// Undated granules (whose `date` cannot be parsed as `YYYY-MM-DD`) are excluded.
pub fn pick_nearest_granule(
    granules: &[OperaGranule],
    target: CalendarDate,
) -> Option<&OperaGranule> {
    granules
        .iter()
        .filter_map(|g| {
            let d = CalendarDate::parse(&g.date).ok()?;
            Some((d.days_between(target).abs(), g))
        })
        .min_by_key(|&(dist, _)| dist)
        .map(|(_, g)| g)
}

#[cfg(feature = "http")]
fn first_band_stats(res: StatisticsResult) -> Result<BandStatistics> {
    res.bands
        .into_values()
        .next()
        .ok_or_else(|| OperaError::NoResults("titiler-cmr returned empty band statistics".into()))
}

/// Performs two-date OPERA change detection via NASA CMR + `titiler-cmr /statistics`.
#[cfg(feature = "http")]
pub fn detect_change(opts: &ChangeDetectionOptions) -> Result<ChangeDetectionResult> {
    let prod = find_product(&opts.product)?;
    if !prod.raster_tiles_supported {
        return Err(OperaError::InvalidInput(format!(
            "product '{}' uses the '{}' backend (HDF5/NetCDF); detect_change currently supports rasterio COG products (DSWx, DIST, RTC-S1)",
            prod.short_name, prod.backend
        )));
    }
    let before_target = CalendarDate::parse(&opts.before_date)?;
    let after_target = CalendarDate::parse(&opts.after_date)?;
    let window = opts.window_days.clamp(0, 90);
    let aoi = opts
        .aoi_geojson
        .clone()
        .unwrap_or_else(|| opts.bbox.to_geojson_feature());

    // Search both halves around `target` (descending before/on target, ascending on/after target)
    // so an exact-date scene is never pushed off page 1 by later acquisitions.
    let find_near = |target: CalendarDate| -> Result<(OperaGranule, usize)> {
        let mut candidates = Vec::new();
        let left = search_granules_sorted(
            &CmrSearchParams {
                product: prod.short_name.clone(),
                bbox: Some(opts.bbox),
                start: Some(target.add_days(-window).to_string()),
                end: Some(target.to_string()),
                limit: 50,
                deduplicate_versions: true,
                newest_per_tile: false,
                cmr_endpoint: opts.cmr_endpoint.clone(),
            },
            false,
        )?;
        candidates.extend(left.granules);
        if window > 0 {
            let right = search_granules_sorted(
                &CmrSearchParams {
                    product: prod.short_name.clone(),
                    bbox: Some(opts.bbox),
                    start: Some(target.to_string()),
                    end: Some(target.add_days(window).to_string()),
                    limit: 50,
                    deduplicate_versions: true,
                    newest_per_tile: false,
                    cmr_endpoint: opts.cmr_endpoint.clone(),
                },
                true,
            )?;
            candidates.extend(right.granules);
        }
        let chosen = pick_nearest_granule(&candidates, target)
            .cloned()
            .ok_or_else(|| {
                OperaError::NoResults(format!(
                    "no {} granules found within +/-{window} days of {target}",
                    prod.short_name
                ))
            })?;
        let mut same_day_tiles: Vec<&str> = candidates
            .iter()
            .filter(|g| g.date == chosen.date && !g.tile.is_empty())
            .map(|g| g.tile.as_str())
            .collect();
        same_day_tiles.sort_unstable();
        same_day_tiles.dedup();
        Ok((chosen, same_day_tiles.len()))
    };

    let (before_g, before_tile_count) = find_near(before_target)?;
    let (after_g, after_tile_count) = find_near(after_target)?;
    let mut warnings = Vec::new();
    if before_tile_count > 1 {
        warnings.push(format!(
            "baseline date {} spans {} distinct tiles/bursts in the AOI; single-granule statistics cover only granule '{}' (tile {})",
            before_g.date, before_tile_count, before_g.id, before_g.tile
        ));
    }
    if after_tile_count > 1 {
        warnings.push(format!(
            "comparison date {} spans {} distinct tiles/bursts in the AOI; single-granule statistics cover only granule '{}' (tile {})",
            after_g.date, after_tile_count, after_g.id, after_g.tile
        ));
    }
    if !before_g.tile.is_empty() && !after_g.tile.is_empty() && before_g.tile != after_g.tile {
        warnings.push(format!(
            "baseline granule '{}' (tile {}) and comparison granule '{}' (tile {}) come from different tiles/bursts; deltas may reflect footprint differences",
            before_g.id, before_g.tile, after_g.id, after_g.tile
        ));
    }
    let concept_id = before_g
        .concept_id
        .clone()
        .or_else(|| after_g.concept_id.clone())
        .ok_or_else(|| OperaError::NoResults("granule missing collection concept_id".into()))?;
    let band = opts
        .band
        .clone()
        .or_else(|| prod.choose_band(&before_g.bands))
        .ok_or_else(|| {
            OperaError::InvalidInput(format!(
                "no default band available for product '{}'",
                prod.short_name
            ))
        })?;

    let expression = opts
        .expression
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let categorical = expression.is_none() && is_categorical_band(&prod.short_name, Some(&band));
    let is_water = expression.is_none() && is_dswx_water_band(&prod.short_name, Some(&band));
    let is_binary_water =
        expression.is_none() && is_dswx_binary_water_band(&prod.short_name, Some(&band));
    let include_inundated_veg = prod.short_name.to_ascii_uppercase().contains("DSWX-S1");
    let endpoint = opts
        .titiler_endpoint
        .as_deref()
        .unwrap_or(DEFAULT_TITILER_ENDPOINT);

    let fetch_obs = |target: CalendarDate, g: &OperaGranule| -> Result<ObservationSummary> {
        let mut q = TitilerQuery::rasterio(&concept_id);
        q.endpoint = endpoint.to_string();
        q.product = Some(prod.short_name.clone());
        q.granule_ur = Some(g.id.clone());
        q.assets = vec![band.clone()];
        q.expression = expression.clone();
        let stats = first_band_stats(fetch_statistics(&q, &aoi, categorical, None)?)?;
        let (open_w, total_w) = if is_water {
            extract_dswx_water_areas_for_band(&stats, is_binary_water, include_inundated_veg)
        } else {
            (None, None)
        };
        Ok(ObservationSummary {
            target_date: target.to_string(),
            granule_id: g.id.clone(),
            acquired_date: g.date.clone(),
            begin_date: g.begin_date.clone(),
            tile: g.tile.clone(),
            band: band.clone(),
            stats,
            open_water_km2: open_w,
            total_water_km2: total_w,
        })
    };

    let before = fetch_obs(before_target, &before_g)?;
    let after = fetch_obs(after_target, &after_g)?;
    let class_deltas = if categorical {
        compute_class_deltas(&before.stats, &after.stats)
    } else {
        Vec::new()
    };
    let mean_delta = before.stats.mean.zip(after.stats.mean).map(|(b, a)| a - b);
    let open_water_delta_km2 = before
        .open_water_km2
        .zip(after.open_water_km2)
        .map(|(b, a)| a - b);
    let open_water_percent_change = before
        .open_water_km2
        .zip(open_water_delta_km2)
        .and_then(|(b, d)| (b > 0.0).then_some((d / b) * 100.0));
    let total_water_delta_km2 = before
        .total_water_km2
        .zip(after.total_water_km2)
        .map(|(b, a)| a - b);
    let total_water_percent_change = before
        .total_water_km2
        .zip(total_water_delta_km2)
        .and_then(|(b, d)| (b > 0.0).then_some((d / b) * 100.0));

    Ok(ChangeDetectionResult {
        product: prod.short_name,
        concept_id,
        band,
        bbox: opts.bbox,
        categorical,
        before,
        after,
        mean_delta,
        open_water_delta_km2,
        open_water_percent_change,
        total_water_delta_km2,
        total_water_percent_change,
        class_deltas,
        warnings,
    })
}

#[cfg(not(feature = "http"))]
pub fn detect_change(_opts: &ChangeDetectionOptions) -> Result<ChangeDetectionResult> {
    Err(OperaError::HttpDisabled)
}

/// Options for multi-date OPERA time-series analysis (`analyze_time_series`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesOptions {
    pub product: String,
    pub bbox: BBox,
    pub aoi_geojson: Option<Value>,
    pub start: String,
    pub end: String,
    /// Optional minimum day spacing between sampled observations.
    pub interval_days: Option<i64>,
    /// Maximum number of observations to evaluate (required; recommended: 12, capped at 100).
    pub max_observations: usize,
    pub band: Option<String>,
    pub expression: Option<String>,
    pub titiler_endpoint: Option<String>,
    pub cmr_endpoint: Option<String>,
}

/// Summary trend metrics over a numeric series `(day_offset, value)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesTrend {
    pub metric: String,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub net_change: f64,
    /// Ordinary least-squares linear trend slope in `metric_units / day`.
    pub slope_per_day: Option<f64>,
}

/// Record of a sampled granule whose statistics request failed during time-series analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedObservation {
    pub date: String,
    pub granule_id: String,
    pub error: String,
}

/// Result of [`analyze_time_series`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesResult {
    pub product: String,
    pub concept_id: String,
    pub band: String,
    pub bbox: BBox,
    pub start: String,
    pub end: String,
    pub observations: Vec<ObservationSummary>,
    /// Sampled granules whose `titiler-cmr` statistics requests failed (disclosed for transparency).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_observations: Vec<FailedObservation>,
    pub trend: Option<SeriesTrend>,
    /// Advisory warnings (e.g. when multiple same-day tiles cover the AOI).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

impl TimeSeriesResult {
    /// Exports observations as RFC 4180 CSV (`date,granule_id,tile,mean,valid_percent,open_water_km2,total_water_km2`).
    pub fn to_csv(&self) -> String {
        let mut out = String::from(
            "date,granule_id,tile,mean,valid_percent,open_water_km2,total_water_km2\r\n",
        );
        for obs in &self.observations {
            let opt = |v: Option<f64>| v.map(|x| format!("{x:.6}")).unwrap_or_default();
            out.push_str(&format!(
                "{},{},{},{},{},{},{}\r\n",
                csv_escape(&obs.acquired_date),
                csv_escape(&obs.granule_id),
                csv_escape(&obs.tile),
                opt(obs.stats.mean),
                opt(obs.stats.valid_percent),
                opt(obs.open_water_km2),
                opt(obs.total_water_km2),
            ));
        }
        out
    }
}

/// Samples dated granules chronologically with optional minimum `interval_days` spacing and count cap.
///
/// Keeps one representative granule per acquisition date (sorted deterministically by granule ID).
/// Granules without a valid `YYYY-MM-DD` acquisition date are excluded regardless of spacing.
pub fn sample_granules_chronologically(
    granules: Vec<OperaGranule>,
    interval_days: Option<i64>,
    max_observations: usize,
) -> Vec<OperaGranule> {
    let mut dated: Vec<(CalendarDate, OperaGranule)> = granules
        .into_iter()
        .filter_map(|g| CalendarDate::parse(&g.date).ok().map(|d| (d, g)))
        .collect();
    dated.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));

    // Keep one representative granule per acquisition date first.
    let mut by_date: Vec<(CalendarDate, OperaGranule)> = Vec::new();
    for (d, g) in dated {
        if by_date.last().is_some_and(|(prev_d, _)| *prev_d == d) {
            continue;
        }
        by_date.push((d, g));
    }

    let spaced: Vec<OperaGranule> = if let Some(min_gap) = interval_days.filter(|&d| d > 0) {
        let mut kept: Vec<OperaGranule> = Vec::new();
        let mut last_day: Option<CalendarDate> = None;
        for (d, g) in by_date {
            if last_day.is_none_or(|prev| d.days_between(prev) >= min_gap) {
                last_day = Some(d);
                kept.push(g);
            }
        }
        kept
    } else {
        by_date.into_iter().map(|(_, g)| g).collect()
    };

    let cap = max_observations.clamp(1, 100);
    if spaced.len() <= cap {
        return spaced;
    }
    // Evenly subsample across the time window while preserving start and end dates.
    let n = spaced.len();
    (0..cap)
        .map(|i| {
            let idx = if cap == 1 {
                0
            } else {
                (i * (n - 1)) / (cap - 1)
            };
            spaced[idx].clone()
        })
        .collect()
}

/// Computes summary stats and OLS linear slope over `(day_offset, value)` points.
pub fn compute_series_trend(metric: &str, points: &[(f64, f64)]) -> Option<SeriesTrend> {
    if points.is_empty() {
        return None;
    }
    let min = points.iter().map(|&(_, y)| y).fold(f64::INFINITY, f64::min);
    let max = points
        .iter()
        .map(|&(_, y)| y)
        .fold(f64::NEG_INFINITY, f64::max);
    let mean = points.iter().map(|&(_, y)| y).sum::<f64>() / (points.len() as f64);
    let net_change = points.last().unwrap().1 - points.first().unwrap().1;

    let slope_per_day = if points.len() >= 2 {
        let n = points.len() as f64;
        let mean_x = points.iter().map(|&(x, _)| x).sum::<f64>() / n;
        let num: f64 = points.iter().map(|&(x, y)| (x - mean_x) * (y - mean)).sum();
        let den: f64 = points.iter().map(|&(x, _)| (x - mean_x).powi(2)).sum();
        (den > 1e-12).then_some(num / den)
    } else {
        None
    };

    Some(SeriesTrend {
        metric: metric.to_string(),
        min,
        max,
        mean,
        net_change,
        slope_per_day,
    })
}

/// Retrieves candidate granules across `start..=end`, falling back to evenly spaced sub-windows
/// when CMR reports more than 500 hits so early acquisitions are not truncated.
///
/// Returns `(concept_id, candidate_granules, warnings)`.
#[cfg(feature = "http")]
pub fn fetch_time_series_candidates(
    product: &str,
    bbox: BBox,
    start: &str,
    end: &str,
    target_samples: usize,
    cmr_endpoint: Option<&str>,
) -> Result<(Option<String>, Vec<OperaGranule>, Vec<String>)> {
    let prod = find_product(product)?;
    let start_date = CalendarDate::parse(start)?;
    let end_date = CalendarDate::parse(end)?;
    if start_date > end_date {
        return Err(OperaError::InvalidInput(format!(
            "start date {start_date} must be on or before end date {end_date}"
        )));
    }
    let search = search_granules(&CmrSearchParams {
        product: prod.short_name.clone(),
        bbox: Some(bbox),
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        limit: 500,
        deduplicate_versions: true,
        newest_per_tile: false,
        cmr_endpoint: cmr_endpoint.map(ToString::to_string),
    })?;
    let mut concept_id = search.concept_id.clone();
    let mut candidate_granules = Vec::new();
    let mut warnings = Vec::new();
    if search.hits > 500 && target_samples > 1 {
        let inclusive_days = end_date.days_between(start_date).max(0) + 1;
        let buckets = (target_samples.clamp(2, 24) as i64).min(inclusive_days);
        let mut failed_buckets = 0i64;
        let mut first_err: Option<String> = None;
        for i in 0..buckets {
            let start_offset = (i * inclusive_days) / buckets;
            let end_offset = (((i + 1) * inclusive_days) / buckets - 1)
                .max(start_offset)
                .min(inclusive_days - 1);
            let b_start = start_date.add_days(start_offset);
            let b_end = start_date.add_days(end_offset);
            match search_granules_sorted(
                &CmrSearchParams {
                    product: prod.short_name.clone(),
                    bbox: Some(bbox),
                    start: Some(b_start.to_string()),
                    end: Some(b_end.to_string()),
                    limit: 5,
                    deduplicate_versions: true,
                    newest_per_tile: false,
                    cmr_endpoint: cmr_endpoint.map(ToString::to_string),
                },
                true,
            ) {
                Ok(sub) => {
                    if concept_id.is_none() {
                        concept_id = sub.concept_id;
                    }
                    candidate_granules.extend(sub.granules);
                }
                Err(err) => {
                    failed_buckets += 1;
                    if first_err.is_none() {
                        first_err = Some(err.to_string());
                    }
                }
            }
        }
        if failed_buckets == buckets {
            return Err(OperaError::Http(format!(
                "all {buckets} CMR sub-window searches failed{}",
                first_err
                    .map(|e| format!(" (first error: {e})"))
                    .unwrap_or_default()
            )));
        }
        if failed_buckets > 0 {
            warnings.push(format!(
                "{failed_buckets} of {buckets} CMR sub-window searches failed{}; temporal coverage may be incomplete",
                first_err
                    .map(|e| format!(" (first error: {e})"))
                    .unwrap_or_default()
            ));
        }
    } else {
        candidate_granules = search.granules;
    }
    Ok((concept_id, candidate_granules, warnings))
}

#[cfg(not(feature = "http"))]
pub fn fetch_time_series_candidates(
    _product: &str,
    _bbox: BBox,
    _start: &str,
    _end: &str,
    _target_samples: usize,
    _cmr_endpoint: Option<&str>,
) -> Result<(Option<String>, Vec<OperaGranule>, Vec<String>)> {
    Err(OperaError::HttpDisabled)
}

/// Runs a multi-date OPERA time-series analysis across `start..=end`.
#[cfg(feature = "http")]
pub fn analyze_time_series(opts: &TimeSeriesOptions) -> Result<TimeSeriesResult> {
    let prod = find_product(&opts.product)?;
    if !prod.raster_tiles_supported {
        return Err(OperaError::InvalidInput(format!(
            "product '{}' uses the '{}' backend (HDF5/NetCDF); analyze_time_series currently supports rasterio COG products (DSWx, DIST, RTC-S1)",
            prod.short_name, prod.backend
        )));
    }
    let aoi = opts
        .aoi_geojson
        .clone()
        .unwrap_or_else(|| opts.bbox.to_geojson_feature());

    let (concept_id, candidate_granules, mut warnings) = fetch_time_series_candidates(
        &prod.short_name,
        opts.bbox,
        &opts.start,
        &opts.end,
        opts.max_observations,
        opts.cmr_endpoint.as_deref(),
    )?;

    if candidate_granules.is_empty() {
        return Err(OperaError::NoResults(format!(
            "no {} granules found between {} and {}",
            prod.short_name, opts.start, opts.end
        )));
    }

    let concept_id = concept_id
        .or_else(|| candidate_granules.iter().find_map(|g| g.concept_id.clone()))
        .ok_or_else(|| OperaError::NoResults("granules missing collection concept_id".into()))?;

    // Check whether any acquisition date has multiple distinct tiles covering the AOI.
    let mut tiles_per_date: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for g in &candidate_granules {
        if !g.date.is_empty() && !g.tile.is_empty() {
            tiles_per_date
                .entry(g.date.clone())
                .or_default()
                .insert(g.tile.clone());
        }
    }
    let multi_tile_dates = tiles_per_date.values().filter(|s| s.len() > 1).count();
    if multi_tile_dates > 0 {
        warnings.push(format!(
            "{multi_tile_dates} acquisition date(s) span multiple tiles/bursts in the AOI; each observation evaluates one representative granule per date"
        ));
    }

    let sampled = sample_granules_chronologically(
        candidate_granules,
        opts.interval_days,
        opts.max_observations,
    );
    let Some(first_sampled) = sampled.first() else {
        return Err(OperaError::NoResults(format!(
            "no dated {} granules remained after sampling between {} and {}",
            prod.short_name, opts.start, opts.end
        )));
    };
    let band = opts
        .band
        .clone()
        .or_else(|| prod.choose_band(&first_sampled.bands))
        .ok_or_else(|| {
            OperaError::InvalidInput(format!(
                "no default band available for product '{}'",
                prod.short_name
            ))
        })?;
    let expression = opts
        .expression
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let categorical = expression.is_none() && is_categorical_band(&prod.short_name, Some(&band));
    let is_water = expression.is_none() && is_dswx_water_band(&prod.short_name, Some(&band));
    let is_binary_water =
        expression.is_none() && is_dswx_binary_water_band(&prod.short_name, Some(&band));
    let include_inundated_veg = prod.short_name.to_ascii_uppercase().contains("DSWX-S1");
    let endpoint = opts
        .titiler_endpoint
        .as_deref()
        .unwrap_or(DEFAULT_TITILER_ENDPOINT);

    let mut observations = Vec::with_capacity(sampled.len());
    let mut failed_observations = Vec::new();
    for g in &sampled {
        let mut q = TitilerQuery::rasterio(&concept_id);
        q.endpoint = endpoint.to_string();
        q.product = Some(prod.short_name.clone());
        q.granule_ur = Some(g.id.clone());
        q.assets = vec![band.clone()];
        q.expression = expression.clone();
        match fetch_statistics(&q, &aoi, categorical, None).and_then(first_band_stats) {
            Ok(stats) => {
                let (open_w, total_w) = if is_water {
                    extract_dswx_water_areas_for_band(
                        &stats,
                        is_binary_water,
                        include_inundated_veg,
                    )
                } else {
                    (None, None)
                };
                observations.push(ObservationSummary {
                    target_date: g.date.clone(),
                    granule_id: g.id.clone(),
                    acquired_date: g.date.clone(),
                    begin_date: g.begin_date.clone(),
                    tile: g.tile.clone(),
                    band: band.clone(),
                    stats,
                    open_water_km2: open_w,
                    total_water_km2: total_w,
                });
            }
            Err(err) => {
                failed_observations.push(FailedObservation {
                    date: g.date.clone(),
                    granule_id: g.id.clone(),
                    error: err.to_string(),
                });
            }
        }
    }

    if observations.is_empty() {
        let detail = failed_observations
            .first()
            .map(|f| format!(" (first error on {}: {})", f.granule_id, f.error))
            .unwrap_or_default();
        return Err(OperaError::NoResults(format!(
            "failed to compute statistics for any sampled granule{detail}"
        )));
    }

    let origin = CalendarDate::parse(&observations[0].acquired_date).ok();
    let (metric_name, pts): (&str, Vec<(f64, f64)>) = if is_water {
        let pts = observations
            .iter()
            .filter_map(|o| {
                let d = CalendarDate::parse(&o.acquired_date).ok()?;
                let x = d.days_between(origin?) as f64;
                Some((x, o.total_water_km2?))
            })
            .collect();
        ("total_water_km2", pts)
    } else {
        let pts = observations
            .iter()
            .filter_map(|o| {
                let d = CalendarDate::parse(&o.acquired_date).ok()?;
                let x = d.days_between(origin?) as f64;
                Some((x, o.stats.mean?))
            })
            .collect();
        ("band_mean", pts)
    };
    let trend = compute_series_trend(metric_name, &pts);

    Ok(TimeSeriesResult {
        product: prod.short_name,
        concept_id,
        band,
        bbox: opts.bbox,
        start: opts.start.clone(),
        end: opts.end.clone(),
        observations,
        failed_observations,
        trend,
        warnings,
    })
}

#[cfg(not(feature = "http"))]
pub fn analyze_time_series(_opts: &TimeSeriesOptions) -> Result<TimeSeriesResult> {
    Err(OperaError::HttpDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::titiler::ClassBreakdown;

    fn mock_stats(open_px: f64, partial_px: f64) -> BandStatistics {
        BandStatistics {
            min: Some(0.0),
            max: Some(2.0),
            mean: Some(0.5),
            std: Some(0.5),
            median: Some(0.0),
            count: Some(1000.0),
            valid_pixels: Some(1000.0),
            valid_percent: Some(100.0),
            percentile_2: None,
            percentile_98: None,
            histogram: None,
            class_breakdown: vec![
                ClassBreakdown {
                    class_value: 1,
                    label: Some("Open water".into()),
                    pixel_count: open_px,
                    percent: open_px / 10.0,
                    area_km2: open_px * 0.0009,
                },
                ClassBreakdown {
                    class_value: 2,
                    label: Some("Partial surface water".into()),
                    pixel_count: partial_px,
                    percent: partial_px / 10.0,
                    area_km2: partial_px * 0.0009,
                },
            ],
        }
    }

    #[test]
    fn computes_class_deltas_and_linear_trend() {
        let b = mock_stats(100.0, 50.0);
        let a = mock_stats(300.0, 100.0);
        let deltas = compute_class_deltas(&b, &a);
        assert_eq!(deltas.len(), 2);
        assert!((deltas[0].delta_area_km2 - 200.0 * 0.0009).abs() < 1e-9);
        assert!((deltas[0].percent_change.unwrap() - 200.0).abs() < 1e-9);

        let trend = compute_series_trend(
            "total_water_km2",
            &[(0.0, 10.0), (10.0, 20.0), (20.0, 30.0)],
        )
        .unwrap();
        assert!((trend.slope_per_day.unwrap() - 1.0).abs() < 1e-9);
        assert!((trend.net_change - 20.0).abs() < 1e-9);
    }

    #[test]
    fn samples_granules_with_interval_and_cap() {
        let mk = |date: &str| OperaGranule {
            id: format!("G_{date}"),
            concept_id: Some("C1".into()),
            product: "OPERA_L3_DSWX-HLS_V1".into(),
            begin_date: Some(format!("{date}T00:00:00Z")),
            end_date: None,
            date: date.into(),
            time: "00:00".into(),
            tile: "T1".into(),
            bbox: None,
            rings: vec![],
            holes: vec![],
            bands: vec!["B01_WTR".into()],
            data_links: vec![],
        };
        let gs = vec![
            mk(""),
            mk("2024-01-01"),
            mk("2024-01-03"),
            mk("2024-01-10"),
            mk("2024-01-20"),
            mk("2024-01-30"),
        ];
        let sampled = sample_granules_chronologically(gs, Some(7), 10);
        let dates: Vec<&str> = sampled.iter().map(|g| g.date.as_str()).collect();
        assert_eq!(
            dates,
            vec!["2024-01-01", "2024-01-10", "2024-01-20", "2024-01-30"]
        );
        assert!(
            pick_nearest_granule(&[mk("")], CalendarDate::parse("2024-01-01").unwrap()).is_none()
        );
        assert_eq!(csv_escape("tile,rev\"1"), "\"tile,rev\"\"1\"");
    }
}
