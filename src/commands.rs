//! Built-in script commands
//!
//! Implements the standard testscript command set, with Go-compatible behavior
//! for pattern matching (always regex), env display, and file operations.

use std::collections::HashMap;
use std::process::Command as ProcessCommand;
use crate::engine::{Cmd, CmdResult, CmdUsage, BoxedCmd, first_non_flag};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;
use similar::TextDiff;

/// Return the default set of built-in commands
pub fn default_commands() -> HashMap<String, BoxedCmd> {
    let mut cmds: HashMap<String, BoxedCmd> = HashMap::new();
    cmds.insert("exec".into(), Box::new(ExecCmd));
    cmds.insert("stdout".into(), Box::new(StdoutCmd));
    cmds.insert("stderr".into(), Box::new(StderrCmd));
    cmds.insert("cmp".into(), Box::new(CmpCmd));
    cmds.insert("cmpenv".into(), Box::new(CmpEnvCmd));
    cmds.insert("exists".into(), Box::new(ExistsCmd));
    cmds.insert("grep".into(), Box::new(GrepCmd));
    cmds.insert("cat".into(), Box::new(CatCmd));
    cmds.insert("cd".into(), Box::new(CdCmd));
    cmds.insert("cp".into(), Box::new(CpCmd));
    cmds.insert("echo".into(), Box::new(EchoCmd));
    cmds.insert("env".into(), Box::new(EnvCmd));
    cmds.insert("mkdir".into(), Box::new(MkdirCmd));
    cmds.insert("rm".into(), Box::new(RmCmd));
    cmds.insert("stop".into(), Box::new(StopCmd));
    cmds.insert("skip".into(), Box::new(SkipCmd));
    cmds.insert("replace".into(), Box::new(ReplaceCmd));
    cmds.insert("mv".into(), Box::new(MvCmd));
    cmds.insert("chmod".into(), Box::new(ChmodCmd));
    cmds.insert("symlink".into(), Box::new(SymlinkCmd));
    cmds.insert("sleep".into(), Box::new(SleepCmd));
    cmds.insert("wait".into(), Box::new(WaitCmd));
    cmds
}

// ──────────────────────────────────────────────────────────
// exec — execute a subprocess
// ──────────────────────────────────────────────────────────

struct ExecCmd;

impl Cmd for ExecCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::usage("exec", "program [args...]"));
        }

        // Go-compatible: convert forward slashes to OS path separator
        // (Go does filepath.FromSlash(args[0]) before lookPath)
        let program = args[0].replace('/', &std::path::MAIN_SEPARATOR.to_string());
        let cmd_args = &args[1..];

        // Use the script's PATH to look up the executable (Go-compatible)
        let resolved = look_path(state, &program).map_err(|e| {
            ScriptError::new(ErrorKind::CommandFailed,
                format!("failed to execute '{}': {}", program, e))
        })?;

        let mut cmd = ProcessCommand::new(&resolved);
        cmd.args(cmd_args);
        cmd.current_dir(&state.pwd);

        // Pipe stdout/stderr so wait_with_output() captures them
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Set environment
        cmd.env_clear();
        for (k, v) in state.environ() {
            cmd.env(k, v);
        }

        // Always spawn — the engine decides whether to wait or push to background.
        // Go uses cmd.Start() and returns a WaitFunc closure.
        let child = cmd.spawn().map_err(|e| {
            ScriptError::new(ErrorKind::CommandFailed,
                format!("failed to execute '{}': {}", program, e))
        })?;

        Ok(CmdResult::Background(crate::engine::WaitHandle::Process(child)))
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Execute a command".into(),
            args: "program [args...]".into(),
            regexp_args: None,
            async_: true,
        }
    }
}

// ──────────────────────────────────────────────────────────
// stdout — match stdout against pattern
// ──────────────────────────────────────────────────────────

struct StdoutCmd;

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

struct StderrCmd;

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

/// Parse a pattern, detecting regex (/.../) vs literal ('...' or plain).
/// DEPRECATED: kept for backward reference only. All patterns are now regex.
#[allow(dead_code)]
fn parse_pattern(s: &str) -> (bool, String) {
    if s.starts_with('/') && s.ends_with('/') && s.len() > 1 {
        (true, s[1..s.len()-1].to_string())
    } else {
        (false, s.to_string())
    }
}

// ──────────────────────────────────────────────────────────
// cmp — compare files
// ──────────────────────────────────────────────────────────

struct CmpCmd;

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

struct CmpEnvCmd;

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

// ──────────────────────────────────────────────────────────
// exists — check if files exist
// ──────────────────────────────────────────────────────────

struct ExistsCmd;

impl Cmd for ExistsCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::usage("exists", "[-readonly] [-exec] file..."));
        }

        let mut check_readonly = false;
        let mut check_exec = false;
        let mut files: Vec<&str> = Vec::new();

        // Go-compatible: flags must come before file arguments.
        // Once a non-flag argument is found, all remaining args are treated as files.
        let mut parsing_flags = true;
        for arg in args {
            if parsing_flags {
                match arg.as_str() {
                    "-readonly" => { check_readonly = true; continue; }
                    "-exec" => { check_exec = true; continue; }
                    _ => { parsing_flags = false; }
                }
            }
            files.push(arg);
        }

        for file in &files {
            let path = state.resolve_path(file);
            if !path.exists() {
                return Err(ScriptError::new(ErrorKind::FileNotFound,
                    format!("{} does not exist", file)));
            }

            if check_readonly {
                let meta = std::fs::metadata(&path).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("{}: {}", file, e))
                })?;
                if !meta.permissions().readonly() {
                    return Err(ScriptError::new(ErrorKind::Other,
                        format!("{} exists but is writable", file)));
                }
            }

            if check_exec {
                // Go-compatible: exec check is skipped on Windows
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let meta = std::fs::metadata(&path).map_err(|e| {
                        ScriptError::new(ErrorKind::Io, format!("{}: {}", file, e))
                    })?;
                    if meta.permissions().mode() & 0o111 == 0 {
                        return Err(ScriptError::new(ErrorKind::Other,
                            format!("{} exists but is not executable", file)));
                    }
                }
                // On Windows, exec check is intentionally skipped (Go compat)
            }
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Check that files exist".into(),
            args: "[-readonly] [-exec] file...".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// grep — regex search in a file
// ──────────────────────────────────────────────────────────

struct GrepCmd;

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
// cat — concatenate files to stdout
// ──────────────────────────────────────────────────────────

struct CatCmd;

impl Cmd for CatCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::usage("cat", "file..."));
        }

        let mut output = String::new();
        for file in args {
            // Go-compatible: cat always reads from disk (never virtual stdout/stderr)
            let path = state.resolve_path(file);
            let content = std::fs::read_to_string(&path).map_err(|e| {
                ScriptError::new(ErrorKind::FileNotFound, format!("{}: {}", file, e))
            })?;
            output.push_str(&content);
        }

        // Go-compatible: cat returns WaitFunc → engine logs and sets stdout/stderr
        state.stdout = output;
        state.stderr = String::new();
        // Go-compatible: engine logs stdout from WaitFunc results
        if !state.stdout.is_empty() {
            state.logf(&format!("[stdout]\n{}", state.stdout));
        }
        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Concatenate files and print to stdout".into(),
            args: "file...".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// cd — change directory
// ──────────────────────────────────────────────────────────

struct CdCmd;

impl Cmd for CdCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.len() != 1 {
            return Err(ScriptError::usage("cd", "dir"));
        }

        state.chdir(&args[0]).map_err(|e| {
            ScriptError::new(ErrorKind::FileNotFound, format!("cd {}: {}", args[0], e))
        })?;

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Change working directory".into(),
            args: "dir".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// cp — copy files
// ──────────────────────────────────────────────────────────

struct CpCmd;

impl Cmd for CpCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.len() < 2 {
            return Err(ScriptError::usage("cp", "src... dst"));
        }

        let dst = &args[args.len() - 1];
        let srcs = &args[..args.len() - 1];

        let dst_path = state.resolve_path(dst);
        let dst_is_dir = dst_path.is_dir();

        // Go-compatible: error if multiple sources and dst is not a directory
        if srcs.len() > 1 && !dst_is_dir {
            return Err(ScriptError::new(ErrorKind::Io,
                format!("cp: destination is not a directory")));
        }

        for src in srcs {
            // Read source: support "stdout" and "stderr" virtual files
            let (data, mode) = match src.as_str() {
                "stdout" => (state.stdout.as_bytes().to_vec(), 0o666u32),
                "stderr" => (state.stderr.as_bytes().to_vec(), 0o666u32),
                _ => {
                    let src_path = state.resolve_path(src);
                    let data = std::fs::read(&src_path).map_err(|e| {
                        ScriptError::new(ErrorKind::FileNotFound, format!("cp: {}: {}", src, e))
                    })?;
                    // Preserve file mode on Unix
                    #[cfg(unix)]
                    let mode = {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::metadata(&src_path)
                            .map(|m| m.permissions().mode() & 0o777)
                            .unwrap_or(0o666)
                    };
                    #[cfg(not(unix))]
                    let mode = 0o666u32;
                    (data, mode)
                }
            };

            let target = if dst_is_dir && dst_path.is_dir() {
                let name = std::path::Path::new(src).file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new(src));
                dst_path.join(name)
            } else {
                dst_path.clone()
            };

            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).ok();
            }

            std::fs::write(&target, &data).map_err(|e| {
                ScriptError::new(ErrorKind::Io, format!("cp: write {}: {}", target.display(), e))
            })?;

            // Set permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&target,
                    std::fs::Permissions::from_mode(mode));
            }
            let _ = mode; // suppress unused warning on non-unix
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Copy files".into(),
            args: "src... dst".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// echo — print to stdout buffer
// ──────────────────────────────────────────────────────────

struct EchoCmd;

impl Cmd for EchoCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        // Go-compatible: echo returns WaitFunc → engine logs and sets stdout/stderr
        state.stdout = args.join(" ") + "\n";
        state.stderr = String::new();
        if !state.stdout.is_empty() {
            state.logf(&format!("[stdout]\n{}", state.stdout));
        }
        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Print arguments to stdout buffer".into(),
            args: "[string...]".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// env — set/print environment variables
// ──────────────────────────────────────────────────────────

struct EnvCmd;

impl Cmd for EnvCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.is_empty() {
            // Print all environment variables
            let mut output = String::new();
            for (k, v) in state.environ() {
                output.push_str(&format!("{}={}\n", k, v));
            }
            // Go-compatible: env returns WaitFunc → engine sets both stdout and stderr
            state.stdout = output;
            state.stderr = String::new();
            // Go-compatible: engine logs stdout from WaitFunc results
            if !state.stdout.is_empty() {
                state.logf(&format!("[stdout]\n{}", state.stdout));
            }
            return Ok(CmdResult::Ok);
        }

        let mut output = String::new();
        for arg in args {
            if let Some(eq_pos) = arg.find('=') {
                let key = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                state.setenv(key, value);
            } else {
                // Go-compatible: always print KEY=value, even if unset (value is empty)
                let val = state.getenv(arg).unwrap_or("");
                output.push_str(&format!("{}={}\n", arg, val));
            }
        }

        if !output.is_empty() {
            // Go-compatible: env returns WaitFunc → engine sets both stdout and stderr
            state.stdout = output;
            state.stderr = String::new();
            // Go-compatible: engine logs stdout from WaitFunc results
            state.logf(&format!("[stdout]\n{}", state.stdout));
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Set or print environment variables".into(),
            args: "[key[=value]...]".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// mkdir — create directories
// ──────────────────────────────────────────────────────────

struct MkdirCmd;

impl Cmd for MkdirCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::usage("mkdir", "dir..."));
        }

        for dir in args {
            let path = state.resolve_path(dir);
            std::fs::create_dir_all(&path).map_err(|e| {
                ScriptError::new(ErrorKind::Io, format!("mkdir {}: {}", dir, e))
            })?;
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Create directories".into(),
            args: "dir...".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// rm — remove files/directories
// ──────────────────────────────────────────────────────────

struct RmCmd;

impl Cmd for RmCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.is_empty() {
            return Err(ScriptError::usage("rm", "path..."));
        }

        for path_str in args {
            let path = state.resolve_path(path_str);
            if path.is_dir() {
                std::fs::remove_dir_all(&path).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("rm {}: {}", path_str, e))
                })?;
            } else if path.exists() {
                std::fs::remove_file(&path).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("rm {}: {}", path_str, e))
                })?;
            }
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Remove files or directories".into(),
            args: "path...".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// mv — rename/move files
// ──────────────────────────────────────────────────────────

struct MvCmd;

impl Cmd for MvCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.len() != 2 {
            return Err(ScriptError::usage("mv", "old new"));
        }

        let src = state.resolve_path(&args[0]);
        let dst = state.resolve_path(&args[1]);

        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        std::fs::rename(&src, &dst).map_err(|e| {
            ScriptError::new(ErrorKind::Io, format!("mv {} {}: {}", args[0], args[1], e))
        })?;

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Rename or move files".into(),
            args: "old new".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// replace — string replacement in file
// ──────────────────────────────────────────────────────────

struct ReplaceCmd;

impl Cmd for ReplaceCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        // replace [old new]... file
        // Go-compatible: requires odd number of args (len(args)%2 == 1).
        // 1 arg = just file (0 pairs, no-op rewrite). 3+ args = pairs + file.
        if args.is_empty() || args.len() % 2 == 0 {
            return Err(ScriptError::usage("replace", "[old new]... file"));
        }

        let file = &args[args.len() - 1];
        let pairs = &args[..args.len() - 1];

        // Go-compatible: replace always reads from disk (no virtual stdout/stderr).
        // Go uses os.ReadFile(s.Path(args[len-1])).
        let path = state.resolve_path(file);
        let mut content = std::fs::read_to_string(&path).map_err(|e| {
            ScriptError::new(ErrorKind::FileNotFound, format!("{}: {}", file, e))
        })?;

        // Go-compatible: unquote escape sequences like \n, \t, etc.
        // Go uses strconv.Unquote(`"` + arg + `"`) which interprets Go string escapes.
        for pair in pairs.chunks(2) {
            let old = go_unquote(&pair[0]);
            let new = go_unquote(&pair[1]);
            content = content.replace(&old, &new);
        }

        std::fs::write(&path, content).map_err(|e| {
            ScriptError::new(ErrorKind::Io, format!("replace: write {}: {}", file, e))
        })?;

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Replace strings in a file".into(),
            args: "[old new]... file".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// stop — stop script execution
// ──────────────────────────────────────────────────────────

struct StopCmd;

impl Cmd for StopCmd {
    fn run(&self, _state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        // Go-compatible: at most 1 argument
        if args.len() > 1 {
            return Err(ScriptError::usage("stop", "[msg]"));
        }
        let msg = if args.is_empty() {
            "stopped".to_string()
        } else {
            args[0].clone()
        };
        Ok(CmdResult::Stop(msg))
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Stop script execution".into(),
            args: "[message]".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// skip — skip the test
// ──────────────────────────────────────────────────────────

struct SkipCmd;

impl Cmd for SkipCmd {
    fn run(&self, _state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        let msg = if args.is_empty() {
            "skipped".to_string()
        } else {
            args.join(" ")
        };
        Ok(CmdResult::Skip(msg))
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Skip the test".into(),
            args: "[reason]".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// look_path — find executable using script's PATH (Go-compatible)
// ──────────────────────────────────────────────────────────

/// Look up an executable by name using the script's PATH environment variable.
///
/// This is the Rust equivalent of Go's `lookPath()` from `cmds.go`.
/// Unlike `std::process::Command` which uses the parent process PATH,
/// this uses the PATH from the script's environment state.
fn look_path(state: &State, command: &str) -> Result<String, String> {
    let command_path = std::path::Path::new(command);

    // If the command contains a path separator, use it directly
    if command.contains(std::path::MAIN_SEPARATOR) || command.contains('/') {
        return Ok(command.to_string());
    }

    #[cfg(windows)]
    let extensions: Vec<String> = {
        // Go-compatible: use parent process's PathExt, NOT the script's.
        // Go comment: "If PathExt is set in the command's environment,
        // cmd.Start fails with 'parameter is invalid'."
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        pathext.split(';').map(|s| s.to_lowercase()).collect()
    };

    #[cfg(windows)]
    let search_ext = {
        let cmd_ext = command_path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_lowercase()))
            .unwrap_or_default();
        !extensions.iter().any(|ext| ext.eq_ignore_ascii_case(&cmd_ext))
    };

    let path_env = state.getenv("PATH").unwrap_or("");

    for dir in std::env::split_paths(path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }

        #[cfg(windows)]
        {
            if search_ext {
                // Try each extension
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        for ext in &extensions {
                            let expected = format!("{}{}", command, ext);
                            if entry.file_name().to_string_lossy().eq_ignore_ascii_case(&expected)
                                && !entry.file_type().map(|t| t.is_dir()).unwrap_or(true)
                            {
                                return Ok(dir.join(entry.file_name()).to_string_lossy().to_string());
                            }
                        }
                    }
                }
            } else {
                let path = dir.join(command);
                if path.is_file() {
                    return Ok(path.to_string_lossy().to_string());
                }
            }
        }

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join(command);
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                    return Ok(path.to_string_lossy().to_string());
                }
            }
        }
    }

    Err(format!("executable not found: {}", command))
}

/// Go-compatible string unquoting.
///
/// Interprets Go escape sequences like `\n`, `\t`, `\\`, `\uXXXX`, `\UXXXXXXXX`, `\NNN`, etc.
/// Equivalent to Go's `strconv.Unquote("\"" + s + "\"")`.
fn go_unquote(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some('"') => result.push('"'),
            Some('\'') => result.push('\''),
            Some('a') => result.push('\x07'), // bell
            Some('b') => result.push('\x08'), // backspace
            Some('f') => result.push('\x0C'), // form feed
            Some('v') => result.push('\x0B'), // vertical tab
            Some('x') => {
                // \xHH — two hex digits
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(&h) = chars.peek() {
                        if h.is_ascii_hexdigit() {
                            hex.push(h);
                            chars.next();
                        }
                    }
                }
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte as char);
                } else {
                    result.push_str("\\x");
                    result.push_str(&hex);
                }
            }
            Some('u') => {
                // \uXXXX — 4 hex digits (Go unicode escape)
                let mut hex = String::new();
                for _ in 0..4 {
                    if let Some(&h) = chars.peek() {
                        if h.is_ascii_hexdigit() {
                            hex.push(h);
                            chars.next();
                        }
                    }
                }
                if hex.len() == 4 {
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            result.push(ch);
                        } else {
                            result.push_str("\\u");
                            result.push_str(&hex);
                        }
                    } else {
                        result.push_str("\\u");
                        result.push_str(&hex);
                    }
                } else {
                    result.push_str("\\u");
                    result.push_str(&hex);
                }
            }
            Some('U') => {
                // \UXXXXXXXX — 8 hex digits (Go unicode escape)
                let mut hex = String::new();
                for _ in 0..8 {
                    if let Some(&h) = chars.peek() {
                        if h.is_ascii_hexdigit() {
                            hex.push(h);
                            chars.next();
                        }
                    }
                }
                if hex.len() == 8 {
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(code) {
                            result.push(ch);
                        } else {
                            result.push_str("\\U");
                            result.push_str(&hex);
                        }
                    } else {
                        result.push_str("\\U");
                        result.push_str(&hex);
                    }
                } else {
                    result.push_str("\\U");
                    result.push_str(&hex);
                }
            }
            Some(c) if c.is_ascii_digit() && c < '8' => {
                // Octal escape \NNN — up to 3 octal digits (Go compat)
                let mut oct = String::new();
                oct.push(c);
                for _ in 0..2 {
                    if let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() && d < '8' {
                            oct.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(byte) = u8::from_str_radix(&oct, 8) {
                    result.push(byte as char);
                } else {
                    result.push('\\');
                    result.push_str(&oct);
                }
            }
            Some(other) => {
                // Unknown escape — preserve as-is
                result.push('\\');
                result.push(other);
            }
            None => {
                result.push('\\');
            }
        }
    }
    result
}

/// Parse an unsigned integer with Go-compatible base-0 detection.
///
/// Equivalent to Go's `strconv.ParseUint(s, 0, 32)`:
/// - "0x" or "0X" prefix → hex
/// - "0o" or "0O" prefix → octal
/// - "0b" or "0B" prefix → binary
/// - "0" prefix (without letter) → octal
/// - otherwise → decimal
fn parse_uint_base0(s: &str) -> Result<u32, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty string".to_string());
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else if let Some(oct) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
        u32::from_str_radix(oct, 8).map_err(|e| e.to_string())
    } else if let Some(bin) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        u32::from_str_radix(bin, 2).map_err(|e| e.to_string())
    } else if s.starts_with('0') && s.len() > 1 {
        // Leading 0 without letter → octal (Go compat)
        u32::from_str_radix(s, 8).map_err(|e| e.to_string())
    } else {
        s.parse::<u32>().map_err(|e| e.to_string())
    }
}

// ──────────────────────────────────────────────────────────
// chmod — change file mode bits
// ──────────────────────────────────────────────────────────

struct ChmodCmd;

impl Cmd for ChmodCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.len() < 2 {
            return Err(ScriptError::usage("chmod", "perm paths..."));
        }

        // Go-compatible: uses strconv.ParseUint(args[0], 0, 32) which auto-detects base:
        // "0777" → octal (0 prefix), "0x1ff" → hex, "511" → decimal
        let perm = parse_uint_base0(&args[0]).map_err(|_| {
            ScriptError::new(ErrorKind::SyntaxError, format!("invalid mode: {}", args[0]))
        })?;

        // Validate only file permission bits (Go: perm&uint64(fs.ModePerm) != perm)
        if perm & 0o777 != perm {
            return Err(ScriptError::new(ErrorKind::SyntaxError,
                format!("invalid mode: {}", args[0])));
        }

        for arg in &args[1..] {
            let path = state.resolve_path(arg);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path,
                    std::fs::Permissions::from_mode(perm))
                    .map_err(|e| {
                        ScriptError::new(ErrorKind::Io, format!("chmod {}: {}", arg, e))
                    })?;
            }
            #[cfg(not(unix))]
            {
                // On Windows, set readonly if no write bits
                let readonly = perm & 0o222 == 0;
                let meta = std::fs::metadata(&path).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("chmod {}: {}", arg, e))
                })?;
                let mut perms = meta.permissions();
                perms.set_readonly(readonly);
                std::fs::set_permissions(&path, perms).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("chmod {}: {}", arg, e))
                })?;
            }
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Change file mode bits".into(),
            args: "perm paths...".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// symlink — create symbolic link
// ──────────────────────────────────────────────────────────

struct SymlinkCmd;

impl Cmd for SymlinkCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        // Go syntax: symlink path -> target
        if args.len() != 3 || args[1] != "->" {
            return Err(ScriptError::usage("symlink", "path -> target"));
        }

        let link_path = state.resolve_path(&args[0]);
        // Go-compatible: target is NOT resolved with s.Path — it's relative
        // to the directory the link is in
        let target = args[2].replace('/', &std::path::MAIN_SEPARATOR.to_string());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link_path).map_err(|e| {
                ScriptError::new(ErrorKind::Io, format!("symlink: {}", e))
            })?;
        }
        #[cfg(windows)]
        {
            // On Windows, try dir symlink first, fall back to file symlink
            let target_resolved = if std::path::Path::new(&target).is_absolute() {
                std::path::PathBuf::from(&target)
            } else {
                link_path.parent().unwrap_or(std::path::Path::new(".")).join(&target)
            };
            if target_resolved.is_dir() {
                std::os::windows::fs::symlink_dir(&target, &link_path).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("symlink: {}", e))
                })?;
            } else {
                std::os::windows::fs::symlink_file(&target, &link_path).map_err(|e| {
                    ScriptError::new(ErrorKind::Io, format!("symlink: {}", e))
                })?;
            }
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Create a symbolic link".into(),
            args: "path -> target".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

// ──────────────────────────────────────────────────────────
// sleep — sleep for a duration
// ──────────────────────────────────────────────────────────

struct SleepCmd;

impl Cmd for SleepCmd {
    fn run(&self, _state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if args.len() != 1 {
            return Err(ScriptError::usage("sleep", "duration"));
        }

        let duration = parse_go_duration(&args[0]).map_err(|e| {
            ScriptError::new(ErrorKind::SyntaxError, format!("sleep: {}", e))
        })?;

        // Always spawn a thread — matches Go's pattern where sleep always
        // returns a WaitFunc. The engine decides fg (join immediately) vs bg.
        let handle = std::thread::spawn(move || {
            std::thread::sleep(duration);
            Ok(())
        });

        Ok(CmdResult::Background(crate::engine::WaitHandle::Thread(handle)))
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Sleep for a specified duration".into(),
            args: "duration".into(),
            regexp_args: None,
            async_: true,
        }
    }
}

/// Parse a Go-style duration string (e.g., "1s", "100ms", "1m30s", "500us").
fn parse_go_duration(s: &str) -> Result<std::time::Duration, String> {
    let mut total_nanos: u128 = 0;
    let mut chars = s.chars().peekable();
    let mut found_unit = false;

    while chars.peek().is_some() {
        // Parse number (potentially with decimal)
        let mut num_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num_str.push(c);
                chars.next();
            } else {
                break;
            }
        }

        if num_str.is_empty() {
            return Err(format!("invalid duration: {}", s));
        }

        let num: f64 = num_str.parse().map_err(|_| format!("invalid duration: {}", s))?;

        // Parse unit
        let mut unit = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_alphabetic() || c == 'µ' {
                unit.push(c);
                chars.next();
            } else {
                break;
            }
        }

        let nanos_per_unit: f64 = match unit.as_str() {
            "ns" => 1.0,
            "us" | "µs" => 1_000.0,
            "ms" => 1_000_000.0,
            "s" => 1_000_000_000.0,
            "m" => 60_000_000_000.0,
            "h" => 3_600_000_000_000.0,
            _ => return Err(format!("unknown unit {:?} in duration {:?}", unit, s)),
        };

        total_nanos += (num * nanos_per_unit) as u128;
        found_unit = true;
    }

    if !found_unit {
        return Err(format!("missing unit in duration {:?}", s));
    }

    Ok(std::time::Duration::from_nanos(total_nanos as u64))
}

// ──────────────────────────────────────────────────────────
// wait — wait for background commands
// ──────────────────────────────────────────────────────────

struct WaitCmd;

impl Cmd for WaitCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        if !args.is_empty() {
            return Err(ScriptError::usage("wait", ""));
        }

        let bg_cmds: Vec<crate::state::BackgroundCmd> = std::mem::take(&mut state.background);

        let mut stdouts = Vec::new();
        let mut stderrs = Vec::new();
        let mut errors = Vec::new();

        for bg in bg_cmds {
            let before_args = if bg.args.is_empty() { "" } else { " " };
            state.logf(&format!("[background] {}{}{}", bg.name, before_args,
                bg.args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ")));

            let (stdout, stderr, err) = bg.handle.wait();

            if !stdout.is_empty() {
                state.logf(&format!("[stdout]\n{}", stdout));
                stdouts.push(stdout);
            }
            if !stderr.is_empty() {
                state.logf(&format!("[stderr]\n{}", stderr));
                stderrs.push(stderr);
            }

            if let Some(err_detail) = err {
                let err_msg = format!("{}: {}", bg.name, err_detail);
                state.logf(&format!("[{}]", err_msg));
                if bg.negate {
                    // Expected failure — ok
                } else if !bg.may_fail {
                    errors.push(err_msg);
                }
            } else if bg.negate {
                let err_msg = format!("{}: unexpected success", bg.name);
                errors.push(err_msg);
            }
        }

        // Go-compatible: stdout/stderr are the concatenation of all background outputs
        state.stdout = stdouts.join("");
        state.stderr = stderrs.join("");

        if !errors.is_empty() {
            return Err(ScriptError::new(ErrorKind::WaitError, errors.join("\n")));
        }

        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Wait for background commands to complete".into(),
            args: "".into(),
            regexp_args: None,
            async_: false,
        }
    }
}

/// Quote an argument for display, matching Go's `quoteArgs`.
///
/// Go checks `strings.ContainsAny(arg, "'"+argSepChars)` where
/// `argSepChars = " \t\r\n#"`. If the arg contains any of those
/// characters, it wraps in single quotes and doubles embedded `'`.
fn quote_arg(s: &str) -> String {
    const NEED_QUOTE: &[char] = &['\'', ' ', '\t', '\r', '\n', '#'];
    if s.contains(NEED_QUOTE) {
        format!("'{}'", s.replace('\'', "''"))
    } else {
        s.to_string()
    }
}

// ──────────────────────────────────────────────────────────
// help — display command help
// ──────────────────────────────────────────────────────────

pub struct HelpCmd {
    /// (name, args, summary) for each command
    cmd_info: Vec<(String, String, String)>,
    /// Condition names
    cond_names: Vec<String>,
}

impl HelpCmd {
    pub fn new(cmd_info: Vec<(String, String, String)>, cond_names: Vec<String>) -> Self {
        Self { cmd_info, cond_names }
    }
}

impl Cmd for HelpCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        let mut output = String::new();

        if args.is_empty() {
            // List all commands
            for (name, args_str, summary) in &self.cmd_info {
                if args_str.is_empty() {
                    output.push_str(&format!("{}\n    {}\n", name, summary));
                } else {
                    output.push_str(&format!("{} {}\n    {}\n", name, args_str, summary));
                }
            }

            if !self.cond_names.is_empty() {
                output.push_str("\nconditions:\n");
                for name in &self.cond_names {
                    output.push_str(&format!("    {}\n", name));
                }
            }
        } else {
            // Help for specific commands
            for name in args {
                if let Some((_, args_str, summary)) = self.cmd_info.iter().find(|(n, _, _)| n == name) {
                    if args_str.is_empty() {
                        output.push_str(&format!("{}\n    {}\n", name, summary));
                    } else {
                        output.push_str(&format!("{} {}\n    {}\n", name, args_str, summary));
                    }
                } else {
                    output.push_str(&format!("{}: unknown command\n", name));
                }
            }
        }

        state.stdout = output;
        state.stderr = String::new();
        // Go-compatible: help returns WaitFunc → engine logs stdout
        if !state.stdout.is_empty() {
            state.logf(&format!("[stdout]\n{}", state.stdout));
        }
        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Display help for commands".into(),
            args: "[command...]".into(),
            regexp_args: None,
            async_: false,
        }
    }
}
