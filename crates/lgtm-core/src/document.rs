//! In-memory representation of a single file participating in a diff or merge.
//!
//! A [`DiffDocument`] is normalized to LF internally so the diff engine and
//! UI see a single line-ending convention. The original encoding and line
//! ending are recorded so that saving round-trips faithfully.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The line-ending convention detected in a loaded file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// `\n` everywhere.
    Lf,
    /// `\r\n` everywhere.
    CrLf,
    /// The file mixes `\n` and `\r\n`. On save we convert to LF and emit a
    /// `tracing::warn!`.
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
    /// Returns [`Error::FileTooLarge`] if the file exceeds [`HARD_SIZE_LIMIT`].
    /// `/dev/null` (and a missing file at the same path) is treated as empty,
    /// so that git difftool can pass it for newly created or deleted files.
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
        let path = path.as_ref().to_path_buf();

        if path == Path::new("/dev/null") {
            return Ok(Self::empty_for(path));
        }

        let metadata = std::fs::metadata(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let size_bytes = metadata.len();

        if size_bytes > HARD_SIZE_LIMIT {
            return Err(Error::FileTooLarge {
                path,
                size: size_bytes,
                limit: HARD_SIZE_LIMIT,
            });
        }

        let bytes = std::fs::read(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;

        if content_inspector::inspect(&bytes).is_binary() {
            return Ok(Self {
                path,
                encoding: encoding_rs::UTF_8,
                line_ending: LineEnding::Lf,
                is_binary: true,
                content: String::new(),
                size_bytes,
            });
        }

        let mut detector = chardetng::EncodingDetector::new();
        detector.feed(&bytes, true);
        let encoding = detector.guess(None, true);

        let (decoded, _, _had_errors) = encoding.decode(&bytes);
        let raw = decoded.into_owned();

        let line_ending = detect_line_ending(&raw);
        let content = normalize_line_endings(raw);

        Ok(Self {
            path,
            encoding,
            line_ending,
            is_binary: false,
            content,
            size_bytes,
        })
    }

    /// Construct an empty document that records `path` for display purposes.
    ///
    /// Used when git passes `/dev/null` for a newly added or deleted file:
    /// the other side renders as all-insert or all-delete.
    pub fn empty_for(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            encoding: encoding_rs::UTF_8,
            line_ending: LineEnding::Lf,
            is_binary: false,
            content: String::new(),
            size_bytes: 0,
        }
    }

    /// Replace the document content (e.g. after a user edit).
    pub fn set_content(&mut self, content: String) {
        self.content = content;
    }

    /// Save the document to `path`, re-encoding to the original encoding and
    /// restoring the original line endings.
    ///
    /// [`LineEnding::Mixed`] documents are written as LF with a warning.
    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let restored = match self.line_ending {
            LineEnding::Lf => std::borrow::Cow::Borrowed(self.content.as_str()),
            LineEnding::CrLf => std::borrow::Cow::Owned(self.content.replace('\n', "\r\n")),
            LineEnding::Mixed => {
                tracing::warn!(
                    "{} had mixed line endings at load; saving as LF",
                    self.path.display()
                );
                std::borrow::Cow::Borrowed(self.content.as_str())
            }
        };
        let (encoded, _, _) = self.encoding.encode(restored.as_ref());
        std::fs::write(path, encoded.as_ref()).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }
}

/// Soft size threshold (bytes). Files larger than this should prompt the
/// user for confirmation before loading.
pub const SOFT_SIZE_LIMIT: u64 = 50 * 1024 * 1024;

/// Hard size threshold (bytes). Files larger than this are refused outright.
pub const HARD_SIZE_LIMIT: u64 = 500 * 1024 * 1024;

/// Detect line ending convention by counting `\r\n` versus bare `\n`.
fn detect_line_ending(text: &str) -> LineEnding {
    let bytes = text.as_bytes();
    let mut crlf = 0usize;
    let mut lf = 0usize;
    for i in 0..bytes.len() {
        if bytes[i] == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
    }
    match (crlf, lf) {
        (0, 0) | (0, _) => LineEnding::Lf,
        (_, 0) => LineEnding::CrLf,
        _ => LineEnding::Mixed,
    }
}

/// Normalize all line endings (`\r\n`, bare `\r`) to LF.
fn normalize_line_endings(text: String) -> String {
    if !text.contains('\r') {
        return text;
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join("lgtm-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn detect_lf() {
        assert_eq!(detect_line_ending("a\nb\nc"), LineEnding::Lf);
    }

    #[test]
    fn detect_crlf() {
        assert_eq!(detect_line_ending("a\r\nb\r\nc"), LineEnding::CrLf);
    }

    #[test]
    fn detect_mixed() {
        assert_eq!(detect_line_ending("a\r\nb\nc"), LineEnding::Mixed);
    }

    #[test]
    fn detect_empty() {
        assert_eq!(detect_line_ending(""), LineEnding::Lf);
    }

    #[test]
    fn detect_no_newlines() {
        assert_eq!(detect_line_ending("a single line"), LineEnding::Lf);
    }

    #[test]
    fn normalize_crlf_to_lf() {
        assert_eq!(normalize_line_endings("a\r\nb\r\n".into()), "a\nb\n");
    }

    #[test]
    fn normalize_bare_cr() {
        assert_eq!(normalize_line_endings("a\rb".into()), "a\nb");
    }

    #[test]
    fn normalize_already_lf_no_alloc() {
        let s: String = "no carriage returns here".into();
        let ptr = s.as_ptr();
        let out = normalize_line_endings(s);
        assert_eq!(
            out.as_ptr(),
            ptr,
            "must not reallocate when nothing changes"
        );
    }

    #[test]
    fn load_lf_file() {
        let p = write_tmp("lf.txt", b"hello\nworld\n");
        let doc = DiffDocument::load(&p).unwrap();
        assert_eq!(doc.content, "hello\nworld\n");
        assert_eq!(doc.line_ending, LineEnding::Lf);
        assert!(!doc.is_binary);
        assert_eq!(doc.size_bytes, 12);
    }

    #[test]
    fn load_crlf_file() {
        let p = write_tmp("crlf.txt", b"hello\r\nworld\r\n");
        let doc = DiffDocument::load(&p).unwrap();
        assert_eq!(doc.content, "hello\nworld\n", "must normalize to LF");
        assert_eq!(doc.line_ending, LineEnding::CrLf);
    }

    #[test]
    fn load_mixed_file() {
        let p = write_tmp("mixed.txt", b"a\r\nb\nc\r\n");
        let doc = DiffDocument::load(&p).unwrap();
        assert_eq!(doc.line_ending, LineEnding::Mixed);
        assert_eq!(doc.content, "a\nb\nc\n");
    }

    #[test]
    fn load_binary_file() {
        let p = write_tmp("bin.dat", &[0x00, 0x01, 0x02, 0xff, 0xfe, 0x00]);
        let doc = DiffDocument::load(&p).unwrap();
        assert!(doc.is_binary);
        assert!(doc.content.is_empty());
    }

    #[test]
    fn load_empty_file() {
        let p = write_tmp("empty.txt", b"");
        let doc = DiffDocument::load(&p).unwrap();
        assert!(!doc.is_binary);
        assert_eq!(doc.content, "");
        assert_eq!(doc.size_bytes, 0);
    }

    #[test]
    fn load_dev_null_treated_as_empty() {
        let doc = DiffDocument::load("/dev/null").unwrap();
        assert_eq!(doc.content, "");
        assert!(!doc.is_binary);
        assert_eq!(doc.size_bytes, 0);
    }

    #[test]
    fn load_missing_returns_io_error() {
        let err = DiffDocument::load("/this/path/does/not/exist/lgtm").unwrap_err();
        assert!(matches!(err, Error::Io { .. }));
    }

    #[test]
    fn save_roundtrip_lf() {
        let src = write_tmp("rt_lf_in.txt", b"hello\nworld\n");
        let doc = DiffDocument::load(&src).unwrap();
        let dst = std::env::temp_dir().join("lgtm-tests/rt_lf_out.txt");
        doc.save_to(&dst).unwrap();
        let raw = std::fs::read(&dst).unwrap();
        assert_eq!(raw, b"hello\nworld\n");
    }

    #[test]
    fn save_roundtrip_crlf() {
        let src = write_tmp("rt_crlf_in.txt", b"hello\r\nworld\r\n");
        let doc = DiffDocument::load(&src).unwrap();
        let dst = std::env::temp_dir().join("lgtm-tests/rt_crlf_out.txt");
        doc.save_to(&dst).unwrap();
        let raw = std::fs::read(&dst).unwrap();
        assert_eq!(raw, b"hello\r\nworld\r\n", "must restore CRLF on save");
    }

    #[test]
    fn save_roundtrip_utf8_bom_or_non_ascii() {
        // chardetng on these bytes should pick UTF-8; the round-trip must
        // preserve the bytes verbatim.
        let src = write_tmp("rt_utf8.txt", "héllo — wörld\n".as_bytes());
        let doc = DiffDocument::load(&src).unwrap();
        assert_eq!(doc.encoding, encoding_rs::UTF_8);
        let dst = std::env::temp_dir().join("lgtm-tests/rt_utf8_out.txt");
        doc.save_to(&dst).unwrap();
        let raw = std::fs::read(&dst).unwrap();
        assert_eq!(raw, "héllo — wörld\n".as_bytes());
    }

    #[test]
    fn empty_for_constructs_empty_doc() {
        let doc = DiffDocument::empty_for("foo.txt");
        assert_eq!(doc.path, PathBuf::from("foo.txt"));
        assert_eq!(doc.content, "");
        assert_eq!(doc.size_bytes, 0);
        assert!(!doc.is_binary);
    }
}
