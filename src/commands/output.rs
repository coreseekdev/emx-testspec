//! stdout / stderr / grep — output pattern matching commands

use crate::engine::{Cmd, CmdResult, CmdUsage, first_non_flag};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;

// ──────────────────────────────────────────────────────────
// stdout — match stdout against pattern
// ──────────────────────────────────────────────────────────

pub(super) struct StdoutCmd;

impl Cmd for StdoutCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        let content = state.stdout.clone();
        match_output("stdout", &content, args, state)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Match stdout against pattern".into(),
            args: "[-count=N] [-q] pattern".into(),
            regexp_args: Some(first_non_flag),
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// stderr — match stderr against pattern
// ──────────────────────────────────────────────────────────

pub(super) struct StderrCmd;

impl Cmd for StderrCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        let content = state.stderr.clone();
        match_output("stderr", &content, args, state)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Match stderr against pattern".into(),
            args: "[-count=N] [-q] pattern".into(),
            regexp_args: Some(first_non_flag),
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// grep — regex search in a file
// ──────────────────────────────────────────────────────────

pub(super) struct GrepCmd;

impl Cmd for GrepCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        let mut count: Option<usize> = None;
        let mut quiet = false;
        let mut positional: Vec<&str> = Vec::new();

        for arg in args {
            if arg.starts_with("-count=") {
                let n: usize = arg[7..].parse().map_err(|_| {
                    ScriptError::usage("grep", "[-count=N] [-q] pattern file")
                })?;
                if n < 1 {
                    return Err(ScriptError::new(ErrorKind::SyntaxError,
                        "grep: bad -count=: must be at least 1".to_string()));
                }
                count = Some(n);
            } else if arg == "-q" {
                quiet = true;
            } else {
                positional.push(arg);
            }
        }

        if positional.len() != 2 {
            return Err(ScriptError::usage("grep", "[-count=N] [-q] pattern file"));
        }

        let pattern = positional[0];
        let filename = positional[1];

        // Go-compatible: grep always reads from disk (not virtual stdout/stderr).
        // Go uses os.ReadFile(s.Path(name)) in the match() function.
        let file_path = state.resolve_path(filename);
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            ScriptError::new(ErrorKind::FileNotFound, format!("{}: {}", filename, e))
        })?;

        let re = compile_regex("grep", pattern)?;

        if let Some(expected) = count {
            let matches: Vec<_> = re.find_iter(&content).collect();
            if matches.len() != expected {
                let mut msg = format!("grep {}: found {} matches, want {}", pattern, matches.len(), expected);
                if !quiet {
                    msg.push_str(&format!("\n{}", content));
                }
                return Err(ScriptError::new(ErrorKind::PatternMismatch, msg));
            }
        } else if !re.is_match(&content) {
            let mut msg = format!("grep: no match for {}", pattern);
            if !quiet {
                msg.push_str(&format!("\n{}", content));
            }
            return Err(ScriptError::new(ErrorKind::PatternMismatch, msg));
        } else if !quiet {
            // Go-compatible: log the matched lines
            if let Some(loc) = re.find(&content) {
                let start = content[..loc.start()].rfind('\n').map(|p| p + 1).unwrap_or(0);
                let end = content[loc.end()..].find('\n').map(|p| loc.end() + p).unwrap_or(content.len());
                let lines = content[start..end].trim_end_matches('\n');
                state.logf(&format!("matched: {}", lines));
            }
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Search for a pattern in a file".into(),
            args: "[-count=N] [-q] pattern file".into(),
            regexp_args: Some(first_non_flag),
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// Shared helpers
// ──────────────────────────────────────────────────────────

/// Compile a regex pattern with multiline mode and a size limit to
/// mitigate ReDoS (CWE-1333).
fn compile_regex(cmd: &str, pattern: &str) -> Result<regex::Regex, ScriptError> {
    regex::RegexBuilder::new(&format!("(?m){}", pattern))
        .size_limit(1 << 20) // 1 MB DFA limit
        .build()
        .map_err(|e| ScriptError::new(ErrorKind::SyntaxError, format!("{}: invalid regex: {}", cmd, e)))
}

/// Shared logic for stdout/stderr pattern matching.
///
/// Go-compatible: pattern is ALWAYS treated as a regex (with `(?m)` prefix).
/// `-count=N` requires N ≥ 1.
fn match_output(name: &str, content: &str, args: &[String], state: &mut State) -> Result<CmdResult, ScriptError> {
    let mut count: Option<usize> = None;
    let mut quiet = false;
    let mut pattern_str: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("-count=") {
            let val = &args[i][7..];
            let n: usize = val.parse().map_err(|_| {
                ScriptError::usage(name, "[-count=N] [-q] pattern")
            })?;
            if n < 1 {
                return Err(ScriptError::new(ErrorKind::SyntaxError,
                    format!("{}: bad -count=: must be at least 1", name)));
            }
            count = Some(n);
        } else if args[i] == "-q" {
            quiet = true;
        } else {
            pattern_str = Some(&args[i]);
            break; // pattern found, stop processing flags
        }
        i += 1;
    }

    let pattern = pattern_str.ok_or_else(|| {
        ScriptError::usage(name, "[-count=N] [-q] pattern")
    })?;

    let re = compile_regex(name, pattern)?;

    if let Some(expected) = count {
        let matches: Vec<_> = re.find_iter(content).collect();
        if matches.len() != expected {
            return Err(ScriptError::new(
                ErrorKind::PatternMismatch,
                format!("{}: pattern /{}/: found {} matches, want {}",
                    name, pattern, matches.len(), expected),
            ));
        }
    } else {
        if !re.is_match(content) {
            let mut msg = format!("{}: no match for pattern /{}/", name, pattern);
            if !quiet {
                msg.push_str(&format!("\ncontent:\n{}", content));
            }
            return Err(ScriptError::new(ErrorKind::PatternMismatch, msg));
        }
        // Go-compatible: log matching lines (Go's match() does this for all callers)
        if !quiet {
            if let Some(loc) = re.find(content) {
                let start = content[..loc.start()].rfind('\n').map(|p| p + 1).unwrap_or(0);
                let end = content[loc.end()..].find('\n').map(|p| loc.end() + p).unwrap_or(content.len());
                let lines = content[start..end].trim_end_matches('\n');
                state.logf(&format!("matched: {}", lines));
            }
        }
    }

    Ok(CmdResult::Ok)
}
