//! Sentinel-1 burst ID parsing, date extraction, and OPERA CSLC filename utilities.
//!
//! Adapted in pure Rust from `opera-adt/opera-utils` (`bursts.py`, `_dates.py`, `constants.py`),
//! supporting both official product burst IDs (`T087-185683-IW2`) and normalized COMPASS /
//! burst-database IDs (`t087_185683_iw2`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{OperaError, Result};
use crate::types::CalendarDate;

/// Parsed Sentinel-1 OPERA burst identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ParsedBurstId {
    /// Relative orbit / track number (`001`..`175`).
    pub track: u16,
    /// Six-digit burst number (e.g. `185683`).
    pub burst_number: u32,
    /// Interferometric Wide subswath index (`1`, `2`, or `3`).
    pub subswath: u8,
    /// Normalized lowercase underscore burst ID (e.g. `t087_185683_iw2`).
    pub normalized: String,
    /// Official uppercase hyphenated burst ID (e.g. `T087-185683-IW2`).
    pub official: String,
}

/// Metadata parsed from an `OPERA_L2_CSLC-S1` or `OPERA_L2_COMPRESSED-CSLC-S1` filename.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedCslcFilename {
    /// Original input filename or URL.
    pub filename: String,
    /// Whether this is a `COMPRESSED-CSLC-S1` product.
    pub is_compressed: bool,
    /// Optional DISP frame identifier (e.g. `11116` from `F11116` on compressed CSLCs).
    pub frame_id: Option<u32>,
    /// Normalized Sentinel-1 burst ID (e.g. `t087_185683_iw2`).
    pub burst_id: String,
    /// Acquisition start UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
    pub start_datetime: String,
    /// Product generation UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
    pub generation_datetime: String,
    /// Sensor identifier (`S1A`, `S1B`, `S1C`, etc.), if present in filename.
    pub sensor: Option<String>,
    /// Polarization (`VV` or `HH`).
    pub polarization: String,
    /// Product version string (e.g. `1.1`).
    pub version: String,
}

/// Normalizes an OPERA Sentinel-1 burst ID to lowercase underscores
/// (e.g. `"T087-185683-IW2"` -> `"t087_185683_iw2"`).
pub fn normalize_burst_id(burst_id: &str) -> String {
    burst_id.trim().to_ascii_lowercase().replace('-', "_")
}

/// Extracts and parses a Sentinel-1 burst ID from a burst string, file path, granule UR, or URL.
///
/// Matches both official (`T087-185683-IW2`) and COMPASS / burst-DB (`t087_185683_iw2`) forms.
pub fn parse_burst_id(input: &str) -> Result<ParsedBurstId> {
    let bytes = input.as_bytes();
    // Pattern: [tT](\d{3})[-_](\d{6})[-_][iI][wW]([1-3]) -> length 15
    if bytes.len() >= 15 {
        for i in 0..=bytes.len() - 15 {
            let s = &bytes[i..i + 15];
            let sep1 = s[4];
            let sep2 = s[11];
            if matches!(s[0], b't' | b'T')
                && s[1..4].iter().all(u8::is_ascii_digit)
                && matches!(sep1, b'-' | b'_')
                && s[5..11].iter().all(u8::is_ascii_digit)
                && matches!(sep2, b'-' | b'_')
                && matches!(s[12], b'i' | b'I')
                && matches!(s[13], b'w' | b'W')
                && matches!(s[14], b'1'..=b'3')
            {
                // Avoid matching in the middle of an alphanumeric token.
                let left_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                let right_ok = i + 15 == bytes.len() || !bytes[i + 15].is_ascii_alphanumeric();
                if !left_ok || !right_ok {
                    continue;
                }
                let track_str = std::str::from_utf8(&s[1..4]).unwrap_or("0");
                let burst_str = std::str::from_utf8(&s[5..11]).unwrap_or("0");
                let subswath = s[14] - b'0';
                let track: u16 = track_str.parse().unwrap_or(0);
                let burst_number: u32 = burst_str.parse().unwrap_or(0);
                let normalized = format!("t{track_str}_{burst_str}_iw{subswath}");
                let official = format!("T{track_str}-{burst_str}-IW{subswath}");
                return Ok(ParsedBurstId {
                    track,
                    burst_number,
                    subswath,
                    normalized,
                    official,
                });
            }
        }
    }
    Err(OperaError::InvalidInput(format!(
        "could not parse Sentinel-1 burst ID from '{input}'"
    )))
}

/// Extracts the normalized burst ID (`t087_185683_iw2`) from a filename, granule UR, or URL.
pub fn get_burst_id(input: &str) -> Result<String> {
    Ok(parse_burst_id(input)?.normalized)
}

/// Sorts filenames, URLs, or granule URs by their parsed normalized burst ID (preserving
/// tie-breaking order lexicographically by the original string).
pub fn sort_by_burst_id(inputs: &[String]) -> Result<Vec<String>> {
    let mut keyed: Vec<(String, String)> = Vec::with_capacity(inputs.len());
    for item in inputs {
        keyed.push((get_burst_id(item)?, item.clone()));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    Ok(keyed.into_iter().map(|(_, item)| item).collect())
}

/// Groups filenames, URLs, or granule URs by their normalized Sentinel-1 burst ID.
pub fn group_by_burst(inputs: &[String]) -> Result<BTreeMap<String, Vec<String>>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in inputs {
        let burst = get_burst_id(item)?;
        groups.entry(burst).or_default().push(item.clone());
    }
    for items in groups.values_mut() {
        items.sort();
    }
    Ok(groups)
}

/// Keeps only inputs whose parsed burst ID matches one of `allowed_burst_ids`
/// (case- and delimiter-insensitive).
pub fn filter_by_burst_id(inputs: &[String], allowed_burst_ids: &[String]) -> Vec<String> {
    let allowed: BTreeSet<String> = allowed_burst_ids
        .iter()
        .map(|b| normalize_burst_id(b))
        .collect();
    inputs
        .iter()
        .filter(|item| {
            get_burst_id(item)
                .map(|b| allowed.contains(&b))
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Extracts the filename component (excluding parent directories, query strings, and GDAL
/// `NETCDF:` / `HDF5:` prefixes) so directory timestamps are not mistaken for acquisition dates.
pub(crate) fn basename_for_parsing(input: &str) -> &str {
    // Strip URL query/fragment if present.
    let without_query = input.split(['?', '#']).next().unwrap_or(input);
    // Strip GDAL subdataset suffix like `file.nc:displacement` when not a Windows drive letter or scheme.
    let stripped = if let Some(rest) = without_query.strip_prefix("NETCDF:\"") {
        rest.split('"').next().unwrap_or(rest)
    } else if let Some(rest) = without_query.strip_prefix("HDF5:\"") {
        rest.split('"').next().unwrap_or(rest)
    } else {
        without_query
    };
    stripped.rsplit(['/', '\\']).next().unwrap_or(stripped)
}

/// Parses an 8-digit compact date `YYYYMMDD` into a validated [`CalendarDate`].
pub(crate) fn parse_compact_date(s: &str) -> Option<CalendarDate> {
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[4..6].parse().ok()?;
    let d: u32 = s[6..8].parse().ok()?;
    if !(1970..=2199).contains(&y) {
        return None;
    }
    CalendarDate::new(y, m, d).ok()
}

/// Parses a 15- or 16-character compact UTC timestamp `YYYYMMDDTHHMMSS[Z]` into
/// `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn parse_compact_datetime_iso(s: &str) -> Option<String> {
    let core = s.strip_suffix('Z').unwrap_or(s);
    if core.len() != 15 || core.as_bytes()[8] != b'T' {
        return None;
    }
    let date = parse_compact_date(&core[0..8])?;
    let time = &core[9..15];
    if !time.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hh: u32 = time[0..2].parse().ok()?;
    let mm: u32 = time[2..4].parse().ok()?;
    let ss: u32 = time[4..6].parse().ok()?;
    if hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    Some(format!("{date}T{hh:02}:{mm:02}:{ss:02}Z"))
}

/// Extracts all `YYYYMMDD` dates occurring in the filename stem (excluding parent directories).
pub fn get_dates(input: &str) -> Vec<CalendarDate> {
    let name = basename_for_parsing(input);
    let bytes = name.as_bytes();
    let mut dates = Vec::new();
    let mut i = 0;
    while i + 8 <= bytes.len() {
        if bytes[i..i + 8].iter().all(u8::is_ascii_digit) {
            let left_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
            let right_ok = i + 8 == bytes.len() || !bytes[i + 8].is_ascii_digit();
            if left_ok
                && right_ok
                && let Ok(slice) = std::str::from_utf8(&bytes[i..i + 8])
                && let Some(date) = parse_compact_date(slice)
            {
                dates.push(date);
                i += 8;
                continue;
            }
        }
        i += 1;
    }
    dates
}

/// Extracts all `YYYYMMDDTHHMMSS[Z]` timestamps from the filename stem as RFC3339 UTC strings.
pub fn get_datetimes(input: &str) -> Vec<String> {
    let name = basename_for_parsing(input);
    let bytes = name.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 15 <= bytes.len() {
        if bytes[i + 8] == b'T'
            && bytes[i..i + 8].iter().all(u8::is_ascii_digit)
            && bytes[i + 9..i + 15].iter().all(u8::is_ascii_digit)
        {
            let left_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
            let has_z = i + 15 < bytes.len() && bytes[i + 15] == b'Z';
            let end = if has_z { i + 16 } else { i + 15 };
            let right_ok = end == bytes.len() || !bytes[end].is_ascii_digit();
            if left_ok
                && right_ok
                && let Ok(slice) = std::str::from_utf8(&bytes[i..end])
                && let Some(iso) = parse_compact_datetime_iso(slice)
            {
                out.push(iso);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Sorts inputs by their extracted filename dates (files with no dates sort last).
pub fn sort_files_by_date(inputs: &[String]) -> Vec<String> {
    let mut keyed: Vec<(Vec<CalendarDate>, String)> =
        inputs.iter().map(|s| (get_dates(s), s.clone())).collect();
    keyed.sort_by(|a, b| match (a.0.is_empty(), b.0.is_empty()) {
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)),
    });
    keyed.into_iter().map(|(_, s)| s).collect()
}

/// Groups inputs by their first extracted acquisition date (`YYYY-MM-DD`).
/// Inputs without any date in their filename stem are omitted.
pub fn group_by_date(inputs: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in inputs {
        if let Some(first) = get_dates(item).first() {
            groups
                .entry(first.to_string())
                .or_default()
                .push(item.clone());
        }
    }
    for items in groups.values_mut() {
        items.sort();
    }
    groups
}

/// Keeps only inputs whose filename dates contain at least one date in `dates`.
pub fn filter_by_date(inputs: &[String], dates: &[CalendarDate]) -> Vec<String> {
    let allowed: BTreeSet<CalendarDate> = dates.iter().copied().collect();
    inputs
        .iter()
        .filter(|item| get_dates(item).iter().any(|d| allowed.contains(d)))
        .cloned()
        .collect()
}

/// Parses an `OPERA_L2_CSLC-S1` or `OPERA_L2_COMPRESSED-CSLC-S1` filename, path, or URL.
pub fn parse_cslc_filename(input: &str) -> Result<ParsedCslcFilename> {
    let name = basename_for_parsing(input);
    let stem = name
        .strip_suffix(".h5")
        .or_else(|| name.strip_suffix(".nc"))
        .or_else(|| name.strip_suffix(".tif"))
        .unwrap_or(name);
    let parts: Vec<&str> = stem.split('_').collect();

    // Standard CSLC-S1:
    // OPERA_L2_CSLC-S1_T087-185683-IW2_20230322T161649Z_20240504T185235Z_S1A_VV_v1.1
    if parts.len() >= 9 && parts[0] == "OPERA" && parts[1] == "L2" && parts[2] == "CSLC-S1" {
        let burst = parse_burst_id(parts[3])?;
        let start_datetime = parse_compact_datetime_iso(parts[4]).ok_or_else(|| {
            OperaError::InvalidInput(format!("invalid CSLC start datetime in '{input}'"))
        })?;
        let generation_datetime = parse_compact_datetime_iso(parts[5]).ok_or_else(|| {
            OperaError::InvalidInput(format!("invalid CSLC generation datetime in '{input}'"))
        })?;
        let sensor = parts[6].to_string();
        let polarization = parts[7].to_string();
        let version = parts[8]
            .strip_prefix('v')
            .ok_or_else(|| OperaError::InvalidInput(format!("invalid CSLC version in '{input}'")))?
            .to_string();
        return Ok(ParsedCslcFilename {
            filename: input.to_string(),
            is_compressed: false,
            frame_id: None,
            burst_id: burst.normalized,
            start_datetime,
            generation_datetime,
            sensor: Some(sensor),
            polarization,
            version,
        });
    }

    // Compressed CSLC-S1:
    // OPERA_L2_COMPRESSED-CSLC-S1_F11116_T087-185683-IW2_20230322T161649Z_20230101T161649Z_20230322T161649Z_20240504T185235Z_VV_v1.1
    // (frame_id token F\d+ is optional)
    if parts.len() >= 10
        && parts[0] == "OPERA"
        && parts[1] == "L2"
        && parts[2] == "COMPRESSED-CSLC-S1"
    {
        let (frame_id, offset) = if let Some(digits) = parts[3].strip_prefix('F') {
            if let Ok(fid) = digits.parse::<u32>() {
                (Some(fid), 4)
            } else {
                (None, 3)
            }
        } else {
            (None, 3)
        };
        if parts.len() < offset + 7 {
            return Err(OperaError::InvalidInput(format!(
                "invalid COMPRESSED-CSLC-S1 filename '{input}'"
            )));
        }
        let burst = parse_burst_id(parts[offset])?;
        let start_datetime = parse_compact_datetime_iso(parts[offset + 1]).ok_or_else(|| {
            OperaError::InvalidInput(format!(
                "invalid compressed CSLC start datetime in '{input}'"
            ))
        })?;
        let generation_datetime =
            parse_compact_datetime_iso(parts[offset + 4]).ok_or_else(|| {
                OperaError::InvalidInput(format!(
                    "invalid compressed CSLC generation datetime in '{input}'"
                ))
            })?;
        let polarization = parts[offset + 5].to_string();
        let version = parts[offset + 6]
            .strip_prefix('v')
            .ok_or_else(|| OperaError::InvalidInput(format!("invalid CSLC version in '{input}'")))?
            .to_string();
        return Ok(ParsedCslcFilename {
            filename: input.to_string(),
            is_compressed: true,
            frame_id,
            burst_id: burst.normalized,
            start_datetime,
            generation_datetime,
            sensor: None,
            polarization,
            version,
        });
    }

    Err(OperaError::InvalidInput(format!(
        "not a recognized OPERA CSLC-S1 filename: '{input}'"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_burst_ids() {
        assert_eq!(normalize_burst_id("T087-185683-IW2"), "t087_185683_iw2");
        let parsed = parse_burst_id(
            "OPERA_L2_CSLC-S1_T087-185683-IW2_20230322T161649Z_20240504T185235Z_S1A_VV_v1.1.h5",
        )
        .unwrap();
        assert_eq!(parsed.track, 87);
        assert_eq!(parsed.burst_number, 185_683);
        assert_eq!(parsed.subswath, 2);
        assert_eq!(parsed.normalized, "t087_185683_iw2");
        assert_eq!(parsed.official, "T087-185683-IW2");

        let compass = parse_burst_id("inputs/t042_088905_iw1_20230101.h5").unwrap();
        assert_eq!(compass.normalized, "t042_088905_iw1");
        assert_eq!(compass.official, "T042-088905-IW1");
        assert!(parse_burst_id("no_burst_here.tif").is_err());
    }

    #[test]
    fn groups_sorts_and_filters_by_burst_and_date() {
        let files = vec![
            "/tmp/20190101/t087_185684_iw2_20230322.h5".to_string(),
            "/tmp/20190101/t087_185683_iw1_20230310.h5".to_string(),
            "/tmp/20190101/T087-185683-IW1_20230322.h5".to_string(),
        ];
        // Directory date 20190101 must be ignored.
        assert_eq!(
            get_dates(&files[0]),
            vec![CalendarDate::new(2023, 3, 22).unwrap()]
        );

        let grouped = group_by_burst(&files).unwrap();
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["t087_185683_iw1"].len(), 2);
        assert_eq!(grouped["t087_185684_iw2"].len(), 1);

        let filtered = filter_by_burst_id(&files, &["T087-185684-IW2".to_string()]);
        assert_eq!(filtered.len(), 1);

        let by_date = group_by_date(&files);
        assert_eq!(by_date["2023-03-22"].len(), 2);
        assert_eq!(by_date["2023-03-10"].len(), 1);

        let sorted_dates = sort_files_by_date(&files);
        assert!(sorted_dates[0].contains("20230310"));
    }

    #[test]
    fn parses_cslc_and_compressed_cslc_filenames() {
        let std_cslc = parse_cslc_filename(
            "https://datapool.asf.alaska.edu/OPERA_L2_CSLC-S1_T087-185683-IW2_20230322T161649Z_20240504T185235Z_S1A_VV_v1.1.h5",
        )
        .unwrap();
        assert!(!std_cslc.is_compressed);
        assert_eq!(std_cslc.burst_id, "t087_185683_iw2");
        assert_eq!(std_cslc.start_datetime, "2023-03-22T16:16:49Z");
        assert_eq!(std_cslc.sensor.as_deref(), Some("S1A"));
        assert_eq!(std_cslc.version, "1.1");

        let comp = parse_cslc_filename(
            "OPERA_L2_COMPRESSED-CSLC-S1_F11116_T087-185683-IW2_20230322T161649Z_20230101T161649Z_20230322T161649Z_20240504T185235Z_VV_v1.0.h5",
        )
        .unwrap();
        assert!(comp.is_compressed);
        assert_eq!(comp.frame_id, Some(11116));
        assert_eq!(comp.polarization, "VV");
    }
}
