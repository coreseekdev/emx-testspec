//! exec — execute a subprocess

use std::process::Command as ProcessCommand;
use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;

pub(super) struct ExecCmd;

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
