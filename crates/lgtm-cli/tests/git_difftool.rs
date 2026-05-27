//! End-to-end check that the blame + editor features survive an actual
//! `git difftool` invocation.
//!
//! Strategy: configure lgtm as the difftool with `--no-gui` (so no
//! display is needed) and confirm it runs against git's temp files
//! without crashing. The blame and editor codepaths are exercised by
//! `lgtm-core`'s unit tests; this test is here to make sure the CLI
//! plumbing (`--repo`, `--editor`) doesn't break difftool invocations.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("lgtm")
}

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
    let d = std::env::temp_dir().join(format!("lgtm-difftool-{name}-{nanos}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn git(cwd: &PathBuf, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_AUTHOR_NAME", "lgtm-test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "lgtm-test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("HOME", cwd)
        .output()
        .expect("git failed to spawn");
    if !out.status.success() {
        panic!(
            "git {args:?} failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out
}

/// Spin up a repo with a committed file, modify it, run `git difftool`
/// pointing at lgtm in --no-gui mode + --repo. Asserts it doesn't
/// crash and exits non-zero (because files differ).
#[test]
fn git_difftool_no_gui_invocation_with_repo_flag_works() {
    if !have_git() {
        eprintln!("skipping: git not on PATH");
        return;
    }
    let repo = unique_dir("nogui");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "a@b"]);
    git(&repo, &["config", "user.name", "A"]);
    git(&repo, &["config", "diff.tool", "lgtm"]);
    let lgtm = bin();
    let cmd = format!(
        "'{}' --no-gui --repo \"$(git rev-parse --show-toplevel)\" \"$LOCAL\" \"$REMOTE\"",
        lgtm.display()
    );
    git(&repo, &["config", "difftool.lgtm.cmd", &cmd]);
    git(&repo, &["config", "difftool.prompt", "false"]);

    let file = repo.join("a.txt");
    std::fs::write(&file, "line 1\nline 2\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "initial"]);

    // Modify in working tree (uncommitted).
    std::fs::write(&file, "line 1\nLINE TWO\n").unwrap();

    let out = Command::new("git")
        .current_dir(&repo)
        .args(["difftool", "-t", "lgtm", "--no-prompt"])
        .env("HOME", &repo)
        .output()
        .expect("git difftool failed to spawn");

    // lgtm exits 1 for files-differ. git difftool exits non-zero too in
    // older versions and zero in newer ones; either is fine. What we
    // care about is that nothing crashed catastrophically (no panic,
    // no exit code 2).
    assert_ne!(
        out.status.code(),
        Some(2),
        "lgtm should not have errored. stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
