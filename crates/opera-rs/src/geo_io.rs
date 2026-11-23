//! Pure-Rust geospatial vector and raster I/O powered by [`terrio`].
//!
//! Enables reading AOI polygons from GeoJSON, GeoPackage (`.gpkg`), or Shapefile (`.shp`)
//! with automatic CRS reprojection to WGS 84 (`EPSG:4326`), exporting granule footprints and
//! flood-extent polygons to GeoJSON/GeoPackage/Shapefile, and inspecting local OPERA
//! GeoTIFFs/COGs without GDAL.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use terrio::crs::SpatialRef;
use terrio::vector::{Coord, Crs, Ring, layer_extent, reproject_layer};
use terrio::{
    Feature, FieldDef, FieldType, FieldValue, Geometry, GeometryType, Layer, VectorOpenOptions,
    VectorWriteOptions,
};

use crate::error::{OperaError, Result};
use crate::products::{OPERA_PIXEL_AREA_KM2, class_label, is_categorical_band};
use crate::titiler::ClassBreakdown;
use crate::types::BBox;

fn path_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| OperaError::InvalidInput(format!("non-UTF-8 path: {}", path.display())))
}

/// Reads any vector file supported by `terrio` (GeoJSON, GeoPackage, Shapefile) and returns
/// its WGS 84 (`EPSG:4326`) bounding box.
pub fn read_aoi_bbox(path: impl AsRef<Path>) -> Result<BBox> {
    let p = path_str(path.as_ref())?;
    let ds = terrio::open_vector(p, &VectorOpenOptions::default())?;
    let wgs84 = SpatialRef::parse("EPSG:4326")?;
    let mut combined: Option<BBox> = None;

    for layer in ds.layers {
        let reprojected = match &layer.crs {
            Some(crs) if crs.epsg != Some(4326) => reproject_layer(&layer, &wgs84)?,
            _ => layer,
        };
        if let Some((min_x, min_y, max_x, max_y)) = layer_extent(&reprojected) {
            let b = BBox::new(min_x, min_y, max_x, max_y)?;
            combined = Some(match combined {
                Some(acc) => acc.union(b),
                None => b,
            });
        }
    }

    combined.ok_or_else(|| {
        OperaError::NoResults(format!(
            "no vector geometries found in '{}'",
            path.as_ref().display()
        ))
    })
}

/// Writes a GeoJSON `FeatureCollection` to disk.
///
/// For `.geojson` / `.json` extensions, writes pretty-printed GeoJSON directly; for `.gpkg`,
/// `.shp`, or other vector formats, converts to a `terrio::Layer` (`EPSG:4326`) and writes
/// via pure-Rust `terrio::write_vector`.
pub fn write_feature_collection(
    fc: &Value,
    out_path: impl AsRef<Path>,
    layer_name: &str,
) -> Result<usize> {
    let path = out_path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let feats = fc
        .get("features")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OperaError::InvalidInput(
                "expected GeoJSON FeatureCollection with 'features' array".into(),
            )
        })?;

    if ext == "geojson" || ext == "json" {
        std::fs::write(path, serde_json::to_string_pretty(fc)?)?;
        return Ok(feats.len());
    }

    let mut layer = Layer::new(layer_name.to_string());
    layer.crs = Some(Crs::new().with_epsg(4326));
    layer.geom_type = Some(GeometryType::MultiPolygon);

    // Collect union of scalar property keys from features.
    let mut field_names: Vec<String> = Vec::new();
    for feat in feats {
        if let Some(props) = feat.get("properties").and_then(Value::as_object) {
            for k in props.keys() {
                if !field_names.iter().any(|existing| existing == k) {
                    field_names.push(k.clone());
                }
            }
        }
    }
    for name in &field_names {
        layer.schema.add_field(FieldDef::new(name, FieldType::Text));
    }

    for (idx, feat) in feats.iter().enumerate() {
        let Some(geom_val) = feat.get("geometry") else {
            continue;
        };
        let Some(geometry) = geojson_to_terrio_geometry(geom_val) else {
            continue;
        };
        let props = feat.get("properties").and_then(Value::as_object);
        let attributes: Vec<FieldValue> = field_names
            .iter()
            .map(|k| match props.and_then(|m| m.get(k)) {
                None | Some(Value::Null) => FieldValue::Null,
                Some(Value::String(s)) => FieldValue::Text(s.clone()),
                Some(Value::Bool(b)) => FieldValue::Text(b.to_string()),
                Some(Value::Number(n)) => FieldValue::Text(n.to_string()),
                Some(other) => FieldValue::Text(other.to_string()),
            })
            .collect();
        layer.features.push(Feature {
            fid: (idx + 1) as u64,
            geometry: Some(geometry),
            attributes,
        });
    }

    let count = layer.features.len();
    terrio::write_vector(&[layer], path_str(path)?, &VectorWriteOptions::default())?;
    Ok(count)
}

fn parse_ring(ring: &Value) -> Option<Ring> {
    let arr = ring.as_array()?;
    let coords: Vec<Coord> = arr
        .iter()
        .filter_map(|pt| {
            let pair = pt.as_array()?;
            Some(Coord::xy(pair.first()?.as_f64()?, pair.get(1)?.as_f64()?))
        })
        .collect();
    (coords.len() >= 3).then(|| Ring::new(coords))
}

fn parse_polygon_rings(rings_val: &Value) -> Option<(Ring, Vec<Ring>)> {
    let mut rings: Vec<Ring> = rings_val
        .as_array()?
        .iter()
        .filter_map(parse_ring)
        .collect();
    if rings.is_empty() {
        return None;
    }
    let outer = rings.remove(0);
    Some((outer, rings))
}

fn geojson_to_terrio_geometry(val: &Value) -> Option<Geometry> {
    let gtype = val.get("type")?.as_str()?;
    let coords = val.get("coordinates")?;
    match gtype {
        "Polygon" => Some(Geometry::MultiPolygon(vec![parse_polygon_rings(coords)?])),
        "MultiPolygon" => {
            let polys: Vec<(Ring, Vec<Ring>)> = coords
                .as_array()?
                .iter()
                .filter_map(parse_polygon_rings)
                .collect();
            (!polys.is_empty()).then_some(Geometry::MultiPolygon(polys))
        }
        _ => None,
    }
}

/// Summary of a local OPERA GeoTIFF/COG raster inspected via `terrio`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalRasterSummary {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub crs: Option<String>,
    pub pixel_size_x: f64,
    pub pixel_size_y: f64,
    pub nodata: Option<f64>,
    pub valid_pixels: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub class_breakdown: Vec<ClassBreakdown>,
}

/// Inspects a local OPERA GeoTIFF/COG on disk using pure-Rust `terrio::open_raster`.
pub fn inspect_local_raster(
    path: impl AsRef<Path>,
    product_hint: Option<&str>,
    band_hint: Option<&str>,
) -> Result<LocalRasterSummary> {
    let p_str = path_str(path.as_ref())?;
    let ds = terrio::open_raster(p_str)?;
    let width = ds.raster.cols;
    let height = ds.raster.rows;
    let nodata = ds.raster.nodata.is_finite().then_some(ds.raster.nodata);
    let pixel_size_x = ds.raster.cell_size_x.abs();
    let pixel_size_y = ds.raster.cell_size_y.abs();
    let crs = ds
        .raster
        .crs
        .epsg
        .map(|code| format!("EPSG:{code}"))
        .or_else(|| ds.raster.crs.wkt.clone());

    let categorical = product_hint
        .map(|p| is_categorical_band(p, band_hint.or(Some("B01_WTR"))))
        .unwrap_or(false);

    // Compute pixel area in km^2: if projected meters, use actual pixel size; else 30 m OPERA default.
    let pixel_area_km2 = if pixel_size_x > 0.5 && pixel_size_y > 0.5 {
        (pixel_size_x * pixel_size_y) / 1_000_000.0
    } else {
        OPERA_PIXEL_AREA_KM2
    };

    let mut valid_pixels = 0usize;
    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    let mut sum = 0.0f64;
    let mut class_counts: BTreeMap<i64, usize> = BTreeMap::new();

    for row in 0..height {
        for col in 0..width {
            let Some(v) = ds.raster.get_raw(0, row as isize, col as isize) else {
                continue;
            };
            if !v.is_finite() || nodata.is_some_and(|nd| (v - nd).abs() < 1e-12) {
                continue;
            }
            valid_pixels += 1;
            min_v = min_v.min(v);
            max_v = max_v.max(v);
            sum += v;
            if categorical {
                *class_counts.entry(v.round() as i64).or_insert(0) += 1;
            }
        }
    }

    let class_breakdown = class_counts
        .into_iter()
        .map(|(cls, cnt)| ClassBreakdown {
            class_value: cls,
            label: product_hint
                .and_then(|p| class_label(p, band_hint, cls))
                .map(ToString::to_string),
            pixel_count: cnt as f64,
            percent: if valid_pixels > 0 {
                (cnt as f64 / valid_pixels as f64) * 100.0
            } else {
                0.0
            },
            area_km2: (cnt as f64) * pixel_area_km2,
        })
        .collect();

    Ok(LocalRasterSummary {
        path: path.as_ref().display().to_string(),
        width,
        height,
        crs,
        pixel_size_x,
        pixel_size_y,
        nodata,
        valid_pixels,
        min: (valid_pixels > 0).then_some(min_v),
        max: (valid_pixels > 0).then_some(max_v),
        mean: (valid_pixels > 0).then_some(sum / valid_pixels as f64),
        class_breakdown,
    })
}

/// Convenience helper returning a GeoJSON Feature for a vector file's combined WGS 84 bbox.
pub fn aoi_file_to_geojson(path: impl AsRef<Path>) -> Result<(BBox, Value)> {
    let bbox = read_aoi_bbox(path)?;
    Ok((bbox, json!(bbox.to_geojson_feature())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_geojson_and_geopackage_via_terrio() {
        let dir = tempfile::tempdir().unwrap();
        let bbox = BBox::new(-119.9, 35.8, -119.4, 36.3).unwrap();
        let fc = json!({
            "type": "FeatureCollection",
            "features": [bbox.to_geojson_feature()]
        });

        let geojson_path = dir.path().join("aoi.geojson");
        assert_eq!(
            write_feature_collection(&fc, &geojson_path, "aoi").unwrap(),
            1
        );
        let read_back = read_aoi_bbox(&geojson_path).unwrap();
        assert!((read_back.west - bbox.west).abs() < 1e-6);
        assert!((read_back.north - bbox.north).abs() < 1e-6);

        let gpkg_path = dir.path().join("aoi.gpkg");
        assert_eq!(write_feature_collection(&fc, &gpkg_path, "aoi").unwrap(), 1);
        let read_gpkg = read_aoi_bbox(&gpkg_path).unwrap();
        assert!((read_gpkg.east - bbox.east).abs() < 1e-6);
    }
}
