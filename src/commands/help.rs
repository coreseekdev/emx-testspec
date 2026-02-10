//! help — display command help

use crate::engine::{Cmd, CmdResult, CmdUsage};
use crate::error::ScriptError;
use crate::state::State;

pub struct HelpCmd {
    /// (name, args, summary) for each command
    cmd_info: Vec<(String, String, String)>,
    /// Condition names
    cond_names: Vec<String>,
}

impl HelpCmd {
    pub fn new(cmd_info: Vec<(String, String, String)>, cond_names: Vec<String>) -> Self {
        Self { cmd_info, cond_names }
    }
}

impl Cmd for HelpCmd {
    fn run(&self, state: &mut State, args: &[String]) -> Result<CmdResult, ScriptError> {
        let mut output = String::new();

        if args.is_empty() {
            // List all commands
            for (name, args_str, summary) in &self.cmd_info {
                if args_str.is_empty() {
                    output.push_str(&format!("{}\n    {}\n", name, summary));
                } else {
                    output.push_str(&format!("{} {}\n    {}\n", name, args_str, summary));
                }
            }

            if !self.cond_names.is_empty() {
                output.push_str("\nconditions:\n");
                for name in &self.cond_names {
                    output.push_str(&format!("    {}\n", name));
                }
            }
        } else {
            // Help for specific commands
            for name in args {
                if let Some((_, args_str, summary)) = self.cmd_info.iter().find(|(n, _, _)| n == name) {
                    if args_str.is_empty() {
                        output.push_str(&format!("{}\n    {}\n", name, summary));
                    } else {
                        output.push_str(&format!("{} {}\n    {}\n", name, args_str, summary));
                    }
                } else {
                    output.push_str(&format!("{}: unknown command\n", name));
                }
            }
        }

        state.stdout = output;
        state.stderr = String::new();
        // Go-compatible: help returns WaitFunc → engine logs stdout
        if !state.stdout.is_empty() {
            state.logf(&format!("[stdout]\n{}", state.stdout));
        }
        Ok(CmdResult::Ok)
    }

    fn usage(&self) -> CmdUsage {
        CmdUsage {
            summary: "Display help for commands".into(),
            args: "[command...]".into(),
            regexp_args: None,
            async_: false,
        }
    }
}
