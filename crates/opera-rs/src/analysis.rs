//! High-level NASA OPERA analytical workflows: two-date change detection and time-series trends.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(feature = "http")]
use crate::DEFAULT_TITILER_ENDPOINT;
use crate::cmr::OperaGranule;
#[cfg(feature = "http")]
use crate::cmr::{CmrSearchParams, search_granules};
use crate::error::{OperaError, Result};
use crate::products::{DSWX_OPEN_WATER_CLASS, DSWX_PARTIAL_WATER_CLASS};
#[cfg(feature = "http")]
use crate::products::{find_product, is_categorical_band, is_dswx_water_band};
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
    /// Search window in days on each side of `before_date` and `after_date` (default: 10).
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

        if let (Some(b_ow), Some(a_ow), Some(d_ow)) = (
            self.before.open_water_km2,
            self.after.open_water_km2,
            self.open_water_delta_km2,
        ) {
            md.push_str("## Surface Water Summary\n\n");
            md.push_str(&format!(
                "- **Open water (class 1):** {:.3} km^2 -> {:.3} km^2 (delta: {:+.3} km^2{})\n",
                b_ow,
                a_ow,
                d_ow,
                self.open_water_percent_change
                    .map(|p| format!(", {:+.1}%", p))
                    .unwrap_or_default()
            ));
            if let (Some(b_tw), Some(a_tw), Some(d_tw)) = (
                self.before.total_water_km2,
                self.after.total_water_km2,
                self.total_water_delta_km2,
            ) {
                md.push_str(&format!(
                    "- **Total surface water (classes 1+2):** {:.3} km^2 -> {:.3} km^2 (delta: {:+.3} km^2{})\n\n",
                    b_tw,
                    a_tw,
                    d_tw,
                    self.total_water_percent_change
                        .map(|p| format!(", {:+.1}%", p))
                        .unwrap_or_default()
                ));
            }
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
        md
    }
}

/// Extracts `(open_water_km2, total_water_km2)` from categorical DSWx class statistics.
pub fn extract_dswx_water_areas(stats: &BandStatistics) -> (Option<f64>, Option<f64>) {
    if stats.class_breakdown.is_empty() {
        return (None, None);
    }
    let open = stats
        .class_breakdown
        .iter()
        .find(|c| c.class_value == i64::from(DSWX_OPEN_WATER_CLASS))
        .map_or(0.0, |c| c.area_km2);
    let partial = stats
        .class_breakdown
        .iter()
        .find(|c| c.class_value == i64::from(DSWX_PARTIAL_WATER_CLASS))
        .map_or(0.0, |c| c.area_km2);
    (Some(open), Some(open + partial))
}

/// Computes class-by-class deltas between two categorical [`BandStatistics`] snapshots.
pub fn compute_class_deltas(before: &BandStatistics, after: &BandStatistics) -> Vec<ClassDelta> {
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

/// Picks the granule whose acquisition date is closest to `target`.
pub fn pick_nearest_granule(
    granules: &[OperaGranule],
    target: CalendarDate,
) -> Option<&OperaGranule> {
    granules.iter().min_by_key(|g| {
        CalendarDate::parse(&g.date)
            .map(|d| d.days_between(target).abs())
            .unwrap_or(i64::MAX)
    })
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
    let before_target = CalendarDate::parse(&opts.before_date)?;
    let after_target = CalendarDate::parse(&opts.after_date)?;
    let window = opts.window_days.clamp(0, 90);
    let aoi = opts
        .aoi_geojson
        .clone()
        .unwrap_or_else(|| opts.bbox.to_geojson_feature());

    let find_near = |target: CalendarDate| -> Result<OperaGranule> {
        let s = target.add_days(-window).to_string();
        let e = target.add_days(window).to_string();
        let res = search_granules(&CmrSearchParams {
            product: prod.short_name.clone(),
            bbox: Some(opts.bbox),
            start: Some(s.clone()),
            end: Some(e.clone()),
            limit: 50,
            deduplicate_versions: true,
            newest_per_tile: false,
            cmr_endpoint: opts.cmr_endpoint.clone(),
        })?;
        pick_nearest_granule(&res.granules, target)
            .cloned()
            .ok_or_else(|| {
                OperaError::NoResults(format!(
                    "no {} granules found within +/-{window} days of {target}",
                    prod.short_name
                ))
            })
    };

    let before_g = find_near(before_target)?;
    let after_g = find_near(after_target)?;
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

    let categorical =
        opts.expression.is_none() && is_categorical_band(&prod.short_name, Some(&band));
    let is_water = opts.expression.is_none() && is_dswx_water_band(&prod.short_name, Some(&band));
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
        q.expression = opts.expression.clone();
        let stats = first_band_stats(fetch_statistics(&q, &aoi, categorical, None)?)?;
        let (open_w, total_w) = if is_water {
            extract_dswx_water_areas(&stats)
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
    /// Maximum number of observations to evaluate (default: 12, capped at 100).
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
    pub trend: Option<SeriesTrend>,
}

impl TimeSeriesResult {
    /// Exports observations as CSV (`date,granule_id,tile,mean,valid_percent,open_water_km2,total_water_km2`).
    pub fn to_csv(&self) -> String {
        let mut out = String::from(
            "date,granule_id,tile,mean,valid_percent,open_water_km2,total_water_km2\n",
        );
        for obs in &self.observations {
            let opt = |v: Option<f64>| v.map(|x| format!("{x:.6}")).unwrap_or_default();
            out.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                obs.acquired_date,
                obs.granule_id,
                obs.tile,
                opt(obs.stats.mean),
                opt(obs.stats.valid_percent),
                opt(obs.open_water_km2),
                opt(obs.total_water_km2),
            ));
        }
        out
    }
}

/// Samples granules chronologically with optional minimum `interval_days` spacing and count cap.
pub fn sample_granules_chronologically(
    mut granules: Vec<OperaGranule>,
    interval_days: Option<i64>,
    max_observations: usize,
) -> Vec<OperaGranule> {
    granules.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.id.cmp(&b.id)));
    // Keep one representative granule per acquisition date first.
    let mut by_date: Vec<OperaGranule> = Vec::new();
    for g in granules {
        if by_date.last().is_some_and(|prev| prev.date == g.date) {
            continue;
        }
        by_date.push(g);
    }

    let spaced = if let Some(min_gap) = interval_days.filter(|&d| d > 0) {
        let mut kept: Vec<OperaGranule> = Vec::new();
        let mut last_day: Option<CalendarDate> = None;
        for g in by_date {
            if let Ok(d) = CalendarDate::parse(&g.date)
                && last_day.is_none_or(|prev| d.days_between(prev) >= min_gap)
            {
                last_day = Some(d);
                kept.push(g);
            }
        }
        kept
    } else {
        by_date
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

/// Runs a multi-date OPERA time-series analysis across `start..=end`.
#[cfg(feature = "http")]
pub fn analyze_time_series(opts: &TimeSeriesOptions) -> Result<TimeSeriesResult> {
    let prod = find_product(&opts.product)?;
    let _ = CalendarDate::parse(&opts.start)?;
    let _ = CalendarDate::parse(&opts.end)?;
    let aoi = opts
        .aoi_geojson
        .clone()
        .unwrap_or_else(|| opts.bbox.to_geojson_feature());

    let search = search_granules(&CmrSearchParams {
        product: prod.short_name.clone(),
        bbox: Some(opts.bbox),
        start: Some(opts.start.clone()),
        end: Some(opts.end.clone()),
        limit: 200,
        deduplicate_versions: true,
        newest_per_tile: false,
        cmr_endpoint: opts.cmr_endpoint.clone(),
    })?;
    if search.granules.is_empty() {
        return Err(OperaError::NoResults(format!(
            "no {} granules found between {} and {}",
            prod.short_name, opts.start, opts.end
        )));
    }

    let concept_id = search
        .concept_id
        .clone()
        .ok_or_else(|| OperaError::NoResults("granules missing collection concept_id".into()))?;
    let sampled =
        sample_granules_chronologically(search.granules, opts.interval_days, opts.max_observations);
    let band = opts
        .band
        .clone()
        .or_else(|| prod.choose_band(&sampled[0].bands))
        .ok_or_else(|| {
            OperaError::InvalidInput(format!(
                "no default band available for product '{}'",
                prod.short_name
            ))
        })?;
    let categorical =
        opts.expression.is_none() && is_categorical_band(&prod.short_name, Some(&band));
    let is_water = opts.expression.is_none() && is_dswx_water_band(&prod.short_name, Some(&band));
    let endpoint = opts
        .titiler_endpoint
        .as_deref()
        .unwrap_or(DEFAULT_TITILER_ENDPOINT);

    let mut observations = Vec::with_capacity(sampled.len());
    for g in &sampled {
        let mut q = TitilerQuery::rasterio(&concept_id);
        q.endpoint = endpoint.to_string();
        q.product = Some(prod.short_name.clone());
        q.granule_ur = Some(g.id.clone());
        q.assets = vec![band.clone()];
        q.expression = opts.expression.clone();
        if let Ok(stat_res) = fetch_statistics(&q, &aoi, categorical, None)
            && let Ok(stats) = first_band_stats(stat_res)
        {
            let (open_w, total_w) = if is_water {
                extract_dswx_water_areas(&stats)
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
    }

    if observations.is_empty() {
        return Err(OperaError::NoResults(
            "failed to compute statistics for any sampled granule".into(),
        ));
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
        trend,
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
            bands: vec!["B01_WTR".into()],
            data_links: vec![],
        };
        let gs = vec![
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
    }
}
