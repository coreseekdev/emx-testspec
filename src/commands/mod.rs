//! Built-in script commands
//!
//! Implements the standard testscript command set, with Go-compatible behavior
//! for pattern matching (always regex), env display, and file operations.

mod exec;
mod output;
mod cmp;
mod file_ops;
mod flow;
mod env;
mod text;
mod help;

use std::collections::HashMap;
use crate::engine::BoxedCmd;

pub use help::HelpCmd;

/// Return the default set of built-in commands
pub fn default_commands() -> HashMap<String, BoxedCmd> {
    let mut cmds: HashMap<String, BoxedCmd> = HashMap::new();
    cmds.insert("exec".into(), Box::new(exec::ExecCmd));
    cmds.insert("stdout".into(), Box::new(output::StdoutCmd));
    cmds.insert("stderr".into(), Box::new(output::StderrCmd));
    cmds.insert("cmp".into(), Box::new(cmp::CmpCmd));
    cmds.insert("cmpenv".into(), Box::new(cmp::CmpEnvCmd));
    cmds.insert("exists".into(), Box::new(file_ops::ExistsCmd));
    cmds.insert("grep".into(), Box::new(output::GrepCmd));
    cmds.insert("cat".into(), Box::new(file_ops::CatCmd));
    cmds.insert("cd".into(), Box::new(env::CdCmd));
    cmds.insert("cp".into(), Box::new(file_ops::CpCmd));
    cmds.insert("echo".into(), Box::new(env::EchoCmd));
    cmds.insert("env".into(), Box::new(env::EnvCmd));
    cmds.insert("mkdir".into(), Box::new(file_ops::MkdirCmd));
    cmds.insert("rm".into(), Box::new(file_ops::RmCmd));
    cmds.insert("stop".into(), Box::new(flow::StopCmd));
    cmds.insert("skip".into(), Box::new(flow::SkipCmd));
    cmds.insert("replace".into(), Box::new(text::ReplaceCmd));
    cmds.insert("mv".into(), Box::new(file_ops::MvCmd));
    cmds.insert("chmod".into(), Box::new(file_ops::ChmodCmd));
    cmds.insert("symlink".into(), Box::new(file_ops::SymlinkCmd));
    cmds.insert("sleep".into(), Box::new(flow::SleepCmd));
    cmds.insert("wait".into(), Box::new(flow::WaitCmd));
    cmds
}
