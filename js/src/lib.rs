//! WebAssembly bindings for `opera-rs` (`--no-default-features`, 100% pure Rust).
//!
//! Exposes NASA OPERA product catalog lookups, CMR UMM-G parsing and footprint assembly,
//! `titiler-cmr` URL builders and categorical area statistics parsers, Sentinel-1 burst and
//! DISP-S1 filename utilities, DISP-S1 version deduplication, and missing-data subset
//! optimization for browsers and Node.js.

use opera_rs::bursts::{get_dates, group_by_burst, parse_burst_id, parse_cslc_filename};
use opera_rs::cmr::{
    assemble_search_result, build_granule_search_url, deduplicate_latest_versions,
    keep_newest_per_tile, parse_granule_page_json,
};
use opera_rs::disp::{DISP_S1_SHORT_NAME, DispProduct, UrlType, deduplicate_disp_products};
use opera_rs::missing_data::get_missing_data_options_limited;
use opera_rs::products::{
    LayerStyle, catalog, expression_presets, find_product, legend_entries, should_use_categorical,
};
use opera_rs::titiler::{
    TitilerQuery, build_point_url, build_statistics_url, build_tile_url_template,
    build_tilejson_url, parse_point_response, parse_statistics_response,
};
use opera_rs::types::{BBox, CalendarDate, TitilerBackend, url_encode};
use opera_rs::{DEFAULT_CMR_ENDPOINT, DEFAULT_TITILER_ENDPOINT, OperaError};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init_panic_hook() {
    console_error_panic_hook::set_once();
}

fn to_js_err(err: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&err.to_string())
}

fn to_json_string<T: serde::Serialize>(val: &T) -> Result<String, JsValue> {
    serde_json::to_string(val).map_err(to_js_err)
}

fn clean_opt(opt: Option<String>) -> Option<String> {
    opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn resolve_browser_endpoint(explicit: Option<String>) -> Result<String, JsValue> {
    let raw = clean_opt(explicit).unwrap_or_else(|| DEFAULT_TITILER_ENDPOINT.to_string());
    if !(raw.starts_with("https://") || raw.starts_with("http://")) {
        return Err(to_js_err(format!(
            "titiler endpoint must start with http:// or https://, got '{raw}'"
        )));
    }
    if raw.contains('?') || raw.contains('#') {
        return Err(to_js_err(format!(
            "titiler endpoint must not contain query or fragment components, got '{raw}'"
        )));
    }
    Ok(raw)
}

#[wasm_bindgen]
pub fn version() -> String {
    opera_rs::VERSION.to_string()
}

#[wasm_bindgen]
pub fn catalog_json() -> Result<String, JsValue> {
    to_json_string(&catalog())
}

#[wasm_bindgen]
pub fn find_product_json(query: &str) -> Result<String, JsValue> {
    let prod = find_product(query).map_err(to_js_err)?;
    to_json_string(&prod)
}

#[wasm_bindgen]
pub fn expression_presets_json(product: &str, band: &str) -> Result<String, JsValue> {
    let prod = find_product(product)
        .map(|p| p.short_name)
        .unwrap_or_else(|_| product.to_string());
    to_json_string(&expression_presets(&prod, band))
}

#[wasm_bindgen]
pub fn legend_entries_json(
    product: &str,
    band: Option<String>,
    water_only: bool,
) -> Result<String, JsValue> {
    let prod = find_product(product)
        .map(|p| p.short_name)
        .unwrap_or_else(|_| product.to_string());
    let b = clean_opt(band);
    let entries: Vec<Value> = legend_entries(&prod, b.as_deref(), water_only)
        .into_iter()
        .map(|(label, rgba)| {
            json!({
                "label": label,
                "rgba": rgba,
                "css": format!("rgba({}, {}, {}, {:.2})", rgba[0], rgba[1], rgba[2], f64::from(rgba[3]) / 255.0),
            })
        })
        .collect();
    to_json_string(&entries)
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn build_cmr_search_url_js(
    product: &str,
    west: f64,
    south: f64,
    east: f64,
    north: f64,
    start: Option<String>,
    end: Option<String>,
    limit: usize,
) -> Result<String, JsValue> {
    let prod = find_product(product).map_err(to_js_err)?;
    let bbox = BBox::new(west, south, east, north).map_err(to_js_err)?;
    let s = clean_opt(start);
    let e = clean_opt(end);
    build_granule_search_url(
        DEFAULT_CMR_ENDPOINT,
        &prod.short_name,
        Some(bbox),
        s.as_deref(),
        e.as_deref(),
        limit.clamp(1, 200),
        1,
    )
    .map_err(to_js_err)
}

#[wasm_bindgen]
pub fn parse_cmr_page_json(
    body: &str,
    product: &str,
    deduplicate_versions: bool,
    newest_per_tile: bool,
) -> Result<String, JsValue> {
    let prod = find_product(product)
        .map(|p| p.short_name)
        .unwrap_or_else(|_| product.to_string());
    let (hits, mut granules) = parse_granule_page_json(body, &prod).map_err(to_js_err)?;
    if deduplicate_versions {
        granules = deduplicate_latest_versions(granules);
    }
    if newest_per_tile {
        granules = keep_newest_per_tile(granules);
    }
    let result = assemble_search_result(&prod, hits, granules);
    to_json_string(&result)
}

#[allow(clippy::too_many_arguments)]
fn build_query(
    concept_id: &str,
    product: &str,
    backend: &str,
    granule_ur: Option<String>,
    temporal: Option<String>,
    band: Option<String>,
    water_only: bool,
    expression: Option<String>,
    rescale: Option<String>,
    endpoint: Option<String>,
) -> Result<(TitilerQuery, String, Option<String>, bool), JsValue> {
    let parsed_backend: TitilerBackend = backend.parse().map_err(to_js_err)?;
    let ep = resolve_browser_endpoint(endpoint)?;
    let prod = find_product(product).map_err(to_js_err)?;
    let band_clean = clean_opt(band);
    let expr_clean = clean_opt(expression);
    let rescale_clean = clean_opt(rescale);

    let chosen_bands: Vec<String> = if parsed_backend == TitilerBackend::Rasterio {
        match band_clean.clone() {
            Some(b) => vec![b],
            None => prod.default_bands.first().cloned().into_iter().collect(),
        }
    } else {
        Vec::new()
    };
    let chosen_vars: Vec<String> = if parsed_backend == TitilerBackend::Xarray {
        match band_clean.clone() {
            Some(v) => vec![v],
            None => vec!["displacement".to_string()],
        }
    } else {
        Vec::new()
    };

    let active_band = chosen_bands.first().cloned();
    let style = LayerStyle::for_request(
        &prod.short_name,
        active_band.as_deref(),
        expr_clean.as_deref(),
        water_only,
        rescale_clean.as_deref(),
        None,
    );
    let categorical = parsed_backend == TitilerBackend::Rasterio
        && should_use_categorical(
            &prod.short_name,
            active_band.as_deref(),
            expr_clean.as_deref(),
            None,
        );

    let mut q = TitilerQuery::rasterio(concept_id);
    q.endpoint = ep;
    q.backend = parsed_backend;
    q.product = Some(prod.short_name.clone());
    q.granule_ur = clean_opt(granule_ur);
    q.temporal = clean_opt(temporal);
    q.assets = chosen_bands;
    q.variables = chosen_vars;
    if parsed_backend == TitilerBackend::Xarray {
        q.group = Some("/".to_string());
    }
    q.expression = expr_clean;
    q.apply_style(&style);
    Ok((q, prod.short_name, active_band, categorical))
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn build_titiler_urls_json(
    concept_id: &str,
    product: &str,
    backend: &str,
    granule_ur: Option<String>,
    temporal: Option<String>,
    band: Option<String>,
    water_only: bool,
    expression: Option<String>,
    rescale: Option<String>,
    endpoint: Option<String>,
    tile_size: u32,
) -> Result<String, JsValue> {
    let (query, short_name, active_band, categorical) = build_query(
        concept_id, product, backend, granule_ur, temporal, band, water_only, expression, rescale,
        endpoint,
    )?;
    let size = tile_size.clamp(64, 1024);
    to_json_string(&json!({
        "product": short_name,
        "band": active_band,
        "categorical": categorical,
        "tilejson_url": build_tilejson_url(&query),
        "tile_url_template": build_tile_url_template(&query, size),
        "statistics_url": build_statistics_url(&query, categorical, None),
    }))
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn build_point_url_js(
    concept_id: &str,
    product: &str,
    backend: &str,
    granule_ur: Option<String>,
    temporal: Option<String>,
    band: Option<String>,
    expression: Option<String>,
    lon: f64,
    lat: f64,
    endpoint: Option<String>,
) -> Result<String, JsValue> {
    if !lon.is_finite()
        || !lat.is_finite()
        || !(-180.0..=180.0).contains(&lon)
        || !(-90.0..=90.0).contains(&lat)
    {
        return Err(to_js_err(OperaError::InvalidInput(format!(
            "invalid coordinates ({lon}, {lat})"
        ))));
    }
    let (query, _, _, _) = build_query(
        concept_id, product, backend, granule_ur, temporal, band, false, expression, None, endpoint,
    )?;
    Ok(build_point_url(&query, lon, lat))
}

#[wasm_bindgen]
pub fn parse_point_response_json(
    body: &str,
    product: &str,
    band: Option<String>,
) -> Result<String, JsValue> {
    let prod = find_product(product)
        .map(|p| p.short_name)
        .unwrap_or_else(|_| product.to_string());
    let b = clean_opt(band);
    let parsed = parse_point_response(body, Some(&prod), b.as_deref()).map_err(to_js_err)?;
    to_json_string(&parsed)
}

#[wasm_bindgen]
pub fn parse_statistics_response_json(
    body: &str,
    product: &str,
    band: Option<String>,
    categorical: bool,
) -> Result<String, JsValue> {
    let prod = find_product(product)
        .map(|p| p.short_name)
        .unwrap_or_else(|_| product.to_string());
    let b = clean_opt(band);
    let parsed = parse_statistics_response(body, Some(&prod), b.as_deref(), categorical)
        .map_err(to_js_err)?;
    to_json_string(&parsed)
}

#[wasm_bindgen]
pub fn normalize_burst_id_js(input: &str) -> Result<String, JsValue> {
    parse_burst_id(input)
        .map(|p| p.normalized)
        .map_err(to_js_err)
}

#[wasm_bindgen]
pub fn parse_burst_id_json(input: &str) -> Result<String, JsValue> {
    let parsed = parse_burst_id(input).map_err(to_js_err)?;
    to_json_string(&parsed)
}

#[wasm_bindgen]
pub fn parse_cslc_filename_json(input: &str) -> Result<String, JsValue> {
    let parsed = parse_cslc_filename(input).map_err(to_js_err)?;
    to_json_string(&parsed)
}

#[wasm_bindgen]
pub fn group_by_burst_json(files_json: &str) -> Result<String, JsValue> {
    let files: Vec<String> = serde_json::from_str(files_json).map_err(to_js_err)?;
    let grouped = group_by_burst(&files).map_err(to_js_err)?;
    to_json_string(&grouped)
}

#[wasm_bindgen]
pub fn get_dates_json(input: &str) -> Result<String, JsValue> {
    let dates: Vec<String> = get_dates(input)
        .into_iter()
        .map(|d| d.to_string())
        .collect();
    to_json_string(&dates)
}

#[wasm_bindgen]
pub fn parse_disp_filename_json(input: &str) -> Result<String, JsValue> {
    let parsed = DispProduct::from_filename(input).map_err(to_js_err)?;
    to_json_string(&parsed)
}

#[wasm_bindgen]
pub fn deduplicate_disp_filenames_json(files_json: &str) -> Result<String, JsValue> {
    let files: Vec<String> = serde_json::from_str(files_json).map_err(to_js_err)?;
    let mut products = Vec::with_capacity(files.len());
    for f in &files {
        products.push(DispProduct::from_filename(f).map_err(to_js_err)?);
    }
    let before = products.len();
    let deduped = deduplicate_disp_products(products);
    let removed = before.saturating_sub(deduped.len());
    to_json_string(&json!({
        "products": deduped,
        "input_count": before,
        "duplicates_removed": removed,
    }))
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn build_disp_search_url_js(
    frame_id: Option<u32>,
    west: Option<f64>,
    south: Option<f64>,
    east: Option<f64>,
    north: Option<f64>,
    start: Option<String>,
    end: Option<String>,
    product_version: Option<String>,
    limit: usize,
) -> Result<String, JsValue> {
    let bbox = match (west, south, east, north) {
        (Some(w), Some(s), Some(e), Some(n)) => Some(BBox::new(w, s, e, n).map_err(to_js_err)?),
        (None, None, None, None) => None,
        _ => {
            return Err(to_js_err(
                "specify all four bbox coordinates (west, south, east, north) or none",
            ));
        }
    };
    if frame_id.is_none() && bbox.is_none() {
        return Err(to_js_err("specify frame_id or bbox for DISP-S1 search"));
    }
    let base = DEFAULT_CMR_ENDPOINT.trim_end_matches('/');
    let mut url = format!(
        "{base}/granules.umm_json?short_name={}&page_size={}&page_num=1&sort_key=-start_date",
        url_encode(DISP_S1_SHORT_NAME),
        limit.clamp(1, 200)
    );
    if let Some(fid) = frame_id {
        url.push_str(&format!(
            "&attribute[]={}",
            url_encode(&format!("int,FRAME_NUMBER,{fid}"))
        ));
    }
    if let Some(ver) = clean_opt(product_version) {
        url.push_str(&format!(
            "&attribute[]={}",
            url_encode(&format!("string,PRODUCT_VERSION,{ver}"))
        ));
    }
    if let Some(b) = bbox {
        url.push_str(&format!(
            "&bounding_box={}",
            url_encode(&b.to_query_string())
        ));
    }
    match (clean_opt(start), clean_opt(end)) {
        (Some(s), Some(e)) => {
            let sd = CalendarDate::parse(&s).map_err(to_js_err)?;
            let ed = CalendarDate::parse(&e).map_err(to_js_err)?;
            if sd > ed {
                return Err(to_js_err(format!(
                    "start date {sd} must be on or before end date {ed}"
                )));
            }
            url.push_str(&format!(
                "&temporal={}",
                url_encode(&format!("{sd}T00:00:00Z,{ed}T23:59:59Z"))
            ));
        }
        (Some(s), None) => {
            let sd = CalendarDate::parse(&s).map_err(to_js_err)?;
            url.push_str(&format!(
                "&temporal={}",
                url_encode(&format!("{sd}T00:00:00Z,"))
            ));
        }
        (None, Some(e)) => {
            let ed = CalendarDate::parse(&e).map_err(to_js_err)?;
            url.push_str(&format!(
                "&temporal={}",
                url_encode(&format!(",{ed}T23:59:59Z"))
            ));
        }
        (None, None) => {}
    }
    Ok(url)
}

#[wasm_bindgen]
pub fn parse_disp_cmr_page_json(
    body: &str,
    url_type: &str,
    deduplicate: bool,
) -> Result<String, JsValue> {
    let parsed_url_type: UrlType = url_type.parse().map_err(to_js_err)?;
    let root: Value = serde_json::from_str(body).map_err(to_js_err)?;
    let hits = root.get("hits").and_then(Value::as_u64).unwrap_or(0);
    let items = root
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| to_js_err("CMR response missing 'items' array"))?;
    let mut products = Vec::new();
    for item in items {
        if let Ok(prod) = DispProduct::from_umm(item, parsed_url_type) {
            products.push(prod);
        }
    }
    let before = products.len();
    let final_products = if deduplicate {
        deduplicate_disp_products(products)
    } else {
        products
    };
    let duplicates_removed = before.saturating_sub(final_products.len());
    let mut frame_ids: Vec<u32> = final_products.iter().map(|p| p.frame_id).collect();
    frame_ids.sort_unstable();
    frame_ids.dedup();
    to_json_string(&json!({
        "total_hits": hits,
        "duplicates_removed": duplicates_removed,
        "frame_ids": frame_ids,
        "products": final_products,
    }))
}

#[wasm_bindgen]
pub fn get_missing_data_options_json(
    files_json: &str,
    max_options: Option<usize>,
) -> Result<String, JsValue> {
    let files: Vec<String> = serde_json::from_str(files_json).map_err(to_js_err)?;
    let opts = get_missing_data_options_limited(&files, max_options).map_err(to_js_err)?;
    to_json_string(&opts)
}
