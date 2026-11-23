//! PyO3 Python extension module for `opera-rs`.

use std::ffi::OsString;
use std::path::PathBuf;

use opera_rs::OperaError;
use opera_rs::analysis::{
    ChangeDetectionOptions, TimeSeriesOptions, analyze_time_series, detect_change,
};
use opera_rs::cmr::{CmrSearchParams, resolve_aoi_layer, resolve_concept_id, search_granules};
use opera_rs::flood_extent::{FloodExtentOptions, derive_flood_extent};
use opera_rs::geo_io::{inspect_local_raster, read_aoi_bbox, write_feature_collection};
use opera_rs::geocode::geocode_place;
use opera_rs::products::{
    LayerStyle, catalog, expression_presets, find_product, should_use_categorical,
};
use opera_rs::titiler::{
    TitilerQuery, build_tile_url_template, build_tilejson_url, fetch_bbox_statistics, fetch_point,
    fetch_tilejson, resolve_titiler_endpoint,
};
use opera_rs::types::{BBox, TitilerBackend};
use opera_rs::viz::{PreviewOptions, render_preview_png};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

fn to_py_err(err: OperaError) -> PyErr {
    match err {
        OperaError::InvalidInput(msg) => PyValueError::new_err(msg),
        other => PyRuntimeError::new_err(other.to_string()),
    }
}

fn to_json_string<T: serde::Serialize>(val: &T) -> PyResult<String> {
    serde_json::to_string(val).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

fn parse_bbox_opt(bbox: Option<[f64; 4]>) -> PyResult<Option<BBox>> {
    bbox.map(BBox::from_array).transpose().map_err(to_py_err)
}

#[pyfunction]
fn version() -> &'static str {
    opera_rs::VERSION
}

#[pyfunction]
#[pyo3(signature = (args, self_cmd=None))]
fn cli_main(args: Vec<String>, self_cmd: Option<Vec<String>>) -> i32 {
    let self_os = self_cmd.map(|v| v.into_iter().map(OsString::from).collect());
    opera_cli::run_with_self_command(args, self_os)
}

#[pyfunction]
fn catalog_json() -> PyResult<String> {
    to_json_string(&catalog())
}

#[pyfunction]
fn find_product_json(query: &str) -> PyResult<String> {
    let prod = find_product(query).map_err(to_py_err)?;
    to_json_string(&prod)
}

#[pyfunction]
fn expression_presets_json(product: &str, band: &str) -> PyResult<String> {
    to_json_string(&expression_presets(product, band))
}

#[pyfunction]
#[pyo3(signature = (place, endpoint=None))]
fn geocode_json(py: Python<'_>, place: &str, endpoint: Option<&str>) -> PyResult<String> {
    let place = place.to_string();
    let endpoint = endpoint.map(ToString::to_string);
    let res = py
        .detach(|| geocode_place(&place, endpoint.as_deref()))
        .map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (product="OPERA_L3_DSWX-HLS_V1", bbox=None, start=None, end=None, limit=50, deduplicate_versions=true, newest_per_tile=false, cmr_endpoint=None))]
fn search_json(
    py: Python<'_>,
    product: &str,
    bbox: Option<[f64; 4]>,
    start: Option<String>,
    end: Option<String>,
    limit: usize,
    deduplicate_versions: bool,
    newest_per_tile: bool,
    cmr_endpoint: Option<String>,
) -> PyResult<String> {
    let params = CmrSearchParams {
        product: product.to_string(),
        bbox: parse_bbox_opt(bbox)?,
        start,
        end,
        limit,
        deduplicate_versions,
        newest_per_tile,
        cmr_endpoint,
    };
    let res = py.detach(|| search_granules(&params)).map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (concept_id, product=None, backend="rasterio", granule_ur=None, temporal=None, bands=None, rescale=None, colormap=None, water_only=false, expression=None, endpoint=None, tile_size=512))]
fn tile_urls_json(
    concept_id: &str,
    product: Option<&str>,
    backend: &str,
    granule_ur: Option<String>,
    temporal: Option<String>,
    bands: Option<Vec<String>>,
    rescale: Option<&str>,
    colormap: Option<&str>,
    water_only: bool,
    expression: Option<String>,
    endpoint: Option<&str>,
    tile_size: u32,
) -> PyResult<String> {
    if water_only && product.is_none() {
        return Err(PyValueError::new_err("water_only requires product"));
    }
    let parsed_backend: TitilerBackend = backend.parse().map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint).map_err(to_py_err)?;
    let chosen_bands = bands.unwrap_or_default();
    let style = product
        .map(|p| {
            LayerStyle::for_request(
                p,
                chosen_bands.first().map(String::as_str),
                expression.as_deref(),
                water_only,
                rescale,
                colormap,
            )
        })
        .unwrap_or(LayerStyle {
            colormap: None,
            colormap_name: colormap.map(ToString::to_string),
            rescale: rescale.map(ToString::to_string),
        });
    let mut query = TitilerQuery::rasterio(concept_id);
    query.endpoint = ep;
    query.backend = parsed_backend;
    query.product = product.map(ToString::to_string);
    query.granule_ur = granule_ur;
    query.temporal = temporal;
    query.assets = chosen_bands;
    query.expression = expression;
    query.apply_style(&style);
    to_json_string(&serde_json::json!({
        "tilejson_url": build_tilejson_url(&query),
        "tile_url_template": build_tile_url_template(&query, tile_size),
    }))
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (product, lon, lat, concept_id=None, granule_ur=None, temporal=None, band=None, expression=None, endpoint=None))]
fn point_json(
    py: Python<'_>,
    product: &str,
    lon: f64,
    lat: f64,
    concept_id: Option<String>,
    granule_ur: Option<String>,
    temporal: Option<String>,
    band: Option<String>,
    expression: Option<String>,
    endpoint: Option<String>,
) -> PyResult<String> {
    let prod = find_product(product).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let res = py
        .detach(|| {
            let cid = match concept_id {
                Some(c) => c,
                None => resolve_concept_id(&prod.short_name, None)?,
            };
            let mut q = TitilerQuery::rasterio(&cid);
            q.endpoint = ep;
            q.backend = prod.backend;
            q.product = Some(prod.short_name.clone());
            q.granule_ur = granule_ur;
            q.temporal = temporal;
            if let Some(b) = band.or_else(|| prod.default_bands.first().cloned()) {
                if prod.backend == TitilerBackend::Xarray {
                    q.variables = vec![b];
                } else {
                    q.assets = vec![b];
                }
            }
            q.expression = expression;
            fetch_point(&q, lon, lat)
        })
        .map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (product, bbox, concept_id=None, granule_ur=None, start=None, end=None, band=None, expression=None, categorical=None, histogram_bins=None, endpoint=None))]
fn stats_json(
    py: Python<'_>,
    product: &str,
    bbox: [f64; 4],
    concept_id: Option<String>,
    granule_ur: Option<String>,
    start: Option<String>,
    end: Option<String>,
    band: Option<String>,
    expression: Option<String>,
    categorical: Option<bool>,
    histogram_bins: Option<u32>,
    endpoint: Option<String>,
) -> PyResult<String> {
    let parsed_bbox = BBox::from_array(bbox).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let product = product.to_string();
    let res = py
        .detach(|| {
            let (prod, cid, ur, chosen_band) = if let Some(g) = granule_ur {
                let prod = find_product(&product)?;
                let c = match concept_id {
                    Some(c) => c,
                    None => resolve_concept_id(&prod.short_name, None)?,
                };
                let b = band
                    .or_else(|| prod.default_bands.first().cloned())
                    .ok_or_else(|| OperaError::InvalidInput("specify band".into()))?;
                (prod, c, Some(g), b)
            } else {
                let resolved = resolve_aoi_layer(
                    &product,
                    parsed_bbox,
                    start,
                    end,
                    1,
                    band,
                    concept_id,
                    None,
                )?;
                let g_id = resolved.granules.first().map(|g| g.id.clone());
                (resolved.product, resolved.concept_id, g_id, resolved.band)
            };

            let is_cat = should_use_categorical(
                &prod.short_name,
                Some(&chosen_band),
                expression.as_deref(),
                categorical,
            );
            let mut q = TitilerQuery::rasterio(&cid);
            q.endpoint = ep;
            q.product = Some(prod.short_name);
            q.granule_ur = ur;
            q.assets = vec![chosen_band];
            q.expression = expression;
            fetch_bbox_statistics(&q, parsed_bbox, is_cat, histogram_bins)
        })
        .map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (product, bbox, before_date, after_date, window_days=10, band=None, expression=None, endpoint=None))]
fn detect_change_json(
    py: Python<'_>,
    product: &str,
    bbox: [f64; 4],
    before_date: &str,
    after_date: &str,
    window_days: i64,
    band: Option<String>,
    expression: Option<String>,
    endpoint: Option<String>,
) -> PyResult<String> {
    let parsed_bbox = BBox::from_array(bbox).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let opts = ChangeDetectionOptions {
        product: product.to_string(),
        bbox: parsed_bbox,
        aoi_geojson: None,
        before_date: before_date.to_string(),
        after_date: after_date.to_string(),
        window_days,
        band,
        expression,
        titiler_endpoint: Some(ep),
        cmr_endpoint: None,
    };
    let res = py.detach(|| detect_change(&opts)).map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (product, bbox, start, end, interval_days=None, max_observations=12, band=None, expression=None, endpoint=None))]
fn analyze_time_series_json(
    py: Python<'_>,
    product: &str,
    bbox: [f64; 4],
    start: &str,
    end: &str,
    interval_days: Option<i64>,
    max_observations: usize,
    band: Option<String>,
    expression: Option<String>,
    endpoint: Option<String>,
) -> PyResult<String> {
    let parsed_bbox = BBox::from_array(bbox).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let opts = TimeSeriesOptions {
        product: product.to_string(),
        bbox: parsed_bbox,
        aoi_geojson: None,
        start: start.to_string(),
        end: end.to_string(),
        interval_days,
        max_observations,
        band,
        expression,
        titiler_endpoint: Some(ep),
        cmr_endpoint: None,
    };
    let res = py
        .detach(|| analyze_time_series(&opts))
        .map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (bbox, start, end, products=None, max_granules=6, max_tiles_across=4, min_area_km2=0.02, endpoint=None))]
fn derive_flood_extent_json(
    py: Python<'_>,
    bbox: [f64; 4],
    start: &str,
    end: &str,
    products: Option<Vec<String>>,
    max_granules: usize,
    max_tiles_across: usize,
    min_area_km2: f64,
    endpoint: Option<String>,
) -> PyResult<String> {
    let parsed_bbox = BBox::from_array(bbox).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let mut opts = FloodExtentOptions::new(parsed_bbox, start, end);
    if let Some(p) = products.filter(|v| !v.is_empty()) {
        opts.products = p;
    }
    opts.max_granules_per_product = max_granules;
    opts.max_tiles_across = max_tiles_across;
    opts.min_area_km2 = min_area_km2;
    opts.titiler_endpoint = Some(ep);
    let res = py
        .detach(|| derive_flood_extent(&opts))
        .map_err(to_py_err)?;
    to_json_string(&res)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (product, bbox, start=None, end=None, band=None, water_only=false, max_dim=1024, basemap=true, title=None, output_path=None, endpoint=None))]
fn preview_png<'py>(
    py: Python<'py>,
    product: &str,
    bbox: [f64; 4],
    start: Option<String>,
    end: Option<String>,
    band: Option<String>,
    water_only: bool,
    max_dim: usize,
    basemap: bool,
    title: Option<String>,
    output_path: Option<String>,
    endpoint: Option<String>,
) -> PyResult<Bound<'py, PyBytes>> {
    let parsed_bbox = BBox::from_array(bbox).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let product = product.to_string();
    let bytes = py
        .detach(|| -> opera_rs::Result<Vec<u8>> {
            let resolved =
                resolve_aoi_layer(&product, parsed_bbox, start, end, 3, band, None, None)?;
            let prod = resolved.product;
            let cid = resolved.concept_id;
            let chosen_band = resolved.band;
            let style =
                LayerStyle::for_band(&prod.short_name, Some(&chosen_band), water_only, None, None);
            let queries: Vec<TitilerQuery> = resolved
                .granules
                .iter()
                .map(|g| {
                    TitilerQuery::for_granule_band(
                        &ep,
                        &cid,
                        &prod.short_name,
                        &g.id,
                        &chosen_band,
                        None,
                        Some(&style),
                    )
                })
                .collect::<opera_rs::Result<Vec<_>>>()?;
            let mut opts = PreviewOptions::new(parsed_bbox, &prod.short_name, Some(chosen_band));
            opts.water_only = water_only;
            opts.max_dim = max_dim;
            opts.basemap = basemap;
            opts.legend = style.colormap.is_some();
            opts.title = title;
            let bytes = render_preview_png(&queries, &opts)?;
            if let Some(p) = output_path {
                let path = PathBuf::from(p);
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, &bytes)?;
            }
            Ok(bytes)
        })
        .map_err(to_py_err)?;
    Ok(PyBytes::new(py, &bytes))
}

#[pyfunction]
#[pyo3(signature = (product, concept_id=None, granule_ur=None, band=None, water_only=false, endpoint=None))]
fn fetch_tilejson_json(
    py: Python<'_>,
    product: &str,
    concept_id: Option<String>,
    granule_ur: Option<String>,
    band: Option<String>,
    water_only: bool,
    endpoint: Option<String>,
) -> PyResult<String> {
    let prod = find_product(product).map_err(to_py_err)?;
    let ep = resolve_titiler_endpoint(endpoint.as_deref()).map_err(to_py_err)?;
    let tj = py
        .detach(|| {
            let cid = match concept_id {
                Some(c) => c,
                None => resolve_concept_id(&prod.short_name, None)?,
            };
            let chosen_band = band.or_else(|| prod.default_bands.first().cloned());
            let style = LayerStyle::for_band(
                &prod.short_name,
                chosen_band.as_deref(),
                water_only,
                None,
                None,
            );
            let mut q = TitilerQuery::rasterio(&cid);
            q.endpoint = ep;
            q.product = Some(prod.short_name);
            q.granule_ur = granule_ur;
            if let Some(b) = chosen_band {
                q.assets = vec![b];
            }
            q.apply_style(&style);
            fetch_tilejson(&q)
        })
        .map_err(to_py_err)?;
    to_json_string(&tj)
}

#[pyfunction]
fn read_aoi_bbox_py(py: Python<'_>, path: &str) -> PyResult<[f64; 4]> {
    let path = path.to_string();
    py.detach(|| read_aoi_bbox(&path).map(|b| b.to_array()))
        .map_err(to_py_err)
}

#[pyfunction]
#[pyo3(signature = (feature_collection_json, out_path, layer_name="opera"))]
fn write_vector_py(
    py: Python<'_>,
    feature_collection_json: &str,
    out_path: &str,
    layer_name: &str,
) -> PyResult<usize> {
    let fc: serde_json::Value = serde_json::from_str(feature_collection_json)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    let out_path = out_path.to_string();
    let layer_name = layer_name.to_string();
    py.detach(|| write_feature_collection(&fc, &out_path, &layer_name))
        .map_err(to_py_err)
}

#[pyfunction]
#[pyo3(signature = (path, product=None, band=None))]
fn inspect_raster_json(
    py: Python<'_>,
    path: &str,
    product: Option<&str>,
    band: Option<&str>,
) -> PyResult<String> {
    let path = path.to_string();
    let product = product.map(ToString::to_string);
    let band = band.map(ToString::to_string);
    let res = py
        .detach(|| inspect_local_raster(&path, product.as_deref(), band.as_deref()))
        .map_err(to_py_err)?;
    to_json_string(&res)
}

#[pymodule]
fn _opera_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", opera_rs::VERSION)?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(cli_main, m)?)?;
    m.add_function(wrap_pyfunction!(catalog_json, m)?)?;
    m.add_function(wrap_pyfunction!(find_product_json, m)?)?;
    m.add_function(wrap_pyfunction!(expression_presets_json, m)?)?;
    m.add_function(wrap_pyfunction!(geocode_json, m)?)?;
    m.add_function(wrap_pyfunction!(search_json, m)?)?;
    m.add_function(wrap_pyfunction!(tile_urls_json, m)?)?;
    m.add_function(wrap_pyfunction!(fetch_tilejson_json, m)?)?;
    m.add_function(wrap_pyfunction!(point_json, m)?)?;
    m.add_function(wrap_pyfunction!(stats_json, m)?)?;
    m.add_function(wrap_pyfunction!(detect_change_json, m)?)?;
    m.add_function(wrap_pyfunction!(analyze_time_series_json, m)?)?;
    m.add_function(wrap_pyfunction!(derive_flood_extent_json, m)?)?;
    m.add_function(wrap_pyfunction!(preview_png, m)?)?;
    m.add_function(wrap_pyfunction!(read_aoi_bbox_py, m)?)?;
    m.add_function(wrap_pyfunction!(write_vector_py, m)?)?;
    m.add_function(wrap_pyfunction!(inspect_raster_json, m)?)?;
    Ok(())
}
