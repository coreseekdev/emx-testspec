//! Script engine
//!
//! The Engine holds command and condition registries.
//! It is stateless config — one engine can run many scripts.

use std::collections::HashMap;
use crate::error::ScriptError;
use crate::parser::ArgFragment;
use crate::state::State;

/// Result returned by a command execution
pub enum CmdResult {
    /// Command completed successfully
    Ok,
    /// Command completed, script should stop
    Stop(String),
    /// Command completed, test should be skipped
    Skip(String),
    /// Command started asynchronously — returns a WaitHandle to harvest later
    Background(WaitHandle),
}

/// A handle to an asynchronous operation that can be waited on.
///
/// This generalizes the Go `WaitFunc` pattern: both subprocess (`exec`) and
/// thread-based (`sleep`) async commands return a `WaitHandle`.
pub enum WaitHandle {
    /// A running subprocess (exec)
    Process(std::process::Child),
    /// A running thread (sleep) — the JoinHandle returns (stdout, stderr, error).
    Thread(std::thread::JoinHandle<Result<(), String>>),
}

impl WaitHandle {
    /// Wait for the async operation to complete.
    /// Returns (stdout, stderr, Option<error_message>).
    pub fn wait(self) -> (String, String, Option<String>) {
        match self {
            WaitHandle::Process(child) => {
                match child.wait_with_output() {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let err = if output.status.success() {
                            None
                        } else {
                            Some(format!("exit code {}", output.status.code().unwrap_or(-1)))
                        };
                        (stdout, stderr, err)
                    }
                    Err(e) => (String::new(), String::new(), Some(e.to_string())),
                }
            }
            WaitHandle::Thread(handle) => {
                match handle.join() {
                    Ok(Ok(())) => (String::new(), String::new(), None),
                    Ok(Err(e)) => (String::new(), String::new(), Some(e)),
                    Err(_) => (String::new(), String::new(), Some("thread panicked".into())),
                }
            }
        }
    }
}

/// Usage information for a command
pub struct CmdUsage {
    /// One-line summary
    pub summary: String,
    /// Argument syntax
    pub args: String,
    /// Reports which arguments should be treated as regular expressions.
    /// Takes the raw (unexpanded, joined) argument strings and returns indices
    /// of arguments that are regex patterns.
    /// If None, no arguments are treated as regexp (no QuoteMeta escaping on expand).
    pub regexp_args: Option<fn(&[String]) -> Vec<usize>>,
    /// Whether this command can be run in the background with `&`.
    /// Only `exec` and `sleep` should set this to true.
    pub async_: bool,
}

/// A command that can be executed in a script
pub trait Cmd: Send + Sync {
    /// Execute the command
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError>;

    /// Return usage information
    fn usage(&self) -> CmdUsage;
}

/// A boxed command
pub type BoxedCmd = Box<dyn Cmd>;

/// The script engine — holds command and condition registries
pub struct Engine {
    /// Registered commands
    pub commands: HashMap<String, BoxedCmd>,
    /// Registered conditions
    pub conditions: HashMap<String, crate::conditions::BoxedCondition>,
    /// Whether to suppress command logging
    pub quiet: bool,
}

impl Engine {
    /// Create a new engine with default commands and conditions
    pub fn new() -> Self {
        let mut commands = crate::commands::default_commands();
        let conditions = crate::conditions::default_conditions();

        // Build the help command with knowledge of all registered commands/conditions.
        // Must be done after populating both registries.
        let mut cmd_help: Vec<(String, String, String)> = commands.iter()
            .map(|(name, cmd)| {
                let u = cmd.usage();
                (name.clone(), u.args.clone(), u.summary.clone())
            })
            .collect();
        cmd_help.sort_by(|a, b| a.0.cmp(&b.0));

        let mut cond_help: Vec<String> = conditions.keys().cloned().collect();
        cond_help.sort();

        commands.insert("help".into(),
            Box::new(crate::commands::HelpCmd::new(cmd_help, cond_help)));

        Self {
            commands,
            conditions,
            quiet: false,
        }
    }

    /// Register a custom command
    pub fn register_command(&mut self, name: impl Into<String>, cmd: BoxedCmd) {
        self.commands.insert(name.into(), cmd);
    }

    /// Register a custom condition
    pub fn register_condition(&mut self, name: impl Into<String>, cond: crate::conditions::BoxedCondition) {
        self.conditions.insert(name.into(), cond);
    }

    /// Execute a script from text (the comment section of a txtar archive)
    pub fn execute(
        &self,
        state: &mut State,
        script: &str,
        filename: &str,
    ) -> Result<(), ScriptError> {
        for (i, line) in script.lines().enumerate() {
            let line_number = i + 1;

            // Lines starting with # are section comments — log and skip.
            // Go-compatible: only lines where '#' is the very first character
            // (no leading whitespace) are section comments.
            if line.starts_with('#') {
                if !self.quiet {
                    state.logf(&format!("{}", line.trim()));
                }
                continue;
            }

            let parsed = match crate::parser::parse_line(line, line_number) {
                Ok(Some(p)) => p,
                Ok(None) => continue, // blank line
                Err(e) => {
                    return Err(ScriptError::syntax(e.message)
                        .with_location(filename, line_number));
                }
            };

            // Log the raw line
            if !self.quiet {
                state.logf(&format!("> {}", parsed.raw.trim()));
            }

            // Evaluate conditions
            let mut skip_line = false;
            for cond in &parsed.conditions {
                let result = self.eval_condition(state, cond)?;
                if !result {
                    skip_line = true;
                    break;
                }
            }
            if skip_line {
                state.logf("[condition not met]");
                continue;
            }

            // Look up command
            let cmd = self.commands.get(&parsed.command).ok_or_else(|| {
                ScriptError::syntax(format!("unknown command: {}", parsed.command))
                    .with_location(filename, line_number)
            })?;

            // Determine which args are regexp (for QuoteMeta-style expansion)
            let usage = cmd.usage();

            // Validate background usage: only async commands can use &
            if parsed.background && !usage.async_ {
                return Err(ScriptError::new(
                    crate::error::ErrorKind::SyntaxError,
                    format!("command {} does not support background execution (&)", parsed.command),
                ).with_location(filename, line_number));
            }

            let regexp_arg_indices = if let Some(regexp_args_fn) = usage.regexp_args {
                // Build raw (unexpanded, joined) args for the regexp_args function
                let raw_joined: Vec<String> = parsed.raw_args.iter()
                    .map(|frags| frags.iter().map(|f| f.s.as_str()).collect())
                    .collect();
                regexp_args_fn(&raw_joined)
            } else {
                Vec::new()
            };

            // Expand arguments: fragment-aware, with regexp escaping for regex args
            let expanded_args = expand_args(state, &parsed.raw_args, &regexp_arg_indices);

            // Execute command
            let result = cmd.run(state, &expanded_args);

            match result {
                Ok(CmdResult::Ok) => {
                    if parsed.negate {
                        return Err(ScriptError::new(
                            crate::error::ErrorKind::UnexpectedSuccess,
                            format!("command succeeded unexpectedly: {}", parsed.raw.trim()),
                        ).with_location(filename, line_number));
                    }
                }
                Ok(CmdResult::Stop(msg)) => {
                    state.logf(&format!("STOP: {}", msg));
                    return Ok(());
                }
                Ok(CmdResult::Skip(msg)) => {
                    return Err(ScriptError::skip(msg).with_location(filename, line_number));
                }
                Ok(CmdResult::Background(handle)) => {
                    if parsed.background {
                        // Push to background queue — will be harvested by `wait`
                        state.background.push(crate::state::BackgroundCmd {
                            handle,
                            name: parsed.command.clone(),
                            args: expanded_args,
                            negate: parsed.negate,
                            may_fail: parsed.may_fail,
                        });
                        // Go-compatible: clear stdout/stderr for background commands
                        state.stdout = String::new();
                        state.stderr = String::new();
                    } else {
                        // Foreground: wait immediately via WaitHandle
                        let (stdout, stderr, err) = handle.wait();

                        state.stdout = stdout;
                        state.stderr = stderr;
                        // exit_code only meaningful for Process handles
                        state.exit_code = None;

                        // Go-compatible: always log stdout/stderr (not gated by quiet)
                        if !state.stdout.is_empty() {
                            state.logf(&format!("[stdout]\n{}", state.stdout));
                        }
                        if !state.stderr.is_empty() {
                            state.logf(&format!("[stderr]\n{}", state.stderr));
                        }

                        if let Some(err_msg) = err {
                            let err = ScriptError::new(crate::error::ErrorKind::CommandFailed, err_msg);
                            if parsed.negate {
                                // Go-compatible: log expected error as [<err>]
                                state.logf(&format!("[{}]", err.message));
                            } else if parsed.may_fail {
                                state.logf(&format!("[{}]", err.message));
                            } else {
                                return Err(err.with_location(filename, line_number)
                                    .with_command(&parsed.command)
                                    .with_args(expanded_args.clone()));
                            }
                        } else if parsed.negate {
                            return Err(ScriptError::new(
                                crate::error::ErrorKind::UnexpectedSuccess,
                                format!("command succeeded unexpectedly: {}", parsed.raw.trim()),
                            ).with_location(filename, line_number));
                        }
                    }
                }
                Err(e) => {
                    if parsed.negate {
                        // Expected failure — continue
                        if !self.quiet {
                            state.logf(&format!("[expected failure: {}]", e.message));
                        }
                    } else if parsed.may_fail {
                        // May fail — continue
                        if !self.quiet {
                            state.logf(&format!("[allowed failure: {}]", e.message));
                        }
                    } else {
                        return Err(e.with_location(filename, line_number)
                            .with_command(&parsed.command)
                            .with_args(expanded_args.clone()));
                    }
                }
            }
        }

        Ok(())
    }

    /// Evaluate a condition — follows Go's `conditionsActive()` closely.
    ///
    /// The condition tag may be "name" or "name:suffix".
    /// Prefix conditions (like GOOS) require a suffix; non-prefix conditions reject suffixes.
    fn eval_condition(&self, state: &crate::state::State, cond: &crate::parser::ScriptCondition) -> Result<bool, ScriptError> {
        // Split tag on first ':' to separate prefix conditions
        let (prefix, suffix, has_colon) = if let Some(colon) = cond.tag.find(':') {
            (&cond.tag[..colon], Some(&cond.tag[colon + 1..]), true)
        } else {
            (cond.tag.as_str(), None, false)
        };

        let condition = if has_colon {
            let c = self.conditions.get(prefix).ok_or_else(|| {
                // Go-compatible: "unknown condition prefix %q"
                let mut known: Vec<&str> = self.conditions.keys().map(|s| s.as_str()).collect();
                known.sort();
                ScriptError::syntax(format!("unknown condition prefix {:?}; known: {:?}", prefix, known))
            })?;
            // Validate prefix usage
            if !c.is_prefix() {
                return Err(ScriptError::syntax(
                    format!("condition {:?} cannot be used with a suffix", prefix),
                ));
            }
            c
        } else {
            let c = self.conditions.get(prefix).ok_or_else(|| {
                // Go-compatible: "unknown condition %q"
                ScriptError::syntax(format!("unknown condition {:?}", prefix))
            })?;
            // Validate non-prefix usage
            if c.is_prefix() {
                return Err(ScriptError::syntax(
                    format!("condition {:?} requires a suffix", prefix),
                ));
            }
            c
        };

        let result = condition.eval(state, suffix.unwrap_or("")).map_err(|e| {
            // Go-compatible: "evaluating condition %q: <err>"
            ScriptError::syntax(format!("evaluating condition {:?}: {}", cond.tag, e.message))
        })?;
        Ok(if cond.negate { !result } else { result })
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// Expand arguments from raw fragments, respecting quoted/unquoted and regexp escaping.
///
/// This is the Rust equivalent of Go's `expandArgs()`. For each argument:
/// - Quoted fragments are emitted verbatim (no expansion)
/// - Unquoted fragments have env vars expanded via `state.expand_env()`
/// - If the argument index is in `regexp_args`, env var values are regex-escaped
///   (equivalent to Go's `regexp.QuoteMeta`)
pub fn expand_args(state: &State, raw_args: &[Vec<ArgFragment>], regexp_args: &[usize]) -> Vec<String> {
    let mut args = Vec::with_capacity(raw_args.len());
    for (i, frags) in raw_args.iter().enumerate() {
        let is_regexp = regexp_args.contains(&i);
        let mut buf = String::new();
        for frag in frags {
            if frag.quoted {
                buf.push_str(&frag.s);
            } else {
                buf.push_str(&state.expand_env(&frag.s, is_regexp));
            }
        }
        args.push(buf);
    }
    args
}

/// Returns indices of the first non-flag argument.
/// Go-compatible: skips args starting with `-`, also handles `--` separator.
/// Used as the `regexp_args` function for stdout, stderr, grep commands.
pub fn first_non_flag(raw_args: &[String]) -> Vec<usize> {
    for (i, arg) in raw_args.iter().enumerate() {
        if !arg.starts_with('-') {
            return vec![i];
        }
        if arg == "--" {
            if i + 1 < raw_args.len() {
                return vec![i + 1];
            }
            return vec![];
        }
    }
    vec![]
}
