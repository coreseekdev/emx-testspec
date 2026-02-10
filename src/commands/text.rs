//! Text manipulation commands: replace

use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::{ScriptError, ErrorKind};
use crate::state::State;

// ──────────────────────────────────────────────────────────
// replace — string replacement in file
// ──────────────────────────────────────────────────────────

pub(super) struct ReplaceCmd;

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
// Helpers
// ──────────────────────────────────────────────────────────

/// Go-compatible string unquoting.
///
/// Interprets Go escape sequences like `\n`, `\t`, `\\`, `\uXXXX`, `\UXXXXXXXX`, `\NNN`, etc.
/// Equivalent to Go's `strconv.Unquote("\"" + s + "\"")`.
pub(super) fn go_unquote(s: &str) -> String {
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
