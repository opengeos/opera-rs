//! Pure-Rust library for searching, visualizing, and analyzing NASA OPERA satellite products
//! via NASA CMR and `titiler-cmr`.
//!
//! # Features
//! - `http` (default): Live HTTPS queries to NASA CMR, `titiler-cmr`, and OpenStreetMap Nominatim
//!   using `ureq` + `rustls` (no Earthdata login required). For `wasm32` targets, compile with
//!   `--no-default-features`.
//! - `geo` (default): Pure-Rust geospatial vector/raster I/O (GeoJSON, GeoPackage, Shapefile,
//!   GeoTIFF/COG, CRS reprojection) powered by [`terrio`].
//! - `parallel` (default): Multi-threaded tile fetching and raster rendering via `rayon`.

pub mod analysis;
pub mod cmr;
pub mod error;
pub mod flood_extent;
#[cfg(feature = "geo")]
pub mod geo_io;
pub mod geocode;
pub mod products;
pub mod titiler;
pub mod types;
pub mod viz;

pub use analysis::{
    ChangeDetectionOptions, ChangeDetectionResult, ClassDelta, FailedObservation,
    ObservationSummary, SeriesTrend, TimeSeriesOptions, TimeSeriesResult, analyze_time_series,
    detect_change, fetch_time_series_candidates,
};
pub use cmr::{
    CmrSearchParams, GranuleSearchResult, OperaGranule, ResolvedAoiLayer, resolve_aoi_layer,
    resolve_concept_id, search_granules,
};
pub use error::{OperaError, Result};
pub use flood_extent::{FloodExtentOptions, FloodExtentResult, derive_flood_extent};
#[cfg(feature = "geo")]
pub use geo_io::{
    LocalRasterSummary, aoi_file_to_geojson, inspect_local_raster, read_aoi_bbox,
    write_feature_collection,
};
pub use geocode::{DEFAULT_NOMINATIM_ENDPOINT, GeocodedPlace, geocode_place};
pub use products::{
    DIST_STATUS_COLORMAP_JSON, DSWX_OPEN_WATER_CLASS, DSWX_PARTIAL_WATER_CLASS,
    DSWX_WTR_COLORMAP_JSON, DSWX_WTR_WATER_ONLY_COLORMAP_JSON, ExpressionPreset, LayerStyle,
    OPERA_PIXEL_AREA_KM2, OPERA_PIXEL_SIZE_METERS, OperaProduct, catalog, class_label,
    expression_presets, find_product, is_categorical_band, is_dswx_water_band, legend_entries,
};
pub use titiler::{
    BandStatistics, ClassBreakdown, PointAssetValue, PointResult, StatisticsResult, TileJson,
    TitilerQuery, build_point_url, build_statistics_url, build_tile_url_template,
    build_tilejson_url, build_timeseries_tilejson_url, fetch_bbox_statistics, fetch_point,
    fetch_statistics, fetch_tilejson, fetch_timeseries_tilejson, resolve_titiler_endpoint,
};
pub use types::{BBox, CalendarDate, TitilerBackend};
pub use viz::{
    CARTO_POSITRON_TEMPLATE, CARTO_VOYAGER_TEMPLATE, HtmlTileLayer, PreviewOptions,
    build_html_tile_layers, build_interactive_html, combine_side_by_side, encode_animated_gif,
    encode_png_rgba, export_interactive_html, render_animation_gif, render_change_comparison_png,
    render_preview_png,
};

/// Package version string.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default public NASA CMR search base URL.
pub const DEFAULT_CMR_ENDPOINT: &str = "https://cmr.earthdata.nasa.gov/search";

/// Default `titiler-cmr` endpoint (handles Earthdata authentication server-side).
pub const DEFAULT_TITILER_ENDPOINT: &str = "https://titiler-cmr.opengeos.org";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_version_and_default_endpoints() {
        assert!(!VERSION.is_empty());
        assert!(DEFAULT_CMR_ENDPOINT.starts_with("https://"));
        assert!(DEFAULT_TITILER_ENDPOINT.starts_with("https://"));
    }
}
