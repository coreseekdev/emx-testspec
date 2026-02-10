//! cmp / cmpenv — file comparison commands

use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;
use similar::TextDiff;

// ──────────────────────────────────────────────────────────
// cmp — compare files
// ──────────────────────────────────────────────────────────

pub(super) struct CmpCmd;

impl Cmd for CmpCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        cmp_files(state, args, false)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Compare two files".into(),
            args: "[-q] file1 file2".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// cmpenv — compare files with env expansion
// ──────────────────────────────────────────────────────────

pub(super) struct CmpEnvCmd;

impl Cmd for CmpEnvCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        cmp_files(state, args, true)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Compare files with environment variable expansion".into(),
            args: "[-q] file1 file2".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

/// Shared cmp/cmpenv implementation
fn cmp_files(state: &mut State, args: &[String], expand_env: bool) -> Result<CmdResult, ScriptError> {
    let mut quiet = false;
    let mut files: Vec<&str> = Vec::new();

    for arg in args {
        if arg == "-q" {
            quiet = true;
        } else {
            files.push(arg);
        }
    }

    if files.len() != 2 {
        return Err(ScriptError::usage("cmp", "[-q] file1 file2"));
    }

    let mut content1 = state.read_file(files[0]).map_err(|e| {
        ScriptError::new(ErrorKind::FileNotFound, format!("{}: {}", files[0], e))
    })?;

    // Go-compatible: file2 is ALWAYS read from disk (not virtual stdout/stderr).
    // Go uses os.ReadFile(s.Path(name2)) for file2.
    let file2_path = state.resolve_path(files[1]);
    let mut content2 = std::fs::read_to_string(&file2_path).map_err(|e| {
        ScriptError::new(ErrorKind::FileNotFound, format!("{}: {}", files[1], e))
    })?;

    // Normalize CRLF → LF for cross-platform consistency
    // (state.read_file normalizes file1; apply same to file2 for parity)
    content2 = content2.replace("\r\n", "\n");

    if expand_env {
        content1 = state.expand(&content1);
        content2 = state.expand(&content2);
    }

    if content1 != content2 {
        let msg = format!("{} and {} differ", files[0], files[1]);
        if !quiet {
            // Go-compatible: log unified diff output (Go uses internal/diff.Diff)
            let diff = TextDiff::from_lines(&content1, &content2);
            let udiff = diff.unified_diff()
                .header(files[0], files[1])
                .to_string();
            state.logf(&format!("{}", udiff));
        }
        return Err(ScriptError::new(ErrorKind::ComparisonFailed, msg));
    }

    Ok(CmdResult::Ok)
}
