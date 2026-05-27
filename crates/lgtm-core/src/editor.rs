// The `setsid()` FFI call below and the Rust 2024 `env::set_var` in tests
// both require unsafe; the rest of the module is safe.
#![allow(unsafe_code)]
//! Launch an external editor on a file (optionally at a specific line).
//!
//! Editor resolution priority:
//!
//! 1. An explicit override (CLI's `--editor <CMD>`)
//! 2. `LGTM_EDITOR`
//! 3. `$VISUAL`
//! 4. `$EDITOR`
//! 5. Platform fallback: `code` → `nvim` → `vim` → `notepad` (Windows)
//!    → `open -t` (macOS, opens in default text editor)
//!
//! The launcher detects the editor by the basename of the resolved
//! command and uses the right line-number syntax. See [`LineArgStyle`].

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// In difftool mode the file path is a temp snapshot; try to resolve it
/// back to a working-tree file inside `repo` by basename.
///
/// Behavior:
/// - If `file_path` is already inside `repo`, return its canonicalized form.
/// - Otherwise walk the repo (respecting `.gitignore` like everything else
///   in lgtm) looking for files with the same basename.
///   - Exactly one match → return it.
///   - Zero or multiple matches → return `None` (honest about uncertainty;
///     don't guess and surprise the user).
pub fn resolve_real_path(repo: &Path, file_path: &Path) -> Option<PathBuf> {
    let canon_repo = std::fs::canonicalize(repo).ok()?;
    if let Ok(canon_file) = std::fs::canonicalize(file_path) {
        if canon_file.starts_with(&canon_repo) {
            return Some(canon_file);
        }
    }
    let basename = file_path.file_name()?;
    let mut matches: Vec<PathBuf> = Vec::new();
    let walker = ignore::WalkBuilder::new(&canon_repo).build();
    for entry in walker.flatten() {
        let is_file = entry
            .file_type()
            .map(|ft: std::fs::FileType| ft.is_file())
            .unwrap_or(false);
        if is_file && entry.file_name() == basename {
            matches.push(entry.into_path());
            if matches.len() > 1 {
                return None;
            }
        }
    }
    matches.into_iter().next()
}

/// How to pass a line number to a given editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineArgStyle {
    /// `vim`/`nvim`/`emacs`-style: `+LINE FILE`.
    Plus,
    /// `code`-style: `--goto FILE:LINE`.
    CodeGoto,
    /// `hx`/`subl`-style: `FILE:LINE` as a single positional argument.
    PathColon,
    /// JetBrains-style: `--line LINE FILE`.
    DashLine,
    /// Unknown editor: pass the file path only; line is ignored.
    None,
}

/// A resolved editor command, ready to be launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorLauncher {
    /// The first token of the command (binary name). Used both as the
    /// argv[0] and to drive [`LineArgStyle`] detection.
    pub program: String,
    /// Extra args from the resolved command (e.g. `open -t` carries `-t`
    /// as a pre-positional flag).
    pub leading_args: Vec<String>,
    /// Line-passing style detected from `program`.
    pub line_arg_style: LineArgStyle,
}

impl EditorLauncher {
    /// Resolve an editor according to the documented priority order.
    ///
    /// `cli_override` wins; then env vars in spec order; then a platform
    /// fallback that probes PATH for a known editor. Returns
    /// [`Error::Git`] is never used here — failures bubble up as
    /// [`Error::InvalidMergeInputs`] is also wrong — see [`Error::Walk`]
    /// no, none of these fit. We use a dedicated [`Error::Io`] flavor
    /// indirectly by wrapping the missing-editor case in a message.
    ///
    /// In practice this only errors when *every* fallback has been
    /// tried (extremely rare on systems with any text editor installed).
    pub fn resolve(cli_override: Option<&str>) -> Result<Self> {
        if let Some(raw) = cli_override {
            return Ok(parse_command(raw));
        }
        for var in ["LGTM_EDITOR", "VISUAL", "EDITOR"] {
            if let Ok(v) = std::env::var(var) {
                if !v.trim().is_empty() {
                    return Ok(parse_command(&v));
                }
            }
        }
        for fallback in fallback_candidates() {
            if which_exists(fallback.split_whitespace().next().unwrap_or("")) {
                return Ok(parse_command(fallback));
            }
        }
        Err(Error::Walk(
            "no editor found: set $EDITOR / LGTM_EDITOR / pass --editor".into(),
        ))
    }

    /// Build the [`Command`] that would launch this editor on `path` at
    /// `line` (or with no line argument if `line` is `None`).
    ///
    /// Exposed separately from [`open`](Self::open) so tests can inspect
    /// the resulting argv without actually spawning a process.
    pub fn build_command(&self, path: &Path, line: Option<usize>) -> Command {
        let mut cmd = Command::new(&self.program);
        for arg in &self.leading_args {
            cmd.arg(arg);
        }
        match (self.line_arg_style, line) {
            (LineArgStyle::Plus, Some(n)) => {
                cmd.arg(format!("+{n}")).arg(path);
            }
            (LineArgStyle::CodeGoto, Some(n)) => {
                cmd.arg("--goto").arg(format!("{}:{n}", path.display()));
            }
            (LineArgStyle::PathColon, Some(n)) => {
                cmd.arg(format!("{}:{n}", path.display()));
            }
            (LineArgStyle::DashLine, Some(n)) => {
                cmd.arg("--line").arg(format!("{n}")).arg(path);
            }
            _ => {
                cmd.arg(path);
            }
        }
        cmd
    }

    /// Launch the editor fire-and-forget. The child is fully detached;
    /// closing lgtm does not kill it.
    pub fn open(&self, path: &Path, line: Option<usize>) -> Result<()> {
        let mut cmd = self.build_command(path, line);
        detach(&mut cmd);
        cmd.spawn().map_err(|source| Error::Io {
            path: PathBuf::from(&self.program),
            source,
        })?;
        Ok(())
    }
}

/// Parse a shell-ish command into program + leading args. Whitespace
/// splits; quoting is intentionally not handled — anything fancy should
/// go through a shell wrapper.
fn parse_command(raw: &str) -> EditorLauncher {
    let mut parts = raw.split_whitespace().map(|s| s.to_string());
    let program = parts.next().unwrap_or_default();
    let leading_args: Vec<String> = parts.collect();
    let style = detect_style(&program);
    EditorLauncher {
        program,
        leading_args,
        line_arg_style: style,
    }
}

fn detect_style(program: &str) -> LineArgStyle {
    // basename: split on both unix and windows separators so behavior is
    // identical regardless of host OS (someone might set $EDITOR to a
    // Windows-style path even when invoking from WSL etc.).
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let base = base.trim_end_matches(".exe").to_ascii_lowercase();
    match base.as_str() {
        "vim" | "nvim" | "vi" | "neovim" | "emacs" | "emacsclient" | "nano" | "kak" | "kakoune"
        | "micro" => LineArgStyle::Plus,
        "code" | "code-insiders" | "codium" | "vscodium" => LineArgStyle::CodeGoto,
        "hx" | "helix" | "subl" | "sublime_text" => LineArgStyle::PathColon,
        "idea" | "pycharm" | "webstorm" | "rustrover" | "clion" | "goland" | "phpstorm"
        | "rubymine" | "intellij" | "rider" | "datagrip" => LineArgStyle::DashLine,
        _ => LineArgStyle::None,
    }
}

fn fallback_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["code", "nvim", "vim", "notepad"]
    } else if cfg!(target_os = "macos") {
        // open -t opens the user's default text editor.
        &["code", "nvim", "vim", "open -t"]
    } else {
        &["code", "nvim", "vim", "nano"]
    }
}

fn which_exists(program: &str) -> bool {
    if program.is_empty() {
        return false;
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return true;
        }
        if cfg!(target_os = "windows") {
            let with_exe = dir.join(format!("{program}.exe"));
            if with_exe.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and has no effect on the parent.
    unsafe {
        cmd.pre_exec(|| {
            let _ = libc_setsid();
            Ok(())
        });
    }
}

#[cfg(unix)]
fn libc_setsid() -> i32 {
    // We don't depend on `libc`; declare the symbol ourselves.
    // SAFETY (extern block): `setsid` is part of POSIX libc, always
    // available on unix targets, and the signature matches.
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    // SAFETY: setsid() is async-signal-safe.
    unsafe { setsid() }
}

#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program().to_string_lossy().into_owned())
            .chain(cmd.get_args().map(|s| s.to_string_lossy().into_owned()))
            .collect()
    }

    #[test]
    fn detect_style_vim_family_is_plus() {
        for p in ["vim", "nvim", "vi", "emacs", "kakoune", "nano"] {
            assert_eq!(detect_style(p), LineArgStyle::Plus, "{p}");
        }
    }

    #[test]
    fn detect_style_vscode_family_is_code_goto() {
        for p in ["code", "code-insiders", "codium", "VSCodium"] {
            assert_eq!(detect_style(p), LineArgStyle::CodeGoto, "{p}");
        }
    }

    #[test]
    fn detect_style_jetbrains_family_is_dash_line() {
        for p in ["idea", "pycharm", "WebStorm", "RustRover"] {
            assert_eq!(detect_style(p), LineArgStyle::DashLine, "{p}");
        }
    }

    #[test]
    fn detect_style_path_colon_family() {
        for p in ["hx", "helix", "subl"] {
            assert_eq!(detect_style(p), LineArgStyle::PathColon, "{p}");
        }
    }

    #[test]
    fn detect_style_unknown_is_none() {
        assert_eq!(detect_style("totally-made-up-editor"), LineArgStyle::None);
    }

    #[test]
    fn detect_style_strips_windows_exe_suffix() {
        assert_eq!(detect_style("vim.exe"), LineArgStyle::Plus);
        assert_eq!(detect_style("Code.exe"), LineArgStyle::CodeGoto);
    }

    #[test]
    fn detect_style_handles_absolute_path() {
        assert_eq!(detect_style("/usr/local/bin/nvim"), LineArgStyle::Plus);
        assert_eq!(
            detect_style("C:\\Program Files\\Microsoft VS Code\\Code.exe"),
            LineArgStyle::CodeGoto
        );
    }

    #[test]
    fn parse_command_splits_program_and_args() {
        let l = parse_command("open -t");
        assert_eq!(l.program, "open");
        assert_eq!(l.leading_args, vec!["-t"]);
        assert_eq!(l.line_arg_style, LineArgStyle::None);
    }

    #[test]
    fn parse_command_with_empty_input_yields_empty_program() {
        let l = parse_command("");
        assert!(l.program.is_empty());
        assert!(l.leading_args.is_empty());
    }

    #[test]
    fn build_command_vim_uses_plus_line_first() {
        let l = parse_command("vim");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), Some(42));
        assert_eq!(argv(&cmd), vec!["vim", "+42", "/tmp/file.rs"]);
    }

    #[test]
    fn build_command_code_uses_goto_with_colon() {
        let l = parse_command("code");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), Some(42));
        assert_eq!(argv(&cmd), vec!["code", "--goto", "/tmp/file.rs:42"]);
    }

    #[test]
    fn build_command_helix_uses_path_colon() {
        let l = parse_command("hx");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), Some(42));
        assert_eq!(argv(&cmd), vec!["hx", "/tmp/file.rs:42"]);
    }

    #[test]
    fn build_command_idea_uses_dash_line() {
        let l = parse_command("idea");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), Some(42));
        assert_eq!(argv(&cmd), vec!["idea", "--line", "42", "/tmp/file.rs"]);
    }

    #[test]
    fn build_command_unknown_editor_passes_file_only() {
        let l = parse_command("madeup");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), Some(42));
        assert_eq!(argv(&cmd), vec!["madeup", "/tmp/file.rs"]);
    }

    #[test]
    fn build_command_no_line_passes_file_only() {
        let l = parse_command("vim");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), None);
        assert_eq!(argv(&cmd), vec!["vim", "/tmp/file.rs"]);
    }

    #[test]
    fn build_command_preserves_leading_args() {
        // e.g. "open -t" on macOS: the -t must come before the path.
        let l = parse_command("open -t");
        let cmd = l.build_command(Path::new("/tmp/file.rs"), Some(42));
        // open has no line-arg syntax, so line is dropped.
        assert_eq!(argv(&cmd), vec!["open", "-t", "/tmp/file.rs"]);
    }

    /// Serializes env-var manipulation across tests. Cargo runs tests in
    /// parallel by default and `set_var` is process-global.
    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn clear_editor_env() {
        // SAFETY: held under env_lock so no other test races us.
        unsafe {
            std::env::remove_var("LGTM_EDITOR");
            std::env::remove_var("VISUAL");
            std::env::remove_var("EDITOR");
        }
    }

    #[test]
    fn resolve_picks_lgtm_editor_over_visual_and_editor() {
        let _g = env_lock().lock().unwrap();
        clear_editor_env();
        // SAFETY: protected by env_lock.
        unsafe {
            std::env::set_var("LGTM_EDITOR", "lgtm-test-editor-xyz");
            std::env::set_var("VISUAL", "visual-editor");
            std::env::set_var("EDITOR", "editor-editor");
        }
        let l = EditorLauncher::resolve(None).unwrap();
        clear_editor_env();
        assert_eq!(l.program, "lgtm-test-editor-xyz");
    }

    #[test]
    fn resolve_cli_override_wins() {
        let _g = env_lock().lock().unwrap();
        clear_editor_env();
        unsafe {
            std::env::set_var("LGTM_EDITOR", "should-not-win");
        }
        let l = EditorLauncher::resolve(Some("override-editor")).unwrap();
        clear_editor_env();
        assert_eq!(l.program, "override-editor");
    }

    #[test]
    fn resolve_via_visual_when_lgtm_editor_unset() {
        let _g = env_lock().lock().unwrap();
        clear_editor_env();
        unsafe {
            std::env::set_var("VISUAL", "the-visual-editor");
        }
        let l = EditorLauncher::resolve(None).unwrap();
        clear_editor_env();
        assert_eq!(l.program, "the-visual-editor");
    }

    #[test]
    fn resolve_via_editor_when_lgtm_and_visual_unset() {
        let _g = env_lock().lock().unwrap();
        clear_editor_env();
        unsafe {
            std::env::set_var("EDITOR", "the-editor-editor");
        }
        let l = EditorLauncher::resolve(None).unwrap();
        clear_editor_env();
        assert_eq!(l.program, "the-editor-editor");
    }

    /// End-to-end smoke test: use `echo` as the editor, actually spawn,
    /// and verify the spawn succeeded. Proves the integration path
    /// (parse → build_command → spawn) works end-to-end without leaving
    /// a window open.
    #[test]
    #[cfg(unix)]
    fn open_with_echo_spawns_without_error() {
        let l = parse_command("echo");
        // echo isn't in our line_arg_style map, so it'll just receive
        // the path as its sole argument.
        let r = l.open(Path::new("/tmp/lgtm-editor-smoke.txt"), Some(99));
        assert!(r.is_ok(), "expected spawn to succeed: {r:?}");
    }

    fn unique_repo(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("lgtm-resolve-{name}-{nanos}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resolve_real_path_finds_unique_basename() {
        let repo = unique_repo("unique");
        std::fs::write(repo.join("a.txt"), b"hi").unwrap();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("sub/b.txt"), b"hi").unwrap();
        // A "temp file" elsewhere with basename b.txt.
        let tmp = unique_repo("temp").join("b.txt");
        std::fs::write(&tmp, b"snapshot").unwrap();
        let resolved = resolve_real_path(&repo, &tmp).unwrap();
        assert!(resolved.ends_with("sub/b.txt"));
    }

    #[test]
    fn resolve_real_path_returns_none_when_ambiguous() {
        let repo = unique_repo("amb");
        std::fs::write(repo.join("dup.txt"), b"hi").unwrap();
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        std::fs::write(repo.join("sub/dup.txt"), b"hi").unwrap();
        let tmp = unique_repo("amb2").join("dup.txt");
        std::fs::write(&tmp, b"snapshot").unwrap();
        assert!(resolve_real_path(&repo, &tmp).is_none());
    }

    #[test]
    fn resolve_real_path_returns_none_when_basename_missing() {
        let repo = unique_repo("missing");
        std::fs::write(repo.join("a.txt"), b"hi").unwrap();
        let tmp = unique_repo("missing2").join("not-here.txt");
        std::fs::write(&tmp, b"snapshot").unwrap();
        assert!(resolve_real_path(&repo, &tmp).is_none());
    }

    #[test]
    fn resolve_real_path_passes_through_when_already_in_repo() {
        let repo = unique_repo("in-repo");
        let file = repo.join("x.txt");
        std::fs::write(&file, b"hi").unwrap();
        let resolved = resolve_real_path(&repo, &file).unwrap();
        assert!(resolved.ends_with("x.txt"));
    }
}
