//! emx-testspec CLI
//!
//! Run testspec E2E tests from txtar files.

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use emx_testspec::{TestRunner, RunConfig};

#[derive(Parser, Debug)]
#[command(name = "emx-testspec")]
#[command(author = "nzinfo <li.monan@gmail.com>")]
#[command(version)]
#[command(about = "Run testspec E2E tests from txtar files")]
struct Cli {
    /// Directory or file to test
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Only run tests whose name contains this string
    #[arg(short = 'f', long)]
    filter: Option<String>,

    /// Verbose output: show script execution log
    #[arg(short, long)]
    verbose: bool,

    /// Keep working directories after test (for debugging)
    #[arg(short = 'k', long = "keep")]
    keep: bool,

    /// Root directory for working directories
    #[arg(long = "workdir")]
    workdir: Option<PathBuf>,

    /// File extensions to match [default: .txtar]
    #[arg(long = "ext", default_value = ".txtar")]
    extensions: Vec<String>,

    /// List available commands and conditions
    #[arg(long = "list-commands")]
    list_commands: bool,

    /// Environment variables to set (KEY=VALUE)
    #[arg(short = 'e', long)]
    env_vars: Vec<String>,

    /// Show number of tests without running
    #[arg(long = "count")]
    count: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.list_commands {
        print_commands();
        return ExitCode::SUCCESS;
    }

    // Determine if path is a file or directory
    let is_file = cli.path.extension().map_or(false, |ext| {
        cli.extensions.iter().any(|e| e.trim_start_matches('.') == ext)
    }) || cli.path.is_file();

    let config = RunConfig {
        dir: if is_file {
            cli.path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
        } else {
            cli.path.clone()
        },
        filter: if is_file {
            // Use file stem (without extension) for filter
            cli.path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        } else {
            cli.filter
        },
        workdir_root: cli.workdir,
        preserve_work: cli.keep,
        verbose: cli.verbose,
        extensions: cli.extensions,
        setup: None,
    };

    let runner = TestRunner::new(config);

    if cli.count {
        match runner.count_tests() {
            Ok(count) => {
                println!("Found {} test(s)", count);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("error: {}", e);
                return ExitCode::FAILURE;
            }
        }
    }

    let result = match runner.run_all() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Print results
    for case in &result.cases {
        if case.skipped {
            println!("SKIP  {} - {}", case.name, case.error.as_deref().unwrap_or(""));
        } else if case.passed {
            println!("PASS  {} ({}ms)", case.name, case.duration.as_millis());
            if cli.verbose && !case.log.is_empty() {
                for line in case.log.lines() {
                    println!("      {}", line);
                }
            }
        } else {
            println!("FAIL  {}", case.name);
            if let Some(ref err) = case.error {
                for line in err.lines() {
                    println!("      {}", line);
                }
            }
            if !case.log.is_empty() {
                println!("      --- log ---");
                for line in case.log.lines() {
                    println!("      {}", line);
                }
            }
            if let Some(ref wd) = case.workdir {
                println!("      workdir: {}", wd.display());
            }
        }
    }

    println!();
    println!("{}", result.summary());

    if result.all_passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_commands() {
    println!("Built-in commands:");
    println!();

    let engine = emx_testspec::Engine::new();
    let mut cmds: Vec<_> = engine.commands.iter().collect();
    cmds.sort_by_key(|(name, _)| (*name).clone());

    for (name, cmd) in &cmds {
        let usage = cmd.usage();
        println!("  {:<12} {} {}", name, usage.summary, usage.args);
    }

    println!();
    println!("Built-in conditions:");
    println!();

    let mut conds: Vec<_> = engine.conditions.iter().collect();
    conds.sort_by_key(|(name, _)| (*name).clone());

    for (name, cond) in &conds {
        println!("  {:<12} {}", name, cond.summary());
    }

    println!();
    println!("Prefixes:");
    println!("  !            Command must fail");
    println!("  ?            Command may fail or succeed");
    println!("  [cond]       Conditional execution");
    println!("  [!cond]      Negated condition");
}
