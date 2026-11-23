//! Error types for `opera-rs`.

use thiserror::Error;

/// Unified error type returned by `opera-rs` operations.
#[derive(Debug, Error)]
pub enum OperaError {
    /// Caller supplied an invalid argument (e.g., malformed date, bbox, or parameter).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Requested OPERA product short name or alias was not recognized.
    #[error("unknown OPERA product '{0}'")]
    UnknownProduct(String),

    /// No matching granules, tiles, or observations were found for the query.
    #[error("no results found: {0}")]
    NoResults(String),

    /// Network or HTTP transport error while querying CMR, `titiler-cmr`, or Nominatim.
    #[error("HTTP error: {0}")]
    Http(String),

    /// JSON serialization or deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Image encoding or decoding error (PNG or GIF).
    #[error("image error: {0}")]
    Image(String),

    /// Geospatial raster/vector I/O or CRS transformation error via `terrio`.
    #[cfg(feature = "geo")]
    #[error("geospatial I/O error: {0}")]
    Geo(#[from] terrio::Error),

    /// Operation requires the `http` feature, which was disabled at compile time.
    #[error("HTTP support is disabled; rebuild `opera-rs` with the `http` feature enabled")]
    HttpDisabled,
}

/// Convenience result alias for `opera-rs`.
pub type Result<T> = std::result::Result<T, OperaError>;
