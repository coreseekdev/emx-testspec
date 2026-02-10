# emx-testspec - AI Development Guide

## Project Overview

**emx-testspec** is a test specification engine for CLI E2E testing, inspired by Go's testscript. This document provides context for AI assistants working on this codebase.

## Architecture

### Core Components

1. **Engine** (`src/engine.rs`)
   - Command registry and execution
   - CommandResult abstraction
   - Argument expansion utilities

2. **Parser** (`src/parser.rs`)
   - Line-based script parsing
   - Prefix detection (`!`, `?`)
   - Condition parsing (`[unix]`, `[!windows]`)
   - Argument fragmentation (quotes, variables)

3. **Runner** (`src/runner.rs`)
   - Test discovery and execution
   - Working directory management
   - Test result aggregation
   - Txtar integration

4. **State** (`src/state.rs`)
   - Execution context
   - Environment variables
   - Virtual files (stdout, stderr)
   - File system operations

5. **Commands** (`src/commands.rs`)
   - Built-in command implementations
   - Process execution (exec)
   - File operations (cmp, grep, cat, cp, mv, rm, mkdir)
   - Flow control (cd, echo, env, sleep, stop, skip)

6. **Conditions** (`src/conditions.rs`)
   - Platform detection (unix, windows, darwin, linux)
   - Architecture detection (amd64, arm64)
   - Executable detection (`[exec:program]`)

## Key Design Decisions

### Line-Based Parsing

Scripts are parsed line-by-line:
- Simpler than full AST
- Error messages include line numbers
- Easier to debug test failures

### Virtual File System

Commands operate in a sandboxed working directory:
- Isolated from actual filesystem
- `stdout` and `stderr` captured as virtual files
- Automatic cleanup (unless `--keep` is set)

### Regex Matching

Pattern matching uses Rust's `regex` crate:
- Automatic multi-line mode (`(?m)`)
- DFA-based (1MB limit to prevent ReDoS)
- Escaped in arguments to prevent injection

### Environment Variable Expansion

Three types of expansion:
1. `$VAR` / `${VAR}` - Environment variables
2. `${/}` - Path separator (OS-dependent)
3. `${:}` - Path list separator (OS-dependent)

### Async Process Support

Commands can run in background:
```txtar
exec server &
exec client
wait  # Collect all background processes
```

## Testing Strategy

### Unit Tests

Located in `src/` modules:
- Parser tests for syntax edge cases
- Command tests for individual operations
- Condition tests for platform detection

### Integration Tests

Located in `tests/integration.rs`:
- Discovers and runs `.txtar` files
- Full end-to-end test execution

Run integration tests:
```bash
cargo test --test integration
```

### Example Test Files

Typical test structure:
```txtar
# Test description

exec mytool input.txt
stdout 'expected output'

! exec mytool --invalid
stderr 'error message'

-- input.txt --
test data

-- expected.txt --
expected output
```

## Common Tasks

### Adding a New Command

1. Implement `Cmd` trait in `commands.rs`
2. Add to `default_commands()` function
3. Add documentation in `--help` output
4. Add tests for command behavior

Example:
```rust
pub struct MyCmd;

impl Cmd for MyCmd {
    fn usage(&self) -> (String, String) {
        ("mycmd".into(), "[args]".into())
    }

    fn run(&self, args: &[String], state: &mut State) -> Result<CmdResult> {
        // Implementation
        Ok(CmdResult::Success)
    }
}
```

### Adding a New Condition

1. Implement `Condition` trait in `conditions.rs`
2. Add to `default_conditions()` function
3. Document in help text

### Extending Parser

When adding new syntax:
1. Update `parse_line()` in `parser.rs`
2. Add tests for new syntax
3. Update grammar in README

## Error Handling

Uses custom error types in `src/error.rs`:
- `ScriptError` - Test script errors
- `ErrorKind` - Error categorization

Provides:
- Clear error messages
- Line numbers for syntax errors
- Context for command failures

## Performance Considerations

- Tests run sequentially (no parallelism)
- Working directories created fresh for each test
- Regex compilation cached per test
- File operations are synchronous

## Code Style

- Use `anyhow::Result` for error propagation
- Prefer `CmdResult::Success` over explicit success returns
- Include example usage in doc comments
- Use `#[cfg(test)]` for test-only code

## Testing with AI

When running tests:
```bash
cargo test
```

Expected output: 23 tests passing

Integration tests:
```bash
TESTSCRIPT_VERBOSE=1 cargo test --test integration
```

## Known Limitations

1. **No parallel execution** - Tests run sequentially
2. **Windows path handling** - Some edge cases with mixed `/` and `\`
3. **Symlink support** - Limited on Windows
4. **Resource limits** - No CPU/memory limits

## Future Enhancements

Potential improvements:
- [ ] Parallel test execution
- [ ] Test sharding
- [ ] Timeout support
- [ ] Retry mechanism
- [ ] Test profiling
- [ ] JUnit XML output
- [ ] Custom command registration via env var

## Debugging Tips

### Enable Verbose Output

```bash
emx-testspec tests/ -v
```

Shows:
- Each command being executed
- Exit codes
- Stdout/stderr content

### Preserve Working Directories

```bash
emx-testspec tests/ --keep
```

Working directories are preserved in temp location for inspection.

### Environment Variables

```bash
TESTSCRIPT_VERBOSE=1 cargo test
```

Enables verbose logging in integration tests.

## Security Considerations

- **Sandboxed execution** - All operations in working directory
- **Regex limits** - DFA size limited to prevent ReDoS
- **Path validation** - Txtar paths are validated
- **No arbitrary code** - Only predefined commands

## See Also

- [emx-txtar](https://github.com/coreseekdev/emx-txtar) - Test fixture format
- [Go testscript](https://github.com/golang/go/blob/master/src/cmd/go/internal/testscript/testscript.go) - Original Go version
- [Testscript Grammar](https://github.com/roguepeppe/go-internal/blob/master/testscript/testscript.go) - Enhanced grammar
