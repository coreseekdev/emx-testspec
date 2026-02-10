//! Environment commands: cd, echo, env

use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;

// ──────────────────────────────────────────────────────────
// cd — change directory
// ──────────────────────────────────────────────────────────

pub(super) struct CdCmd;

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
// echo — print to stdout buffer
// ──────────────────────────────────────────────────────────

pub(super) struct EchoCmd;

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

pub(super) struct EnvCmd;

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
