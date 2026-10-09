//! Platform-independent core of RunReCap: GPX parsing, Strava-export import,
//! the on-disk run library, filtering and geometry helpers.

pub mod activity;
pub mod geo;
pub mod gpx;
pub mod import;
pub mod library;

pub use activity::{Activity, ActivityKind, Point};
pub use import::ImportReport;
pub use library::Library;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid GPX: {0}")]
    Gpx(String),
    #[error("GPX file contains no track points")]
    NoTrack,
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
    #[error("library index error: {0}")]
    Index(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
