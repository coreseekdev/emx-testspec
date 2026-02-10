//! Test runner
//!
//! Orchestrates running testscript files — discovers txtar files in a directory,
//! creates temp dirs, extracts files, runs scripts, and reports results.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use crate::engine::Engine;
use crate::state::State;

/// Configuration for the test runner
pub struct RunConfig {
    /// Directory containing test scripts (txtar files)
    pub dir: PathBuf,
    /// Optional filter — only run tests matching this pattern
    pub filter: Option<String>,
    /// Root directory for temp working directories
    pub workdir_root: Option<PathBuf>,
    /// Preserve working directories after test (for debugging)
    pub preserve_work: bool,
    /// Setup function called before each test
    pub setup: Option<Box<dyn Fn(&mut SetupEnv) -> Result<(), Box<dyn std::error::Error>> + Send>>,
    /// Verbose mode — print script execution log
    pub verbose: bool,
    /// File extensions to scan (default: [".txtar"])
    pub extensions: Vec<String>,
}

/// Environment available during setup
pub struct SetupEnv {
    /// The working directory for the test
    pub work_dir: PathBuf,
    /// Environment variables to set
    pub env: Vec<(String, String)>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            dir: PathBuf::from("testdata"),
            filter: None,
            workdir_root: None,
            preserve_work: false,
            setup: None,
            verbose: false,
            extensions: vec![".txtar".into()],
        }
    }
}

/// Result of running all tests
#[derive(Debug)]
pub struct TestResult {
    /// Individual test case results
    pub cases: Vec<TestCaseResult>,
    /// Total duration
    pub duration: Duration,
}

impl TestResult {
    /// Check if all tests passed
    pub fn all_passed(&self) -> bool {
        self.cases.iter().all(|c| c.passed || c.skipped)
    }

    /// Count passed tests
    pub fn passed_count(&self) -> usize {
        self.cases.iter().filter(|c| c.passed && !c.skipped).count()
    }

    /// Count failed tests
    pub fn failed_count(&self) -> usize {
        self.cases.iter().filter(|c| !c.passed && !c.skipped).count()
    }

    /// Count skipped tests
    pub fn skipped_count(&self) -> usize {
        self.cases.iter().filter(|c| c.skipped).count()
    }

    /// Format a summary line
    pub fn summary(&self) -> String {
        format!(
            "{} passed, {} failed, {} skipped ({}ms)",
            self.passed_count(),
            self.failed_count(),
            self.skipped_count(),
            self.duration.as_millis(),
        )
    }
}

/// Result of a single test case
#[derive(Debug)]
pub struct TestCaseResult {
    /// Test name (filename without extension)
    pub name: String,
    /// Source file path
    pub file: PathBuf,
    /// Whether the test passed
    pub passed: bool,
    /// Whether the test was skipped
    pub skipped: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Execution log
    pub log: String,
    /// Duration
    pub duration: Duration,
    /// Working directory (if preserved)
    pub workdir: Option<PathBuf>,
}

/// The test runner
pub struct TestRunner {
    engine: Engine,
    config: RunConfig,
}

impl TestRunner {
    /// Create a new runner with the given config
    pub fn new(config: RunConfig) -> Self {
        Self {
            engine: Engine::new(),
            config,
        }
    }

    /// Create a new runner with a custom engine
    pub fn with_engine(engine: Engine, config: RunConfig) -> Self {
        Self { engine, config }
    }

    /// Get mutable reference to the engine (for registering custom commands)
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Discover test files in the configured directory
    pub fn discover(&self) -> Result<Vec<PathBuf>, std::io::Error> {
        let mut files = Vec::new();
        let dir = &self.config.dir;

        if !dir.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("test directory not found: {}", dir.display()),
            ));
        }

        if dir.is_file() {
            // Single file mode
            files.push(dir.clone());
            return Ok(files);
        }

        // Scan directory for test files
        self.scan_dir(dir, &mut files)?;

        files.sort();
        Ok(files)
    }

    fn scan_dir(&self, dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.scan_dir(&path, files)?;
            } else if self.is_test_file(&path) {
                // Apply filter if set
                if let Some(ref filter) = self.config.filter {
                    let name = path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("");
                    if !name.contains(filter.as_str()) {
                        continue;
                    }
                }
                files.push(path);
            }
        }
        Ok(())
    }

    fn is_test_file(&self, path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            self.config.extensions.iter().any(|ext| name.ends_with(ext.as_str()))
        } else {
            false
        }
    }

    /// Run all discovered tests
    pub fn run_all(&self) -> Result<TestResult, std::io::Error> {
        let start = Instant::now();
        let files = self.discover()?;

        let mut cases = Vec::new();
        for file in &files {
            let result = self.run_one(file);
            cases.push(result);
        }

        Ok(TestResult {
            cases,
            duration: start.elapsed(),
        })
    }

    /// Count the number of tests that would be run
    pub fn count_tests(&self) -> Result<usize, std::io::Error> {
        let files = self.discover()?;
        let count = files.len();
        Ok(count)
    }

    /// Run a single test file
    pub fn run_one(&self, file: &Path) -> TestCaseResult {
        let start = Instant::now();
        let name = file.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Phase 1: parse + prepare
        let (archive, tmpdir) = match self.prepare_test(file, &name) {
            Ok(pair) => pair,
            Err(error) => {
                return TestCaseResult {
                    name,
                    file: file.to_path_buf(),
                    passed: false,
                    skipped: false,
                    error: Some(error),
                    log: String::new(),
                    duration: start.elapsed(),
                    workdir: None,
                };
            }
        };

        let workdir = tmpdir.path().to_path_buf();
        let mut state = State::new(workdir.clone());

        // Phase 2: extract + setup + execute
        let (passed, skipped, error) = self.execute_test(file, &archive, &mut state, &workdir);

        // Preserve workdir on failure or if configured
        let preserved_workdir = if self.config.preserve_work || !passed {
            let path = tmpdir.path().to_path_buf();
            std::mem::forget(tmpdir); // leak to preserve
            Some(path)
        } else {
            None
        };

        TestCaseResult {
            name,
            file: file.to_path_buf(),
            passed,
            skipped,
            error,
            log: state.log,
            duration: start.elapsed(),
            workdir: preserved_workdir,
        }
    }

    /// Parse the txtar file and create a working directory.
    fn prepare_test(
        &self,
        file: &Path,
        name: &str,
    ) -> Result<(emx_txtar::Archive, tempfile::TempDir), String> {
        let data = std::fs::read_to_string(file)
            .map_err(|e| format!("failed to read file: {}", e))?;

        let decoder = emx_txtar::Decoder::new();
        let archive = decoder.decode(&data)
            .map_err(|e| format!("failed to parse txtar: {}", e))?;

        let tmpdir = self.create_workdir(name)
            .map_err(|e| format!("failed to create workdir: {}", e))?;

        Ok((archive, tmpdir))
    }

    /// Extract files, run setup, and execute the script. Returns (passed, skipped, error).
    fn execute_test(
        &self,
        file: &Path,
        archive: &emx_txtar::Archive,
        state: &mut State,
        workdir: &Path,
    ) -> (bool, bool, Option<String>) {
        // Extract archive files
        if let Err(e) = state.extract_files(archive) {
            return (false, false, Some(format!("failed to extract files: {}", e)));
        }

        // Run setup
        if let Some(ref setup) = self.config.setup {
            let mut env = SetupEnv {
                work_dir: workdir.to_path_buf(),
                env: Vec::new(),
            };
            if let Err(e) = setup(&mut env) {
                return (false, false, Some(format!("setup failed: {}", e)));
            }
            for (k, v) in env.env {
                state.setenv(k, v);
            }
        }

        // Execute the script
        let script = &archive.comment;
        let filename = file.to_string_lossy().to_string();

        match self.engine.execute(state, script, &filename) {
            Ok(()) => (true, false, None),
            Err(e) if e.is_skip() => (true, true, Some(e.message.clone())),
            Err(e) if e.is_stop() => (true, false, None),
            Err(e) => (false, false, Some(e.to_string())),
        }
    }

    fn create_workdir(&self, name: &str) -> Result<tempfile::TempDir, std::io::Error> {
        let prefix = format!("testscript-{}-", name);
        if let Some(ref root) = self.config.workdir_root {
            std::fs::create_dir_all(root)?;
            tempfile::Builder::new()
                .prefix(&prefix)
                .tempdir_in(root)
        } else {
            tempfile::Builder::new()
                .prefix(&prefix)
                .tempdir()
        }
    }
}

/// Builder API for convenient test runner construction
pub struct TestRunnerBuilder {
    config: RunConfig,
    engine: Option<Engine>,
}

impl TestRunnerBuilder {
    /// Start building a runner for the given directory
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            config: RunConfig {
                dir: dir.into(),
                ..Default::default()
            },
            engine: None,
        }
    }

    /// Set the test filter pattern
    pub fn filter(mut self, filter: impl Into<String>) -> Self {
        self.config.filter = Some(filter.into());
        self
    }

    /// Set the working directory root
    pub fn workdir_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.config.workdir_root = Some(root.into());
        self
    }

    /// Preserve working directories after tests
    pub fn preserve_work(mut self, preserve: bool) -> Self {
        self.config.preserve_work = preserve;
        self
    }

    /// Enable verbose output
    pub fn verbose(mut self, verbose: bool) -> Self {
        self.config.verbose = verbose;
        self
    }

    /// Set file extensions to scan
    pub fn extensions(mut self, exts: Vec<String>) -> Self {
        self.config.extensions = exts;
        self
    }

    /// Use a custom engine
    pub fn engine(mut self, engine: Engine) -> Self {
        self.engine = Some(engine);
        self
    }

    /// Build and return the runner
    pub fn build(self) -> TestRunner {
        if let Some(engine) = self.engine {
            TestRunner::with_engine(engine, self.config)
        } else {
            TestRunner::new(self.config)
        }
    }

    /// Build and run all tests
    pub fn run(self) -> Result<TestResult, std::io::Error> {
        self.build().run_all()
    }
}

/// Convenience function: create a runner builder for a directory
pub fn run(dir: impl Into<PathBuf>) -> TestRunnerBuilder {
    TestRunnerBuilder::new(dir)
}

/// Run testscript files and integrate with `#[test]` by panicking on failure.
///
/// Usage in cargo tests:
/// ```rust,ignore
/// #[test]
/// fn test_scripts() {
///     emx_testscript::run_and_assert("tests/testdata");
/// }
/// ```
pub fn run_and_assert(dir: impl Into<PathBuf>) {
    run_and_assert_with(dir, |_| {});
}

/// Like `run_and_assert` but allows engine customization.
pub fn run_and_assert_with(dir: impl Into<PathBuf>, customize: impl FnOnce(&mut Engine)) {
    let dir = dir.into();
    let mut engine = Engine::new();
    customize(&mut engine);

    let config = RunConfig {
        dir,
        verbose: std::env::var("TESTSCRIPT_VERBOSE").is_ok(),
        preserve_work: std::env::var("TESTSCRIPT_WORK").is_ok(),
        ..Default::default()
    };

    let runner = TestRunner::with_engine(engine, config);
    let result = runner.run_all().expect("failed to run tests");

    // Print results
    for case in &result.cases {
        if case.skipped {
            eprintln!("SKIP  {}: {}", case.name, case.error.as_deref().unwrap_or(""));
        } else if case.passed {
            eprintln!("PASS  {} ({}ms)", case.name, case.duration.as_millis());
        } else {
            eprintln!("FAIL  {}", case.name);
            if let Some(ref err) = case.error {
                eprintln!("  {}", err);
            }
            if !case.log.is_empty() {
                eprintln!("  --- log ---");
                for line in case.log.lines() {
                    eprintln!("  {}", line);
                }
            }
            if let Some(ref wd) = case.workdir {
                eprintln!("  workdir: {}", wd.display());
            }
        }
    }

    eprintln!("\n{}", result.summary());

    if !result.all_passed() {
        panic!("{} test(s) failed", result.failed_count());
    }
}
