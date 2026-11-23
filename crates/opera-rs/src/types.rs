//! Shared geospatial and temporal primitives (`BBox`, `Date`, `TitilerBackend`, URL encoding).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{OperaError, Result};

/// Maximum Web Mercator latitude (EPSG:3857 projection limit).
pub const MAX_MERCATOR_LAT: f64 = 85.051_128_779_8;

/// WGS 84 bounding box `[west, south, east, north]` in decimal degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub west: f64,
    pub south: f64,
    pub east: f64,
    pub north: f64,
}

impl BBox {
    /// Creates and validates a WGS 84 bounding box (`west < east`, `south < north`).
    pub fn new(west: f64, south: f64, east: f64, north: f64) -> Result<Self> {
        if ![west, south, east, north].iter().all(|v| v.is_finite()) {
            return Err(OperaError::InvalidInput(
                "bbox coordinates must be finite numbers".into(),
            ));
        }
        if !(-180.0..=180.0).contains(&west)
            || !(-180.0..=180.0).contains(&east)
            || !(-90.0..=90.0).contains(&south)
            || !(-90.0..=90.0).contains(&north)
        {
            return Err(OperaError::InvalidInput(format!(
                "bbox coordinates out of WGS 84 range: [{west}, {south}, {east}, {north}]"
            )));
        }
        if west >= east || south >= north {
            return Err(OperaError::InvalidInput(format!(
                "bbox must satisfy west < east and south < north, got [{west}, {south}, {east}, {north}]"
            )));
        }
        Ok(Self {
            west,
            south,
            east,
            north,
        })
    }

    /// Creates a validated `BBox` from `[west, south, east, north]`.
    pub fn from_array([w, s, e, n]: [f64; 4]) -> Result<Self> {
        Self::new(w, s, e, n)
    }

    /// Returns `[west, south, east, north]`.
    pub fn to_array(self) -> [f64; 4] {
        [self.west, self.south, self.east, self.north]
    }

    /// Clamps latitude to Web Mercator bounds (`[-85.0511287798, 85.0511287798]`).
    pub fn clamp_mercator(self) -> Result<Self> {
        let w = self.west.clamp(-180.0, 180.0);
        let e = self.east.clamp(-180.0, 180.0);
        let s = self.south.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
        let n = self.north.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
        Self::new(w, s, e, n)
    }

    /// Center point `(lon, lat)`.
    pub fn center(self) -> (f64, f64) {
        (
            0.5 * (self.west + self.east),
            0.5 * (self.south + self.north),
        )
    }

    /// Approximate area in square kilometers using an equirectangular projection at mean latitude.
    pub fn area_km2(self) -> f64 {
        let mean_lat_rad = (0.5 * (self.south + self.north)).to_radians();
        let width_m = (self.east - self.west) * 111_320.0 * mean_lat_rad.cos().abs();
        let height_m = (self.north - self.south) * 111_320.0;
        (width_m * height_m) / 1_000_000.0
    }

    /// Converts the bounding box into a GeoJSON `Feature` with a closed `Polygon` geometry.
    pub fn to_geojson_feature(self) -> Value {
        json!({
            "type": "Feature",
            "geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [self.west, self.south],
                    [self.east, self.south],
                    [self.east, self.north],
                    [self.west, self.north],
                    [self.west, self.south]
                ]]
            },
            "properties": {
                "bbox": self.to_array()
            }
        })
    }

    /// Expands small boxes symmetrically so both spans are at least `min_span_deg`,
    /// preserving valid WGS 84 coordinate ordering (`west < east`, `south < north`)
    /// and shifting clipped padding to the opposite endpoint near world limits.
    pub fn expand_min_span(self, min_span_deg: f64) -> Self {
        fn expand_axis(
            lo: f64,
            hi: f64,
            min_span: f64,
            bound_lo: f64,
            bound_hi: f64,
        ) -> (f64, f64) {
            let target = min_span.min(bound_hi - bound_lo);
            let span = hi - lo;
            if span >= target {
                return (lo, hi);
            }
            let pad = (target - span) * 0.5;
            let mut new_lo = lo - pad;
            let mut new_hi = hi + pad;
            if new_lo < bound_lo {
                new_hi = (new_hi + (bound_lo - new_lo)).min(bound_hi);
                new_lo = bound_lo;
            }
            if new_hi > bound_hi {
                new_lo = (new_lo - (new_hi - bound_hi)).max(bound_lo);
                new_hi = bound_hi;
            }
            (new_lo, new_hi)
        }
        let min_span = min_span_deg.max(0.0);
        let (west, east) = expand_axis(self.west, self.east, min_span, -180.0, 180.0);
        let (south, north) = expand_axis(self.south, self.north, min_span, -90.0, 90.0);
        Self {
            west,
            south,
            east,
            north,
        }
    }

    /// Returns the union of two bounding boxes.
    pub fn union(self, other: Self) -> Self {
        Self {
            west: self.west.min(other.west),
            south: self.south.min(other.south),
            east: self.east.max(other.east),
            north: self.north.max(other.north),
        }
    }

    /// Formats compactly as `"west,south,east,north"` rounded to 6 decimal places.
    pub fn to_query_string(self) -> String {
        format!(
            "{},{},{},{}",
            fmt_coord(self.west),
            fmt_coord(self.south),
            fmt_coord(self.east),
            fmt_coord(self.north),
        )
    }
}

impl fmt::Display for BBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_query_string())
    }
}

impl FromStr for BBox {
    type Err = OperaError;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
        let parts: Vec<&str> = trimmed
            .split([',', ' '])
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() != 4 {
            return Err(OperaError::InvalidInput(format!(
                "expected 4 bbox numbers 'west,south,east,north', got '{s}'"
            )));
        }
        let nums: std::result::Result<Vec<f64>, _> =
            parts.iter().map(|p| p.parse::<f64>()).collect();
        let nums = nums.map_err(|_| {
            OperaError::InvalidInput(format!("invalid numeric coordinate in bbox '{s}'"))
        })?;
        Self::new(nums[0], nums[1], nums[2], nums[3])
    }
}

fn fmt_coord(x: f64) -> String {
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Reader backend used by `titiler-cmr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitilerBackend {
    /// Cloud-Optimized GeoTIFF / GeoTIFF backend (used by OPERA DSWx, DIST, RTC-S1).
    #[default]
    Rasterio,
    /// NetCDF / HDF5 / Zarr backend (for multidimensional products such as DISP-S1 or CSLC-S1).
    Xarray,
}

impl TitilerBackend {
    /// Path segment used in `titiler-cmr` URLs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rasterio => "rasterio",
            Self::Xarray => "xarray",
        }
    }
}

impl fmt::Display for TitilerBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TitilerBackend {
    type Err = OperaError;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rasterio" => Ok(Self::Rasterio),
            "xarray" => Ok(Self::Xarray),
            other => Err(OperaError::InvalidInput(format!(
                "unknown titiler-cmr backend '{other}': expected 'rasterio' or 'xarray'"
            ))),
        }
    }
}

/// Calendar date `(year, month, day)` with proleptic Gregorian day arithmetic (no external date crate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CalendarDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl CalendarDate {
    /// Creates a validated calendar date.
    pub fn new(year: i32, month: u32, day: u32) -> Result<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return Err(OperaError::InvalidInput(format!(
                "invalid calendar date: {year:04}-{month:02}-{day:02}"
            )));
        }
        Ok(Self { year, month, day })
    }

    /// Parses `YYYY-MM-DD` (ignoring any trailing `T...` timestamp suffix).
    pub fn parse(s: &str) -> Result<Self> {
        let prefix = s.trim().get(..10).ok_or_else(|| {
            OperaError::InvalidInput(format!("invalid date '{s}': expected YYYY-MM-DD"))
        })?;
        let mut parts = prefix.split('-');
        let (Some(y), Some(m), Some(d), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(OperaError::InvalidInput(format!(
                "invalid date '{s}': expected YYYY-MM-DD"
            )));
        };
        let year: i32 = y
            .parse()
            .map_err(|_| OperaError::InvalidInput(format!("invalid year in date '{s}'")))?;
        let month: u32 = m
            .parse()
            .map_err(|_| OperaError::InvalidInput(format!("invalid month in date '{s}'")))?;
        let day: u32 = d
            .parse()
            .map_err(|_| OperaError::InvalidInput(format!("invalid day in date '{s}'")))?;
        Self::new(year, month, day)
    }

    /// Converts the date to days since 1970-01-01 (Howard Hinnant's civil-to-days algorithm).
    pub fn days_since_epoch(self) -> i64 {
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = (y - era * 400) as u64;
        let m = u64::from(self.month);
        let d = u64::from(self.day);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe as i64 - 719_468
    }

    /// Converts days since 1970-01-01 back to a calendar date (Howard Hinnant's algorithm).
    pub fn from_days_since_epoch(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = (y + i64::from(m <= 2)) as i32;
        Self {
            year,
            month: m as u32,
            day: d as u32,
        }
    }

    /// Shifts this date by `delta_days` (positive or negative).
    pub fn add_days(self, delta_days: i64) -> Self {
        Self::from_days_since_epoch(self.days_since_epoch() + delta_days)
    }

    /// Signed day difference `self - other`.
    pub fn days_between(self, other: Self) -> i64 {
        self.days_since_epoch() - other.days_since_epoch()
    }

    /// Formats as `YYYY-MM-DDT00:00:00Z`.
    pub fn start_of_day_rfc3339(self) -> String {
        format!("{self}T00:00:00Z")
    }

    /// Formats as `YYYY-MM-DDT23:59:59.999Z` to cover the full end day including fractional seconds.
    pub fn end_of_day_rfc3339(self) -> String {
        format!("{self}T23:59:59.999Z")
    }
}

impl fmt::Display for CalendarDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Number of days in a Gregorian `(year, month)`.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 31,
    }
}

/// Percent-encodes a URL query component (RFC 3986 unreserved characters kept verbatim).
pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Escapes regex metacharacters for `titiler-cmr`'s `assets_regex` query parameter.
pub fn regex_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if "\\.^$|?*+()[]{}".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_bbox() {
        let b: BBox = "-121.8,38.4,-121.2,38.8".parse().unwrap();
        assert_eq!(b.to_array(), [-121.8, 38.4, -121.2, 38.8]);
        assert!(b.area_km2() > 2000.0 && b.area_km2() < 3000.0);
        assert_eq!(b.to_geojson_feature()["geometry"]["type"], "Polygon");
        assert!("10,20,5,30".parse::<BBox>().is_err());
        assert!("-200,0,0,10".parse::<BBox>().is_err());
        let arctic = BBox::new(0.0, 86.0, 1.0, 87.0)
            .unwrap()
            .expand_min_span(0.1);
        assert!(arctic.south < arctic.north);
        assert_eq!(arctic.north, 87.0);
    }

    #[test]
    fn calendar_date_roundtrips_leap_years() {
        let d = CalendarDate::parse("2024-02-28").unwrap();
        assert_eq!(d.add_days(1).to_string(), "2024-02-29");
        assert_eq!(d.add_days(2).to_string(), "2024-03-01");
        let non_leap = CalendarDate::parse("2023-02-28").unwrap();
        assert_eq!(non_leap.add_days(1).to_string(), "2023-03-01");
        assert!(CalendarDate::parse("2023-02-29").is_err());
        assert_eq!(
            CalendarDate::parse("2024-10-27T12:34:56Z").unwrap(),
            CalendarDate::new(2024, 10, 27).unwrap()
        );
        assert_eq!(
            CalendarDate::parse("2024-11-05")
                .unwrap()
                .days_between(CalendarDate::parse("2024-10-27").unwrap()),
            9
        );
    }

    #[test]
    fn encodes_urls_and_regex() {
        assert_eq!(url_encode("a,b c/d"), "a%2Cb%20c%2Fd");
        assert_eq!(regex_escape("B01_WTR.tif"), "B01_WTR\\.tif");
    }
}
