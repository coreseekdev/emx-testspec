//! File operation commands: exists, cat, cp, mkdir, rm, mv, chmod, symlink

use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;

// ──────────────────────────────────────────────────────────
// exists — check if files exist
// ──────────────────────────────────────────────────────────

pub(super) struct ExistsCmd;

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
// cat — concatenate files to stdout
// ──────────────────────────────────────────────────────────

pub(super) struct CatCmd;

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
// cp — copy files
// ──────────────────────────────────────────────────────────

pub(super) struct CpCmd;

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
// mkdir — create directories
// ──────────────────────────────────────────────────────────

pub(super) struct MkdirCmd;

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

pub(super) struct RmCmd;

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

pub(super) struct MvCmd;

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
// chmod — change file mode bits
// ──────────────────────────────────────────────────────────

pub(super) struct ChmodCmd;

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

pub(super) struct SymlinkCmd;

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
// Helpers
// ──────────────────────────────────────────────────────────

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
