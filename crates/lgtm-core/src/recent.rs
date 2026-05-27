// Rust 2024 made env::set_var unsafe; we only use it in a single test
// (config_dir_respects_lgtm_config_dir_env) but the crate-root forbids
// unsafe by default, so opt in here.
#![allow(unsafe_code)]
//! Persistent "recently opened" list for the menubar's
//! **File → Open Recent** submenu.
//!
//! Stored as a tiny TSV in the platform's per-user config dir:
//!
//! - Linux: `$XDG_CONFIG_HOME/lgtm/recent.tsv` or `$HOME/.config/lgtm/recent.tsv`
//! - macOS: `$HOME/Library/Application Support/lgtm/recent.tsv`
//! - Windows: `%APPDATA%\lgtm\recent.tsv`
//!
//! Each line is `mode<TAB>left<TAB>right<NL>` where `mode` is `file` or
//! `folder`. Capped at [`MAX_RECENT`] entries; older entries are dropped
//! on push.
//!
//! All errors are treated as routine — a missing or unreadable file just
//! yields an empty list rather than failing the GUI startup.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use crate::error::Result;

/// Maximum number of recent entries kept on disk.
pub const MAX_RECENT: usize = 10;

/// Whether an entry refers to two files (diff mode) or two folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecentMode {
    /// Two-file diff (`lgtm a b`).
    File,
    /// Two-folder diff (`lgtm --dir a b`).
    Folder,
}

impl RecentMode {
    fn as_str(self) -> &'static str {
        match self {
            RecentMode::File => "file",
            RecentMode::Folder => "folder",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            _ => None,
        }
    }
}

/// One row in the recent-files list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    /// Mode (file/folder).
    pub mode: RecentMode,
    /// Left side path.
    pub left: PathBuf,
    /// Right side path.
    pub right: PathBuf,
}

impl RecentEntry {
    /// Construct an entry. The path strings should already be absolute
    /// (callers usually canonicalize via [`std::fs::canonicalize`]).
    pub fn new(mode: RecentMode, left: PathBuf, right: PathBuf) -> Self {
        Self { mode, left, right }
    }
}

/// In-memory cache of the recent-files list, backed by a file on disk.
///
/// Load with [`Self::load`], mutate with [`Self::push`], and (optionally)
/// persist with [`Self::save`]. `push` does **not** auto-save; call
/// [`Self::save`] when convenient — the GUI saves on every push so a
/// crash never loses a fresh entry.
#[derive(Debug, Default, Clone)]
pub struct RecentList {
    entries: VecDeque<RecentEntry>,
}

impl RecentList {
    /// Read the on-disk list. A missing or malformed file yields an
    /// empty list (silent fallback — recent files are a nice-to-have,
    /// never block the GUI on them).
    pub fn load() -> Self {
        let Some(path) = default_path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    /// Read from an explicit path. Returns an empty list on any error.
    pub fn load_from(path: &Path) -> Self {
        let mut out = Self::default();
        let Ok(body) = std::fs::read_to_string(path) else {
            return out;
        };
        for line in body.lines() {
            if let Some(entry) = parse_line(line) {
                out.entries.push_back(entry);
                if out.entries.len() >= MAX_RECENT {
                    break;
                }
            }
        }
        out
    }

    /// Persist the list to the default path. Creates parent dirs as
    /// needed. Silent success when the platform has no config dir;
    /// returns an error only when actual IO fails.
    pub fn save(&self) -> Result<()> {
        let Some(path) = default_path() else {
            return Ok(());
        };
        self.save_to(&path)
    }

    /// Persist to an explicit path. Creates parent dirs as needed.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| crate::error::Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut body = String::new();
        for e in &self.entries {
            body.push_str(e.mode.as_str());
            body.push('\t');
            body.push_str(&e.left.display().to_string());
            body.push('\t');
            body.push_str(&e.right.display().to_string());
            body.push('\n');
        }
        std::fs::write(path, body).map_err(|source| crate::error::Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// Borrow the entries in most-recent-first order.
    pub fn entries(&self) -> impl Iterator<Item = &RecentEntry> {
        self.entries.iter()
    }

    /// True when the list has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Push a new entry to the front. If an entry with the same
    /// `(mode, left, right)` already exists it's removed first so the
    /// pushed copy ends up at position 0 (LRU). The list is then
    /// truncated to [`MAX_RECENT`].
    pub fn push(&mut self, entry: RecentEntry) {
        self.entries.retain(|e| e != &entry);
        self.entries.push_front(entry);
        while self.entries.len() > MAX_RECENT {
            self.entries.pop_back();
        }
    }

    /// Clear the list. Caller should follow with [`Self::save`] to persist.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn parse_line(line: &str) -> Option<RecentEntry> {
    let mut parts = line.splitn(3, '\t');
    let mode = RecentMode::parse(parts.next()?)?;
    let left = PathBuf::from(parts.next()?);
    let right = PathBuf::from(parts.next()?);
    if left.as_os_str().is_empty() || right.as_os_str().is_empty() {
        return None;
    }
    Some(RecentEntry { mode, left, right })
}

/// Resolve the platform's per-user config directory for lgtm. Returns
/// `None` if no relevant env var is set (extremely rare).
fn default_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("lgtm").join("recent.tsv"))
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("LGTM_CONFIG_DIR") {
        if !v.is_empty() {
            return Some(PathBuf::from(v));
        }
    }
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        // Linux + other unix
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg));
            }
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("lgtm-recent-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("recent.tsv")
    }

    fn entry(mode: RecentMode, left: &str, right: &str) -> RecentEntry {
        RecentEntry::new(mode, PathBuf::from(left), PathBuf::from(right))
    }

    #[test]
    fn load_from_missing_file_yields_empty() {
        let list = RecentList::load_from(Path::new("/no/such/file/lgtm-recent.tsv"));
        assert!(list.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = tmp_path("round-trip");
        let mut list = RecentList::default();
        list.push(entry(RecentMode::File, "/a/b.txt", "/c/d.txt"));
        list.push(entry(RecentMode::Folder, "/dir1", "/dir2"));
        list.save_to(&path).unwrap();

        let loaded = RecentList::load_from(&path);
        let got: Vec<_> = loaded.entries().cloned().collect();
        // Most-recent-first ordering preserved.
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].mode, RecentMode::Folder);
        assert_eq!(got[0].left, PathBuf::from("/dir1"));
        assert_eq!(got[1].mode, RecentMode::File);
        assert_eq!(got[1].left, PathBuf::from("/a/b.txt"));
    }

    #[test]
    fn push_moves_existing_entry_to_front() {
        let mut list = RecentList::default();
        list.push(entry(RecentMode::File, "a", "b"));
        list.push(entry(RecentMode::File, "c", "d"));
        list.push(entry(RecentMode::File, "a", "b")); // duplicate
        let got: Vec<_> = list.entries().cloned().collect();
        assert_eq!(got.len(), 2, "duplicate must dedupe");
        assert_eq!(got[0].left, PathBuf::from("a"));
        assert_eq!(got[1].left, PathBuf::from("c"));
    }

    #[test]
    fn push_caps_at_max_recent() {
        let mut list = RecentList::default();
        for i in 0..(MAX_RECENT + 5) {
            list.push(entry(
                RecentMode::File,
                &format!("/left/{i}"),
                &format!("/right/{i}"),
            ));
        }
        assert_eq!(list.entries().count(), MAX_RECENT);
        // Newest entry is the most recent push.
        assert_eq!(
            list.entries().next().unwrap().left,
            PathBuf::from(format!("/left/{}", MAX_RECENT + 4))
        );
    }

    #[test]
    fn load_skips_malformed_lines() {
        let path = tmp_path("malformed");
        std::fs::write(
            &path,
            "file\t/a\t/b\n\
             bogus-mode\t/x\t/y\n\
             folder\t/d1\t/d2\n\
             \n\
             file\t\t/empty-left-bad\n",
        )
        .unwrap();
        let list = RecentList::load_from(&path);
        let got: Vec<_> = list.entries().cloned().collect();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].mode, RecentMode::File);
        assert_eq!(got[0].left, PathBuf::from("/a"));
        assert_eq!(got[1].mode, RecentMode::Folder);
    }

    #[test]
    fn save_creates_parent_dirs() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let deep = std::env::temp_dir()
            .join(format!("lgtm-recent-deep-{nanos}"))
            .join("a")
            .join("b")
            .join("c")
            .join("recent.tsv");
        let mut list = RecentList::default();
        list.push(entry(RecentMode::File, "/x", "/y"));
        list.save_to(&deep).unwrap();
        assert!(deep.exists());
    }

    #[test]
    fn clear_empties_the_list() {
        let mut list = RecentList::default();
        list.push(entry(RecentMode::File, "/a", "/b"));
        list.push(entry(RecentMode::File, "/c", "/d"));
        list.clear();
        assert!(list.is_empty());
    }

    #[test]
    fn config_dir_respects_lgtm_config_dir_env() {
        // SAFETY: set_var is unsafe in Rust 2024. Single-threaded test.
        let saved = std::env::var_os("LGTM_CONFIG_DIR");
        unsafe {
            std::env::set_var("LGTM_CONFIG_DIR", "/some/explicit/dir");
        }
        let d = config_dir();
        unsafe {
            match saved {
                Some(v) => std::env::set_var("LGTM_CONFIG_DIR", v),
                None => std::env::remove_var("LGTM_CONFIG_DIR"),
            }
        }
        assert_eq!(d, Some(PathBuf::from("/some/explicit/dir")));
    }
}
