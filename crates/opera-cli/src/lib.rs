//! Command-line interface and Model Context Protocol (MCP) server for `opera-rs`.

pub mod mcp;
mod usage;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use opera_rs::analysis::{
    ChangeDetectionOptions, TimeSeriesOptions, analyze_time_series, detect_change,
    fetch_time_series_candidates,
};
use opera_rs::burst_db::{
    FrameBBox, OrbitPass, frame_ids_from_geojson, get_frame_bbox, get_frame_geojson,
    get_frame_ids_for_burst, get_frame_orbit_pass, get_frame_to_burst_mapping,
    get_intersecting_frames, get_intersecting_frames_for_point, reproject_bbox_to_wgs84,
};
use opera_rs::bursts::{get_dates, group_by_burst, parse_burst_id};
use opera_rs::cmr::{CmrSearchParams, resolve_aoi_layer, resolve_concept_id, search_granules};
use opera_rs::disp::{DispSearchParams, UrlType, search_disp_products};
use opera_rs::flood_extent::{FloodExtentOptions, derive_flood_extent};
use opera_rs::geo_io::{inspect_local_raster, read_aoi_bbox, write_feature_collection};
use opera_rs::geocode::geocode_place;
use opera_rs::missing_data::get_missing_data_options_limited;
use opera_rs::products::{
    LayerStyle, catalog, expression_presets, find_product, should_use_categorical,
};
use opera_rs::titiler::{
    TitilerQuery, build_tile_url_template, build_tilejson_url, build_timeseries_tilejson_url,
    fetch_bbox_statistics, fetch_point, fetch_tilejson, fetch_timeseries_tilejson,
    resolve_titiler_endpoint,
};
use opera_rs::types::{BBox, TitilerBackend};
use opera_rs::viz::{
    PreviewOptions, build_html_tile_layers, export_interactive_html, render_animation_gif,
    render_change_comparison_png, render_preview_png,
};
use opera_rs::{OperaError, Result};
use serde_json::{Value, json};

/// Command-line interface for NASA OPERA search, visualization, analysis, and MCP serving.
#[derive(Debug, Parser)]
#[command(
    name = "opera",
    version,
    about = "Search, visualize, and analyze NASA OPERA satellite data (CMR, titiler-cmr, MCP)",
    subcommand_required = false
)]
pub struct Cli {
    /// Print the machine-readable JSON command/argument catalog and exit.
    #[arg(long, global = true)]
    pub json_usage: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Shared AOI selector flags accepted by spatial commands (`--bbox`, `--place`, or `--aoi-file`).
#[derive(Debug, Clone, Args)]
#[group(multiple = false)]
pub struct AoiArgs {
    /// WGS 84 bounding box as 'west,south,east,north'.
    #[arg(long, value_name = "W,S,E,N", allow_hyphen_values = true)]
    pub bbox: Option<String>,

    /// Named place to geocode via OpenStreetMap Nominatim (e.g. 'Tulare Lake, California').
    #[arg(long)]
    pub place: Option<String>,

    /// Path to a GeoJSON, GeoPackage (.gpkg), or Shapefile (.shp) AOI (auto-reprojected to WGS 84).
    #[arg(long, value_name = "FILE")]
    pub aoi_file: Option<PathBuf>,
}

impl AoiArgs {
    fn resolve(&self) -> Result<Option<(BBox, Option<String>)>> {
        if let Some(raw) = &self.bbox {
            return Ok(Some((raw.parse::<BBox>()?, None)));
        }
        if let Some(path) = &self.aoi_file {
            return Ok(Some((read_aoi_bbox(path)?, None)));
        }
        if let Some(place) = &self.place {
            let geo = geocode_place(place, None)?;
            return Ok(Some((geo.bbox, Some(geo.display_name))));
        }
        Ok(None)
    }

    fn resolve_required(&self) -> Result<(BBox, Option<String>)> {
        self.resolve()?.ok_or_else(|| {
            OperaError::InvalidInput(
                "specify an area of interest with --bbox, --place, or --aoi-file".into(),
            )
        })
    }
}

/// Shared `titiler-cmr` endpoint resolver (delegates to [`opera_rs::titiler::resolve_titiler_endpoint`]).
fn resolve_endpoint(explicit: Option<&str>) -> Result<String> {
    resolve_titiler_endpoint(explicit)
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// List supported NASA OPERA products, aliases, band defaults, and expression presets.
    Products {
        /// Optional product short name or alias to inspect in detail.
        product: Option<String>,
        /// Optional band token to show ready-made band-math expression presets.
        #[arg(long)]
        band: Option<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Resolve a place name (e.g. 'Valencia Region, Spain') to a WGS 84 bounding box.
    Geocode {
        /// Place name to search via OpenStreetMap Nominatim.
        place: String,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Search NASA CMR for OPERA granules, footprints, dates, and available bands.
    Search {
        /// OPERA product short name or alias (e.g. dswx-hls, dswx-s1, dist-alert, rtc-s1, disp-s1).
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Inclusive start date (YYYY-MM-DD).
        #[arg(long)]
        start: Option<String>,
        /// Inclusive end date (YYYY-MM-DD).
        #[arg(long)]
        end: Option<String>,
        /// Maximum number of granules to return (1-500).
        #[arg(long, default_value_t = 25)]
        limit: usize,
        /// Keep only the newest granule per tile/burst id.
        #[arg(long)]
        newest_per_tile: bool,
        /// Keep all reprocessed versions instead of deduplicating to the latest production run.
        #[arg(long)]
        all_versions: bool,
        /// Write granule footprints to a GeoJSON, GeoPackage (.gpkg), or Shapefile (.shp).
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Build or fetch a titiler-cmr TileJSON and XYZ tile URL template (rasterio or xarray).
    Tilejson {
        /// OPERA product short name or alias (used to resolve collection concept-id and default styles).
        #[arg(short, long)]
        product: Option<String>,
        /// Explicit CMR collection concept-id (overrides product lookup).
        #[arg(long)]
        concept_id: Option<String>,
        /// Reader backend: rasterio (COG/GeoTIFF) or xarray (NetCDF/HDF5/Zarr).
        #[arg(long, default_value = "rasterio")]
        backend: String,
        /// Pin to a single CMR GranuleUR.
        #[arg(long)]
        granule: Option<String>,
        /// Temporal filter (RFC3339 instant or start/end range).
        #[arg(long)]
        temporal: Option<String>,
        /// Rasterio band/asset name(s) (repeatable).
        #[arg(long = "band")]
        bands: Vec<String>,
        /// Xarray variable name(s) (repeatable).
        #[arg(long = "variable")]
        variables: Vec<String>,
        /// Xarray group path (e.g. /science/grids).
        #[arg(long)]
        group: Option<String>,
        /// Rescale stretch 'min,max'.
        #[arg(long, allow_hyphen_values = true)]
        rescale: Option<String>,
        /// Named titiler colormap (e.g. gray, terrain, viridis, blues).
        #[arg(long)]
        colormap: Option<String>,
        /// DSWx WTR water-only colormap (hides cloud, snow, and ocean).
        #[arg(long)]
        water_only: bool,
        /// rio-tiler band-math expression (selected band is b1).
        #[arg(long)]
        expression: Option<String>,
        /// Optional ISO-8601 duration step (e.g. P1M) to query the timeseries TileJSON endpoint.
        #[arg(long)]
        timeseries_step: Option<String>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Only construct and print the TileJSON / XYZ URLs without making an HTTP request.
        #[arg(long)]
        url_only: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Query raster pixel or xarray variable values at a longitude/latitude point via titiler-cmr.
    Point {
        /// Longitude (EPSG:4326).
        #[arg(long, allow_hyphen_values = true)]
        lon: f64,
        /// Latitude (EPSG:4326).
        #[arg(long, allow_hyphen_values = true)]
        lat: f64,
        /// OPERA product short name or alias.
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        /// Explicit CMR collection concept-id (optional; auto-resolved if omitted).
        #[arg(long)]
        concept_id: Option<String>,
        /// Pin to a single CMR GranuleUR.
        #[arg(long)]
        granule: Option<String>,
        /// Temporal filter (RFC3339 instant or start/end).
        #[arg(long)]
        temporal: Option<String>,
        /// Band/asset token (e.g. B01_WTR, VV, VEG-DIST-STATUS).
        #[arg(long)]
        band: Option<String>,
        /// rio-tiler band-math expression (selected band is b1).
        #[arg(long)]
        expression: Option<String>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Compute zonal AOI statistics (including 30 m categorical class area breakdowns in km^2).
    Stats {
        /// OPERA product short name or alias.
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Explicit CMR collection concept-id (optional).
        #[arg(long)]
        concept_id: Option<String>,
        /// Pin to a single CMR GranuleUR.
        #[arg(long)]
        granule: Option<String>,
        /// Inclusive start date (YYYY-MM-DD) when granule is omitted.
        #[arg(long)]
        start: Option<String>,
        /// Inclusive end date (YYYY-MM-DD) when granule is omitted.
        #[arg(long)]
        end: Option<String>,
        /// Band/asset token (defaults to product preferred band).
        #[arg(long)]
        band: Option<String>,
        /// rio-tiler band-math expression (selected band is b1).
        #[arg(long)]
        expression: Option<String>,
        /// Request categorical histogram (auto-enabled for DSWx WTR and DIST STATUS).
        #[arg(long)]
        categorical: bool,
        /// Histogram bin count for continuous bands.
        #[arg(long)]
        histogram_bins: Option<u32>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Detect two-date OPERA surface change (before vs. after stats, class area deltas, optional preview).
    Change {
        /// OPERA product short name or alias (e.g. dswx-hls, dswx-s1, dist-alert, rtc-s1).
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Baseline date (YYYY-MM-DD).
        #[arg(long)]
        before: String,
        /// Comparison date (YYYY-MM-DD).
        #[arg(long)]
        after: String,
        /// Search window in days on each side of before/after dates.
        #[arg(long, default_value_t = 10)]
        window_days: i64,
        /// Band/asset token (defaults to product preferred band).
        #[arg(long)]
        band: Option<String>,
        /// rio-tiler band-math expression (selected band is b1).
        #[arg(long)]
        expression: Option<String>,
        /// Write a side-by-side Before | After comparison PNG to this path.
        #[arg(long, value_name = "FILE")]
        preview_out: Option<PathBuf>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Output a Markdown report instead of plain summary text.
        #[arg(long)]
        markdown: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Analyze multi-date OPERA surface water or backscatter time-series trends over an AOI.
    Timeseries {
        /// OPERA product short name or alias.
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Inclusive start date (YYYY-MM-DD).
        #[arg(long)]
        start: String,
        /// Inclusive end date (YYYY-MM-DD).
        #[arg(long)]
        end: String,
        /// Minimum spacing in days between sampled observations.
        #[arg(long)]
        interval_days: Option<i64>,
        /// Maximum number of observations to evaluate (1-100).
        #[arg(long, default_value_t = 12)]
        max_observations: usize,
        /// Band/asset token (defaults to product preferred band).
        #[arg(long)]
        band: Option<String>,
        /// rio-tiler band-math expression.
        #[arg(long)]
        expression: Option<String>,
        /// Write time-series table to a CSV file.
        #[arg(long, value_name = "FILE")]
        csv_out: Option<PathBuf>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Vectorize observed flood-water extent polygons from OPERA DSWx-HLS and/or DSWx-S1 tiles.
    FloodExtent {
        #[command(flatten)]
        aoi: AoiArgs,
        /// Inclusive event window start (YYYY-MM-DD).
        #[arg(long)]
        start: String,
        /// Inclusive event window end (YYYY-MM-DD).
        #[arg(long)]
        end: String,
        /// DSWx products to union (defaults to both dswx-hls and cloud-penetrating dswx-s1).
        #[arg(long = "product")]
        products: Vec<String>,
        /// Maximum granules per product to mosaic.
        #[arg(long, default_value_t = 6)]
        max_granules: usize,
        /// Maximum tiles across the longer AOI dimension (controls zoom level).
        #[arg(long, default_value_t = 4)]
        max_tiles_across: usize,
        /// Minimum polygon area in km^2 to retain.
        #[arg(long, default_value_t = 0.02)]
        min_area_km2: f64,
        /// Write derived flood polygons to GeoJSON, GeoPackage (.gpkg), or Shapefile (.shp).
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Emit structured JSON output (includes GeoJSON FeatureCollection when -o is omitted).
        #[arg(long)]
        json: bool,
    },

    /// Render a PNG map preview of an OPERA layer over an AOI (with basemap, scale bar, legend).
    Preview {
        /// OPERA product short name or alias.
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Pin to one or more CMR GranuleURs (if omitted, searches CMR for newest matching scene).
        #[arg(long = "granule")]
        granules: Vec<String>,
        /// Inclusive start date (YYYY-MM-DD) when granule is omitted.
        #[arg(long)]
        start: Option<String>,
        /// Inclusive end date (YYYY-MM-DD) when granule is omitted.
        #[arg(long)]
        end: Option<String>,
        /// Maximum granules to mosaic when granule is omitted.
        #[arg(long, default_value_t = 3)]
        max_granules: usize,
        /// Band/asset token (defaults to product preferred band).
        #[arg(long)]
        band: Option<String>,
        /// Rescale stretch 'min,max'.
        #[arg(long, allow_hyphen_values = true)]
        rescale: Option<String>,
        /// Named titiler colormap.
        #[arg(long)]
        colormap: Option<String>,
        /// DSWx WTR water-only colormap (hides clouds, snow, and ocean).
        #[arg(long)]
        water_only: bool,
        /// rio-tiler band-math expression (selected band is b1).
        #[arg(long)]
        expression: Option<String>,
        /// Maximum image width or height in pixels.
        #[arg(long, default_value_t = 1024)]
        max_dim: usize,
        /// Maximum tiles across the longer AOI dimension.
        #[arg(long, default_value_t = 4)]
        max_tiles_across: usize,
        /// Disable CARTO Voyager basemap compositing.
        #[arg(long)]
        no_basemap: bool,
        /// Disable metric scale bar overlay.
        #[arg(long)]
        no_scale_bar: bool,
        /// Disable categorical legend overlay.
        #[arg(long)]
        no_legend: bool,
        /// Optional title banner drawn on the preview.
        #[arg(long)]
        title: Option<String>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Output PNG path (required on CLI; omit in `opera mcp` to receive inline image content).
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Render an animated time-series GIF across OPERA acquisition dates for an AOI.
    Animate {
        /// OPERA product short name or alias.
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Inclusive start date (YYYY-MM-DD).
        #[arg(long)]
        start: String,
        /// Inclusive end date (YYYY-MM-DD).
        #[arg(long)]
        end: String,
        /// Maximum number of frames (dates) to render.
        #[arg(long, default_value_t = 8)]
        max_frames: usize,
        /// Minimum spacing in days between frames.
        #[arg(long)]
        interval_days: Option<i64>,
        /// Frame delay in centiseconds (minimum 10 = 0.10 s/frame; default 75 = 0.75 s/frame).
        #[arg(long, default_value_t = 75, value_parser = clap::value_parser!(u16).range(10..))]
        delay_cs: u16,
        /// Band/asset token.
        #[arg(long)]
        band: Option<String>,
        /// DSWx WTR water-only colormap.
        #[arg(long)]
        water_only: bool,
        /// Maximum frame width or height in pixels.
        #[arg(long, default_value_t = 640)]
        max_dim: usize,
        /// Disable CARTO Voyager basemap compositing.
        #[arg(long)]
        no_basemap: bool,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Output GIF path.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Export a single-file interactive MapLibre GL JS HTML viewer (requires network access for CDN & tiles).
    ExportHtml {
        /// OPERA product short name or alias.
        #[arg(short, long, default_value = "OPERA_L3_DSWX-HLS_V1")]
        product: String,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Inclusive start date (YYYY-MM-DD).
        #[arg(long)]
        start: Option<String>,
        /// Inclusive end date (YYYY-MM-DD).
        #[arg(long)]
        end: Option<String>,
        /// Maximum granules to embed as interactive tile layers.
        #[arg(long, default_value_t = 4)]
        max_granules: usize,
        /// Band/asset token.
        #[arg(long)]
        band: Option<String>,
        /// DSWx WTR water-only colormap.
        #[arg(long)]
        water_only: bool,
        /// Map page title.
        #[arg(long)]
        title: Option<String>,
        /// Custom titiler-cmr endpoint URL.
        #[arg(long)]
        endpoint: Option<String>,
        /// Output HTML file path.
        #[arg(short, long, value_name = "FILE")]
        output: PathBuf,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Inspect a local OPERA GeoTIFF/COG on disk via pure-Rust terrio (CRS, stats, class areas).
    Inspect {
        /// Path to a local GeoTIFF/COG file.
        path: PathBuf,
        /// Optional OPERA product hint (e.g. dswx-hls, dist-alert) to decode class labels & areas.
        #[arg(short, long)]
        product: Option<String>,
        /// Optional band hint (e.g. B01_WTR, VEG-DIST-STATUS).
        #[arg(long)]
        band: Option<String>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Parse, normalize, or group Sentinel-1 burst IDs and dates from filenames or granule URs (opera-utils).
    BurstId {
        /// One or more burst IDs, filenames, granule URs, or URLs.
        #[arg(required = true)]
        inputs: Vec<String>,
        /// Group inputs by normalized burst ID.
        #[arg(long)]
        group: bool,
        /// Also extract acquisition dates (YYYY-MM-DD) from each filename stem.
        #[arg(long)]
        dates: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Look up DISP-S1 frame metadata, UTM/WGS 84 bounds, orbit pass, and burst IDs from opera-adt/burst_db.
    Frame {
        /// Numeric DISP-S1 frame ID (e.g. 11114), or a Sentinel-1 burst ID with --from-burst.
        id: String,
        /// Treat ID as a Sentinel-1 burst ID (e.g. T042-088905-IW1) and return its DISP-S1 frame IDs.
        #[arg(long)]
        from_burst: bool,
        /// Report frame bounds in WGS 84 latitude/longitude (EPSG:4326) instead of native UTM.
        #[arg(long)]
        latlon: bool,
        /// Print only the 4-element bounding box array [xmin, ymin, xmax, ymax].
        #[arg(long)]
        bounds_only: bool,
        /// Print only constituent normalized burst IDs (one per line or JSON array).
        #[arg(long)]
        burst_ids_only: bool,
        /// Optional path to a local frame-to-burst (or burst-to-frame with --from-burst) JSON/ZIP file.
        #[arg(long, value_name = "FILE")]
        db_file: Option<PathBuf>,
        /// Optional path to a local frame-geometries-simple GeoJSON/ZIP file (used for orbit pass and -o export).
        #[arg(long, value_name = "FILE")]
        geom_file: Option<PathBuf>,
        /// Optional output file (.geojson, .gpkg, .shp) to export the frame geometry.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Find DISP-S1 frames whose simplified WGS 84 geometry intersects an AOI or point (opera-utils).
    FramesIntersect {
        #[command(flatten)]
        aoi: AoiArgs,
        /// Optional single point longitude (EPSG:4326), paired with --lat.
        #[arg(long, allow_hyphen_values = true, requires = "lat")]
        lon: Option<f64>,
        /// Optional single point latitude (EPSG:4326), paired with --lon.
        #[arg(long, allow_hyphen_values = true, requires = "lon")]
        lat: Option<f64>,
        /// Print only matching integer frame IDs (one per line or JSON array).
        #[arg(long)]
        ids_only: bool,
        /// Optional path to a local frame-geometries-simple GeoJSON/ZIP file.
        #[arg(long, value_name = "FILE")]
        db_file: Option<PathBuf>,
        /// Write matching frame geometries to GeoJSON, GeoPackage (.gpkg), or Shapefile (.shp).
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Compute optimal Sentinel-1 burst/date subsets for missing CSLC acquisitions (opera-utils).
    MissingData {
        /// CSLC filenames, granule URs, or URLs (may also be supplied via --file).
        inputs: Vec<String>,
        /// Read newline-separated CSLC filenames/URLs from a text file.
        #[arg(short, long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Maximum number of ranked subset options to return.
        #[arg(long, default_value_t = 5)]
        max_options: usize,
        /// Write each ranked option's selected file list to disk.
        #[arg(long)]
        write_options: bool,
        /// Filename prefix used with --write-options.
        #[arg(long, default_value = "option_")]
        output_prefix: String,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Search NASA CMR for OPERA L3 DISP-S1 surface displacement products by frame ID or AOI (opera-utils).
    DispSearch {
        /// Optional DISP-S1 frame ID (e.g. 11115, 20697).
        #[arg(long)]
        frame_id: Option<u32>,
        #[command(flatten)]
        aoi: AoiArgs,
        /// Inclusive secondary-acquisition start date (YYYY-MM-DD).
        #[arg(long)]
        start: Option<String>,
        /// Inclusive secondary-acquisition end date (YYYY-MM-DD).
        #[arg(long)]
        end: Option<String>,
        /// Product version filter (default: 1.0; pass empty string for all versions).
        #[arg(long, default_value = "1.0")]
        product_version: String,
        /// Preferred download URL protocol: https or s3.
        #[arg(long, default_value = "https")]
        url_type: String,
        /// Keep superseded duplicate (reference, secondary) pairs instead of deduplicating.
        #[arg(long)]
        all_versions: bool,
        /// Maximum number of CMR items to fetch (1-2000).
        #[arg(long, default_value_t = 500)]
        limit: usize,
        /// Print only download URLs (one per line).
        #[arg(long)]
        urls_only: bool,
        /// Emit structured JSON output.
        #[arg(long)]
        json: bool,
    },

    /// Start the Model Context Protocol (MCP) server over stdin/stdout for AI agents.
    Mcp,
}

/// Runs the CLI with the given arguments and returns the process exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with_self_command(args, None)
}

/// Runs the CLI, optionally overriding the child command line used by `opera mcp` to spawn tools.
///
/// The standalone Rust binary passes `None` (re-invoking `std::env::current_exe()`). Language
/// wrappers (such as the Python wheel) can pass e.g. `[sys.executable, "-P", "-m", "opera_rs"]`
/// so MCP tool calls invoke the bundled extension without needing a separate native binary on `PATH`.
pub fn run_with_self_command<I, T>(args: I, self_cmd: Option<Vec<OsString>>) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let os_args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if let Some(usage_res) = usage::requested(&os_args) {
        return match usage_res {
            Ok(doc) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&doc).expect("serialize --json-usage")
                );
                0
            }
            Err(msg) => {
                eprintln!("error: {msg}");
                2
            }
        };
    }

    let parsed = usage::with_clap_stack(move || Cli::try_parse_from(os_args));
    let cli = match parsed {
        Ok(c) => c,
        Err(err) => {
            let code = err.exit_code();
            let _ = err.print();
            return code;
        }
    };

    let Some(command) = cli.command else {
        let mut cmd = usage::with_clap_stack(<Cli as clap::CommandFactory>::command);
        let _ = cmd.print_help();
        println!();
        return 0;
    };

    if matches!(command, Commands::Mcp) {
        let cmdline = match self_cmd {
            Some(c) if !c.is_empty() => c,
            _ => match mcp::this_binary() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: could not determine opera executable path: {e}");
                    return 1;
                }
            },
        };
        return match mcp::serve(cmdline) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("error: MCP server failed: {e}");
                1
            }
        };
    }

    match dispatch(command) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn print_json(val: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(val)?);
    Ok(())
}

fn write_output(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

fn require_band(band: Option<String>) -> Result<String> {
    band.ok_or_else(|| OperaError::InvalidInput("specify --band".into()))
}

fn dispatch(command: Commands) -> Result<()> {
    match command {
        Commands::Products {
            product,
            band,
            json: as_json,
        } => {
            if let Some(name) = product {
                let prod = find_product(&name)?;
                let presets = band
                    .as_deref()
                    .or_else(|| prod.default_bands.first().map(String::as_str))
                    .map(|b| expression_presets(&prod.short_name, b))
                    .unwrap_or_default();
                if as_json {
                    print_json(&json!({
                        "product": prod,
                        "expression_presets": presets,
                    }))?;
                } else {
                    println!("{} ({})", prod.short_name, prod.short_title);
                    println!("  Title:       {}", prod.title);
                    println!("  Description: {}", prod.description);
                    println!("  Backend:     {}", prod.backend);
                    println!("  Bands:       {}", prod.default_bands.join(", "));
                    if !presets.is_empty() {
                        println!("  Expressions:");
                        for p in presets {
                            println!("    - {}: {}", p.label, p.expression);
                        }
                    }
                }
            } else {
                let all = catalog();
                if as_json {
                    print_json(&json!({"products": all}))?;
                } else {
                    for p in all {
                        println!(
                            "{:<28} {:<14} [{}] {}",
                            p.short_name, p.short_title, p.backend, p.description
                        );
                    }
                }
            }
            Ok(())
        }

        Commands::Geocode {
            place,
            json: as_json,
        } => {
            let res = geocode_place(&place, None)?;
            if as_json {
                print_json(&serde_json::to_value(&res)?)?;
            } else {
                println!("{}", res.display_name);
                println!("bbox: {} ({:.2} km^2)", res.bbox, res.bbox.area_km2());
            }
            Ok(())
        }

        Commands::Search {
            product,
            aoi,
            start,
            end,
            limit,
            newest_per_tile,
            all_versions,
            output,
            json: as_json,
        } => {
            let resolved_aoi = aoi.resolve()?;
            let bbox = resolved_aoi.as_ref().map(|(b, _)| *b);
            let res = search_granules(&CmrSearchParams {
                product,
                bbox,
                start,
                end,
                limit,
                deduplicate_versions: !all_versions,
                newest_per_tile,
                cmr_endpoint: None,
            })?;
            let written = if let Some(path) = &output {
                Some((
                    path.display().to_string(),
                    write_feature_collection(&res.feature_collection, path, "opera_footprints")?,
                ))
            } else {
                None
            };
            if as_json {
                let mut v = serde_json::to_value(&res)?;
                if let Some((p, count)) = written {
                    // Omit bulky GeoJSON inline when saved to disk so agent context stays compact.
                    v["written_footprints"] = json!({"path": p, "features": count});
                    if let Some(obj) = v.as_object_mut() {
                        obj.remove("feature_collection");
                    }
                }
                if let Some((_, Some(place_name))) = resolved_aoi {
                    v["geocoded_place"] = json!(place_name);
                }
                print_json(&v)?;
            } else {
                println!(
                    "{}: {} returned ({} total CMR hits, {} dates)",
                    res.product,
                    res.granules.len(),
                    res.hits,
                    res.dates.len()
                );
                if let Some(cid) = &res.concept_id {
                    println!("collection concept_id: {cid}");
                }
                for g in &res.granules {
                    println!(
                        "  {} | {} {} | tile={} | bands=[{}]",
                        g.id,
                        g.date,
                        g.time,
                        g.tile,
                        g.bands.join(",")
                    );
                }
                if let Some((p, count)) = written {
                    println!("Wrote {count} footprints to {p}");
                }
            }
            Ok(())
        }

        Commands::Tilejson {
            product,
            concept_id,
            backend,
            granule,
            temporal,
            bands,
            variables,
            group,
            rescale,
            colormap,
            water_only,
            expression,
            timeseries_step,
            endpoint,
            url_only,
            json: as_json,
        } => {
            let backend_parsed: TitilerBackend = backend.parse()?;
            let prod_info = product.as_deref().map(find_product).transpose()?;
            let resolved_cid = match concept_id {
                Some(cid) => cid,
                None => {
                    let prod_name = prod_info
                        .as_ref()
                        .map(|p| p.short_name.as_str())
                        .or(product.as_deref())
                        .ok_or_else(|| {
                            OperaError::InvalidInput(
                                "provide either --product or --concept-id".into(),
                            )
                        })?;
                    resolve_concept_id(prod_name, None)?
                }
            };
            let prod_name = prod_info.as_ref().map(|p| p.short_name.clone()).or(product);
            let chosen_bands = if bands.is_empty() && backend_parsed == TitilerBackend::Rasterio {
                prod_info
                    .as_ref()
                    .and_then(|p| p.default_bands.first().cloned())
                    .into_iter()
                    .collect()
            } else {
                bands
            };
            let style = prod_name
                .as_deref()
                .map(|p| {
                    LayerStyle::for_request(
                        p,
                        chosen_bands.first().map(String::as_str),
                        expression.as_deref(),
                        water_only,
                        rescale.as_deref(),
                        colormap.as_deref(),
                    )
                })
                .unwrap_or(LayerStyle {
                    colormap: None,
                    colormap_name: colormap,
                    rescale,
                });

            let mut query = TitilerQuery::rasterio(&resolved_cid);
            query.endpoint = resolve_endpoint(endpoint.as_deref())?;
            query.backend = backend_parsed;
            query.product = prod_name;
            query.granule_ur = granule;
            query.temporal = temporal;
            query.assets = chosen_bands;
            query.variables = variables;
            query.group = group;
            query.expression = expression;
            query.apply_style(&style);

            let tile_template = build_tile_url_template(&query, 512);
            if url_only {
                let tj_url = match timeseries_step.as_deref() {
                    Some(step) => build_timeseries_tilejson_url(&query, Some(step), None),
                    None => build_tilejson_url(&query),
                };
                if as_json {
                    print_json(&json!({
                        "concept_id": resolved_cid,
                        "tilejson_url": tj_url,
                        "tile_url_template": tile_template,
                    }))?;
                } else {
                    println!("TileJSON URL: {tj_url}");
                    println!("XYZ Template: {tile_template}");
                }
                return Ok(());
            }

            if let Some(step) = timeseries_step {
                let ts = fetch_timeseries_tilejson(&query, Some(&step), None)?;
                if as_json {
                    print_json(&serde_json::to_value(&ts)?)?;
                } else {
                    println!("Returned {} timeseries TileJSON entries", ts.len());
                    for (k, v) in ts {
                        println!(
                            "  {k}: {}",
                            v.tiles.first().map(String::as_str).unwrap_or("")
                        );
                    }
                }
            } else {
                let tj = fetch_tilejson(&query)?;
                if as_json {
                    print_json(&json!({
                        "concept_id": resolved_cid,
                        "tilejson": tj,
                        "tile_url_template": tile_template,
                    }))?;
                } else {
                    println!("concept_id: {resolved_cid}");
                    for t in &tj.tiles {
                        println!("tile: {t}");
                    }
                }
            }
            Ok(())
        }

        Commands::Point {
            lon,
            lat,
            product,
            concept_id,
            granule,
            temporal,
            band,
            expression,
            endpoint,
            json: as_json,
        } => {
            let prod = find_product(&product)?;
            let cid = match concept_id {
                Some(c) => c,
                None => resolve_concept_id(&prod.short_name, None)?,
            };
            let chosen_band = band.or_else(|| prod.default_bands.first().cloned());
            let mut query = TitilerQuery::rasterio(&cid);
            query.endpoint = resolve_endpoint(endpoint.as_deref())?;
            query.backend = prod.backend;
            query.product = Some(prod.short_name.clone());
            query.granule_ur = granule;
            query.temporal = temporal;
            if let Some(b) = chosen_band {
                if prod.backend == TitilerBackend::Xarray {
                    query.variables = vec![b];
                } else {
                    query.assets = vec![b];
                }
            }
            query.expression = expression;

            let res = fetch_point(&query, lon, lat)?;
            if as_json {
                print_json(&serde_json::to_value(&res)?)?;
            } else {
                println!(
                    "Point ({:.6}, {:.6}):",
                    res.coordinates[0], res.coordinates[1]
                );
                for asset in &res.assets {
                    println!("  {}", asset.name);
                    for (idx, val) in asset.values.iter().enumerate() {
                        let fallback_name = format!("band_{}", idx + 1);
                        let bname = asset
                            .band_names
                            .get(idx)
                            .map(String::as_str)
                            .unwrap_or(&fallback_name);
                        let label = asset.class_labels.get(idx).and_then(Option::as_deref);
                        let v_str = val
                            .map(|v| format!("{v}"))
                            .unwrap_or_else(|| "nodata".into());
                        match label {
                            Some(lbl) => println!("    {bname}: {v_str} ({lbl})"),
                            None => println!("    {bname}: {v_str}"),
                        }
                    }
                }
            }
            Ok(())
        }

        Commands::Stats {
            product,
            aoi,
            concept_id,
            granule,
            start,
            end,
            band,
            expression,
            categorical,
            histogram_bins,
            endpoint,
            json: as_json,
        } => {
            let (bbox, place_name) = aoi.resolve_required()?;
            let (prod, cid, resolved_granule, chosen_band) = if let Some(g) = granule {
                let prod = find_product(&product)?;
                let c = match concept_id {
                    Some(cid) => cid,
                    None => resolve_concept_id(&prod.short_name, None)?,
                };
                let b = require_band(band.or_else(|| prod.default_bands.first().cloned()))?;
                (prod, c, Some(g), b)
            } else {
                let resolved =
                    resolve_aoi_layer(&product, bbox, start, end, 1, band, concept_id, None)?;
                let g_id = resolved.granules.first().map(|g| g.id.clone());
                (resolved.product, resolved.concept_id, g_id, resolved.band)
            };

            let is_cat = should_use_categorical(
                &prod.short_name,
                Some(&chosen_band),
                expression.as_deref(),
                categorical.then_some(true),
            );
            let mut query = TitilerQuery::rasterio(&cid);
            query.endpoint = resolve_endpoint(endpoint.as_deref())?;
            query.product = Some(prod.short_name.clone());
            query.granule_ur = resolved_granule.clone();
            query.assets = vec![chosen_band.clone()];
            query.expression = expression;

            let stats = fetch_bbox_statistics(&query, bbox, is_cat, histogram_bins)?;
            if as_json {
                print_json(&json!({
                    "product": prod.short_name,
                    "concept_id": cid,
                    "granule": resolved_granule,
                    "band": chosen_band,
                    "bbox": bbox,
                    "geocoded_place": place_name,
                    "categorical": is_cat,
                    "statistics": stats,
                }))?;
            } else {
                println!(
                    "{} ({}) band={} bbox={}",
                    prod.short_name,
                    resolved_granule.as_deref().unwrap_or("mosaic"),
                    chosen_band,
                    bbox
                );
                for (bname, b) in &stats.bands {
                    let valid_str = b
                        .valid_percent
                        .map(|v| format!("{v:.1}%"))
                        .unwrap_or_else(|| "n/a".into());
                    println!(
                        "  {bname}: min={:?} max={:?} mean={:?} valid={valid_str}",
                        b.min, b.max, b.mean
                    );
                    for cls in &b.class_breakdown {
                        println!(
                            "    class {:>3} ({:<24}): {:>9.0} px | {:>5.1}% | {:>8.3} km^2",
                            cls.class_value,
                            cls.label.as_deref().unwrap_or("-"),
                            cls.pixel_count,
                            cls.percent,
                            cls.area_km2
                        );
                    }
                }
            }
            Ok(())
        }

        Commands::Change {
            product,
            aoi,
            before,
            after,
            window_days,
            band,
            expression,
            preview_out,
            endpoint,
            markdown,
            json: as_json,
        } => {
            let (bbox, _) = aoi.resolve_required()?;
            let ep = resolve_endpoint(endpoint.as_deref())?;
            let res = detect_change(&ChangeDetectionOptions {
                product,
                bbox,
                aoi_geojson: None,
                before_date: before,
                after_date: after,
                window_days,
                band,
                expression: expression.clone(),
                titiler_endpoint: Some(ep.clone()),
                cmr_endpoint: None,
            })?;

            if let Some(preview_path) = &preview_out {
                let bytes = render_change_comparison_png(&res, bbox, &ep, expression.as_deref())?;
                write_output(preview_path, bytes)?;
            }

            if as_json {
                print_json(&serde_json::to_value(&res)?)?;
            } else if markdown {
                print!("{}", res.to_markdown());
            } else {
                println!(
                    "{} ({}) change: {} -> {}",
                    res.product, res.band, res.before.acquired_date, res.after.acquired_date
                );
                if let (Some(b), Some(a), Some(d)) = (
                    res.before.open_water_km2,
                    res.after.open_water_km2,
                    res.open_water_delta_km2,
                ) {
                    println!("  Open water:  {b:.3} -> {a:.3} km^2 (delta {d:+.3} km^2)");
                }
                if let (Some(b), Some(a), Some(d)) = (
                    res.before.total_water_km2,
                    res.after.total_water_km2,
                    res.total_water_delta_km2,
                ) {
                    println!("  Total water: {b:.3} -> {a:.3} km^2 (delta {d:+.3} km^2)");
                }
                if let Some(dm) = res.mean_delta {
                    println!("  Band mean delta: {dm:+.4}");
                }
                for w in &res.warnings {
                    println!("  Warning: {w}");
                }
                if let Some(p) = preview_out {
                    println!("  Saved comparison preview to {}", p.display());
                }
            }
            Ok(())
        }

        Commands::Timeseries {
            product,
            aoi,
            start,
            end,
            interval_days,
            max_observations,
            band,
            expression,
            csv_out,
            endpoint,
            json: as_json,
        } => {
            let (bbox, _) = aoi.resolve_required()?;
            let res = analyze_time_series(&TimeSeriesOptions {
                product,
                bbox,
                aoi_geojson: None,
                start,
                end,
                interval_days,
                max_observations,
                band,
                expression,
                titiler_endpoint: Some(resolve_endpoint(endpoint.as_deref())?),
                cmr_endpoint: None,
            })?;
            if let Some(csv_path) = &csv_out {
                write_output(csv_path, res.to_csv())?;
            }
            if as_json {
                print_json(&serde_json::to_value(&res)?)?;
            } else {
                println!(
                    "{} ({}) time series: {} observations ({} to {})",
                    res.product,
                    res.band,
                    res.observations.len(),
                    res.start,
                    res.end
                );
                for obs in &res.observations {
                    if let Some(tw) = obs.total_water_km2 {
                        let open_str = obs
                            .open_water_km2
                            .map(|v| format!("{v:.3} km^2"))
                            .unwrap_or_else(|| "n/a".into());
                        println!(
                            "  {} | open={open_str} | total={tw:.3} km^2 | {}",
                            obs.acquired_date, obs.granule_id
                        );
                    } else {
                        let mean_str = obs
                            .stats
                            .mean
                            .map(|v| format!("{v:.4}"))
                            .unwrap_or_else(|| "n/a".into());
                        println!(
                            "  {} | mean={mean_str} | {}",
                            obs.acquired_date, obs.granule_id
                        );
                    }
                }
                if !res.failed_observations.is_empty() {
                    println!(
                        "  Note: {} sampled observation(s) failed statistics retrieval.",
                        res.failed_observations.len()
                    );
                }
                for w in &res.warnings {
                    println!("  Warning: {w}");
                }
                if let Some(tr) = &res.trend {
                    println!(
                        "  Trend ({}): min={:.3}, max={:.3}, mean={:.3}, net={:+.3}, slope={:?}/day",
                        tr.metric, tr.min, tr.max, tr.mean, tr.net_change, tr.slope_per_day
                    );
                }
            }
            Ok(())
        }

        Commands::FloodExtent {
            aoi,
            start,
            end,
            products,
            max_granules,
            max_tiles_across,
            min_area_km2,
            output,
            endpoint,
            json: as_json,
        } => {
            let (bbox, _) = aoi.resolve_required()?;
            let mut opts = FloodExtentOptions::new(bbox, start, end);
            if !products.is_empty() {
                opts.products = products;
            }
            opts.max_granules_per_product = max_granules;
            opts.max_tiles_across = max_tiles_across;
            opts.min_area_km2 = min_area_km2;
            opts.titiler_endpoint = Some(resolve_endpoint(endpoint.as_deref())?);

            let res = derive_flood_extent(&opts)?;
            if let Some(out_path) = &output {
                write_feature_collection(&res.feature_collection, out_path, "opera_flood_extent")?;
            }
            if as_json {
                let mut val = serde_json::to_value(&res)?;
                if let Some(out_path) = &output {
                    // Omit bulky GeoJSON inline when saved to disk so agent context stays compact.
                    val["output_path"] = json!(out_path.display().to_string());
                    if let Some(obj) = val.as_object_mut() {
                        obj.remove("feature_collection");
                    }
                }
                print_json(&val)?;
            } else {
                println!(
                    "Derived {} observed water polygon(s) totaling {:.3} km^2 (zoom {})",
                    res.polygon_count, res.total_area_km2, res.zoom
                );
                if res.failed_tiles > 0 {
                    println!(
                        "Warning: {} tile request(s) failed; flood coverage may be incomplete.",
                        res.failed_tiles
                    );
                }
                println!("Provenance: {}", res.provenance);
                if let Some(out_path) = &output {
                    println!("Saved flood extent to {}", out_path.display());
                }
            }
            Ok(())
        }

        Commands::Preview {
            product,
            aoi,
            granules,
            start,
            end,
            max_granules,
            band,
            rescale,
            colormap,
            water_only,
            expression,
            max_dim,
            max_tiles_across,
            no_basemap,
            no_scale_bar,
            no_legend,
            title,
            endpoint,
            output,
            json: as_json,
        } => {
            let (bbox, place_name) = aoi.resolve_required()?;
            let ep = resolve_endpoint(endpoint.as_deref())?;

            let (prod, cid, chosen_granules, chosen_band, date_label) = if !granules.is_empty() {
                let prod = find_product(&product)?;
                let cid = resolve_concept_id(&prod.short_name, None)?;
                let b = require_band(band.or_else(|| prod.default_bands.first().cloned()))?;
                (prod, cid, granules, b, None)
            } else {
                let resolved =
                    resolve_aoi_layer(&product, bbox, start, end, max_granules, band, None, None)?;
                let urs: Vec<String> = resolved.granules.into_iter().map(|g| g.id).collect();
                (
                    resolved.product,
                    resolved.concept_id,
                    urs,
                    resolved.band,
                    resolved.latest_date,
                )
            };

            let style = LayerStyle::for_request(
                &prod.short_name,
                Some(&chosen_band),
                expression.as_deref(),
                water_only,
                rescale.as_deref(),
                colormap.as_deref(),
            );
            let queries: Vec<TitilerQuery> = chosen_granules
                .iter()
                .map(|ur| {
                    TitilerQuery::for_granule_band(
                        &ep,
                        &cid,
                        &prod.short_name,
                        ur,
                        &chosen_band,
                        expression.clone(),
                        Some(&style),
                    )
                })
                .collect::<Result<Vec<_>>>()?;

            let banner = title.or_else(|| {
                let base = format!("{} {}", prod.short_title, chosen_band);
                Some(match date_label.as_deref().filter(|d| !d.is_empty()) {
                    Some(d) => format!("{base} {d}"),
                    None => base,
                })
            });
            let mut opts = PreviewOptions::new(bbox, &prod.short_name, Some(chosen_band.clone()));
            opts.water_only = water_only;
            opts.max_dim = max_dim;
            opts.max_tiles_across = max_tiles_across;
            opts.basemap = !no_basemap;
            opts.scale_bar = !no_scale_bar;
            opts.legend = !no_legend && style.colormap.is_some();
            opts.title = banner;

            let png_bytes = render_preview_png(&queries, &opts)?;
            write_output(&output, &png_bytes)?;
            if as_json {
                print_json(&json!({
                    "product": prod.short_name,
                    "band": chosen_band,
                    "granules": chosen_granules,
                    "granule_count": chosen_granules.len(),
                    "bbox": bbox,
                    "geocoded_place": place_name,
                    "bytes": png_bytes.len(),
                    "output": output.display().to_string(),
                }))?;
            } else {
                println!(
                    "Rendered {} ({}, {} granule(s){}) preview ({} bytes) to {}",
                    prod.short_name,
                    chosen_band,
                    chosen_granules.len(),
                    place_name
                        .as_deref()
                        .map(|p| format!(", {p}"))
                        .unwrap_or_default(),
                    png_bytes.len(),
                    output.display()
                );
            }
            Ok(())
        }

        Commands::Animate {
            product,
            aoi,
            start,
            end,
            max_frames,
            interval_days,
            delay_cs,
            band,
            water_only,
            max_dim,
            no_basemap,
            endpoint,
            output,
            json: as_json,
        } => {
            let prod = find_product(&product)?;
            let (bbox, _) = aoi.resolve_required()?;
            let ep = resolve_endpoint(endpoint.as_deref())?;
            let frame_cap = max_frames.clamp(1, 30);
            let (concept_id, candidates, warnings) = fetch_time_series_candidates(
                &prod.short_name,
                bbox,
                &start,
                &end,
                frame_cap,
                None,
            )?;
            if !as_json {
                for w in &warnings {
                    eprintln!("warning: {w}");
                }
            }
            if candidates.is_empty() {
                return Err(OperaError::NoResults(format!(
                    "no {} granules found to animate",
                    prod.short_name
                )));
            }
            let cid = concept_id
                .or_else(|| candidates.iter().find_map(|g| g.concept_id.clone()))
                .ok_or_else(|| OperaError::NoResults("missing concept_id".into()))?;
            let chosen_band = require_band(
                band.or_else(|| candidates.iter().find_map(|g| prod.choose_band(&g.bands))),
            )?;
            let matching_band: Vec<_> = candidates
                .into_iter()
                .filter(|g| {
                    g.bands.is_empty()
                        || g.bands.iter().any(|b| b.eq_ignore_ascii_case(&chosen_band))
                })
                .collect();
            let sampled = opera_rs::analysis::sample_granules_chronologically(
                matching_band,
                interval_days,
                frame_cap,
            );
            if sampled.is_empty() {
                return Err(OperaError::NoResults(format!(
                    "no dated {} granules with band '{chosen_band}' available to animate",
                    prod.short_name
                )));
            }

            let (gif_bytes, dates) = render_animation_gif(
                bbox,
                &prod.short_name,
                &prod.short_title,
                &cid,
                &sampled,
                &chosen_band,
                water_only,
                max_dim,
                !no_basemap,
                delay_cs,
                &ep,
            )?;
            write_output(&output, &gif_bytes)?;
            if as_json {
                print_json(&json!({
                    "product": prod.short_name,
                    "band": chosen_band,
                    "frames": dates.len(),
                    "dates": dates,
                    "bytes": gif_bytes.len(),
                    "output": output.display().to_string(),
                    "warnings": warnings,
                }))?;
            } else {
                println!(
                    "Wrote {}-frame animated GIF ({} bytes) to {}",
                    dates.len(),
                    gif_bytes.len(),
                    output.display()
                );
            }
            Ok(())
        }

        Commands::ExportHtml {
            product,
            aoi,
            start,
            end,
            max_granules,
            band,
            water_only,
            title,
            endpoint,
            output,
            json: as_json,
        } => {
            let (bbox, place_name) = aoi.resolve_required()?;
            let ep = resolve_endpoint(endpoint.as_deref())?;
            let resolved =
                resolve_aoi_layer(&product, bbox, start, end, max_granules, band, None, None)?;
            let prod = resolved.product;
            let cid = resolved.concept_id;
            let chosen_band = resolved.band;
            let style =
                LayerStyle::for_band(&prod.short_name, Some(&chosen_band), water_only, None, None);

            let layers = build_html_tile_layers(
                &prod.short_name,
                &prod.short_title,
                &cid,
                &resolved.granules,
                &chosen_band,
                &style,
                &ep,
            )?;

            let page_title = title.unwrap_or_else(|| {
                format!(
                    "NASA OPERA {} ({}){}",
                    prod.short_title,
                    chosen_band,
                    place_name
                        .as_deref()
                        .map(|p| format!(" - {p}"))
                        .unwrap_or_default()
                )
            });

            export_interactive_html(
                &output,
                &page_title,
                bbox,
                &layers,
                Some(&resolved.feature_collection),
                Some(&style),
            )?;

            if as_json {
                print_json(&json!({
                    "title": page_title,
                    "product": prod.short_name,
                    "band": chosen_band,
                    "layers": layers.len(),
                    "output": output.display().to_string(),
                }))?;
            } else {
                println!(
                    "Exported interactive MapLibre HTML ({} layer(s)) to {}",
                    layers.len(),
                    output.display()
                );
            }
            Ok(())
        }

        Commands::Inspect {
            path,
            product,
            band,
            json: as_json,
        } => {
            let summary = inspect_local_raster(&path, product.as_deref(), band.as_deref())?;
            if as_json {
                print_json(&serde_json::to_value(&summary)?)?;
            } else {
                println!(
                    "{}: {}x{} | CRS={} | pixel={}x{} | valid={} px",
                    summary.path,
                    summary.width,
                    summary.height,
                    summary.crs.as_deref().unwrap_or("unknown"),
                    summary.pixel_size_x,
                    summary.pixel_size_y,
                    summary.valid_pixels
                );
                if !summary.class_breakdown.is_empty() {
                    for c in &summary.class_breakdown {
                        println!(
                            "  class {:>3} ({:<24}): {:>9.0} px | {:>5.1}% | {:>8.3} km^2",
                            c.class_value,
                            c.label.as_deref().unwrap_or("-"),
                            c.pixel_count,
                            c.percent,
                            c.area_km2
                        );
                    }
                }
            }
            Ok(())
        }

        Commands::BurstId {
            inputs,
            group,
            dates,
            json: as_json,
        } => {
            if group {
                let grouped = group_by_burst(&inputs)?;
                if as_json {
                    print_json(&serde_json::to_value(&grouped)?)?;
                } else {
                    for (burst, items) in &grouped {
                        println!("{burst} ({} files):", items.len());
                        for item in items {
                            println!("  {item}");
                        }
                    }
                }
            } else {
                let mut rows = Vec::with_capacity(inputs.len());
                for item in &inputs {
                    let parsed = parse_burst_id(item)?;
                    let extracted_dates: Vec<String> = if dates {
                        get_dates(item).into_iter().map(|d| d.to_string()).collect()
                    } else {
                        Vec::new()
                    };
                    rows.push(json!({
                        "input": item,
                        "burst_id": parsed.normalized,
                        "official": parsed.official,
                        "track": parsed.track,
                        "burst_number": parsed.burst_number,
                        "subswath": parsed.subswath,
                        "dates": extracted_dates,
                    }));
                }
                if as_json {
                    print_json(&json!({ "items": rows }))?;
                } else {
                    for row in &rows {
                        let bid = row["burst_id"].as_str().unwrap_or("");
                        if dates {
                            let ds: Vec<&str> = row["dates"]
                                .as_array()
                                .map(|a| a.iter().filter_map(Value::as_str).collect())
                                .unwrap_or_default();
                            println!("{bid}\t{}", ds.join(","));
                        } else {
                            println!("{bid}");
                        }
                    }
                }
            }
            Ok(())
        }

        Commands::Frame {
            id,
            from_burst,
            latlon,
            bounds_only,
            burst_ids_only,
            db_file,
            geom_file,
            output,
            json: as_json,
        } => {
            if from_burst {
                let frame_ids = get_frame_ids_for_burst(&id, db_file.as_deref())?;
                if as_json {
                    print_json(&json!({
                        "burst_id": opera_rs::bursts::normalize_burst_id(&id),
                        "frame_ids": frame_ids,
                    }))?;
                } else {
                    for fid in &frame_ids {
                        println!("{fid}");
                    }
                }
                return Ok(());
            }

            let frame_id: u32 = id.trim().parse().map_err(|_| {
                OperaError::InvalidInput(format!(
                    "expected numeric frame ID (got '{id}'; pass --from-burst for burst IDs)"
                ))
            })?;

            if bounds_only {
                let fb = get_frame_bbox(frame_id, latlon, db_file.as_deref())?;
                print_json(&serde_json::to_value(fb.bbox)?)?;
                return Ok(());
            }

            let mut info = get_frame_to_burst_mapping(frame_id, db_file.as_deref())?;
            if burst_ids_only {
                if as_json {
                    print_json(&serde_json::to_value(&info.burst_ids)?)?;
                } else {
                    for b in &info.burst_ids {
                        println!("{b}");
                    }
                }
                return Ok(());
            }

            // Compute requested bbox once and propagate WGS 84 reprojection errors consistently
            // across both JSON and text output modes.
            let fb = if latlon {
                let wgs = match info.bbox_wgs84 {
                    Some(b) => b,
                    None => reproject_bbox_to_wgs84(
                        info.epsg,
                        [info.xmin, info.ymin, info.xmax, info.ymax],
                    )?,
                };
                FrameBBox {
                    frame_id,
                    epsg: 4326,
                    bbox: wgs.to_array(),
                }
            } else {
                FrameBBox {
                    frame_id,
                    epsg: info.epsg,
                    bbox: [info.xmin, info.ymin, info.xmax, info.ymax],
                }
            };

            let written = if let Some(out_path) = output {
                let fc = get_frame_geojson(Some(&[frame_id]), geom_file.as_deref())?;
                if let Some(pass_str) = fc
                    .get("features")
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.first())
                    .and_then(|f| f.get("properties"))
                    .and_then(|p| p.get("orbit_pass"))
                    .and_then(Value::as_str)
                {
                    info.orbit_pass = Some(pass_str.parse::<OrbitPass>()?);
                }
                let count = write_feature_collection(&fc, &out_path, "disp_frame")?;
                Some((out_path, count))
            } else {
                if geom_file.is_some() {
                    info.orbit_pass = Some(get_frame_orbit_pass(frame_id, geom_file.as_deref())?);
                }
                None
            };

            if as_json {
                let mut val = serde_json::to_value(&info)?;
                val["bbox_epsg"] = json!(fb.epsg);
                val["bbox"] = json!(fb.bbox);
                if let Some((ref p, count)) = written {
                    val["written"] = json!({ "path": p, "features": count });
                }
                print_json(&val)?;
            } else {
                println!(
                    "Frame {} | EPSG:{} | bbox={:?} | land={} | north_america={} | bursts={}",
                    info.frame_id,
                    fb.epsg,
                    fb.bbox,
                    info.is_land,
                    info.is_north_america,
                    info.burst_ids.len()
                );
                if let Some(pass) = info.orbit_pass {
                    println!("Orbit pass: {pass}");
                }
                if let Some((p, count)) = written {
                    println!("Wrote {count} frame geometry to {}", p.display());
                }
            }
            Ok(())
        }

        Commands::FramesIntersect {
            aoi,
            lon,
            lat,
            ids_only,
            db_file,
            output,
            json: as_json,
        } => {
            let has_point = lon.is_some() || lat.is_some();
            let has_aoi = aoi.bbox.is_some() || aoi.place.is_some() || aoi.aoi_file.is_some();
            if has_point && has_aoi {
                return Err(OperaError::InvalidInput(
                    "specify either --lon/--lat or --bbox/--place/--aoi-file, not both".into(),
                ));
            }
            let fc = if let (Some(x), Some(y)) = (lon, lat) {
                get_intersecting_frames_for_point(x, y, db_file.as_deref())?
            } else if let Some((b, _)) = aoi.resolve()? {
                get_intersecting_frames(b, db_file.as_deref())?
            } else {
                return Err(OperaError::InvalidInput(
                    "specify an AOI with --bbox, --place, --aoi-file, or --lon/--lat".into(),
                ));
            };
            let ids = frame_ids_from_geojson(&fc);
            let written = if let Some(out_path) = output {
                let count = write_feature_collection(&fc, &out_path, "disp_frames")?;
                Some((out_path, count))
            } else {
                None
            };

            if ids_only {
                if as_json {
                    print_json(&serde_json::to_value(&ids)?)?;
                } else {
                    for id in &ids {
                        println!("{id}");
                    }
                }
            } else if as_json {
                if let Some((p, count)) = written {
                    print_json(&json!({
                        "frame_ids": ids,
                        "count": ids.len(),
                        "written": { "path": p, "features": count },
                    }))?;
                } else {
                    print_json(&fc)?;
                }
            } else {
                println!(
                    "Found {} intersecting DISP-S1 frame(s): {}",
                    ids.len(),
                    ids.iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                if let Some((p, count)) = written {
                    println!("Wrote {count} frame geometries to {}", p.display());
                }
            }
            Ok(())
        }

        Commands::MissingData {
            inputs,
            file,
            max_options,
            write_options,
            output_prefix,
            json: as_json,
        } => {
            let mut all_files = inputs;
            if let Some(list_path) = file {
                let text = std::fs::read_to_string(&list_path)?;
                for line in text.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        all_files.push(trimmed.to_string());
                    }
                }
            }
            if all_files.is_empty() {
                return Err(OperaError::InvalidInput(
                    "provide CSLC filenames as positional arguments or via --file".into(),
                ));
            }
            let options = get_missing_data_options_limited(&all_files, Some(max_options.max(1)))?;

            let mut written_paths = Vec::new();
            if write_options {
                for (idx, opt) in options.iter().enumerate() {
                    let path = format!(
                        "{output_prefix}{}_bursts_{}_burst_ids_{}_dates_{}.txt",
                        idx + 1,
                        opt.total_num_bursts,
                        opt.num_burst_ids,
                        opt.num_dates
                    );
                    let mut body = opt.inputs.join("\n");
                    if !body.is_empty() {
                        body.push('\n');
                    }
                    std::fs::write(&path, body)?;
                    written_paths.push(path);
                }
            }

            if as_json {
                print_json(&json!({
                    "options": options,
                    "written_files": written_paths,
                }))?;
            } else {
                println!(
                    "{:<8} {:<10} {:<12} {:<16} {:<16}",
                    "Option", "# Dates", "# Burst IDs", "Selected Bursts", "Excluded Bursts"
                );
                for (idx, opt) in options.iter().enumerate() {
                    println!(
                        "{:<8} {:<10} {:<12} {:<16} {:<16}",
                        idx + 1,
                        opt.num_dates,
                        opt.num_burst_ids,
                        opt.total_num_bursts,
                        opt.excluded_bursts
                    );
                }
                for path in &written_paths {
                    println!("Wrote {path}");
                }
            }
            Ok(())
        }

        Commands::DispSearch {
            frame_id,
            aoi,
            start,
            end,
            product_version,
            url_type,
            all_versions,
            limit,
            urls_only,
            json: as_json,
        } => {
            let parsed_url_type: UrlType = url_type.parse()?;
            let resolved_bbox = aoi.resolve()?.map(|(b, _)| b);
            if frame_id.is_none() && resolved_bbox.is_none() {
                return Err(OperaError::InvalidInput(
                    "specify --frame-id or an AOI (--bbox, --place, or --aoi-file)".into(),
                ));
            }
            let version_filter = if product_version.trim().is_empty() {
                None
            } else {
                Some(product_version)
            };
            let params = DispSearchParams {
                frame_id,
                bbox: resolved_bbox,
                product_version: version_filter,
                start,
                end,
                url_type: parsed_url_type,
                deduplicate: !all_versions,
                limit,
                cmr_endpoint: None,
            };
            let res = search_disp_products(&params)?;
            if urls_only {
                for p in &res.products {
                    println!("{}", p.filename);
                }
            } else if as_json {
                print_json(&serde_json::to_value(&res)?)?;
            } else {
                println!(
                    "Found {} DISP-S1 product(s) across {} frame(s) (CMR hits={}, duplicates removed={})",
                    res.products.len(),
                    res.frame_ids.len(),
                    res.total_hits,
                    res.duplicates_removed
                );
                for p in res.products.iter().take(20) {
                    println!(
                        "  F{:05} | {} -> {} | v{} | {}",
                        p.frame_id,
                        &p.reference_datetime[..10],
                        &p.secondary_datetime[..10],
                        p.version,
                        p.filename
                    );
                }
            }
            Ok(())
        }

        Commands::Mcp => unreachable!("handled before dispatch"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_products_and_emits_json_usage() {
        assert_eq!(run(["opera", "products", "--json"]), 0);
        assert_eq!(run(["opera", "products", "dswx-hls", "--json"]), 0);
        assert_eq!(run(["opera", "--json-usage"]), 0);
        assert_eq!(run(["opera", "search", "--json-usage"]), 0);
        assert_eq!(run(["opera", "mcp", "--json-usage"]), 2);
    }

    #[test]
    fn builds_tilejson_url_offline_with_negative_rescale_and_bbox() {
        assert_eq!(
            run([
                "opera",
                "tilejson",
                "--concept-id",
                "C2617126679-POCLOUD",
                "--product",
                "rtc-s1",
                "--band",
                "VV",
                "--expression",
                "10*log10(b1)",
                "--rescale",
                "-30,0",
                "--endpoint",
                "https://titiler-cmr.opengeos.org",
                "--url-only",
                "--json"
            ]),
            0
        );
        let parsed = Cli::try_parse_from([
            "opera",
            "search",
            "--product",
            "dswx-hls",
            "--bbox",
            "-119.95,35.85,-119.45,36.25",
        ])
        .expect("parse negative western-hemisphere bbox");
        assert!(matches!(parsed.command, Some(Commands::Search { .. })));
    }

    #[test]
    fn rejects_conflicting_aoi_flags_and_invalid_endpoints() {
        let conflict = Cli::try_parse_from([
            "opera",
            "search",
            "--bbox",
            "-119.95,35.85,-119.45,36.25",
            "--place",
            "Tulare Lake",
        ]);
        assert!(conflict.is_err(), "conflicting AOI flags should fail");

        let preview = Cli::try_parse_from([
            "opera",
            "preview",
            "--bbox",
            "-119.95,35.85,-119.45,36.25",
            "-o",
            "out.png",
            "--json",
        ])
        .expect("preview --json should parse");
        assert!(matches!(
            preview.command,
            Some(Commands::Preview { json: true, .. })
        ));

        assert!(resolve_endpoint(Some("https://titiler-cmr.opengeos.org")).is_ok());
        assert!(resolve_endpoint(Some("http://localhost:8080")).is_ok());
        assert!(resolve_endpoint(Some("file:///etc/passwd")).is_err());
    }

    #[test]
    fn runs_opera_utils_cli_subcommands_offline() {
        assert_eq!(
            run([
                "opera",
                "burst-id",
                "OPERA_L2_CSLC-S1_T087-185683-IW2_20230322T161649Z_20240504T185235Z_S1A_VV_v1.1.h5",
                "--dates",
                "--json"
            ]),
            0
        );
        assert_eq!(
            run([
                "opera",
                "missing-data",
                "t087_185683_iw1_20230101.h5",
                "t087_185683_iw1_20230113.h5",
                "t087_185683_iw2_20230101.h5",
                "--json"
            ]),
            0
        );

        let dir = tempfile::tempdir().unwrap();
        let f2b = dir.path().join("f2b.json");
        std::fs::write(
            &f2b,
            serde_json::to_vec(&json!({
                "data": {
                    "11114": {
                        "epsg": 32610,
                        "is_land": true,
                        "is_north_america": true,
                        "xmin": 546450,
                        "ymin": 4204110,
                        "xmax": 833790,
                        "ymax": 4409070,
                        "burst_ids": ["t042_088905_iw1"]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            run([
                "opera",
                "frame",
                "11114",
                "--latlon",
                "--db-file",
                f2b.to_str().unwrap(),
                "--json"
            ]),
            0
        );

        let geoms = dir.path().join("frames.geojson");
        std::fs::write(
            &geoms,
            serde_json::to_vec(&json!({
                "type": "FeatureCollection",
                "features": [{
                    "type": "Feature",
                    "id": 11114,
                    "properties": {"orbit_pass": "DESCENDING"},
                    "geometry": {
                        "type": "Polygon",
                        "coordinates": [[[-122.5, 37.9], [-119.1, 37.9], [-119.1, 39.8], [-122.5, 39.8], [-122.5, 37.9]]]
                    }
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            run([
                "opera",
                "frames-intersect",
                "--lon",
                "-120.5",
                "--lat",
                "38.5",
                "--ids-only",
                "--db-file",
                geoms.to_str().unwrap(),
                "--json"
            ]),
            0
        );

        let disp_cli = Cli::try_parse_from([
            "opera",
            "disp-search",
            "--frame-id",
            "11115",
            "--end",
            "2024-01-01",
            "--url-type",
            "s3",
            "--json",
        ])
        .expect("parse disp-search");
        assert!(matches!(
            disp_cli.command,
            Some(Commands::DispSearch { .. })
        ));
    }
}
