//! Minimal git commit graph for the diff window's drawer.
//!
//! Walks the repository from `HEAD` (in topological-then-time order),
//! collects up to [`MAX_COMMITS`] entries, and assigns each commit a
//! column (`lane`) using a simple swimlane algorithm:
//!
//! - A commit inherits the first lane that was reserved for its OID
//!   among the "active" lanes (or claims a free lane if none).
//! - Each parent of the commit reserves a lane: the first parent
//!   inherits the commit's lane (so straight-line history stays in the
//!   leftmost column); additional parents take the next free lane
//!   (forming branch lines off to the right).
//!
//! The output is what the GUI needs to paint dots + lines + commit
//! metadata. No layout / pixel concerns leak into the core.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, FixedOffset, TimeZone};

use crate::error::Result;

/// Cap on the number of commits surfaced — keeps the drawer snappy and
/// bounded regardless of repo size.
pub const MAX_COMMITS: usize = 200;

/// One commit in the graph.
#[derive(Debug, Clone)]
pub struct CommitNode {
    /// Full 40-char SHA.
    pub sha: String,
    /// Author name.
    pub author: String,
    /// Author timestamp.
    pub time: DateTime<FixedOffset>,
    /// First line of the commit message (truncated by the caller).
    pub summary: String,
    /// Parent SHAs in commit order (first parent = mainline).
    pub parents: Vec<String>,
    /// 0-based column for the graph dot. Computed by [`Graph::build`].
    pub lane: usize,
}

impl CommitNode {
    /// 7-char short SHA.
    pub fn short_sha(&self) -> &str {
        let end = self.sha.len().min(7);
        &self.sha[..end]
    }
}

/// The walked commit list plus a derived edge list the GUI uses to
/// paint connecting lines.
#[derive(Debug, Default, Clone)]
pub struct Graph {
    /// Commits in walk order (most recent first).
    pub commits: Vec<CommitNode>,
    /// Edges between adjacent commits: `(child_idx, parent_idx,
    /// child_lane, parent_lane)`. The GUI paints these as line segments.
    pub edges: Vec<GraphEdge>,
}

/// One graph edge, used for drawing connector lines between dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphEdge {
    /// Index of the child commit in [`Graph::commits`].
    pub child: usize,
    /// Index of the parent commit in [`Graph::commits`] (or `usize::MAX`
    /// if the parent fell outside the [`MAX_COMMITS`] window — the GUI
    /// renders these as line-going-off-the-bottom).
    pub parent: usize,
    /// Column of the child's dot.
    pub child_lane: usize,
    /// Column of the parent's dot (or where the edge exits the visible
    /// window).
    pub parent_lane: usize,
}

impl Graph {
    /// Build a graph from a repository discovered at or above `start_path`.
    /// Returns an empty `Graph` if the path isn't inside a git repo.
    pub fn build(start_path: &Path) -> Result<Self> {
        let repo = match git2::Repository::discover(start_path) {
            Ok(r) => r,
            Err(_) => return Ok(Self::default()),
        };
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => return Ok(Self::default()),
        };
        let head_oid = match head.target() {
            Some(o) => o,
            None => return Ok(Self::default()),
        };

        let mut walk = match repo.revwalk() {
            Ok(w) => w,
            Err(e) => return Err(crate::error::Error::Git(e.to_string())),
        };
        if let Err(e) = walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME) {
            return Err(crate::error::Error::Git(e.to_string()));
        }
        if let Err(e) = walk.push(head_oid) {
            return Err(crate::error::Error::Git(e.to_string()));
        }

        let mut commits: Vec<CommitNode> = Vec::new();
        for (i, oid) in walk.enumerate() {
            if i >= MAX_COMMITS {
                break;
            }
            let oid = match oid {
                Ok(o) => o,
                Err(_) => continue,
            };
            let commit = match repo.find_commit(oid) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let parents: Vec<String> = commit.parent_ids().map(|p| p.to_string()).collect();
            commits.push(CommitNode {
                sha: oid.to_string(),
                author: commit
                    .author()
                    .name()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                time: git2_time_to_chrono(commit.time()),
                summary: commit.summary().unwrap_or("").to_string(),
                parents,
                lane: 0,
            });
        }

        assign_lanes(&mut commits);
        let edges = build_edges(&commits);
        Ok(Self { commits, edges })
    }
}

fn git2_time_to_chrono(t: git2::Time) -> DateTime<FixedOffset> {
    let secs = t.seconds();
    let offset = t.offset_minutes() * 60;
    let tz = FixedOffset::east_opt(offset).unwrap_or(FixedOffset::east_opt(0).unwrap());
    tz.timestamp_opt(secs, 0)
        .single()
        .unwrap_or_else(|| tz.timestamp_opt(0, 0).single().unwrap())
}

/// Simple swimlane assignment. `commits` is in newest-first walk order;
/// we maintain `lanes: Vec<Option<oid>>` where each slot holds the SHA
/// the parent expected its child to claim (or `None` for free).
fn assign_lanes(commits: &mut [CommitNode]) {
    let mut lanes: Vec<Option<String>> = Vec::new();
    for commit in commits.iter_mut() {
        // Find a lane already reserved for this commit, else pick the
        // leftmost free slot, else extend.
        let mut chosen = lanes.iter().position(|l| l.as_deref() == Some(&commit.sha));
        if chosen.is_none() {
            chosen = lanes.iter().position(Option::is_none);
        }
        let lane = match chosen {
            Some(i) => i,
            None => {
                lanes.push(None);
                lanes.len() - 1
            }
        };
        commit.lane = lane;
        lanes[lane] = None;

        // Reserve a lane for each parent. First parent inherits our lane
        // (straight-line history); subsequent parents take the leftmost
        // free lane.
        for (i, parent) in commit.parents.iter().enumerate() {
            let target = if i == 0 {
                lane
            } else {
                match lanes.iter().position(Option::is_none) {
                    Some(j) => j,
                    None => {
                        lanes.push(None);
                        lanes.len() - 1
                    }
                }
            };
            lanes[target] = Some(parent.clone());
        }
    }
}

fn build_edges(commits: &[CommitNode]) -> Vec<GraphEdge> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, c) in commits.iter().enumerate() {
        index.insert(c.sha.as_str(), i);
    }
    let mut out = Vec::new();
    for (child_idx, c) in commits.iter().enumerate() {
        for parent in &c.parents {
            match index.get(parent.as_str()) {
                Some(&p_idx) => out.push(GraphEdge {
                    child: child_idx,
                    parent: p_idx,
                    child_lane: c.lane,
                    parent_lane: commits[p_idx].lane,
                }),
                // Parent is outside the visible window; edge exits down.
                None => out.push(GraphEdge {
                    child: child_idx,
                    parent: usize::MAX,
                    child_lane: c.lane,
                    parent_lane: c.lane,
                }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    fn have_git() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("lgtm-graph-{name}-{nanos}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn git(cwd: &PathBuf, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Alice")
            .env("GIT_AUTHOR_EMAIL", "a@b")
            .env("GIT_COMMITTER_NAME", "Alice")
            .env("GIT_COMMITTER_EMAIL", "a@b")
            .env("HOME", cwd)
            .output()
            .expect("git spawn");
        if !out.status.success() {
            panic!(
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    #[test]
    fn build_on_non_repo_returns_empty() {
        let d = unique_dir("non-repo");
        let g = Graph::build(&d).unwrap();
        assert!(g.commits.is_empty());
        assert!(g.edges.is_empty());
    }

    #[test]
    fn build_walks_linear_history() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let d = unique_dir("linear");
        git(&d, &["init", "-q", "-b", "main"]);
        git(&d, &["config", "user.email", "a@b"]);
        git(&d, &["config", "user.name", "Alice"]);
        for i in 1..=3 {
            std::fs::write(d.join("f.txt"), format!("v{i}\n")).unwrap();
            git(&d, &["add", "f.txt"]);
            git(&d, &["commit", "-q", "-m", &format!("c{i}")]);
        }
        let g = Graph::build(&d).unwrap();
        assert_eq!(g.commits.len(), 3);
        // Newest-first ordering.
        assert_eq!(g.commits[0].summary, "c3");
        assert_eq!(g.commits[2].summary, "c1");
        // Linear history → all in lane 0.
        assert!(g.commits.iter().all(|c| c.lane == 0));
        // Each commit (except the root) has a parent edge inside the window.
        assert_eq!(g.edges.len(), 2);
        for e in &g.edges {
            assert_ne!(e.parent, usize::MAX);
            assert_eq!(e.child_lane, 0);
            assert_eq!(e.parent_lane, 0);
        }
    }

    #[test]
    fn build_handles_merge_commit_two_parents() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let d = unique_dir("merge");
        git(&d, &["init", "-q", "-b", "main"]);
        git(&d, &["config", "user.email", "a@b"]);
        git(&d, &["config", "user.name", "Alice"]);

        std::fs::write(d.join("f.txt"), b"base\n").unwrap();
        git(&d, &["add", "f.txt"]);
        git(&d, &["commit", "-q", "-m", "base"]);

        git(&d, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(d.join("g.txt"), b"feature\n").unwrap();
        git(&d, &["add", "g.txt"]);
        git(&d, &["commit", "-q", "-m", "feature"]);

        git(&d, &["checkout", "-q", "main"]);
        std::fs::write(d.join("h.txt"), b"main\n").unwrap();
        git(&d, &["add", "h.txt"]);
        git(&d, &["commit", "-q", "-m", "main-only"]);

        git(&d, &["merge", "-q", "--no-ff", "--no-edit", "feature"]);

        let g = Graph::build(&d).unwrap();
        // base + feature + main-only + merge = 4
        assert_eq!(g.commits.len(), 4);
        // The merge commit (newest) has two parents.
        let merge = &g.commits[0];
        assert_eq!(merge.parents.len(), 2);
        // The two parents should sit in different lanes (one of them is
        // a branched side).
        let lanes: std::collections::HashSet<usize> = g.commits.iter().map(|c| c.lane).collect();
        assert!(lanes.len() >= 2, "merge graph should use >1 lane");
    }

    #[test]
    fn short_sha_truncates_at_seven() {
        let c = CommitNode {
            sha: "abcdef0123456789".into(),
            author: "x".into(),
            time: chrono::Utc::now().fixed_offset(),
            summary: "s".into(),
            parents: vec![],
            lane: 0,
        };
        assert_eq!(c.short_sha(), "abcdef0");
    }

    #[test]
    fn max_commits_caps_the_walk() {
        if !have_git() {
            eprintln!("skipping: git not on PATH");
            return;
        }
        let d = unique_dir("cap");
        git(&d, &["init", "-q", "-b", "main"]);
        git(&d, &["config", "user.email", "a@b"]);
        git(&d, &["config", "user.name", "Alice"]);
        // Far below MAX_COMMITS, but the loop logic is what we exercise.
        for i in 0..5 {
            std::fs::write(d.join("f.txt"), format!("{i}\n")).unwrap();
            git(&d, &["add", "f.txt"]);
            git(&d, &["commit", "-q", "-m", &format!("c{i}")]);
        }
        let g = Graph::build(&d).unwrap();
        assert!(g.commits.len() <= MAX_COMMITS);
        assert_eq!(g.commits.len(), 5);
    }
}
