//! In-memory representation of a single file participating in a diff or merge.
//!
//! A [`DiffDocument`] is normalized to LF internally so the diff engine and
//! UI see a single line-ending convention. The original encoding and line
//! ending are recorded so that saving round-trips faithfully.

use std::path::{Path, PathBuf};

use crate::error::Result;

/// The line-ending convention detected in a loaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// `\n` everywhere.
    Lf,
    /// `\r\n` everywhere.
    CrLf,
    /// The file mixes `\n` and `\r\n`. On save we either preserve per-line
    /// (preferred) or convert and warn — see [`DiffDocument::save_to`].
    Mixed,
}

/// A single text file loaded for diff or merge.
///
/// The struct fields are public so the GUI can read them directly without
/// per-field accessors. Mutation should go through [`DiffDocument::set_content`]
/// so that downstream caches can be invalidated.
///
/// ## Example
///
/// ```no_run
/// use lgtm_core::DiffDocument;
/// let doc = DiffDocument::load("README.md")?;
/// assert!(!doc.is_binary);
/// # Ok::<(), lgtm_core::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct DiffDocument {
    /// Path the document was loaded from. May not exist on disk if the
    /// document was constructed via [`DiffDocument::empty_for`] (e.g. for
    /// `/dev/null` arguments from git).
    pub path: PathBuf,
    /// Encoding the file was decoded with. Used to re-encode on save.
    pub encoding: &'static encoding_rs::Encoding,
    /// Line ending convention detected at load time.
    pub line_ending: LineEnding,
    /// Whether the file is binary. Binary files have empty `content` and
    /// are surfaced to the UI as "Binary files differ".
    pub is_binary: bool,
    /// Decoded text content, with all line endings normalized to LF.
    ///
    /// For binary files this is the empty string.
    pub content: String,
    /// Original size on disk in bytes.
    pub size_bytes: u64,
}

impl DiffDocument {
    /// Load a file from disk, detecting encoding, line endings, and
    /// whether the content is binary.
    ///
    /// Returns [`Error::FileTooLarge`](crate::Error::FileTooLarge) if the
    /// file exceeds [`HARD_SIZE_LIMIT`].
    ///
    /// ## Example
    ///
    /// ```no_run
    /// # use lgtm_core::DiffDocument;
    /// let doc = DiffDocument::load("Cargo.toml")?;
    /// println!("loaded {} bytes from {}", doc.size_bytes, doc.path.display());
    /// # Ok::<(), lgtm_core::Error>(())
    /// ```
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let _ = path;
        todo!("step 2: implement encoding/line-ending/binary detection")
    }

    /// Construct an empty document that records `path` for display purposes.
    ///
    /// Used when git passes `/dev/null` for a newly added or deleted file:
    /// the other side renders as all-insert or all-delete.
    pub fn empty_for(path: impl AsRef<Path>) -> Self {
        let _ = path;
        todo!("step 2: trivial constructor")
    }

    /// Replace the document content (e.g. after a user edit) and invalidate
    /// any cached derived state.
    pub fn set_content(&mut self, content: String) {
        let _ = content;
        todo!("step 7: live editing")
    }

    /// Save the document to `path`, re-encoding to the original encoding and
    /// restoring the original line endings.
    ///
    /// For [`LineEnding::Mixed`] documents the implementation will preserve
    /// per-line endings if it tracked them at load, otherwise it writes LF
    /// and emits a `tracing::warn!`.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let _ = path;
        todo!("step 7: encoding/EOL-preserving save")
    }
}

/// Soft size threshold (bytes). Files larger than this should prompt the
/// user for confirmation before loading.
pub const SOFT_SIZE_LIMIT: u64 = 50 * 1024 * 1024;

/// Hard size threshold (bytes). Files larger than this are refused outright.
pub const HARD_SIZE_LIMIT: u64 = 500 * 1024 * 1024;
