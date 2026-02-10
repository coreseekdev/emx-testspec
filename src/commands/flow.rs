//! Flow control commands: stop, skip, sleep, wait

use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;

// ──────────────────────────────────────────────────────────
// stop — stop script execution
// ──────────────────────────────────────────────────────────

pub(super) struct StopCmd;

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

pub(super) struct SkipCmd;

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
// sleep — sleep for a duration
// ──────────────────────────────────────────────────────────

pub(super) struct SleepCmd;

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

// ──────────────────────────────────────────────────────────
// wait — wait for background commands
// ──────────────────────────────────────────────────────────

pub(super) struct WaitCmd;

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

// ──────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────

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
