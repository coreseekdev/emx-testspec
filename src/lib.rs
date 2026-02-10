//! emx-testspec: A testspec engine for CLI E2E testing
//!
//! Inspired by Go's `cmd/internal/script` and `rogpeppe/go-internal/testscript`.
//!
//! # Overview
//!
//! This crate provides a scriptable testing DSL for CLI tools, using txtar archives
//! as test files. Each txtar file's comment section contains script commands, and
//! the file section provides test fixtures.
//!
//! # Script Syntax
//!
//! ```text
//! # Section comment
//! exec mytool arg1 arg2
//! stdout 'expected output'
//! ! exec mytool --bad-flag
//! stderr 'error message'
//! cmp stdout golden.txt
//!
//! -- golden.txt --
//! expected content
//! ```
//!
//! # Commands
//!
//! | Command | Description |
//! |---------|-------------|
//! | `exec` | Execute a command |
//! | `stdout` | Match stdout with pattern |
//! | `stderr` | Match stderr with pattern |
//! | `cmp` | Compare files |
//! | `cmpenv` | Compare files with env expansion |
//! | `exists` | Check file existence |
//! | `grep` | Regex match in file |
//! | `cat` | Print file contents |
//! | `cd` | Change directory |
//! | `cp` | Copy files |
//! | `echo` | Print to stdout buffer |
//! | `env` | Set/print environment |
//! | `mkdir` | Create directories |
//! | `rm` | Remove files |
//! | `stop` | Stop script |
//! | `skip` | Skip test |
//!
//! # Prefixes
//!
//! - `!` - Command must fail
//! - `?` - Command may succeed or fail
//! - `[cond]` - Conditional execution

mod engine;
mod state;
mod parser;
mod commands;
mod conditions;
mod runner;
mod error;

pub use engine::{Engine, Cmd, CmdUsage, CmdResult, expand_args, first_non_flag};
pub use state::State;
pub use parser::{ScriptLine, ArgFragment, parse_line};
pub use commands::default_commands;
pub use conditions::{Condition, default_conditions};
pub use runner::{TestRunner, RunConfig, TestResult, TestCaseResult, TestRunnerBuilder, SetupEnv};
pub use error::{ScriptError, ErrorKind};

// Convenience functions for cargo test integration
pub use runner::{run_and_assert, run_and_assert_with, run};
