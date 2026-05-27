//! Recursive folder comparison.
//!
//! Uses `ignore::WalkBuilder` to walk both trees, applies a cheap identity
//! check (size + mtime) before content comparison, and produces a flat,
//! sorted list of [`FolderEntry`]s.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::{Error, Result};

/// Per-file comparison outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderEntryStatus {
    /// Same on both sides.
    Identical,
    /// Present on both sides but content differs.
    Modified,
    /// Present only on the left.
    LeftOnly,
    /// Present only on the right.
    RightOnly,
    /// One side is a file and the other is a directory (or other type mismatch).
    TypeChanged,
    /// Both sides binary and the contents differ. Distinguished from
    /// [`Modified`](FolderEntryStatus::Modified) so the UI can disable
    /// double-click-to-diff.
    BinaryDiffers,
}

/// One row of a [`FolderDiff`]. Paths are relative to the respective root.
#[derive(Debug, Clone)]
pub struct FolderEntry {
    /// Path relative to [`FolderDiff::left_root`] / [`FolderDiff::right_root`].
    pub relative_path: PathBuf,
    /// Comparison status.
    pub status: FolderEntryStatus,
    /// Size on the left, if present.
    pub left_size: Option<u64>,
    /// Size on the right, if present.
    pub right_size: Option<u64>,
}

/// Result of comparing two directory trees.
#[derive(Debug, Clone)]
pub struct FolderDiff {
    /// Left root the comparison started from.
    pub left_root: PathBuf,
    /// Right root the comparison started from.
    pub right_root: PathBuf,
    /// Sorted, recursive list of entries.
    pub entries: Vec<FolderEntry>,
}

/// Options for [`FolderDiff::compute`].
#[derive(Debug, Clone)]
pub struct FolderDiffOptions {
    /// Follow symlinks (with cycle detection). Default: `false`.
    pub follow_symlinks: bool,
    /// Honor `.gitignore` / `.ignore` files. Default: `true`.
    pub respect_ignore: bool,
}

impl Default for FolderDiffOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            respect_ignore: true,
        }
    }
}

#[derive(Debug, Clone)]
struct EntryMeta {
    is_file: bool,
    size: u64,
    mtime: Option<SystemTime>,
}

impl FolderDiff {
    /// Compute the folder diff between two roots.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// # use lgtm_core::FolderDiff;
    /// # use lgtm_core::folder::FolderDiffOptions;
    /// let diff = FolderDiff::compute("./a", "./b", &FolderDiffOptions::default())?;
    /// println!("{} entries", diff.entries.len());
    /// # Ok::<(), lgtm_core::Error>(())
    /// ```
    pub fn compute(
        left: impl AsRef<Path>,
        right: impl AsRef<Path>,
        options: &FolderDiffOptions,
    ) -> Result<Self> {
        let left_root = left.as_ref().to_path_buf();
        let right_root = right.as_ref().to_path_buf();

        let left_index = walk(&left_root, options)?;
        let right_index = walk(&right_root, options)?;

        let mut all_keys: BTreeSet<PathBuf> = BTreeSet::new();
        for k in left_index.keys() {
            all_keys.insert(k.clone());
        }
        for k in right_index.keys() {
            all_keys.insert(k.clone());
        }

        let mut entries = Vec::with_capacity(all_keys.len());
        for rel in all_keys {
            let l = left_index.get(&rel);
            let r = right_index.get(&rel);
            let status = determine_status(&left_root, &right_root, &rel, l, r);
            entries.push(FolderEntry {
                relative_path: rel,
                status,
                left_size: l.map(|m| m.size),
                right_size: r.map(|m| m.size),
            });
        }

        Ok(Self {
            left_root,
            right_root,
            entries,
        })
    }
}

fn walk(root: &Path, options: &FolderDiffOptions) -> Result<HashMap<PathBuf, EntryMeta>> {
    let mut out: HashMap<PathBuf, EntryMeta> = HashMap::new();
    let walker = ignore::WalkBuilder::new(root)
        .follow_links(options.follow_symlinks)
        .git_ignore(options.respect_ignore)
        .git_exclude(options.respect_ignore)
        .ignore(options.respect_ignore)
        .standard_filters(options.respect_ignore)
        .build();

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => return Err(Error::Walk(e.to_string())),
        };
        let path = entry.path();
        if path == root {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(p) => p.to_path_buf(),
            Err(_) => continue,
        };
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => return Err(Error::Walk(e.to_string())),
        };
        if meta.is_dir() {
            continue;
        }
        out.insert(
            rel,
            EntryMeta {
                is_file: meta.is_file(),
                size: meta.len(),
                mtime: meta.modified().ok(),
            },
        );
    }
    Ok(out)
}

fn determine_status(
    left_root: &Path,
    right_root: &Path,
    rel: &Path,
    l: Option<&EntryMeta>,
    r: Option<&EntryMeta>,
) -> FolderEntryStatus {
    match (l, r) {
        (None, None) => FolderEntryStatus::Identical,
        (Some(_), None) => FolderEntryStatus::LeftOnly,
        (None, Some(_)) => FolderEntryStatus::RightOnly,
        (Some(lm), Some(rm)) => {
            if lm.is_file != rm.is_file {
                return FolderEntryStatus::TypeChanged;
            }
            if lm.size != rm.size {
                return classify_modified(left_root, right_root, rel);
            }
            // Same size: cheap mtime check, then byte compare as last resort.
            if let (Some(a), Some(b)) = (lm.mtime, rm.mtime) {
                if a == b {
                    return FolderEntryStatus::Identical;
                }
            }
            if files_byte_equal(&left_root.join(rel), &right_root.join(rel)) {
                FolderEntryStatus::Identical
            } else {
                classify_modified(left_root, right_root, rel)
            }
        }
    }
}

fn classify_modified(left_root: &Path, right_root: &Path, rel: &Path) -> FolderEntryStatus {
    let left = left_root.join(rel);
    let right = right_root.join(rel);
    if is_binary(&left) || is_binary(&right) {
        FolderEntryStatus::BinaryDiffers
    } else {
        FolderEntryStatus::Modified
    }
}

fn is_binary(p: &Path) -> bool {
    let mut buf = [0u8; 1024];
    match std::fs::File::open(p) {
        Ok(mut f) => {
            use std::io::Read;
            let n = f.read(&mut buf).unwrap_or(0);
            content_inspector::inspect(&buf[..n]).is_binary()
        }
        Err(_) => false,
    }
}

fn files_byte_equal(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(av), Ok(bv)) => av == bv,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("lgtm-folder-{name}-{nanos}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, content: &[u8]) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    fn opts() -> FolderDiffOptions {
        FolderDiffOptions {
            follow_symlinks: false,
            respect_ignore: false,
        }
    }

    #[test]
    fn identical_trees_are_identical() {
        let l = unique_dir("identical-l");
        let r = unique_dir("identical-r");
        write(&l.join("a.txt"), b"hello\n");
        write(&l.join("sub/b.txt"), b"world\n");
        write(&r.join("a.txt"), b"hello\n");
        write(&r.join("sub/b.txt"), b"world\n");
        let d = FolderDiff::compute(&l, &r, &opts()).unwrap();
        assert!(
            d.entries
                .iter()
                .all(|e| e.status == FolderEntryStatus::Identical)
        );
    }

    #[test]
    fn left_only_and_right_only_detected() {
        let l = unique_dir("lo-l");
        let r = unique_dir("lo-r");
        write(&l.join("only_left.txt"), b"x");
        write(&r.join("only_right.txt"), b"y");
        let d = FolderDiff::compute(&l, &r, &opts()).unwrap();
        assert_eq!(d.entries.len(), 2);
        let by: std::collections::HashMap<_, _> = d
            .entries
            .iter()
            .map(|e| (e.relative_path.clone(), e.status))
            .collect();
        assert_eq!(
            by[&PathBuf::from("only_left.txt")],
            FolderEntryStatus::LeftOnly
        );
        assert_eq!(
            by[&PathBuf::from("only_right.txt")],
            FolderEntryStatus::RightOnly
        );
    }

    #[test]
    fn modified_text_file_classified_modified() {
        let l = unique_dir("mod-l");
        let r = unique_dir("mod-r");
        write(&l.join("f.txt"), b"alpha\n");
        write(&r.join("f.txt"), b"alpha\nplus\n");
        let d = FolderDiff::compute(&l, &r, &opts()).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].status, FolderEntryStatus::Modified);
        assert_eq!(d.entries[0].left_size, Some(6));
        assert_eq!(d.entries[0].right_size, Some(11));
    }

    #[test]
    fn modified_binary_classified_binary_differs() {
        let l = unique_dir("bin-l");
        let r = unique_dir("bin-r");
        write(&l.join("b.dat"), &[0u8, 1, 2, 0xff]);
        write(&r.join("b.dat"), &[0u8, 1, 2, 0xff, 0xfe]);
        let d = FolderDiff::compute(&l, &r, &opts()).unwrap();
        assert_eq!(d.entries[0].status, FolderEntryStatus::BinaryDiffers);
    }

    #[test]
    fn same_size_different_content_caught_by_byte_compare() {
        let l = unique_dir("ss-l");
        let r = unique_dir("ss-r");
        // Same size on both sides; mtimes will differ enough to force the
        // byte-compare path.
        write(&l.join("f.txt"), b"abcde");
        std::thread::sleep(std::time::Duration::from_millis(10));
        write(&r.join("f.txt"), b"ABCDE");
        let d = FolderDiff::compute(&l, &r, &opts()).unwrap();
        assert_eq!(d.entries[0].status, FolderEntryStatus::Modified);
    }

    #[test]
    fn entries_are_sorted() {
        let l = unique_dir("sort-l");
        let r = unique_dir("sort-r");
        write(&l.join("z.txt"), b"z");
        write(&l.join("a.txt"), b"a");
        write(&l.join("m.txt"), b"m");
        write(&r.join("a.txt"), b"a");
        let d = FolderDiff::compute(&l, &r, &opts()).unwrap();
        let paths: Vec<_> = d.entries.iter().map(|e| e.relative_path.clone()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }
}
