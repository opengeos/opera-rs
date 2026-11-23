//! Catalog of NASA OPERA satellite products, band defaults, colormaps, and band-math presets.
//!
//! Covers all 9 operational NASA OPERA collections (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN,
//! RTC-S1, RTC-S1-STATIC, CSLC-S1, CSLC-S1-STATIC, and DISP-S1), with render parameters,
//! categorical class colormaps, and class labels matching `opengeos/geolibre-nasa-opera` and
//! `opengeos/rustgis-nasa-opera`.

use serde::{Deserialize, Serialize};

use crate::error::{OperaError, Result};
use crate::types::TitilerBackend;

/// Native pixel spacing in meters for raster OPERA products (30 m grid).
///
/// `titiler-cmr` evaluates `/statistics` in each granule's native CRS, so each unmasked
/// pixel represents `30 m x 30 m = 900 m^2 = 0.0009 km^2`.
pub const OPERA_PIXEL_SIZE_METERS: f64 = 30.0;

/// Area of a single 30 m OPERA pixel in square kilometers (`0.0009 km^2`).
pub const OPERA_PIXEL_AREA_KM2: f64 =
    (OPERA_PIXEL_SIZE_METERS * OPERA_PIXEL_SIZE_METERS) / 1_000_000.0;

/// DSWx WTR class value for open surface water.
pub const DSWX_OPEN_WATER_CLASS: u8 = 1;

/// DSWx WTR class value for partial surface water.
pub const DSWX_PARTIAL_WATER_CLASS: u8 = 2;

/// DSWx WTR categorical colormap JSON (white not-water and fill made transparent).
pub const DSWX_WTR_COLORMAP_JSON: &str = r#"{"0":[0,0,0,0],"1":[0,0,255,255],"2":[135,206,250,255],"252":[0,255,255,255],"253":[128,128,128,255],"254":[0,0,128,255],"255":[0,0,0,0]}"#;

/// DSWx WTR water-only colormap JSON: keeps open water (1) and partial surface water (2),
/// hiding clouds, snow/ice, ocean mask, not-water, and fill.
pub const DSWX_WTR_WATER_ONLY_COLORMAP_JSON: &str = r#"{"0":[0,0,0,0],"1":[0,0,255,255],"2":[135,206,250,255],"252":[0,0,0,0],"253":[0,0,0,0],"254":[0,0,0,0],"255":[0,0,0,0]}"#;

/// DIST-ALERT / DIST-ANN `VEG-DIST-STATUS` categorical colormap JSON.
pub const DIST_STATUS_COLORMAP_JSON: &str = r#"{"0":[0,0,0,0],"1":[255,255,178,255],"2":[254,204,92,255],"3":[253,141,60,255],"4":[240,59,32,255],"5":[220,30,30,255],"6":[189,0,38,255],"7":[150,150,150,255],"8":[90,90,90,255],"255":[0,0,0,0]}"#;

/// Metadata and default rendering settings for a NASA OPERA product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperaProduct {
    /// Exact NASA CMR `short_name` (e.g. `OPERA_L3_DSWX-HLS_V1`).
    pub short_name: String,
    /// Concise display label / alias (e.g. `DSWx-HLS`).
    pub short_title: String,
    /// Full NASA collection title.
    pub title: String,
    /// Concise summary of the product.
    pub description: String,
    /// Default `titiler-cmr` backend (`rasterio` for COGs, `xarray` for HDF5/NetCDF).
    pub backend: TitilerBackend,
    /// Whether `titiler-cmr`'s rasterio COG tile renderer directly supports this product.
    pub raster_tiles_supported: bool,
    /// Preferred default band tokens in order of priority.
    pub default_bands: Vec<String>,
    /// Default `assets_regex` pattern for `titiler-cmr`.
    pub default_bands_regex: Option<String>,
    /// Default rescale stretch (`"min,max"`), if continuous.
    pub default_rescale: Option<String>,
    /// Default named colormap, if continuous.
    pub default_colormap_name: Option<String>,
}

/// Static catalog descriptor used internally before materializing [`OperaProduct`].
struct ProductSpec {
    short_name: &'static str,
    short_title: &'static str,
    title: &'static str,
    description: &'static str,
    backend: TitilerBackend,
    raster_tiles_supported: bool,
    default_bands: &'static [&'static str],
    default_bands_regex: Option<&'static str>,
    default_rescale: Option<&'static str>,
    default_colormap_name: Option<&'static str>,
    aliases: &'static [&'static str],
}

const PRODUCT_SPECS: &[ProductSpec] = &[
    ProductSpec {
        short_name: "OPERA_L3_DSWX-HLS_V1",
        short_title: "DSWx-HLS",
        title: "Dynamic Surface Water Extent from Harmonized Landsat Sentinel-2 (Version 1)",
        description: "30 m optical surface water extent derived from Harmonized Landsat and Sentinel-2 (HLS)",
        backend: TitilerBackend::Rasterio,
        raster_tiles_supported: true,
        default_bands: &["B01_WTR"],
        default_bands_regex: Some("B[0-9]{2}_[A-Z0-9-]+"),
        default_rescale: None,
        default_colormap_name: None,
        aliases: &["dswx-hls", "dswx_hls", "dswx", "water-hls"],
    },
    ProductSpec {
        short_name: "OPERA_L3_DSWX-S1_V1",
        short_title: "DSWx-S1",
        title: "Dynamic Surface Water Extent from Sentinel-1 (Version 1)",
        description: "30 m cloud-penetrating SAR surface water extent derived from Sentinel-1",
        backend: TitilerBackend::Rasterio,
        raster_tiles_supported: true,
        default_bands: &["B01_WTR"],
        default_bands_regex: Some("B[0-9]{2}_[A-Z0-9-]+"),
        default_rescale: None,
        default_colormap_name: None,
        aliases: &["dswx-s1", "dswx_s1", "water-s1", "sar-water"],
    },
    ProductSpec {
        short_name: "OPERA_L3_DIST-ALERT-HLS_V1",
        short_title: "DIST-ALERT",
        title: "Land Surface Disturbance Alert from HLS (Version 1)",
        description: "Near real-time 30 m vegetation and surface disturbance alerts from HLS",
        backend: TitilerBackend::Rasterio,
        raster_tiles_supported: true,
        default_bands: &["VEG-DIST-STATUS"],
        default_bands_regex: Some("[A-Z-]+"),
        default_rescale: Some("0,4"),
        default_colormap_name: None,
        aliases: &["dist-alert", "dist_alert", "dist-alert-hls", "disturbance"],
    },
    ProductSpec {
        short_name: "OPERA_L3_DIST-ANN-HLS_V1",
        short_title: "DIST-ANN",
        title: "Land Surface Disturbance Annual from HLS (Version 1)",
        description: "Annual 30 m land surface disturbance summary from HLS",
        backend: TitilerBackend::Rasterio,
        raster_tiles_supported: true,
        default_bands: &["VEG-DIST-STATUS"],
        default_bands_regex: Some("[A-Z-]+"),
        default_rescale: Some("0,4"),
        default_colormap_name: None,
        aliases: &["dist-ann", "dist_ann", "dist-ann-hls"],
    },
    ProductSpec {
        short_name: "OPERA_L2_RTC-S1_V1",
        short_title: "RTC-S1",
        title: "Radiometric Terrain Corrected SAR Backscatter from Sentinel-1 (Version 1)",
        description: "Analysis-ready 30 m radiometrically terrain-corrected Sentinel-1 gamma-0 backscatter",
        backend: TitilerBackend::Rasterio,
        raster_tiles_supported: true,
        default_bands: &["VV", "HH"],
        default_bands_regex: Some("(VV|VH|HH|HV)"),
        default_rescale: Some("0,0.4"),
        default_colormap_name: Some("gray"),
        aliases: &["rtc-s1", "rtc_s1", "rtc", "sar"],
    },
    ProductSpec {
        short_name: "OPERA_L2_RTC-S1-STATIC_V1",
        short_title: "RTC-S1-STATIC",
        title: "RTC-S1 Static Layers (Version 1)",
        description: "Static geometry and ancillary layers (local incidence angle, layover/shadow mask) for RTC-S1",
        backend: TitilerBackend::Rasterio,
        raster_tiles_supported: true,
        default_bands: &["local_incidence_angle"],
        default_bands_regex: Some("[A-Za-z_]+"),
        default_rescale: Some("0,90"),
        default_colormap_name: Some("viridis"),
        aliases: &["rtc-s1-static", "rtc_s1_static", "rtc-static"],
    },
    ProductSpec {
        short_name: "OPERA_L2_CSLC-S1_V1",
        short_title: "CSLC-S1",
        title: "Coregistered Single-Look Complex from Sentinel-1 (Version 1)",
        description: "Coregistered Sentinel-1 Single-Look Complex (HDF5) interferometric radar bursts",
        backend: TitilerBackend::Xarray,
        raster_tiles_supported: false,
        default_bands: &[],
        default_bands_regex: None,
        default_rescale: None,
        default_colormap_name: None,
        aliases: &["cslc-s1", "cslc_s1", "cslc"],
    },
    ProductSpec {
        short_name: "OPERA_L2_CSLC-S1-STATIC_V1",
        short_title: "CSLC-S1-STATIC",
        title: "CSLC-S1 Static Layers (Version 1)",
        description: "Static geometry and line-of-sight layers (HDF5) for CSLC-S1",
        backend: TitilerBackend::Xarray,
        raster_tiles_supported: false,
        default_bands: &[],
        default_bands_regex: None,
        default_rescale: None,
        default_colormap_name: None,
        aliases: &["cslc-s1-static", "cslc_s1_static", "cslc-static"],
    },
    ProductSpec {
        short_name: "OPERA_L3_DISP-S1_V1",
        short_title: "DISP-S1",
        title: "Surface Displacement from Sentinel-1 (Version 1)",
        description: "Line-of-sight surface displacement and subsidence maps (NetCDF/HDF5) from Sentinel-1 InSAR",
        backend: TitilerBackend::Xarray,
        raster_tiles_supported: false,
        default_bands: &[],
        default_bands_regex: None,
        default_rescale: None,
        default_colormap_name: None,
        aliases: &["disp-s1", "disp_s1", "disp", "displacement", "subsidence"],
    },
];

impl ProductSpec {
    fn to_product(&self) -> OperaProduct {
        OperaProduct {
            short_name: self.short_name.to_string(),
            short_title: self.short_title.to_string(),
            title: self.title.to_string(),
            description: self.description.to_string(),
            backend: self.backend,
            raster_tiles_supported: self.raster_tiles_supported,
            default_bands: self
                .default_bands
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            default_bands_regex: self.default_bands_regex.map(ToString::to_string),
            default_rescale: self.default_rescale.map(ToString::to_string),
            default_colormap_name: self.default_colormap_name.map(ToString::to_string),
        }
    }
}

/// Returns all 9 supported NASA OPERA products in catalog order.
pub fn catalog() -> Vec<OperaProduct> {
    PRODUCT_SPECS.iter().map(ProductSpec::to_product).collect()
}

/// Looks up an OPERA product by exact CMR `short_name`, `short_title`, or alias (case-insensitive).
pub fn find_product(query: &str) -> Result<OperaProduct> {
    let q = query.trim();
    for spec in PRODUCT_SPECS {
        if spec.short_name.eq_ignore_ascii_case(q)
            || spec.short_title.eq_ignore_ascii_case(q)
            || spec.aliases.iter().any(|a| a.eq_ignore_ascii_case(q))
        {
            return Ok(spec.to_product());
        }
    }
    Err(OperaError::UnknownProduct(query.to_string()))
}

impl OperaProduct {
    /// Picks the best default band from a granule's available bands, falling back to the
    /// product's first preferred band or the granule's first band.
    pub fn choose_band(&self, available: &[String]) -> Option<String> {
        for preferred in &self.default_bands {
            if let Some(found) = available.iter().find(|b| b.eq_ignore_ascii_case(preferred)) {
                return Some(found.clone());
            }
        }
        self.default_bands
            .first()
            .cloned()
            .or_else(|| available.first().cloned())
    }
}

/// A ready-made `rio-tiler` band-math expression preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpressionPreset {
    pub id: String,
    pub label: String,
    pub expression: String,
    pub rescale: Option<String>,
    pub colormap_name: Option<String>,
}

/// Returns ready-made band-math expression presets for a product and band.
pub fn expression_presets(short_name: &str, band: &str) -> Vec<ExpressionPreset> {
    let upper_product = short_name.to_ascii_uppercase();
    let upper_band = band.to_ascii_uppercase();

    if upper_product.contains("RTC-S1") && matches!(upper_band.as_str(), "VV" | "VH" | "HH" | "HV")
    {
        return vec![ExpressionPreset {
            id: "db".into(),
            label: "Backscatter dB (10*log10)".into(),
            expression: "10*log10(b1)".into(),
            rescale: Some("-30,0".into()),
            colormap_name: Some("gray".into()),
        }];
    }
    if upper_product.contains("DSWX") && upper_band.contains("WTR") {
        return vec![
            ExpressionPreset {
                id: "open_water".into(),
                label: "Open-water mask (class 1)".into(),
                expression: "where(b1==1,1,0)".into(),
                rescale: Some("0,1".into()),
                colormap_name: Some("blues".into()),
            },
            ExpressionPreset {
                id: "surface_water".into(),
                label: "Surface-water mask (classes 1+2)".into(),
                expression: "where((b1==1)|(b1==2),1,0)".into(),
                rescale: Some("0,1".into()),
                colormap_name: Some("blues".into()),
            },
        ];
    }
    Vec::new()
}

/// Resolved rendering style passed to `titiler-cmr`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerStyle {
    /// Explicit categorical colormap JSON (`{"class": [r, g, b, a]}`).
    pub colormap: Option<String>,
    /// Named `titiler-cmr` colormap (e.g. `gray`, `terrain`, `viridis`, `blues`).
    pub colormap_name: Option<String>,
    /// Continuous stretch `"min,max"`.
    pub rescale: Option<String>,
}

impl LayerStyle {
    /// Resolves the default rendering style for a product/band with optional user overrides.
    ///
    /// Categorical DSWx water bands receive the official DSWx class palette (or water-only
    /// palette when `water_only` is true); DIST status bands receive the DIST status palette.
    /// Supplying a non-empty `rescale` or `colormap_name` overrides the categorical palette.
    pub fn for_band(
        short_name: &str,
        band: Option<&str>,
        water_only: bool,
        rescale_override: Option<&str>,
        colormap_override: Option<&str>,
    ) -> Self {
        let upper_prod = short_name.to_ascii_uppercase();
        let upper_band = band.unwrap_or_default().to_ascii_uppercase();
        let is_dswx = upper_prod.contains("DSWX");

        let mut style = if is_dswx && upper_band.contains("WTR") {
            Self {
                colormap: Some(
                    if water_only {
                        DSWX_WTR_WATER_ONLY_COLORMAP_JSON
                    } else {
                        DSWX_WTR_COLORMAP_JSON
                    }
                    .to_string(),
                ),
                colormap_name: None,
                rescale: None,
            }
        } else if upper_prod.contains("DIST") && upper_band.contains("STATUS") {
            Self {
                colormap: Some(DIST_STATUS_COLORMAP_JSON.to_string()),
                colormap_name: None,
                rescale: None,
            }
        } else if upper_band.contains("DEM") {
            Self::named("0,3000", "terrain")
        } else if matches!(upper_band.as_str(), "VV" | "VH" | "HH" | "HV") {
            Self::named("0,0.4", "gray")
        } else if is_dswx && upper_band.contains("CONF") {
            Self::named("0,100", "viridis")
        } else if upper_band.contains("INCIDENCE_ANGLE") {
            Self::named("0,90", "viridis")
        } else if upper_band.contains("ANOM") {
            Self::named("0,100", "ylorrd")
        } else if upper_band.ends_with("-DATE") {
            Self::named("0,1500", "viridis")
        } else if let Ok(prod) = find_product(short_name) {
            Self {
                colormap: None,
                colormap_name: prod.default_colormap_name,
                rescale: prod.default_rescale,
            }
        } else {
            Self::default()
        };

        let rescale = rescale_override.map(str::trim).filter(|s| !s.is_empty());
        let colormap = colormap_override.map(str::trim).filter(|s| !s.is_empty());
        if rescale.is_some() || colormap.is_some() {
            style.colormap = None;
            if let Some(r) = rescale {
                style.rescale = Some(r.to_string());
            }
            if let Some(c) = colormap {
                style.colormap_name = Some(c.to_ascii_lowercase());
            }
        }
        style
    }

    fn named(rescale: &str, colormap_name: &str) -> Self {
        Self {
            colormap: None,
            colormap_name: Some(colormap_name.to_string()),
            rescale: Some(rescale.to_string()),
        }
    }
}

/// Returns `true` when the product/band stores discrete classification codes.
pub fn is_categorical_band(short_name: &str, band: Option<&str>) -> bool {
    let Some(band) = band else {
        return false;
    };
    let p = short_name.to_ascii_uppercase();
    let b = band.to_ascii_uppercase();
    (p.contains("DSWX") && b.contains("WTR")) || (p.contains("DIST") && b.contains("STATUS"))
}

/// Returns `true` when the product/band is an OPERA DSWx water-classification layer.
pub fn is_dswx_water_band(short_name: &str, band: Option<&str>) -> bool {
    let Some(band) = band else {
        return false;
    };
    short_name.to_ascii_uppercase().contains("DSWX") && band.to_ascii_uppercase().contains("WTR")
}

/// Returns a human-readable class label for categorical OPERA bands (DSWx WTR or DIST STATUS).
pub fn class_label(short_name: &str, band: Option<&str>, class_value: i64) -> Option<&'static str> {
    let p = short_name.to_ascii_uppercase();
    let b = band.unwrap_or_default().to_ascii_uppercase();
    if p.contains("DSWX") && (b.is_empty() || b.contains("WTR")) {
        return match class_value {
            0 => Some("Not water"),
            1 => Some("Open water"),
            2 => Some("Partial surface water"),
            3 => Some("Inundated vegetation"),
            250 => Some("Hand-masked / terrain shadow"),
            251 => Some("Layover / shadow"),
            252 => Some("Snow / ice"),
            253 => Some("Cloud / cloud shadow"),
            254 => Some("Ocean masked"),
            255 => Some("No data / fill"),
            _ => None,
        };
    }
    if p.contains("DIST") && (b.is_empty() || b.contains("STATUS")) {
        return match class_value {
            0 => Some("No disturbance"),
            1 => Some("First detection (<50% loss)"),
            2 => Some("Provisional (<50% loss)"),
            3 => Some("Confirmed (<50% loss)"),
            4 => Some("First detection (>=50% loss)"),
            5 => Some("Provisional (>=50% loss)"),
            6 => Some("Confirmed (>=50% loss)"),
            7 => Some("Finished (<50% loss)"),
            8 => Some("Finished (>=50% loss)"),
            255 => Some("No data / fill"),
            _ => None,
        };
    }
    None
}

/// Legend entry `(label, [r, g, b, a])` for categorical previews.
pub fn legend_entries(
    short_name: &str,
    band: Option<&str>,
    water_only: bool,
) -> Vec<(&'static str, [u8; 4])> {
    let p = short_name.to_ascii_uppercase();
    let b = band.unwrap_or_default().to_ascii_uppercase();
    if p.contains("DSWX") && b.contains("WTR") {
        let mut entries = vec![
            ("Open water", [0, 0, 255, 255]),
            ("Partial water", [135, 206, 250, 255]),
        ];
        if !water_only {
            entries.push(("Snow / ice", [0, 255, 255, 255]));
            entries.push(("Cloud / shadow", [128, 128, 128, 255]));
            entries.push(("Ocean masked", [0, 0, 128, 255]));
        }
        return entries;
    }
    if p.contains("DIST") && b.contains("STATUS") {
        return vec![
            ("Low loss (<50%)", [254, 204, 92, 255]),
            ("Confirmed low", [253, 141, 60, 255]),
            ("High loss (>=50%)", [240, 59, 32, 255]),
            ("Confirmed high", [189, 0, 38, 255]),
        ];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_all_nine_products_and_aliases() {
        let all = catalog();
        assert_eq!(all.len(), 9);
        assert_eq!(
            find_product("dswx-hls").unwrap().short_name,
            "OPERA_L3_DSWX-HLS_V1"
        );
        assert_eq!(
            find_product("DSWX-S1").unwrap().short_name,
            "OPERA_L3_DSWX-S1_V1"
        );
        assert_eq!(
            find_product("rtc").unwrap().short_name,
            "OPERA_L2_RTC-S1_V1"
        );
        assert_eq!(
            find_product("disp-s1").unwrap().short_name,
            "OPERA_L3_DISP-S1_V1"
        );
        assert!(find_product("nonexistent").is_err());
    }

    #[test]
    fn resolves_categorical_and_continuous_styles() {
        let wtr = LayerStyle::for_band("OPERA_L3_DSWX-HLS_V1", Some("B01_WTR"), false, None, None);
        assert_eq!(wtr.colormap.as_deref(), Some(DSWX_WTR_COLORMAP_JSON));
        assert!(wtr.rescale.is_none());

        let water_only =
            LayerStyle::for_band("OPERA_L3_DSWX-S1_V1", Some("B01_WTR"), true, None, None);
        assert_eq!(
            water_only.colormap.as_deref(),
            Some(DSWX_WTR_WATER_ONLY_COLORMAP_JSON)
        );

        let overridden = LayerStyle::for_band(
            "OPERA_L3_DSWX-HLS_V1",
            Some("B01_WTR"),
            false,
            Some("0,2"),
            Some("Blues"),
        );
        assert!(overridden.colormap.is_none());
        assert_eq!(overridden.rescale.as_deref(), Some("0,2"));
        assert_eq!(overridden.colormap_name.as_deref(), Some("blues"));

        let dem = LayerStyle::for_band("OPERA_L3_DSWX-HLS_V1", Some("B10_DEM"), false, None, None);
        assert_eq!(dem.rescale.as_deref(), Some("0,3000"));
        assert_eq!(dem.colormap_name.as_deref(), Some("terrain"));

        assert!(is_categorical_band("OPERA_L3_DSWX-HLS_V1", Some("B01_WTR")));
        assert!(is_categorical_band(
            "OPERA_L3_DIST-ALERT-HLS_V1",
            Some("VEG-DIST-STATUS")
        ));
        assert!(!is_categorical_band("OPERA_L2_RTC-S1_V1", Some("VV")));
        assert_eq!(
            class_label("OPERA_L3_DSWX-HLS_V1", Some("B01_WTR"), 1),
            Some("Open water")
        );
        assert_eq!(expression_presets("OPERA_L2_RTC-S1_V1", "VV").len(), 1);
    }
}
