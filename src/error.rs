//! Script errors

use std::fmt;

/// The kind of script error
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Command execution failed
    CommandFailed,
    /// Command succeeded but was expected to fail (! prefix)
    UnexpectedSuccess,
    /// Pattern match failed
    PatternMismatch,
    /// File comparison failed
    ComparisonFailed,
    /// File not found
    FileNotFound,
    /// File exists when it shouldn't
    FileExists,
    /// Invalid script syntax
    SyntaxError,
    /// Invalid usage of a command
    UsageError,
    /// Skip the test
    Skip,
    /// Stop the script (not an error)
    Stop,
    /// IO error
    Io,
    /// One or more background commands failed
    WaitError,
    /// Other error
    Other,
}

/// A script error with file/line context
#[derive(Debug)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub command: Option<String>,
    pub args: Vec<String>,
}

impl ScriptError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            file: None,
            line: None,
            command: None,
            args: Vec::new(),
        }
    }

    pub fn with_location(mut self, file: impl Into<String>, line: usize) -> Self {
        self.file = Some(file.into());
        self.line = Some(line);
        self
    }

    pub fn with_command(mut self, cmd: impl Into<String>) -> Self {
        self.command = Some(cmd.into());
        self
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    pub fn syntax(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::SyntaxError, msg)
    }

    pub fn usage(cmd: &str, expected: &str) -> Self {
        Self::new(ErrorKind::UsageError, format!("usage: {} {}", cmd, expected))
    }

    pub fn skip(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Skip, msg)
    }

    pub fn stop(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Stop, msg)
    }

    pub fn is_skip(&self) -> bool {
        self.kind == ErrorKind::Skip
    }

    pub fn is_stop(&self) -> bool {
        self.kind == ErrorKind::Stop
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref file) = self.file {
            write!(f, "{}:", file)?;
        }
        if let Some(line) = self.line {
            write!(f, "{}:", line)?;
        }
        if let Some(ref cmd) = self.command {
            if self.args.is_empty() {
                write!(f, " {}: ", cmd)?;
            } else {
                // Go-compatible: file:line: op args: err
                let quoted = self.args.iter().map(|a| {
                    if a.contains(' ') || a.contains('\t') || a.is_empty() {
                        format!("'{}'", a)
                    } else {
                        a.clone()
                    }
                }).collect::<Vec<_>>().join(" ");
                write!(f, " {} {}: ", cmd, quoted)?;
            }
        } else if self.file.is_some() || self.line.is_some() {
            write!(f, " ")?;
        }
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ScriptError {}

impl From<std::io::Error> for ScriptError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, e.to_string())
    }
}
