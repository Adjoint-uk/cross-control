//! Certificate generation errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CertgenError {
    #[error("certificate generation failed: {0}")]
    Generation(String),

    #[error("failed to write certificate: {0}")]
    Io(#[from] std::io::Error),
}
