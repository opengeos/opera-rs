//! Optimal Sentinel-1 burst/date subset selection for handling missing CSLC acquisitions.
//!
//! Adapted in pure Rust from `opera-adt/opera-utils` (`missing_data.py`).
//!
//! When assembling a multi-burst Sentinel-1 stack for InSAR displacement processing, some bursts
//! may be missing on particular acquisition dates. Using different dates for neighboring bursts
//! introduces spatial discontinuities in estimated displacement/velocity. This module finds
//! rectangular `(burst_ids x dates)` subsets that maximize the total number of bursts used
//! while ensuring every selected burst ID has identical acquisition dates.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::bursts::{get_burst_id, get_dates, normalize_burst_id};
use crate::error::{OperaError, Result};
use crate::types::CalendarDate;

/// Cap on combinations of missing-date groups evaluated to avoid exponential blowup
/// (matches `opera-utils` `1e4` cap).
const MAX_MISSING_GROUP_COMBINATIONS: usize = 10_000;

/// One valid rectangular subset of Sentinel-1 bursts and acquisition dates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurstSubsetOption {
    /// Total number of `(burst_id, date)` acquisitions used (`num_burst_ids * num_dates`).
    pub total_num_bursts: usize,
    /// Number of unique burst IDs selected.
    pub num_burst_ids: usize,
    /// Number of unique acquisition dates selected.
    pub num_dates: usize,
    /// Sorted list of normalized burst IDs in this subset.
    pub burst_ids: Vec<String>,
    /// Sorted list of ISO `YYYY-MM-DD` acquisition dates in this subset.
    pub dates: Vec<String>,
    /// Total candidate `(burst_id, date)` acquisitions in the input.
    pub num_candidate_bursts: usize,
    /// Number of candidate acquisitions excluded by this subset.
    pub excluded_bursts: usize,
    /// Subset of original input filenames/URLs matching this option (empty when constructed from pairs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
}

/// Builds a mapping from normalized burst ID to sorted acquisition dates from CSLC filenames/URLs.
///
/// Assumes the first `YYYYMMDD` date in each filename stem is the acquisition date (as in
/// `OPERA_L2_CSLC-S1` products). Returns an error if any `(burst_id, date)` pair is duplicated.
pub fn get_burst_id_to_dates(slc_files: &[String]) -> Result<BTreeMap<String, Vec<CalendarDate>>> {
    let mut pairs: Vec<(String, CalendarDate)> = Vec::with_capacity(slc_files.len());
    for file in slc_files {
        let burst = get_burst_id(file)?;
        let dates = get_dates(file);
        let first = *dates.first().ok_or_else(|| {
            OperaError::InvalidInput(format!("could not extract acquisition date from '{file}'"))
        })?;
        pairs.push((burst, first));
    }
    burst_id_mapping_from_pairs(&pairs)
}

fn burst_id_mapping_from_pairs(
    pairs: &[(String, CalendarDate)],
) -> Result<BTreeMap<String, Vec<CalendarDate>>> {
    let mut counts: BTreeMap<(String, CalendarDate), usize> = BTreeMap::new();
    let mut map: BTreeMap<String, Vec<CalendarDate>> = BTreeMap::new();
    for (burst, date) in pairs {
        let norm = normalize_burst_id(burst);
        *counts.entry((norm.clone(), *date)).or_insert(0) += 1;
        map.entry(norm).or_default().push(*date);
    }

    let dupes: Vec<String> = counts
        .into_iter()
        .filter(|(_, c)| *c > 1)
        .map(|((b, d), _)| format!("{b}:{d}"))
        .collect();
    if !dupes.is_empty() {
        return Err(OperaError::InvalidInput(format!(
            "duplicated (burst_id, date) pairs passed: {}",
            dupes.join(", ")
        )));
    }

    for dates in map.values_mut() {
        dates.sort();
    }
    Ok(map)
}

/// Builds the `(burst_ids, dates, incidence_matrix)` tuple where `incidence[row][col]` is `true`
/// if `burst_ids[row]` was acquired on `dates[col]`.
pub fn get_burst_id_date_incidence(
    burst_id_to_dates: &BTreeMap<String, Vec<CalendarDate>>,
) -> (Vec<String>, Vec<CalendarDate>, Vec<Vec<bool>>) {
    let burst_ids: Vec<String> = burst_id_to_dates.keys().cloned().collect();
    let all_dates_set: BTreeSet<CalendarDate> = burst_id_to_dates
        .values()
        .flat_map(|v| v.iter().copied())
        .collect();
    let dates: Vec<CalendarDate> = all_dates_set.into_iter().collect();

    let mut matrix = Vec::with_capacity(burst_ids.len());
    for burst_id in &burst_ids {
        let present: BTreeSet<CalendarDate> = burst_id_to_dates[burst_id].iter().copied().collect();
        let row: Vec<bool> = dates.iter().map(|d| present.contains(d)).collect();
        matrix.push(row);
    }
    (burst_ids, dates, matrix)
}

/// Generates ranked valid `(burst_ids x dates)` subsets from an incidence matrix.
pub fn generate_burst_subset_options(
    incidence: &[Vec<bool>],
    burst_ids: &[String],
    dates: &[CalendarDate],
) -> Vec<BurstSubsetOption> {
    let n_rows = incidence.len();
    let n_cols = dates.len();
    if n_rows == 0 || n_cols == 0 {
        return Vec::new();
    }

    let num_candidate_bursts: usize = incidence
        .iter()
        .map(|r| r.iter().filter(|&&b| b).count())
        .sum();

    // Collect distinct missing-date column index tuples across all rows.
    let mut missing_groups_set: BTreeSet<Vec<usize>> = BTreeSet::new();
    for row in incidence {
        let missing: Vec<usize> = row
            .iter()
            .enumerate()
            .filter_map(|(col, &present)| (!present).then_some(col))
            .collect();
        missing_groups_set.insert(missing);
    }
    let missing_groups: Vec<Vec<usize>> = missing_groups_set.into_iter().collect();

    // Enumerate powerset of missing-date groups in increasing cardinality (BFS/size order),
    // capped at MAX_MISSING_GROUP_COMBINATIONS.
    let mut exclude_combos: Vec<BTreeSet<usize>> = Vec::new();
    let mut combo_count = 0usize;
    'outer: for k in 0..=missing_groups.len() {
        let mut indices: Vec<usize> = (0..k).collect();
        loop {
            let mut excluded = BTreeSet::new();
            for &idx in &indices {
                excluded.extend(missing_groups[idx].iter().copied());
            }
            exclude_combos.push(excluded);
            combo_count += 1;
            if combo_count >= MAX_MISSING_GROUP_COMBINATIONS {
                break 'outer;
            }
            if k == 0 {
                break;
            }
            // Advance k-combination lexicographically.
            let mut pos = k;
            while pos > 0 {
                pos -= 1;
                if indices[pos] != pos + missing_groups.len() - k {
                    indices[pos] += 1;
                    for j in pos + 1..k {
                        indices[j] = indices[j - 1] + 1;
                    }
                    break;
                }
                if pos == 0 {
                    // Exhausted all combinations of size k.
                    indices.clear();
                    break;
                }
            }
            if indices.is_empty() {
                break;
            }
        }
    }

    let mut tested: BTreeSet<(Vec<usize>, Vec<usize>)> = BTreeSet::new();
    let mut options: Vec<BurstSubsetOption> = Vec::new();

    for excluded_cols in exclude_combos {
        let valid_cols: Vec<usize> = (0..n_cols).filter(|c| !excluded_cols.contains(c)).collect();
        if valid_cols.is_empty() {
            continue;
        }

        let valid_rows: Vec<usize> = (0..n_rows)
            .filter(|&r| valid_cols.iter().all(|&c| incidence[r][c]))
            .collect();
        if valid_rows.is_empty() {
            continue;
        }

        let key = (valid_rows.clone(), valid_cols.clone());
        if !tested.insert(key) {
            continue;
        }

        let num_burst_ids = valid_rows.len();
        let num_dates = valid_cols.len();
        let total_num_bursts = num_burst_ids * num_dates;
        let selected_burst_ids: Vec<String> =
            valid_rows.iter().map(|&r| burst_ids[r].clone()).collect();
        let selected_dates: Vec<String> =
            valid_cols.iter().map(|&c| dates[c].to_string()).collect();

        options.push(BurstSubsetOption {
            total_num_bursts,
            num_burst_ids,
            num_dates,
            burst_ids: selected_burst_ids,
            dates: selected_dates,
            num_candidate_bursts,
            excluded_bursts: num_candidate_bursts.saturating_sub(total_num_bursts),
            inputs: Vec::new(),
        });
    }

    options.sort_by(|a, b| {
        b.total_num_bursts
            .cmp(&a.total_num_bursts)
            .then_with(|| b.num_burst_ids.cmp(&a.num_burst_ids))
            .then_with(|| b.num_dates.cmp(&a.num_dates))
    });
    options
}

/// Computes ranked valid subsets from a list of OPERA CSLC filenames or URLs, populating
/// each option's matching `inputs`.
pub fn get_missing_data_options(slc_files: &[String]) -> Result<Vec<BurstSubsetOption>> {
    if slc_files.is_empty() {
        return Ok(Vec::new());
    }
    let burst_map = get_burst_id_to_dates(slc_files)?;
    let (burst_ids, dates, incidence) = get_burst_id_date_incidence(&burst_map);
    let mut options = generate_burst_subset_options(&incidence, &burst_ids, &dates);

    // Pre-parse each file's (normalized burst_id, first acquisition date) once so input selection
    // matches `get_burst_id_to_dates` and never triggers on generation/reference dates.
    let parsed_files: Vec<(String, CalendarDate, &String)> = slc_files
        .iter()
        .filter_map(|f| {
            let burst = get_burst_id(f).ok()?;
            let acq_date = *get_dates(f).first()?;
            Some((burst, acq_date, f))
        })
        .collect();

    for opt in &mut options {
        let allowed_dates: BTreeSet<CalendarDate> = opt
            .dates
            .iter()
            .filter_map(|d| CalendarDate::parse(d).ok())
            .collect();
        let allowed_bursts: BTreeSet<&str> = opt.burst_ids.iter().map(String::as_str).collect();
        opt.inputs = parsed_files
            .iter()
            .filter(|(b, d, _)| allowed_dates.contains(d) && allowed_bursts.contains(b.as_str()))
            .map(|(_, _, f)| (*f).clone())
            .collect();
    }
    Ok(options)
}

/// Computes ranked valid subsets from explicit `(burst_id, date)` pairs.
pub fn get_missing_data_options_from_pairs(
    pairs: &[(String, CalendarDate)],
) -> Result<Vec<BurstSubsetOption>> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let burst_map = burst_id_mapping_from_pairs(pairs)?;
    let (burst_ids, dates, incidence) = get_burst_id_date_incidence(&burst_map);
    Ok(generate_burst_subset_options(
        &incidence, &burst_ids, &dates,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_optimal_subsets_when_one_burst_misses_a_date() {
        // 3 bursts across 3 dates, except burst 3 is missing on 2023-01-13:
        // Option 1: drop 2023-01-13 -> 3 bursts * 2 dates = 6 bursts (or drop burst 3 -> 2 bursts * 3 dates = 6 bursts).
        // With tie-breaker on num_burst_ids, 3 bursts * 2 dates ranks first!
        let files = vec![
            "OPERA_L2_CSLC-S1_T087-185683-IW1_20230101T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW1_20230113T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW1_20230125T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW2_20230101T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW2_20230113T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW2_20230125T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW3_20230101T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW3_20230125T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
        ];

        let opts = get_missing_data_options(&files).unwrap();
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].total_num_bursts, 6);
        assert_eq!(opts[0].num_burst_ids, 3);
        assert_eq!(opts[0].num_dates, 2);
        assert_eq!(opts[0].excluded_bursts, 2);
        assert_eq!(opts[0].inputs.len(), 6);

        assert_eq!(opts[1].total_num_bursts, 6);
        assert_eq!(opts[1].num_burst_ids, 2);
        assert_eq!(opts[1].num_dates, 3);
        assert_eq!(opts[1].inputs.len(), 6);
    }

    #[test]
    fn rejects_duplicate_burst_date_pairs() {
        let files = vec![
            "t087_185683_iw1_20230101.h5".into(),
            "T087-185683-IW1_20230101.h5".into(),
        ];
        assert!(get_missing_data_options(&files).is_err());
    }

    #[test]
    fn option_inputs_match_acquisition_date_not_generation_date() {
        // Burst 2 was acquired on 2023-02-01 (excluded) but generated on 2023-01-01 (selected).
        let files = vec![
            "OPERA_L2_CSLC-S1_T087-185683-IW1_20230101T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW2_20230101T161649Z_20240504T185235Z_S1A_VV_v1.1.h5"
                .into(),
            "OPERA_L2_CSLC-S1_T087-185683-IW2_20230201T161649Z_20230101T185235Z_S1A_VV_v1.1.h5"
                .into(),
        ];
        let opts = get_missing_data_options(&files).unwrap();
        let opt_two_bursts = opts
            .iter()
            .find(|o| o.num_burst_ids == 2 && o.num_dates == 1)
            .expect("2x1 subset");
        assert_eq!(opt_two_bursts.inputs.len(), 2);
        assert!(
            opt_two_bursts
                .inputs
                .iter()
                .all(|f| !f.contains("20230201T161649Z"))
        );
    }
}
