//! Script line parser
//!
//! Parses testscript lines following Go's `cmd/internal/script` syntax:
//! - `#` at line start is a section comment (whole line ignored)
//! - `#` mid-line terminates the argument list (inline comment)
//! - `!` prefix for expected-failure
//! - `?` prefix for may-fail
//! - `[cond]` for conditional execution
//! - Single-quote strings disable word splitting and env expansion
//! - `''` inside quotes produces a literal `'`
//! - Environment variable expansion (`$VAR`, `${VAR}`) happens in the engine,
//!   not here — the parser preserves fragments with quoted/unquoted tracking.

/// A fragment of a parsed argument, tracking whether it was quoted.
/// Quoted fragments suppress environment variable expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgFragment {
    /// The text content of this fragment
    pub s: String,
    /// If true, this fragment was inside single quotes — disable expansion
    pub quoted: bool,
}

/// A parsed script line
#[derive(Debug, Clone)]
pub struct ScriptLine {
    /// Whether the command is negated (must fail)
    pub negate: bool,
    /// Whether the command may fail or succeed
    pub may_fail: bool,
    /// Conditions that must be true for the command to run
    pub conditions: Vec<ScriptCondition>,
    /// Command name
    pub command: String,
    /// Raw arguments as fragments (quoted/unquoted tracking for expansion)
    pub raw_args: Vec<Vec<ArgFragment>>,
    /// Original line text (for error messages)
    pub raw: String,
    /// Line number in the script
    pub line_number: usize,
    /// Whether the command should run in the background (& suffix)
    pub background: bool,
}

/// A condition guard on a script line
#[derive(Debug, Clone)]
pub struct ScriptCondition {
    /// The full condition tag (e.g. "unix", "GOOS:linux", "exec:git")
    pub tag: String,
    /// Whether the condition is negated
    pub negate: bool,
}

/// Characters that separate arguments (same as Go's argSepChars)
const ARG_SEP_CHARS: &[char] = &[' ', '\t', '\r', '\n', '#'];

/// Parse error returned when a line has invalid syntax
#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Parse a single script line into a ScriptLine.
///
/// Returns `Ok(None)` for blank lines and comment-only lines.
/// Returns `Err(ParseError)` for syntax errors (unterminated quotes, etc.).
///
/// This follows Go's `parse()` function from `engine.go`:
/// - The first unquoted word that isn't `!`, `?`, or `[cond]` becomes the command name
/// - `#` is an inline comment delimiter (outside quotes)
/// - Unterminated quotes are an error
pub fn parse_line(line: &str, line_number: usize) -> Result<Option<ScriptLine>, ParseError> {
    let line_bytes = line.as_bytes();
    let len = line_bytes.len();

    let mut raw_args: Vec<Vec<ArgFragment>> = Vec::new();
    let mut current_frags: Vec<ArgFragment> = Vec::new();
    let mut start: Option<usize> = None; // start of current unquoted/quoted text chunk
    let mut quoted = false;

    // Accumulated command metadata
    let mut negate = false;
    let mut may_fail = false;
    let mut conditions: Vec<ScriptCondition> = Vec::new();
    let mut command: Option<String> = None;

    /// Flush the current word (fragments) into either a prefix/condition/command/arg
    macro_rules! flush_arg {
        () => {
            if current_frags.is_empty() {
                // Nothing to flush
            } else {
                // If no command name yet, first word is a potential prefix or command
                if command.is_none() && current_frags.len() == 1 && !current_frags[0].quoted {
                    let arg = &current_frags[0].s;

                    // Check for ! or ? prefix
                    if arg == "!" {
                        if negate || may_fail {
                            return Err(ParseError {
                                message: "duplicated '!' or '?' token".into(),
                                line: line_number,
                            });
                        }
                        negate = true;
                        current_frags.clear();
                    } else if arg == "?" {
                        if negate || may_fail {
                            return Err(ParseError {
                                message: "duplicated '!' or '?' token".into(),
                                line: line_number,
                            });
                        }
                        may_fail = true;
                        current_frags.clear();
                    } else if arg.starts_with('[') && arg.ends_with(']') {
                        // Condition guard
                        let inner = arg[1..arg.len()-1].trim();
                        let (want_true, tag) = if let Some(rest) = inner.strip_prefix('!') {
                            (false, rest.trim())
                        } else {
                            (true, inner)
                        };
                        if tag.is_empty() {
                            return Err(ParseError {
                                message: "empty condition".into(),
                                line: line_number,
                            });
                        }
                        conditions.push(ScriptCondition {
                            tag: tag.to_string(),
                            negate: !want_true,
                        });
                        current_frags.clear();
                    } else if arg.is_empty() {
                        return Err(ParseError {
                            message: "empty command".into(),
                            line: line_number,
                        });
                    } else {
                        command = Some(arg.clone());
                        current_frags.clear();
                    }
                } else {
                    // It's a command argument
                    raw_args.push(std::mem::take(&mut current_frags));
                }
            }
        };
    }

    let mut i = 0;
    loop {
        if !quoted && (i >= len || ARG_SEP_CHARS.contains(&(line_bytes[i] as char))) {
            // Found arg-separating space or #
            if let Some(s) = start {
                let text = &line[s..i];
                if !text.is_empty() {
                    current_frags.push(ArgFragment {
                        s: text.to_string(),
                        quoted: false,
                    });
                }
                start = None;
            }
            flush_arg!();
            if i >= len || line_bytes[i] == b'#' {
                break;
            }
            i += 1;
            continue;
        }
        if i >= len {
            return Err(ParseError {
                message: "unterminated quoted argument".into(),
                line: line_number,
            });
        }
        if line_bytes[i] == b'\'' {
            if !quoted {
                // Starting a quoted chunk
                if let Some(s) = start {
                    current_frags.push(ArgFragment {
                        s: line[s..i].to_string(),
                        quoted: false,
                    });
                }
                start = Some(i + 1);
                quoted = true;
                i += 1;
                continue;
            }
            // Inside quotes: check for '' (escaped quote)
            if i + 1 < len && line_bytes[i + 1] == b'\'' {
                current_frags.push(ArgFragment {
                    s: line[start.unwrap_or(i)..i].to_string(),
                    quoted: true,
                });
                start = Some(i + 1);
                i += 2; // skip both quotes
                continue;
            }
            // Ending a quoted chunk
            current_frags.push(ArgFragment {
                s: line[start.unwrap_or(i)..i].to_string(),
                quoted: true,
            });
            start = Some(i + 1);
            quoted = false;
            i += 1;
            continue;
        }
        // Regular character — start tracking if not already
        if start.is_none() {
            start = Some(i);
        }
        i += 1;
    }

    if command.is_none() {
        if negate || may_fail || !conditions.is_empty() || !raw_args.is_empty() {
            return Err(ParseError {
                message: "missing command".into(),
                line: line_number,
            });
        }
        // Blank line or comment-only
        return Ok(None);
    }

    // Check for & suffix (background command)
    // Go-compatible: must be a single unquoted fragment containing just "&"
    let mut background = false;
    if let Some(last) = raw_args.last() {
        if last.len() == 1 && !last[0].quoted && last[0].s == "&" {
            background = true;
            raw_args.pop();
        }
    }

    Ok(Some(ScriptLine {
        negate,
        may_fail,
        conditions,
        command: command.unwrap(),
        raw_args,
        raw: line.to_string(),
        line_number,
        background,
    }))
}

/// Expand environment variables in a string.
/// Supports `$VAR` and `${VAR}` syntax.
/// Special variables: `${/}` → path separator, `${:}` → path list separator.
///
/// When `in_regexp` is true, expanded values are escaped with `regex::escape()`
/// (equivalent to Go's `regexp.QuoteMeta`).
pub fn expand_env(s: &str, lookup: &dyn Fn(&str) -> Option<String>, in_regexp: bool) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '$' {
            result.push(c);
            continue;
        }

        // Check for ${VAR} syntax
        if chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            while let Some(&c) = chars.peek() {
                if c == '}' {
                    chars.next();
                    break;
                }
                var_name.push(c);
                chars.next();
            }

            // Special variables
            match var_name.as_str() {
                "/" => result.push(std::path::MAIN_SEPARATOR),
                ":" => {
                    #[cfg(windows)]
                    result.push(';');
                    #[cfg(not(windows))]
                    result.push(':');
                }
                _ => {
                    // Go: undefined vars expand to empty string
                    let val = lookup(&var_name).unwrap_or_default();
                    if in_regexp {
                        result.push_str(&regex::escape(&val));
                    } else {
                        result.push_str(&val);
                    }
                }
            }
        } else {
            // $VAR syntax - read until non-alphanumeric/underscore
            let mut var_name = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    var_name.push(c);
                    chars.next();
                } else {
                    break;
                }
            }

            if var_name.is_empty() {
                result.push('$');
            } else {
                // Go: undefined vars expand to empty string
                let val = lookup(&var_name).unwrap_or_default();
                if in_regexp {
                    result.push_str(&regex::escape(&val));
                } else {
                    result.push_str(&val);
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: join fragments of a raw_arg into a single string (simulating expansion without env)
    fn join_frags(frags: &[ArgFragment]) -> String {
        frags.iter().map(|f| f.s.as_str()).collect()
    }

    /// Helper: collect all args as joined strings
    fn flat_args(line: &ScriptLine) -> Vec<String> {
        line.raw_args.iter().map(|a| join_frags(a)).collect()
    }

    #[test]
    fn test_parse_empty_line() {
        assert!(parse_line("", 1).unwrap().is_none());
        assert!(parse_line("   ", 1).unwrap().is_none());
    }

    #[test]
    fn test_parse_comment() {
        assert!(parse_line("# comment", 1).unwrap().is_none());
        assert!(parse_line("  # indented comment", 1).unwrap().is_none());
    }

    #[test]
    fn test_parse_simple_command() {
        let line = parse_line("exec echo hello", 1).unwrap().unwrap();
        assert_eq!(line.command, "exec");
        assert_eq!(flat_args(&line), vec!["echo", "hello"]);
        assert!(!line.negate);
        assert!(!line.may_fail);
    }

    #[test]
    fn test_parse_negate() {
        let line = parse_line("! exec bad-cmd", 1).unwrap().unwrap();
        assert!(line.negate);
        assert_eq!(line.command, "exec");
    }

    #[test]
    fn test_parse_may_fail() {
        let line = parse_line("? exec maybe-cmd", 1).unwrap().unwrap();
        assert!(line.may_fail);
        assert_eq!(line.command, "exec");
    }

    #[test]
    fn test_parse_conditions() {
        let line = parse_line("[unix] exec unix-cmd", 1).unwrap().unwrap();
        assert_eq!(line.conditions.len(), 1);
        assert_eq!(line.conditions[0].tag, "unix");
        assert!(!line.conditions[0].negate);
    }

    #[test]
    fn test_parse_negated_condition() {
        let line = parse_line("[!windows] exec unix-cmd", 1).unwrap().unwrap();
        assert_eq!(line.conditions[0].tag, "windows");
        assert!(line.conditions[0].negate);
    }

    #[test]
    fn test_parse_prefix_condition() {
        let line = parse_line("[GOOS:linux] exec linux-cmd", 1).unwrap().unwrap();
        assert_eq!(line.conditions[0].tag, "GOOS:linux");
        assert!(!line.conditions[0].negate);
    }

    #[test]
    fn test_parse_quoted_args() {
        let line = parse_line("stdout 'hello world'", 1).unwrap().unwrap();
        assert_eq!(line.command, "stdout");
        let args = &line.raw_args;
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].len(), 1);
        assert_eq!(args[0][0].s, "hello world");
        assert!(args[0][0].quoted);
    }

    #[test]
    fn test_parse_mixed_fragments() {
        // prefix'quoted'suffix becomes one arg with 3 fragments
        let line = parse_line("echo pre'mid'suf", 1).unwrap().unwrap();
        assert_eq!(line.command, "echo");
        let arg = &line.raw_args[0];
        assert_eq!(arg.len(), 3);
        assert_eq!(arg[0].s, "pre");
        assert!(!arg[0].quoted);
        assert_eq!(arg[1].s, "mid");
        assert!(arg[1].quoted);
        assert_eq!(arg[2].s, "suf");
        assert!(!arg[2].quoted);
    }

    #[test]
    fn test_parse_escaped_quote() {
        let line = parse_line("echo 'it''s working'", 1).unwrap().unwrap();
        assert_eq!(line.command, "echo");
        // "it" + "'" + "s working" — three quoted fragments in one arg
        let arg = &line.raw_args[0];
        let text: String = arg.iter().map(|f| f.s.as_str()).collect();
        assert_eq!(text, "it's working");
    }

    #[test]
    fn test_parse_inline_comment() {
        let line = parse_line("echo hello # this is a comment", 1).unwrap().unwrap();
        assert_eq!(line.command, "echo");
        assert_eq!(flat_args(&line), vec!["hello"]);
    }

    #[test]
    fn test_parse_hash_in_quotes() {
        // # inside quotes should NOT be treated as comment
        let line = parse_line("echo 'hello # world'", 1).unwrap().unwrap();
        assert_eq!(line.command, "echo");
        assert_eq!(join_frags(&line.raw_args[0]), "hello # world");
    }

    #[test]
    fn test_parse_unterminated_quote_error() {
        let result = parse_line("echo 'unterminated", 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("unterminated"));
    }

    #[test]
    fn test_parse_missing_command_error() {
        let result = parse_line("! ", 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("missing command"));
    }

    #[test]
    fn test_parse_duplicate_prefix_error() {
        let result = parse_line("! ? exec foo", 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("duplicated"));
    }

    #[test]
    fn test_expand_env_simple() {
        let result = expand_env("hello $NAME", &|key| {
            if key == "NAME" { Some("world".to_string()) } else { None }
        }, false);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_expand_env_braces() {
        let result = expand_env("${HOME}/bin", &|key| {
            if key == "HOME" { Some("/usr/local".to_string()) } else { None }
        }, false);
        assert_eq!(result, "/usr/local/bin");
    }

    #[test]
    fn test_expand_env_path_sep() {
        let result = expand_env("a${/}b", &|_| None, false);
        #[cfg(windows)]
        assert_eq!(result, "a\\b");
        #[cfg(not(windows))]
        assert_eq!(result, "a/b");
    }

    #[test]
    fn test_expand_env_undefined_is_empty() {
        // Go behavior: undefined vars expand to empty string
        let result = expand_env("hello $UNDEF end", &|_| None, false);
        assert_eq!(result, "hello  end");
    }

    #[test]
    fn test_expand_env_in_regexp() {
        // When in_regexp=true, values should be regex-escaped
        let result = expand_env("$PATH", &|key| {
            if key == "PATH" { Some("C:\\work\\go1.4".to_string()) } else { None }
        }, true);
        assert_eq!(result, r"C:\\work\\go1\.4");
    }

    #[test]
    fn test_parse_background() {
        let line = parse_line("exec sleep 1 &", 1).unwrap().unwrap();
        assert!(line.background);
        assert_eq!(line.command, "exec");
        assert_eq!(flat_args(&line), vec!["sleep", "1"]);
    }

    #[test]
    fn test_parse_quoted_ampersand_not_background() {
        // Go-compatible: a quoted '&' should NOT be treated as background
        let line = parse_line("echo '&'", 1).unwrap().unwrap();
        assert!(!line.background);
        assert_eq!(line.command, "echo");
        assert_eq!(join_frags(&line.raw_args[0]), "&");
    }
}
