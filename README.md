# emx-testspec

A testspec engine for CLI E2E testing, inspired by Go's `cmd/internal/script` and `roguepeppe/go-internal/testscript`.

## Overview

`emx-testspec` provides a declarative scripting DSL for testing CLI tools. Tests are written as txtar archives where the comment section contains script commands and the file section provides test fixtures.

## Features

- ✅ **Declarative syntax** - Simple, readable test scripts
- ✅ **Txtar integration** - Test fixtures in the same file
- ✅ **Rich command set** - exec, stdout, stderr, cmp, grep, etc.
- ✅ **Platform conditions** - Conditional execution per OS/arch
- ✅ **Virtual files** - `stdout` and `stderr` as file-like objects
- ✅ **Background processes** - Run commands asynchronously
- ✅ **Flexible matching** - Regex patterns with multi-line support
- ✅ **MIT License** - Free to use in any project

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
emx-testspec = "0.1"
```

Or use via Git:

```toml
[dependencies]
emx-testspec = { git = "https://github.com/coreseekdev/emx-testspec" }
```

Requires [emx-txtar](https://github.com/coreseekdev/emx-txtar) as a dependency.

## Quick Start

### Basic Test

Create `tests/mytool.txtar`:

```txtar
# Test basic functionality

exec mytool input.txt
stdout 'success'

-- input.txt --
test data
```

Run tests:

```rust
use emx_testspec::run_and_assert;

#[test]
fn test_mytool() {
    run_and_assert("tests");
}
```

### Using as CLI

Install the CLI:

```bash
cargo install emx-testspec --git https://github.com/coreseekdev/emx-testspec
```

Run tests:

```bash
emx-testspec tests/                    # Run all tests
emx-testspec tests/test.txtar         # Run single test
emx-testspec tests/ -v                # Verbose output
emx-testspec tests/ -f "basic"        # Filter by name
emx-testspec tests/ --keep            # Preserve work directories
```

## Script Syntax

### Commands

| Command | Description | Example |
|---------|-------------|---------|
| `exec` | Execute a command | `exec mytool arg1 arg2` |
| `stdout` | Match stdout with regex | `stdout 'expected'` |
| `stderr` | Match stderr with regex | `stderr 'error message'` |
| `cmp` | Compare files | `cmp file1 file2` |
| `cmpenv` | Compare with env expansion | `cmpenv file1 file2` |
| `grep` | Search in file | `grep 'pattern' file` |
| `cat` | Print file contents | `cat file` |
| `cd` | Change directory | `cd subdir` |
| `cp` | Copy files | `cp src dst` |
| `mv` | Move/rename files | `mv old new` |
| `rm` | Remove files | `rm file` |
| `mkdir` | Create directories | `mkdir dir` |
| `exists` | Check file existence | `exists file` |
| `env` | Set/get environment | `env KEY=value` |
| `echo` | Print to stdout buffer | `echo text` |
| `sleep` | Wait for duration | `sleep 1s` |
| `stop` | Stop test (non-error) | `stop 'reason'` |
| `skip` | Skip test | `skip 'reason'` |
| `help` | List commands | `help` |

### Prefixes

- `!` - Command **must fail** (exit code ≠ 0)
- `?` - Command **may succeed or fail**
- No prefix - Command **must succeed** (exit code = 0)

Example:
```txtar
exec mytool --valid          # Must succeed
! exec mytool --invalid      # Must fail
? exec mytool --unpredictable  # Either is OK
```

### Conditions

Conditional execution based on platform:

```txtar
[unix] exec unix-tool
[windows] exec windows-tool
[linux] exec linux-tool
[amd64] exec x86_64-tool
[!windows] exec not-windows-tool

[exec:python] exec python-script
```

## Environment Variables

### Built-in Variables

- `$WORK` or `$TMPDIR` - Temporary working directory
- `$PWD` - Current working directory
- `${/}` - OS path separator (`/` or `\`)
- `${:}` - Path list separator (`:` or `;`)

### Usage

```txtar
env KEY=value
echo $KEY
```

In comparison:
```txtar
cmp stdout $WORK/expected.txt
```

## Virtual Files

`stdout` and `stderr` can be used as virtual files:

```txtar
exec mytool
cmp stdout expected.txt
cp stderr debug.log
```

## Advanced Usage

### Background Processes

```txtar
exec server &
sleep 2s
exec client --request
wait
```

### Multi-line Matching

```txtar
exec mytool
stdout 'line1.*line2.*line3'
```

### File Comparison with Diff

```txtar
exec mytool input.txt
cmp stdout output.txt
```

If files differ, unified diff is shown.

### Environment Expansion

```txtar
env NAME=World
echo "Hello $NAME"
```

In `cmpenv`, environment variables are expanded before comparison.

## API Usage

### Running Tests

```rust
use emx_testspec::{TestRunner, RunConfig};

let config = RunConfig {
    dir: "tests".into(),
    filter: None,
    workdir_root: None,
    preserve_work: false,
    verbose: false,
    extensions: vec![".txtar".into()],
    setup: None,
};

let runner = TestRunner::new(config);
let result = runner.run_all()?;

assert!(result.all_passed());
```

### Custom Commands

```rust
use emx_testspec::{Engine, Cmd, CmdResult};
use anyhow::Result;

struct MyCustomCmd;

impl Cmd for MyCustomCmd {
    fn usage(&self) -> (String, String) {
        ("mycmd".into(), "[args]".into())
    }

    fn run(&self, args: &[String], state: &mut State) -> Result<CmdResult> {
        // Custom logic here
        Ok(CmdResult::Success)
    }
}

// Register custom command
let mut engine = Engine::new();
engine.register_command("mycmd", Box::new(MyCustomCmd));
```

## Environment Variables for Testing

| Variable | Effect |
|----------|--------|
| `TESTSCRIPT_VERBOSE=1` | Enable verbose logging |
| `TESTSCRIPT_WORK=1` | Preserve working directories |

## Format Specification

See [spec/00-princ-11-tool-testspec.mx](https://github.com/coreseekdev/emx/spec) for the full grammar specification.

### BNF Grammar

```bnf
script-line = prefix? condition* command-name arg* inline-comment?

prefix      = "!" | "?"
condition   = "[" "!"? cond-name (":" suffix)? "]"
command     = identifier
arg         = quoted-arg | unquoted-arg
quoted-arg  = "'" (char | "''")* "'"
```

## Documentation

- [API Documentation](https://docs.rs/emx-testspec)
- [Grammar Specification](spec/00-princ-11-tool-testspec.mx)

## License

MIT License - see [LICENSE](LICENSE) file for details.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## Related Projects

- [emx-txtar](https://github.com/coreseekdev/emx-txtar) - Txtar archive format
- [Go testscript](https://pkg.go.dev/golang.org/x/tools/cmd/internal/testscript) - Original inspiration
- [roguepeppe/go-internal/testscript](https://pkg.go.dev/github.com/rogpeppe/go-internal/testscript) - Enhanced Go version
