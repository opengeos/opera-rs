# Rust Library

Add `opera-rs` to your `Cargo.toml`:

```toml
[dependencies]
opera-rs = "0.1"
```

## Feature Flags

| Feature | Default | Description |
| --- | --- | --- |
| `http` | Yes | Live HTTPS requests to NASA CMR, `titiler-cmr`, and Nominatim via `ureq` + `rustls` (zero GDAL dependencies) |
| `geo` | Yes | Pure-Rust GeoJSON, GeoPackage, Shapefile, GeoTIFF/COG, and CRS reprojection powered by `terrio` |
| `parallel` | Yes | Multi-threaded tile fetching and raster stitching via `rayon` |

Disable default features (`default-features = false`) for 100% pure-Rust, zero-C/C++ offline or WebAssembly builds containing the product catalog, URL builders, UMM-G / `titiler-cmr` parsers, flood-extent mask vectorizer, and PNG/GIF/HTML encoders:

```toml
[dependencies]
opera-rs = { version = "0.1", default-features = false }
```

## Core Modules

| Module | Responsibilities |
| --- | --- |
| `opera_rs::products` | Catalog of all 9 NASA OPERA products, alias lookup, band selection, categorical colormaps, and expression presets |
| `opera_rs::cmr` | NASA CMR UMM-G granule search, version deduplication, newest-per-tile filtering, and AOI layer resolution |
| `opera_rs::geocode` | OpenStreetMap Nominatim place-to-bbox geocoding |
| `opera_rs::titiler` | `titiler-cmr` URL builders (`rasterio` & `xarray`), TileJSON, point sampling, and AOI zonal statistics |
| `opera_rs::analysis` | Two-date change detection (`detect_change`) and multi-date OLS trend analysis (`analyze_time_series`) |
| `opera_rs::flood_extent` | Water-only tile union, mask stitching, and RFC 7946 polygon vectorization with nested holes |
| `opera_rs::viz` | Pure-Rust PNG preview rendering, before/after comparison panels, animated GIFs, and MapLibre HTML export |
| `opera_rs::geo_io` | Pure-Rust vector AOI reading, GeoJSON/GeoPackage/Shapefile writing, and local GeoTIFF/COG inspection via `terrio` |

## Search and Change Detection Example

```rust,no_run
use opera_rs::{
    BBox, ChangeDetectionOptions, CmrSearchParams, detect_change, search_granules,
};

fn main() -> opera_rs::Result<()> {
    let bbox = BBox::new(-119.95, 35.85, -119.45, 36.25)?;

    let search = search_granules(&CmrSearchParams {
        product: "dswx-hls".into(),
        bbox: Some(bbox),
        start: Some("2023-04-20".into()),
        end: Some("2023-04-30".into()),
        limit: 10,
        ..Default::default()
    })?;
    println!(
        "Returned {} granules ({} total hits)",
        search.granules.len(),
        search.hits
    );

    let change = detect_change(&ChangeDetectionOptions {
        product: "dswx-hls".into(),
        bbox,
        aoi_geojson: None,
        before_date: "2023-01-15".into(),
        after_date: "2023-04-30".into(),
        window_days: 10,
        band: Some("B01_WTR".into()),
        expression: None,
        titiler_endpoint: None,
        cmr_endpoint: None,
    })?;
    println!("{}", change.to_markdown());
    Ok(())
}
```

## Offline Tile URL Construction (`--no-default-features`)

```rust
use opera_rs::products::LayerStyle;
use opera_rs::titiler::{TitilerQuery, build_tile_url_template, build_tilejson_url};

fn main() {
    let style = LayerStyle::for_band("OPERA_L3_DSWX-HLS_V1", Some("B01_WTR"), true, None, None);
    let mut query = TitilerQuery::rasterio("C2617126679-POCLOUD");
    query.assets = vec!["B01_WTR".into()];
    query.apply_style(&style);

    println!("TileJSON: {}", build_tilejson_url(&query));
    println!("Tiles:    {}", build_tile_url_template(&query, 512));
}
```
