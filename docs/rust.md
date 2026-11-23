# Rust Library

Add `opera-rs` to your `Cargo.toml`:

```toml
[dependencies]
opera-rs = "0.1"
```

## Feature Flags

- `http` (default): Live HTTPS requests to NASA CMR, `titiler-cmr`, and Nominatim via pure-Rust `ureq` + `rustls`.
- `geo` (default): Pure-Rust GeoJSON, GeoPackage, Shapefile, GeoTIFF/COG, and CRS reprojection powered by `terrio`.
- `parallel` (default): Multi-threaded tile fetching and raster stitching via `rayon`.

Disable default features (`default-features = false`) for minimal offline or WebAssembly builds containing the product catalog, URL builders, UMM-G / `titiler-cmr` parsers, flood-extent mask vectorizer, and PNG/GIF/HTML encoders.

## Example

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
    println!("Returned {} granules ({} total hits)", search.granules.len(), search.hits);

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
