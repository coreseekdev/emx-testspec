//! Script execution state
//!
//! Holds mutable per-run state: working directory, environment variables,
//! stdout/stderr buffers, and log.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use crate::engine::WaitHandle;

/// A background command waiting to be harvested by `wait`.
pub struct BackgroundCmd {
    /// The async handle (subprocess or thread)
    pub handle: WaitHandle,
    /// Command name (for logging)
    pub name: String,
    /// Command arguments (for logging)
    pub args: Vec<String>,
    /// Whether the command was negated (! prefix)
    pub negate: bool,
    /// Whether the command may fail (? prefix)
    pub may_fail: bool,
}

/// Mutable state for a single script execution
pub struct State {
    /// Initial working directory (archive files extracted here)
    pub workdir: PathBuf,
    /// Current working directory (changed by `cd`)
    pub pwd: PathBuf,
    /// Environment variables — ordered for deterministic subprocess env
    env: Vec<(String, String)>,
    /// Index for O(1) lookup by key → position in `env` vec
    env_index: HashMap<String, usize>,
    /// Last command's stdout
    pub stdout: String,
    /// Last command's stderr
    pub stderr: String,
    /// Last command's exit code
    pub exit_code: Option<i32>,
    /// Execution log
    pub log: String,
    /// Background commands waiting to be harvested
    pub background: Vec<BackgroundCmd>,
}

impl State {
    /// Create a new State with the given working directory
    pub fn new(workdir: PathBuf) -> Self {
        let pwd = workdir.clone();
        let mut state = Self {
            workdir,
            pwd,
            env: Vec::new(),
            env_index: HashMap::new(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            log: String::new(),
            background: Vec::new(),
        };

        // Go-compatible: inherit all parent environment variables by default.
        // Go's NewState(ctx, workdir, initialEnv) defaults to os.Environ()
        // when initialEnv is nil, meaning subprocesses inherit the full env.
        for (key, value) in std::env::vars() {
            state.setenv(&key, &value);
        }

        // Set script-specific environment variables (override inherited ones)
        let workdir_str = state.workdir.to_string_lossy().to_string();
        state.setenv("WORK", workdir_str.clone());
        state.setenv("TMPDIR", workdir_str.clone());
        // Go-compatible: set PWD to workdir initially (Go's NewState does this)
        state.setenv("PWD", workdir_str);

        // Go-compatible: pseudo-variables for platform-independent paths.
        // ${/} expands to the OS path separator, ${:} to the path list separator.
        state.setenv("/", std::path::MAIN_SEPARATOR.to_string());
        #[cfg(windows)]
        state.setenv(":", ";");
        #[cfg(not(windows))]
        state.setenv(":", ":");

        state
    }

    /// Set an environment variable.
    ///
    /// On Windows, environment variable names are case-insensitive
    /// (Go-compatible: exec.Cmd.Environ() deduplicates with EqualFold).
    pub fn setenv(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        let lookup_key = self.find_env_key(&key).map(|s| s.to_string());
        if let Some(existing_key) = lookup_key {
            let idx = self.env_index[&existing_key];
            self.env[idx].1 = value;
        } else {
            let idx = self.env.len();
            self.env.push((key.clone(), value));
            self.env_index.insert(key, idx);
        }
    }

    /// Get an environment variable.
    ///
    /// On Windows, lookup is case-insensitive.
    pub fn getenv(&self, key: &str) -> Option<&str> {
        self.find_env_key(key)
            .and_then(|k| self.env_index.get(k))
            .map(|&idx| self.env[idx].1.as_str())
    }

    /// Find the canonical key name in env_index (case-insensitive on Windows).
    fn find_env_key(&self, key: &str) -> Option<&str> {
        #[cfg(windows)]
        {
            for k in self.env_index.keys() {
                if k.eq_ignore_ascii_case(key) {
                    return Some(k.as_str());
                }
            }
            None
        }
        #[cfg(not(windows))]
        {
            if self.env_index.contains_key(key) {
                Some(key)
            } else {
                None
            }
        }
    }

    /// Get all environment variables as key=value pairs for subprocess.
    ///
    /// Go-compatible: pseudo-variables ${/} and ${:} are excluded from
    /// subprocess environment (Go stores them in envMap but not in env slice).
    pub fn environ(&self) -> Vec<(&str, &str)> {
        self.env.iter()
            .filter(|(k, _)| k != "/" && k != ":")
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    /// Resolve a path relative to the current working directory.
    ///
    /// Go-compatible: equivalent to Go's `s.Path(path)` which calls
    /// `filepath.Clean` on absolute paths and `filepath.Join` (which
    /// includes Clean) on relative paths. This normalizes `.`, `..`,
    /// and double separators.
    pub fn resolve_path(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() {
            clean_path(p)
        } else {
            clean_path(&self.pwd.join(path))
        }
    }

    /// Resolve a path and verify it stays within the workdir sandbox.
    /// Returns an error for paths that would escape via `..` or absolute paths
    /// pointing outside the workdir.
    pub fn resolve_sandboxed(&self, path: &str) -> Result<PathBuf, std::io::Error> {
        let resolved = self.resolve_path(path);
        self.ensure_within_workdir(&resolved)?;
        Ok(resolved)
    }

    /// Verify that a path does not escape the workdir sandbox.
    fn ensure_within_workdir(&self, path: &Path) -> Result<(), std::io::Error> {
        // Normalize by resolving as many components as exist on disk,
        // then check the remainder for `..` traversal.
        let canonical_work = self.workdir.canonicalize().unwrap_or_else(|_| self.workdir.clone());

        // For paths that don't fully exist yet (e.g. mkdir targets), walk
        // the longest existing prefix and verify each `..` doesn't escape.
        let mut check = path.to_path_buf();
        // Try to canonicalize; if the path doesn't exist, canonicalize parent
        if let Ok(canon) = check.canonicalize() {
            check = canon;
        } else if let Some(parent) = check.parent() {
            if let Ok(canon_parent) = parent.canonicalize() {
                if let Some(filename) = path.file_name() {
                    check = canon_parent.join(filename);
                }
            }
        }

        if !check.starts_with(&canonical_work) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "path escapes workdir sandbox: {} (workdir: {})",
                    path.display(),
                    self.workdir.display(),
                ),
            ));
        }
        Ok(())
    }

    /// Change the current working directory.
    /// Also updates the PWD environment variable (Go compatibility).
    pub fn chdir(&mut self, dir: &str) -> Result<(), std::io::Error> {
        let new_pwd = self.resolve_path(dir);
        if !new_pwd.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("directory not found: {}", new_pwd.display()),
            ));
        }
        self.ensure_within_workdir(&new_pwd)?;
        let pwd_str = new_pwd.to_string_lossy().to_string();
        self.pwd = new_pwd;
        self.setenv("PWD", pwd_str);
        Ok(())
    }

    /// Expand environment variables in a string.
    ///
    /// When `in_regexp` is true, expanded values are regex-escaped
    /// (equivalent to Go's `regexp.QuoteMeta`).
    pub fn expand_env(&self, s: &str, in_regexp: bool) -> String {
        crate::parser::expand_env(s, &|key| self.getenv(key).map(|s| s.to_string()), in_regexp)
    }

    /// Expand environment variables in a string (simple mode, no regexp escaping).
    /// Convenience wrapper for backward compatibility.
    pub fn expand(&self, s: &str) -> String {
        self.expand_env(s, false)
    }

    /// Write a log entry
    pub fn logf(&mut self, msg: &str) {
        self.log.push_str(msg);
        if !msg.ends_with('\n') {
            self.log.push('\n');
        }
    }

    /// Extract files from a txtar archive into the working directory.
    /// Rejects file names that would escape the workdir via path traversal.
    ///
    /// Go-compatible: file names have environment variables expanded before use,
    /// and paths are resolved relative to pwd (via `resolve_path`).
    pub fn extract_files(&self, archive: &emx_txtar::Archive) -> Result<(), std::io::Error> {
        for file in &archive.files {
            // Go-compatible: expand env vars in file names (like Go's ExtractFiles)
            let expanded_name = self.expand_env(&file.name, false);
            let path = self.resolve_path(&expanded_name);
            // Ensure archive entries can't escape workdir (CWE-22)
            self.ensure_within_workdir(&path).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("archive file '{}': {}", file.name, e),
                )
            })?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, &file.data)?;
        }
        Ok(())
    }

    /// Read a file, treating "stdout" and "stderr" as virtual files.
    /// Line endings are normalized to LF for consistent comparison.
    pub fn read_file(&self, name: &str) -> Result<String, std::io::Error> {
        let content = match name {
            "stdout" => self.stdout.clone(),
            "stderr" => self.stderr.clone(),
            _ => {
                let path = self.resolve_path(name);
                std::fs::read_to_string(&path)?
            }
        };
        // Normalize CRLF → LF for cross-platform consistency
        Ok(content.replace("\r\n", "\n"))
    }
}

/// Clean a path by resolving `.` and `..` components lexically.
///
/// Equivalent to Go's `filepath.Clean()`. Unlike `canonicalize()`,
/// this does not require the path to exist on disk.
fn clean_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    let mut has_root = false;
    let mut prefix: Option<std::path::Component> = None;

    for component in path.components() {
        match component {
            std::path::Component::RootDir => {
                has_root = true;
                components.clear();
            }
            std::path::Component::Prefix(_) => {
                prefix = Some(component);
                components.clear();
            }
            std::path::Component::CurDir => {
                // skip '.'
            }
            std::path::Component::ParentDir => {
                // Pop last normal component if any; for absolute paths, '..' at root is ignored
                if let Some(last) = components.last() {
                    if matches!(last, std::path::Component::Normal(_)) {
                        components.pop();
                    } else if !has_root {
                        components.push(component);
                    }
                } else if !has_root {
                    components.push(component);
                }
            }
            std::path::Component::Normal(_) => {
                components.push(component);
            }
        }
    }

    if components.is_empty() && !has_root && prefix.is_none() {
        return PathBuf::from(".");
    }

    let mut result = PathBuf::new();
    if let Some(p) = prefix {
        result.push(p);
    }
    if has_root {
        result.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for c in &components {
        result.push(c);
    }
    result
}
