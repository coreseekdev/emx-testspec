//! Script conditions
//!
//! Conditions are used in `[cond]` guards to conditionally execute commands.
//! Follows Go's `cmd/internal/script` condition model with prefix validation.

use std::collections::HashMap;
use crate::error::ScriptError;
use crate::state::State;

/// A condition that can be evaluated
pub trait Condition: Send + Sync {
    /// Evaluate the condition.
    /// `suffix` is for prefix conditions like `GOOS:linux` (suffix = "linux").
    /// For non-prefix conditions, suffix is "".
    fn eval(&self, state: &State, suffix: &str) -> Result<bool, ScriptError>;

    /// Brief description
    fn summary(&self) -> &str;

    /// Whether this is a prefix condition requiring a `:suffix`.
    /// Go-compatible: prefix conditions require a colon-separated suffix.
    fn is_prefix(&self) -> bool;
}

/// Boxed condition
pub type BoxedCondition = Box<dyn Condition>;

/// Return the default set of conditions
pub fn default_conditions() -> HashMap<String, BoxedCondition> {
    let mut conds: HashMap<String, BoxedCondition> = HashMap::new();

    // OS conditions (non-prefix)
    conds.insert("unix".into(), Box::new(BoolCondition {
        summary: "true on Unix-like systems".into(),
        value: cfg!(unix),
    }));
    conds.insert("windows".into(), Box::new(BoolCondition {
        summary: "true on Windows".into(),
        value: cfg!(windows),
    }));
    conds.insert("darwin".into(), Box::new(BoolCondition {
        summary: "true on macOS".into(),
        value: cfg!(target_os = "macos"),
    }));
    conds.insert("linux".into(), Box::new(BoolCondition {
        summary: "true on Linux".into(),
        value: cfg!(target_os = "linux"),
    }));

    // Architecture conditions (non-prefix)
    conds.insert("amd64".into(), Box::new(BoolCondition {
        summary: "true on x86_64".into(),
        value: cfg!(target_arch = "x86_64"),
    }));
    conds.insert("arm64".into(), Box::new(BoolCondition {
        summary: "true on aarch64".into(),
        value: cfg!(target_arch = "aarch64"),
    }));

    // GOOS prefix condition (for Go compatibility)
    conds.insert("GOOS".into(), Box::new(PrefixCondition {
        summary: "match target OS".into(),
        eval_fn: Box::new(|suffix| {
            let os = if cfg!(target_os = "windows") { "windows" }
                else if cfg!(target_os = "linux") { "linux" }
                else if cfg!(target_os = "macos") { "darwin" }
                else { "unknown" };
            os == suffix
        }),
    }));

    // exec prefix condition — check if a program is in PATH
    conds.insert("exec".into(), Box::new(ExecCondition));

    conds
}

/// A static boolean condition (non-prefix)
struct BoolCondition {
    summary: String,
    value: bool,
}

impl Condition for BoolCondition {
    fn eval(&self, _state: &State, suffix: &str) -> Result<bool, ScriptError> {
        // Go-compatible: non-prefix conditions reject suffixes
        if !suffix.is_empty() {
            return Err(ScriptError::syntax(
                format!("condition does not accept a suffix, got :{}", suffix),
            ));
        }
        Ok(self.value)
    }
    fn summary(&self) -> &str {
        &self.summary
    }
    fn is_prefix(&self) -> bool {
        false
    }
}

/// A prefix condition (evaluates based on suffix)
struct PrefixCondition {
    summary: String,
    eval_fn: Box<dyn Fn(&str) -> bool + Send + Sync>,
}

impl Condition for PrefixCondition {
    fn eval(&self, _state: &State, suffix: &str) -> Result<bool, ScriptError> {
        Ok((self.eval_fn)(suffix))
    }
    fn summary(&self) -> &str {
        &self.summary
    }
    fn is_prefix(&self) -> bool {
        true
    }
}

/// Condition that checks if an executable is in PATH (prefix condition)
struct ExecCondition;

impl Condition for ExecCondition {
    fn eval(&self, state: &State, suffix: &str) -> Result<bool, ScriptError> {
        if suffix.is_empty() {
            return Err(ScriptError::syntax("exec condition requires :program suffix"));
        }
        // Go-compatible: use the script's PATH, not the parent's PATH
        let path_env = state.getenv("PATH").unwrap_or("");
        Ok(which_exists(suffix, path_env))
    }

    fn summary(&self) -> &str {
        "true if program is in PATH"
    }

    fn is_prefix(&self) -> bool {
        true
    }
}

/// Check if a program exists in PATH
fn which_exists(name: &str, path_var: &str) -> bool {
    #[cfg(windows)]
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|s| s.to_lowercase())
        .collect();

    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(name);

        #[cfg(unix)]
        {
            if candidate.is_file() {
                return true;
            }
        }

        #[cfg(windows)]
        {
            if candidate.is_file() {
                return true;
            }
            for ext in &extensions {
                let with_ext = candidate.with_extension(ext.trim_start_matches('.'));
                if with_ext.is_file() {
                    return true;
                }
            }
        }
    }
    false
}