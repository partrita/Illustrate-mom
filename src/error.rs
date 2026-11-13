use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur while parsing inputs, reading PDB files, or rendering.
#[derive(Debug, Error)]
pub enum IllustrateError {
    /// I/O error while reading inputs or writing rendered images.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Referenced PDB file could not be located in any search directory.
    #[error("PDB file not found: {path}")]
    PdbNotFound {
        /// Requested PDB path.
        path: PathBuf,
    },

    /// Malformed command card or numeric parameter line in the command deck.
    #[error("invalid command card at line {line}: {message}")]
    InvalidCommand {
        /// 1-based line number in the command input.
        line: usize,
        /// Description of the parsing error.
        message: String,
    },

    /// Malformed atom selection/rendering card inside a `read` command block.
    #[error("invalid atom descriptor at line {line}: {message}")]
    InvalidDescriptor {
        /// 1-based line number in the command input.
        line: usize,
        /// Description of the descriptor error.
        message: String,
    },

    /// Malformed record inside a PDB coordinate file.
    #[error("invalid PDB record at line {line}: {message}")]
    InvalidPdb {
        /// 1-based line number in the PDB file.
        line: usize,
        /// Description of the PDB parsing error.
        message: String,
    },

    /// Atom radius multiplied by scale exceeds the spherical map limit (100 pixels).
    #[error("atom radius * scale ({scaled_radius}) exceeds maximum limit of 100 pixels")]
    RadiusTooLarge {
        /// Computed `radius * scale` value.
        scaled_radius: f32,
    },

    /// Attempted to write an output image with non-positive dimensions.
    #[error("invalid image dimensions: {width}x{height}")]
    InvalidDimensions {
        /// Image width in pixels (`iysize`).
        width: usize,
        /// Image height in pixels (`ixsize`).
        height: usize,
    },

    /// PNG encoding failure.
    #[error("PNG encoding error: {0}")]
    PngEncode(#[from] png::EncodingError),
}

/// Convenience result alias for the `illustrate` crate.
pub type Result<T> = std::result::Result<T, IllustrateError>;
