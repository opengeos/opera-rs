//! Pure-Rust library for searching, visualizing, and analyzing NASA OPERA satellite products
//! via NASA CMR and `titiler-cmr`.

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
