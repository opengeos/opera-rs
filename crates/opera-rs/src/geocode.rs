//! Place-name geocoding via OpenStreetMap Nominatim.
//!
//! Ported from `opengeos/geolibre-nasa-opera/src/lib/opera/geocode.ts`, preserving
//! administrative-boundary scoring, region-preference handling, and minimum AOI span expansion.

use serde::{Deserialize, Serialize};

use crate::error::{OperaError, Result};
use crate::types::{BBox, url_encode};

/// Default public Nominatim search endpoint.
pub const DEFAULT_NOMINATIM_ENDPOINT: &str = "https://nominatim.openstreetmap.org/search";

/// Minimum AOI width/height in degrees returned when geocoding small localities.
const MIN_AOI_SPAN_DEGREES: f64 = 0.1;

/// Resolved place bounding box and human-readable display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeocodedPlace {
    pub query: String,
    pub display_name: String,
    pub bbox: BBox,
}

#[derive(Debug, Deserialize)]
struct NominatimResult {
    boundingbox: Option<Vec<String>>,
    category: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    importance: Option<f64>,
    display_name: Option<String>,
}

/// Normalizes a user place query and detects whether a broad region was requested.
pub fn normalize_place_query(place: &str) -> (String, bool) {
    let lower = place.to_ascii_lowercase();
    let prefers_region = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == "region");

    let mut cleaned = String::new();
    let tokens: Vec<&str> = place.split_whitespace().collect();
    let mut i = 0usize;
    while i < tokens.len() {
        let tok = tokens[i];
        let trimmed = tok.trim_end_matches([',', ';']);
        let trailing = &tok[trimmed.len()..];
        if trimmed.eq_ignore_ascii_case("region") {
            if trailing.is_empty()
                && i + 1 < tokens.len()
                && tokens[i + 1].eq_ignore_ascii_case("of")
            {
                i += 2;
                continue;
            }
            if !trailing.is_empty() {
                cleaned.push_str(trailing);
            }
            i += 1;
            continue;
        }
        if !cleaned.is_empty() && !tok.starts_with(',') {
            cleaned.push(' ');
        }
        cleaned.push_str(tok);
        i += 1;
    }
    let cleaned = cleaned.replace(" ,", ",").trim().to_string();
    let query = if cleaned.is_empty() {
        place.trim().to_string()
    } else {
        cleaned
    };
    (query, prefers_region)
}

/// Builds the Nominatim `jsonv2` lookup URL for a query string.
pub fn build_geocode_url(endpoint: &str, query: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    format!("{base}?q={}&format=jsonv2&limit=8", url_encode(query))
}

/// Parses a Nominatim JSON array response and selects the best-scoring bounded place.
pub fn parse_nominatim_response(
    json_str: &str,
    original_place: &str,
    prefers_region: bool,
) -> Result<GeocodedPlace> {
    let items: Vec<NominatimResult> = serde_json::from_str(json_str)?;
    let mut best: Option<(f64, BBox, String)> = None;

    for item in items {
        let Some(raw) = item.boundingbox else {
            continue;
        };
        if raw.len() != 4 {
            continue;
        }
        let Ok(south) = raw[0].parse::<f64>() else {
            continue;
        };
        let Ok(north) = raw[1].parse::<f64>() else {
            continue;
        };
        let Ok(west) = raw[2].parse::<f64>() else {
            continue;
        };
        let Ok(east) = raw[3].parse::<f64>() else {
            continue;
        };
        let Ok(bbox) = BBox::new(west, south, east, north) else {
            continue;
        };

        let mut score = item.importance.filter(|v| v.is_finite()).unwrap_or(0.0);
        if item.category.as_deref() == Some("boundary")
            && item.kind.as_deref() == Some("administrative")
        {
            score += 5.0;
        }
        if prefers_region {
            let deg_area = (bbox.east - bbox.west) * (bbox.north - bbox.south);
            score += (deg_area + 1.0).log10() * 2.0;
            if deg_area < 0.01 {
                score -= 10.0;
            }
        }
        let name = item
            .display_name
            .unwrap_or_else(|| original_place.to_string());
        if best.as_ref().is_none_or(|(bscore, _, _)| score > *bscore) {
            best = Some((score, bbox, name));
        }
    }

    let (_, bbox, display_name) = best.ok_or_else(|| {
        OperaError::NoResults(format!("no map bounds found for place '{original_place}'"))
    })?;

    Ok(GeocodedPlace {
        query: original_place.trim().to_string(),
        display_name,
        bbox: bbox.expand_min_span(MIN_AOI_SPAN_DEGREES),
    })
}

/// Resolves a named place (e.g. `"Tulare Lake, California"` or `"Valencia Region, Spain"`)
/// into a WGS 84 bounding box using public OpenStreetMap Nominatim.
#[cfg(feature = "http")]
pub fn geocode_place(place: &str, endpoint: Option<&str>) -> Result<GeocodedPlace> {
    let (query, prefers_region) = normalize_place_query(place);
    if query.is_empty() {
        return Err(OperaError::InvalidInput(
            "place name cannot be empty".into(),
        ));
    }
    let url = build_geocode_url(endpoint.unwrap_or(DEFAULT_NOMINATIM_ENDPOINT), &query);
    let body = crate::cmr::http_get_text(&url)?;
    parse_nominatim_response(&body, place, prefers_region)
}

#[cfg(not(feature = "http"))]
pub fn geocode_place(_place: &str, _endpoint: Option<&str>) -> Result<GeocodedPlace> {
    Err(OperaError::HttpDisabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_region_queries_and_scores_admin_boundaries() {
        let (q, pref) = normalize_place_query("Valencia Region, Spain");
        assert_eq!(q, "Valencia, Spain");
        assert!(pref);

        let payload = r#"[
            {"boundingbox":["39.45","39.49","-0.40","-0.35"],"category":"place","type":"city","importance":0.8,"display_name":"Valencia City"},
            {"boundingbox":["37.84","40.79","-1.53","0.51"],"category":"boundary","type":"administrative","importance":0.7,"display_name":"Valencian Community, Spain"}
        ]"#;
        let place = parse_nominatim_response(payload, "Valencia Region, Spain", pref).unwrap();
        assert_eq!(place.display_name, "Valencian Community, Spain");
        assert!(place.bbox.east > 0.5);
    }

    #[test]
    fn expands_small_locality_bbox_to_minimum_span() {
        let payload = r#"[
            {"boundingbox":["39.42","39.44","-0.42","-0.40"],"category":"boundary","type":"administrative","importance":0.6,"display_name":"Paiporta, Valencia, Spain"}
        ]"#;
        let place = parse_nominatim_response(payload, "Paiporta, Valencia, Spain", false).unwrap();
        assert!((place.bbox.east - place.bbox.west) >= 0.099);
        assert!((place.bbox.north - place.bbox.south) >= 0.099);
    }
}
