//! Integration tests for the `lgtm` binary's CLI surface.
//!
//! These tests use the `--no-gui` mode where possible so they can run in CI
//! without a display server. GUI paths are exercised manually.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop(); // strip test binary name
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("lgtm")
}

fn tmp(name: &str, content: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join("lgtm-cli-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(content).unwrap();
    p
}

#[test]
fn no_gui_identical_files_exits_zero_silently() {
    let a = tmp("same_a.txt", b"hello\nworld\n");
    let b = tmp("same_b.txt", b"hello\nworld\n");
    let out = Command::new(bin())
        .args(["--no-gui"])
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "no-gui mode prints nothing when identical, got: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn no_gui_differing_files_print_unified_and_exit_one() {
    let a = tmp("diff_a.txt", b"hello\nworld\n");
    let b = tmp("diff_b.txt", b"hello\nWORLD\n");
    let out = Command::new(bin())
        .args(["--no-gui"])
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("---"));
    assert!(stdout.contains("+++"));
    assert!(stdout.contains("-world"));
    assert!(stdout.contains("+WORLD"));
}

#[test]
fn no_gui_dev_null_left_acts_as_pure_insertion() {
    let b = tmp("only_right.txt", b"new file\ncontents\n");
    let out = Command::new(bin())
        .args(["--no-gui", "/dev/null"])
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("+new file"));
}

#[test]
fn no_gui_missing_file_exits_two() {
    let b = tmp("present.txt", b"x\n");
    let out = Command::new(bin())
        .args(["--no-gui", "/no/such/path/lgtm-test"])
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error"));
}

#[test]
fn no_gui_binary_files_print_short_message() {
    let a = tmp("bin_a.dat", &[0u8, 1, 2, 0xff, 0]);
    let b = tmp("bin_b.dat", &[0u8, 1, 2, 0xfe, 0]);
    let out = Command::new(bin())
        .args(["--no-gui"])
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Binary files"));
    assert!(stdout.contains("differ"));
}

#[test]
fn no_gui_accepts_repo_and_editor_flags_without_complaint() {
    // The new flags must parse cleanly even when --no-gui won't use them.
    let a = tmp("rfl_a.txt", b"hi\n");
    let b = tmp("rfl_b.txt", b"hi\n");
    let out = Command::new(bin())
        .args(["--no-gui", "--repo", "/tmp", "--editor", "echo"])
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn version_prints_ascii_banner_and_tagline() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("╦  ╔═╗╔╦╗╔╦╗"));
    assert!(s.contains("from wtf to lgtm"));
}

#[test]
fn quiet_suppresses_lgtm_banner() {
    let a = tmp("q_a.txt", b"x\n");
    let b = tmp("q_b.txt", b"x\n");
    let out = Command::new(bin())
        .args(["--no-gui", "--quiet"])
        .arg(&a)
        .arg(&b)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    // --quiet currently affects the GUI path; --no-gui identical exits
    // silently regardless. Sanity check there's no "LGTM" emission either.
    assert!(!String::from_utf8_lossy(&out.stderr).contains("LGTM ✓"));
}

#[test]
fn merge_without_args_errors_cleanly() {
    let a = tmp("merge_one.txt", b"x\n");
    let out = Command::new(bin()).arg("--merge").arg(&a).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error"));
}

#[test]
fn merge_headless_auto_resolves_non_conflicting_changes() {
    let local = tmp("h_local.txt", b"a\nLOCAL\nc\n");
    let base = tmp("h_base.txt", b"a\nb\nc\n");
    let remote = tmp("h_remote.txt", b"a\nb\nc\n");
    let merged_dir = std::env::temp_dir().join("lgtm-cli-tests");
    std::fs::create_dir_all(&merged_dir).unwrap();
    let merged = merged_dir.join("h_merged_auto.txt");
    let _ = std::fs::remove_file(&merged);

    let out = Command::new(bin())
        .env("LGTM_HEADLESS_MERGE", "auto")
        .arg("--merge")
        .arg(&local)
        .arg(&base)
        .arg(&remote)
        .arg("--output")
        .arg(&merged)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("LGTM"));
    let body = std::fs::read_to_string(&merged).unwrap();
    assert_eq!(body, "a\nLOCAL\nc\n");
}

#[test]
fn merge_headless_take_local_resolves_conflict() {
    let local = tmp("c_local.txt", b"a\nLOCAL\nc\n");
    let base = tmp("c_base.txt", b"a\nb\nc\n");
    let remote = tmp("c_remote.txt", b"a\nREMOTE\nc\n");
    let merged = std::env::temp_dir()
        .join("lgtm-cli-tests")
        .join("c_merged.txt");
    let _ = std::fs::remove_file(&merged);

    let out = Command::new(bin())
        .env("LGTM_HEADLESS_MERGE", "take-local")
        .arg("--merge")
        .arg(&local)
        .arg(&base)
        .arg(&remote)
        .arg("--output")
        .arg(&merged)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let body = std::fs::read_to_string(&merged).unwrap();
    assert_eq!(body, "a\nLOCAL\nc\n");
}

fn tmp_dir(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!("lgtm-cli-dir-{name}-{nanos}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn folder_no_gui_identical_trees_exit_zero() {
    let l = tmp_dir("folder-id-l");
    let r = tmp_dir("folder-id-r");
    std::fs::write(l.join("a.txt"), b"x").unwrap();
    std::fs::write(r.join("a.txt"), b"x").unwrap();
    let out = Command::new(bin())
        .args(["--no-gui", "--dir"])
        .arg(&l)
        .arg(&r)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn folder_no_gui_modified_tree_exits_one_and_lists_diffs() {
    let l = tmp_dir("folder-mod-l");
    let r = tmp_dir("folder-mod-r");
    std::fs::write(l.join("same.txt"), b"x").unwrap();
    std::fs::write(r.join("same.txt"), b"x").unwrap();
    std::fs::write(l.join("only_left.txt"), b"L").unwrap();
    // Different sizes so the cheap-identity shortcut doesn't kick in.
    std::fs::write(l.join("changed.txt"), b"OLD\n").unwrap();
    std::fs::write(r.join("changed.txt"), b"NEW CONTENT\n").unwrap();

    let out = Command::new(bin())
        .args(["--no-gui", "--dir"])
        .arg(&l)
        .arg(&r)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("changed.txt"));
    assert!(stdout.contains("only_left.txt"));
    assert!(!stdout.contains("same.txt"));
}

#[test]
fn merge_headless_abort_exits_one_and_doesnt_write_output() {
    let local = tmp("a_local.txt", b"a\nLOCAL\nc\n");
    let base = tmp("a_base.txt", b"a\nb\nc\n");
    let remote = tmp("a_remote.txt", b"a\nREMOTE\nc\n");
    let merged = std::env::temp_dir()
        .join("lgtm-cli-tests")
        .join("a_merged.txt");
    let _ = std::fs::remove_file(&merged);

    let out = Command::new(bin())
        .env("LGTM_HEADLESS_MERGE", "abort")
        .arg("--merge")
        .arg(&local)
        .arg(&base)
        .arg(&remote)
        .arg("--output")
        .arg(&merged)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!merged.exists(), "abort must not write the output file");
}
