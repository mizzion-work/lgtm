//! End-to-end git mergetool tests.
//!
//! These tests spin up a real `git` process against a temp repo, configure
//! `lgtm` as the mergetool (in headless mode so no display is required),
//! provoke a merge conflict, and run `git mergetool` against the built
//! binary. They prove the difftool/mergetool integration works for real,
//! not just in unit tests.
//!
//! If `git` is not on PATH the tests skip themselves with a printed note.

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
    let d = std::env::temp_dir().join(format!("lgtm-git-{name}-{nanos}"));
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
        .env("HOME", cwd) // isolate from user config
        .output()
        .expect("git command failed to spawn");
    if !out.status.success() {
        panic!(
            "git {args:?} failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    out
}

/// Run a full conflict workflow: init repo, make conflicting branches,
/// invoke `git mergetool` with lgtm in headless `take-local` mode, and
/// assert that the merged file matches LOCAL.
#[test]
fn git_mergetool_take_local_resolves_real_conflict() {
    if !have_git() {
        eprintln!("skipping: git not on PATH");
        return;
    }

    let repo = unique_dir("merge-take-local");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "lgtm-test"]);

    let file = repo.join("file.txt");
    std::fs::write(&file, "line 1\nshared\nline 3\n").unwrap();
    git(&repo, &["add", "file.txt"]);
    git(&repo, &["commit", "-q", "-m", "base"]);

    // In git's mergetool nomenclature, $LOCAL is HEAD (the branch we're
    // merging into) and $REMOTE is the side being merged in.
    git(&repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(&file, "line 1\nFROM FEATURE\nline 3\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "feature change"]);

    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(&file, "line 1\nFROM MAIN\nline 3\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "main change"]);

    // Now merge feature into main; this will produce a conflict.
    let merge_out = Command::new("git")
        .current_dir(&repo)
        .args(["merge", "--no-edit", "feature"])
        .env("HOME", &repo)
        .output()
        .unwrap();
    assert!(!merge_out.status.success(), "merge should conflict");

    // Configure lgtm as the mergetool and run git mergetool.
    let lgtm = bin();
    let cmd = format!(
        "'{}' --merge \"$LOCAL\" \"$BASE\" \"$REMOTE\" --output \"$MERGED\" --quiet",
        lgtm.display()
    );
    git(&repo, &["config", "mergetool.lgtm.cmd", &cmd]);
    git(&repo, &["config", "mergetool.lgtm.trustExitCode", "true"]);
    git(&repo, &["config", "mergetool.lgtm.keepBackup", "false"]);

    let mergetool_out = Command::new("git")
        .current_dir(&repo)
        .args(["mergetool", "--tool=lgtm", "--no-prompt"])
        .env("HOME", &repo)
        .env("LGTM_HEADLESS_MERGE", "take-local")
        .output()
        .expect("git mergetool failed to spawn");
    assert!(
        mergetool_out.status.success(),
        "git mergetool failed: stdout={} stderr={}",
        String::from_utf8_lossy(&mergetool_out.stdout),
        String::from_utf8_lossy(&mergetool_out.stderr)
    );

    let merged = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        merged, "line 1\nFROM MAIN\nline 3\n",
        "merged file must equal LOCAL ($LOCAL = HEAD = main) after take-local resolution"
    );
}

/// Same workflow but abort: mergetool returns 1, git leaves the file
/// in conflicted state and does NOT mark it resolved.
#[test]
fn git_mergetool_abort_leaves_conflict_unresolved() {
    if !have_git() {
        eprintln!("skipping: git not on PATH");
        return;
    }

    let repo = unique_dir("merge-abort");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "lgtm-test"]);

    let file = repo.join("file.txt");
    std::fs::write(&file, "x\n").unwrap();
    git(&repo, &["add", "file.txt"]);
    git(&repo, &["commit", "-q", "-m", "base"]);

    git(&repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(&file, "L\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "L"]);

    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(&file, "R\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "R"]);

    let _ = Command::new("git")
        .current_dir(&repo)
        .args(["merge", "--no-edit", "feature"])
        .env("HOME", &repo)
        .output()
        .unwrap();

    let lgtm = bin();
    let cmd = format!(
        "'{}' --merge \"$LOCAL\" \"$BASE\" \"$REMOTE\" --output \"$MERGED\" --quiet",
        lgtm.display()
    );
    git(&repo, &["config", "mergetool.lgtm.cmd", &cmd]);
    git(&repo, &["config", "mergetool.lgtm.trustExitCode", "true"]);
    git(&repo, &["config", "mergetool.lgtm.keepBackup", "false"]);

    let out = Command::new("git")
        .current_dir(&repo)
        .args(["mergetool", "--tool=lgtm", "--no-prompt"])
        .env("HOME", &repo)
        .env("LGTM_HEADLESS_MERGE", "abort")
        .output()
        .expect("git mergetool failed to spawn");
    // With trustExitCode=true and lgtm returning 1, git treats the merge
    // as failed and exits non-zero.
    assert!(
        !out.status.success(),
        "git mergetool should have failed when lgtm aborted (status={}); stdout={} stderr={}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let status = Command::new("git")
        .current_dir(&repo)
        .args(["status", "--porcelain"])
        .env("HOME", &repo)
        .output()
        .unwrap();
    let st = String::from_utf8_lossy(&status.stdout);
    assert!(
        st.contains("UU") || st.contains("AA"),
        "file should remain in conflicted state: {st}"
    );
}
