//! Error type for `lgtm-core`.
//!
//! Library APIs return [`Result<T>`]. Application code in `lgtm-cli`
//! converts these into process exit codes (`2` on any [`Error`]).

use std::path::PathBuf;

/// Result alias used throughout `lgtm-core`.
pub type Result<T> = std::result::Result<T, Error>;

/// All errors produced by `lgtm-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file at `path` could not be opened or read.
    #[error("failed to read {path}: {source}")]
    Io {
        /// The path that was being read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The file exceeds the hard size limit (500 MB) and was refused.
    #[error("{path} is {size} bytes; lgtm refuses to load files larger than {limit} bytes")]
    FileTooLarge {
        /// The offending path.
        path: PathBuf,
        /// Observed size in bytes.
        size: u64,
        /// Configured hard limit in bytes.
        limit: u64,
    },

    /// The user declined to load a large (>50 MB) file at the confirm prompt.
    ///
    /// Surfaced by callers that wrap [`DiffDocument::load`](crate::DiffDocument)
    /// with a confirmation step.
    #[error("loading {path} was declined")]
    LoadDeclined {
        /// The path the user declined.
        path: PathBuf,
    },

    /// Encoding conversion failed irrecoverably (extremely rare with
    /// `encoding_rs`, which is lossy-tolerant).
    #[error("could not decode {path} as {encoding}")]
    Decode {
        /// The path that could not be decoded.
        path: PathBuf,
        /// The encoding label that was attempted.
        encoding: String,
    },

    /// A merge operation was asked to compute over inputs of mismatched
    /// shape (for instance, a binary file passed to text merge).
    #[error("three-way merge inputs are invalid: {0}")]
    InvalidMergeInputs(String),

    /// A folder walk failed.
    #[error("folder walk failed: {0}")]
    Walk(String),
}
