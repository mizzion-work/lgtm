//! `git blame` cache for the diff view.
//!
//! [`BlameCache`] resolves blame for files that live inside a git
//! repository, caches the result keyed by `(repo_root, file_path)`, and
//! exposes a `get(line)` lookup the GUI hits on hover.
//!
//! ## Silent-fallback policy
//!
//! Every routine "blame just isn't available" case is reported as `Ok(())`
//! with the cache left empty for that file:
//! - the file isn't inside a git working tree
//! - the file is inside a working tree but untracked
//! - the file exists but its blame can't be computed for any other reason
//!   (the call returns `Ok(())` and `get` returns `None`)
//!
//! [`Error::Git`](crate::Error::Git) is returned only when something is
//! genuinely wrong with the repository (corrupt index, IO error, etc.).
//!
//! Files larger than [`BLAME_LINE_CAP`] lines are deliberately skipped to
//! keep the hover latency bounded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeZone};

use crate::error::Result;

/// Skip blame on files larger than this many lines.
pub const BLAME_LINE_CAP: usize = 50_000;

/// Blame metadata for a single line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlameInfo {
    /// Full 40-char commit SHA. Truncate at display time (`&sha[..7]`).
    pub commit_sha: String,
    /// Author name as recorded by git.
    pub author_name: String,
    /// Author email as recorded by git.
    pub author_email: String,
    /// Commit timestamp in its native timezone.
    pub commit_time: DateTime<FixedOffset>,
    /// First line of the commit message.
    pub summary: String,
}

impl BlameInfo {
    /// 7-char short SHA, suitable for tooltip display.
    pub fn short_sha(&self) -> &str {
        let end = self.commit_sha.len().min(7);
        &self.commit_sha[..end]
    }

    /// Format the commit time as `YYYY-MM-DD` in its native timezone.
    pub fn date_string(&self) -> String {
        self.commit_time.format("%Y-%m-%d").to_string()
    }

    /// Format the commit time relative to `now`: "just now", "yesterday",
    /// "3 months ago", etc.
    pub fn relative_to(&self, now: DateTime<FixedOffset>) -> String {
        let delta = now.signed_duration_since(self.commit_time);
        let secs = delta.num_seconds();
        if secs < 60 {
            "just now".into()
        } else if secs < 3600 {
            format!("{} minute{} ago", secs / 60, plural(secs / 60))
        } else if secs < 86_400 {
            format!("{} hour{} ago", secs / 3600, plural(secs / 3600))
        } else if secs < 86_400 * 2 {
            "yesterday".into()
        } else if secs < 86_400 * 30 {
            format!("{} day{} ago", secs / 86_400, plural(secs / 86_400))
        } else if secs < 86_400 * 365 {
            let months = secs / (86_400 * 30);
            format!("{months} month{} ago", plural(months))
        } else {
            let years = secs / (86_400 * 365);
            format!("{years} year{} ago", plural(years))
        }
    }

    /// First line of the commit summary, truncated with an ellipsis.
    pub fn short_summary(&self, max_chars: usize) -> String {
        if self.summary.chars().count() <= max_chars {
            self.summary.clone()
        } else {
            let head: String = self.summary.chars().take(max_chars - 1).collect();
            format!("{head}…")
        }
    }
}

fn plural(n: i64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Internal mutex-protected state. Cloneable handles via `Arc` so the
/// background loader thread can write back while the UI thread reads.
#[derive(Default)]
struct BlameInner {
    loaded: HashMap<(PathBuf, PathBuf), Vec<Option<BlameInfo>>>,
    misses: std::collections::HashSet<(PathBuf, PathBuf)>,
    /// Files for which a background load is currently in flight. Lets
    /// `request` no-op on the second hover before the first finishes
    /// and lets the UI render a "loading…" state.
    loading: std::collections::HashSet<(PathBuf, PathBuf)>,
}

/// Cache of blame data keyed by `(repo_root, file_path)`.
///
/// Hold one instance per [`DiffApp`](crate) (one for each window).
/// Internally uses `Arc<Mutex<_>>` so [`BlameCache::request`] can hand
/// the cache to a background thread for non-blocking loads.
#[derive(Default, Clone)]
pub struct BlameCache {
    inner: std::sync::Arc<std::sync::Mutex<BlameInner>>,
}

impl BlameCache {
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Synchronous load — blocks until blame is computed. Used by tests
    /// and any caller that wants a deterministic result; the GUI uses
    /// [`Self::request`] instead so the UI thread stays responsive.
    pub fn load(&self, repo_root: &Path, file_path: &Path) -> Result<()> {
        let key = (repo_root.to_path_buf(), file_path.to_path_buf());
        {
            let inner = self.inner.lock().unwrap();
            if inner.loaded.contains_key(&key) || inner.misses.contains(&key) {
                return Ok(());
            }
        }
        let outcome = compute_blame(&key.0, &key.1);
        self.commit_outcome(key, outcome);
        Ok(())
    }

    /// Non-blocking load: spawn a worker thread (if one isn't already
    /// running for this key) and call `on_ready` when the cache state
    /// for this key changes. The GUI passes a closure that captures
    /// `egui::Context::request_repaint` so the tooltip refreshes once
    /// blame is available.
    pub fn request<F>(&self, repo_root: &Path, file_path: &Path, on_ready: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let key = (repo_root.to_path_buf(), file_path.to_path_buf());
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.loaded.contains_key(&key)
                || inner.misses.contains(&key)
                || inner.loading.contains(&key)
            {
                return;
            }
            inner.loading.insert(key.clone());
        }
        let cache = self.clone();
        std::thread::Builder::new()
            .name(format!("lgtm-blame-{}", key.1.display()))
            .spawn(move || {
                let outcome = compute_blame(&key.0, &key.1);
                {
                    let mut inner = cache.inner.lock().unwrap();
                    inner.loading.remove(&key);
                }
                cache.commit_outcome(key, outcome);
                on_ready();
            })
            .ok();
    }

    fn commit_outcome(&self, key: (PathBuf, PathBuf), outcome: BlameOutcome) {
        let mut inner = self.inner.lock().unwrap();
        match outcome {
            BlameOutcome::Loaded(spans) => {
                inner.loaded.insert(key, spans);
            }
            BlameOutcome::Miss => {
                inner.misses.insert(key);
            }
        }
    }

    /// Look up blame for `line` (1-indexed). Returns `None` if the file
    /// hasn't been loaded or the line is out of range.
    pub fn get(&self, repo_root: &Path, file_path: &Path, line: usize) -> Option<BlameInfo> {
        let inner = self.inner.lock().unwrap();
        let key = (repo_root.to_path_buf(), file_path.to_path_buf());
        inner
            .loaded
            .get(&key)
            .and_then(|v| v.get(line))
            .and_then(|opt| opt.clone())
    }

    /// `true` while a background load is in flight for this key.
    pub fn is_loading(&self, repo_root: &Path, file_path: &Path) -> bool {
        let inner = self.inner.lock().unwrap();
        let key = (repo_root.to_path_buf(), file_path.to_path_buf());
        inner.loading.contains(&key)
    }

    /// Whether `load`/`request` has been attempted (successfully or not).
    pub fn was_attempted(&self, repo_root: &Path, file_path: &Path) -> bool {
        let inner = self.inner.lock().unwrap();
        let key = (repo_root.to_path_buf(), file_path.to_path_buf());
        inner.loaded.contains_key(&key) || inner.misses.contains(&key)
    }
}

/// Outcome of a blame computation — internal to the load pipeline.
enum BlameOutcome {
    Loaded(Vec<Option<BlameInfo>>),
    Miss,
}

/// The pure blame computation, free of cache locks. Safe to call from
/// any thread; only touches git2 and the filesystem.
fn compute_blame(repo_root: &Path, file_path: &Path) -> BlameOutcome {
    let Ok(repo) = git2::Repository::discover(repo_root) else {
        return BlameOutcome::Miss;
    };
    let Some(workdir) = repo.workdir().map(Path::to_path_buf) else {
        return BlameOutcome::Miss;
    };
    let Some(relative) = resolve_relative(&workdir, file_path).map(PathBuf::from) else {
        return BlameOutcome::Miss;
    };
    let abs = workdir.join(&relative);
    if let Ok(meta) = std::fs::metadata(&abs) {
        if meta.len() > (BLAME_LINE_CAP as u64) * 256 {
            return BlameOutcome::Miss;
        }
    }
    let line_count = std::fs::read_to_string(&abs)
        .ok()
        .map(|s| s.lines().count())
        .unwrap_or(0);
    if line_count == 0 || line_count > BLAME_LINE_CAP {
        return BlameOutcome::Miss;
    }
    let blame = match repo.blame_file(&relative, None) {
        Ok(b) => b,
        Err(_) => return BlameOutcome::Miss,
    };
    let mut per_line: Vec<Option<BlameInfo>> = vec![None; line_count + 1];
    for hunk in blame.iter() {
        let start = hunk.final_start_line();
        let len = hunk.lines_in_hunk();
        let oid = hunk.final_commit_id();
        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let author = commit.author();
        let info = BlameInfo {
            commit_sha: oid.to_string(),
            author_name: author.name().unwrap_or("").to_string(),
            author_email: author.email().unwrap_or("").to_string(),
            commit_time: git2_time_to_chrono(commit.time()),
            summary: commit.summary().unwrap_or("").to_string(),
        };
        for i in 0..len {
            let line_idx = start + i;
            if line_idx < per_line.len() {
                per_line[line_idx] = Some(info.clone());
            }
        }
    }
    BlameOutcome::Loaded(per_line)
}

fn resolve_relative(workdir: &Path, file_path: &Path) -> Option<String> {
    if let Ok(rel) = file_path.strip_prefix(workdir) {
        return Some(rel.to_string_lossy().into_owned());
    }
    let canon_work = std::fs::canonicalize(workdir).ok()?;
    let canon_file = std::fs::canonicalize(file_path).ok()?;
    canon_file
        .strip_prefix(&canon_work)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

fn git2_time_to_chrono(t: git2::Time) -> DateTime<FixedOffset> {
    let secs = t.seconds();
    let offset_seconds = t.offset_minutes() * 60;
    let tz = FixedOffset::east_opt(offset_seconds).unwrap_or(FixedOffset::east_opt(0).unwrap());
    tz.timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(|| tz.timestamp_opt(0, 0).single().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::process::Command;

    fn unique_repo(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("lgtm-blame-{name}-{nanos}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn git(cwd: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Alice")
            .env("GIT_AUTHOR_EMAIL", "alice@example.com")
            .env("GIT_COMMITTER_NAME", "Alice")
            .env("GIT_COMMITTER_EMAIL", "alice@example.com")
            .env("HOME", cwd)
            .output()
            .expect("git failed to spawn");
        if !out.status.success() {
            panic!(
                "git {args:?} failed: stderr={}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    fn have_git() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn short_sha_truncates_at_7() {
        let info = BlameInfo {
            commit_sha: "abcdef0123456789".into(),
            author_name: "x".into(),
            author_email: "x".into(),
            commit_time: chrono::Utc::now().fixed_offset(),
            summary: "s".into(),
        };
        assert_eq!(info.short_sha(), "abcdef0");
    }

    #[test]
    fn short_sha_handles_unusually_short_input() {
        let info = BlameInfo {
            commit_sha: "abc".into(),
            author_name: "x".into(),
            author_email: "x".into(),
            commit_time: chrono::Utc::now().fixed_offset(),
            summary: "s".into(),
        };
        assert_eq!(info.short_sha(), "abc");
    }

    #[test]
    fn relative_to_buckets() {
        let tz = FixedOffset::east_opt(0).unwrap();
        let then = tz.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        let info = BlameInfo {
            commit_sha: "x".into(),
            author_name: "x".into(),
            author_email: "x".into(),
            commit_time: then,
            summary: "s".into(),
        };
        assert_eq!(info.relative_to(then), "just now");
        assert_eq!(
            info.relative_to(then + chrono::Duration::seconds(30)),
            "just now"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::minutes(1)),
            "1 minute ago"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::minutes(5)),
            "5 minutes ago"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::hours(1)),
            "1 hour ago"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::hours(5)),
            "5 hours ago"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::days(1)),
            "yesterday"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::days(3)),
            "3 days ago"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::days(90)),
            "3 months ago"
        );
        assert_eq!(
            info.relative_to(then + chrono::Duration::days(800)),
            "2 years ago"
        );
    }

    #[test]
    fn short_summary_truncates_with_ellipsis() {
        let info = BlameInfo {
            commit_sha: "x".into(),
            author_name: "x".into(),
            author_email: "x".into(),
            commit_time: chrono::Utc::now().fixed_offset(),
            summary: "this is a fairly long commit summary that goes on and on".into(),
        };
        let short = info.short_summary(20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.ends_with('…'));
        // Short summaries pass through unchanged.
        let info2 = BlameInfo {
            summary: "short".into(),
            ..info
        };
        assert_eq!(info2.short_summary(20), "short");
    }

    #[test]
    fn load_on_non_repo_path_is_silent_success() {
        let dir = unique_repo("non-repo");
        let file = dir.join("foo.txt");
        std::fs::write(&file, "hello\n").unwrap();
        let cache = BlameCache::new();
        cache.load(&dir, &file).unwrap();
        assert!(cache.get(&dir, &file, 1).is_none());
        assert!(cache.was_attempted(&dir, &file));
    }

    #[test]
    fn load_on_untracked_file_is_silent_success() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = unique_repo("untracked");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "a@b"]);
        git(&dir, &["config", "user.name", "Alice"]);
        let file = dir.join("untracked.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let cache = BlameCache::new();
        cache.load(&dir, &file).unwrap();
        assert!(cache.get(&dir, &file, 1).is_none());
    }

    #[test]
    fn load_attributes_lines_to_their_commits() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = unique_repo("lines");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "alice@example.com"]);
        git(&dir, &["config", "user.name", "Alice"]);

        let file = dir.join("a.txt");
        std::fs::write(&file, "line one\nline two\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "initial two lines"]);

        // SHA for the first commit
        let first_sha = String::from_utf8(
            Command::new("git")
                .current_dir(&dir)
                .args(["rev-parse", "HEAD"])
                .env("HOME", &dir)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        std::fs::write(&file, "line one\nline two\nline three\n").unwrap();
        git(&dir, &["commit", "-q", "-am", "add line three"]);

        let second_sha = String::from_utf8(
            Command::new("git")
                .current_dir(&dir)
                .args(["rev-parse", "HEAD"])
                .env("HOME", &dir)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        let cache = BlameCache::new();
        cache.load(&dir, &file).unwrap();

        let l1 = cache.get(&dir, &file, 1).expect("line 1 has blame");
        let l2 = cache.get(&dir, &file, 2).expect("line 2 has blame");
        let l3 = cache.get(&dir, &file, 3).expect("line 3 has blame");

        assert_eq!(l1.commit_sha, first_sha);
        assert_eq!(l2.commit_sha, first_sha);
        assert_eq!(l3.commit_sha, second_sha);
        assert_eq!(l1.author_name, "Alice");
        assert_eq!(l1.summary, "initial two lines");
        assert_eq!(l3.summary, "add line three");
    }

    #[test]
    fn get_returns_none_for_out_of_range_line() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = unique_repo("oob");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "a@b"]);
        git(&dir, &["config", "user.name", "A"]);
        let file = dir.join("a.txt");
        std::fs::write(&file, "only line\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "x"]);

        let cache = BlameCache::new();
        cache.load(&dir, &file).unwrap();
        assert!(cache.get(&dir, &file, 1).is_some());
        assert!(cache.get(&dir, &file, 9999).is_none());
        assert!(cache.get(&dir, &file, 0).is_none()); // we never populate index 0
    }

    #[test]
    fn load_is_idempotent() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = unique_repo("idem");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "a@b"]);
        git(&dir, &["config", "user.name", "A"]);
        let file = dir.join("a.txt");
        std::fs::write(&file, "x\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "x"]);

        let cache = BlameCache::new();
        cache.load(&dir, &file).unwrap();
        // Second call should be a cheap no-op (no panic, no duplication).
        cache.load(&dir, &file).unwrap();
        assert!(cache.get(&dir, &file, 1).is_some());
    }

    #[test]
    fn empty_cache_get_returns_none() {
        let cache = BlameCache::new();
        assert!(
            cache
                .get(Path::new("/nowhere"), Path::new("nope.txt"), 1)
                .is_none()
        );
        assert!(!cache.was_attempted(Path::new("/nowhere"), Path::new("nope.txt")));
    }

    #[test]
    fn request_loads_in_background_and_calls_on_ready() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = unique_repo("async");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "a@b"]);
        git(&dir, &["config", "user.name", "A"]);
        let file = dir.join("a.txt");
        std::fs::write(&file, "x\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "init"]);

        let cache = BlameCache::new();
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ready_clone = ready.clone();
        cache.request(&dir, &file, move || {
            ready_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        // Spin until the worker finishes (with a generous timeout).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if ready.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            ready.load(std::sync::atomic::Ordering::SeqCst),
            "on_ready never fired"
        );
        assert!(cache.get(&dir, &file, 1).is_some());
        assert!(!cache.is_loading(&dir, &file));
    }

    #[test]
    fn duplicate_request_does_not_spawn_a_second_thread() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let dir = unique_repo("dup");
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.email", "a@b"]);
        git(&dir, &["config", "user.name", "A"]);
        let file = dir.join("a.txt");
        std::fs::write(&file, "x\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "init"]);

        let cache = BlameCache::new();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..5 {
            let c = counter.clone();
            cache.request(&dir, &file, move || {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            });
        }
        // Wait for the (single) worker to finish.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if !cache.is_loading(&dir, &file) && cache.was_attempted(&dir, &file) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // 4 of the 5 requests no-op'd (loading set already had the key); the
        // remaining 1 fired and called on_ready exactly once.
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "expected exactly one on_ready callback"
        );
    }
}
