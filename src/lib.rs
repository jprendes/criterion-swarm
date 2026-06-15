//! # criterion-swarm
//!
//! Parallel [Criterion](https://github.com/bheisler/criterion.rs) benchmark runner with
//! CPU pinning and progress reporting.
//!
//! This crate discovers criterion benchmark binaries, enumerates their individual benchmarks,
//! and runs them in parallel—each pinned to a dedicated performance core—with live progress
//! spinners and a summary on completion.
//!
//! ## Usage
//!
//! ```no_run
//! # async fn example() -> anyhow::Result<()> {
//! use criterion_swarm::CriterionSwarm;
//!
//! CriterionSwarm::builder()
//!     .jobs(4)
//!     .run()
//!     .await?;
//! # Ok(())
//! # }
//! ```

mod cpu;
mod discovery;
mod output;
mod process;
mod runner;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Arg, Command};

use self::cpu::PerformanceCoresPool;
use self::discovery::BenchmarkDiscovery;
pub use self::output::{NoopReporter, ProgressReporter};
use self::runner::BenchRunner;

/// Trait for receiving events from benchmark execution.
///
/// Implement this to customize how benchmark progress and results are reported.
/// All methods have default no-op implementations, so you only need to override
/// the ones you care about.
#[allow(unused_variables)]
pub trait Reporter: Send + Sync {
    /// Called once before any benchmarks start, with the list of benchmark names
    /// that will be executed and the number of parallel jobs (CPUs) that will be used.
    fn on_run_start(&self, benchmarks: &[String], jobs: usize) {}

    /// Called when a benchmark starts executing.
    fn on_benchmark_started(&self, benchmark: &str) {}

    /// Called when a benchmark produces an output line (streamed in real time).
    fn on_output_line(&self, benchmark: &str, line: &str) {}

    /// Called when a benchmark completes successfully.
    fn on_benchmark_complete(&self, benchmark: &str, output_lines: &[String]) {}

    /// Called when a benchmark fails.
    fn on_benchmark_failed(&self, benchmark: &str, output_lines: &[String], error: &anyhow::Error) {
    }

    /// Called once after all benchmarks have finished.
    fn on_run_complete(&self) {}
}

/// Builder for configuring and running parallel criterion benchmarks.
///
/// Create via [`CriterionSwarm::builder()`], configure with the setter methods,
/// then call [`.run()`](Self::run) to execute.
#[derive(Clone)]
pub struct CriterionSwarm {
    binaries: Vec<PathBuf>,
    jobs: usize,
    build_args: Vec<String>,
    bench_args: Vec<String>,
    output: Option<Arc<dyn Reporter>>,
}

impl CriterionSwarm {
    /// Create a new builder with default settings.
    pub fn builder() -> Self {
        Self {
            binaries: Vec::new(),
            jobs: 0,
            build_args: Vec::new(),
            bench_args: Vec::new(),
            output: None,
        }
    }

    /// Add a pre-built benchmark binary (skip build step).
    pub fn binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.binaries.push(path.into());
        self
    }

    /// Add multiple pre-built benchmark binaries.
    pub fn binaries(mut self, paths: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        self.binaries.extend(paths.into_iter().map(Into::into));
        self
    }

    /// Number of benchmarks to run in parallel (0 = all P-cores).
    pub fn jobs(mut self, jobs: usize) -> Self {
        self.jobs = jobs;
        self
    }

    /// Add an extra argument to pass to `cargo build` when building benchmarks.
    pub fn build_arg(mut self, arg: impl Into<String>) -> Self {
        self.build_args.push(arg.into());
        self
    }

    /// Add multiple extra arguments to pass to `cargo build` when building benchmarks.
    pub fn build_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.build_args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Add an extra argument to pass to the criterion benchmark binary.
    ///
    /// These arguments are forwarded both during benchmark discovery (`--list`)
    /// and when running each benchmark. `--exact` and the positional filter are
    /// parsed out automatically and handled specially during execution (each
    /// benchmark is always run by exact name).
    pub fn bench_arg(mut self, arg: impl Into<String>) -> Self {
        self.bench_args.push(arg.into());
        self
    }

    /// Add multiple extra arguments to pass to the criterion benchmark binary.
    ///
    /// These arguments are forwarded both during benchmark discovery (`--list`)
    /// and when running each benchmark. `--exact` and the positional filter are
    /// parsed out automatically and handled specially during execution (each
    /// benchmark is always run by exact name).
    pub fn bench_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.bench_args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set a custom output handler for benchmark events.
    ///
    /// The handler receives callbacks for benchmark lifecycle events
    /// (started, output line, completed, failed).
    pub fn output(mut self, output: impl Reporter + 'static) -> Self {
        self.output = Some(Arc::new(output));
        self
    }

    /// Parse `--exact` and the positional filter out of `bench_args`,
    /// returning `(filter, exact, remaining_args)`.
    fn parse_bench_args(&self) -> (Option<String>, bool, Vec<String>) {
        let cmd = Command::new("bench")
            .no_binary_name(true)
            .disable_help_flag(true)
            .disable_version_flag(true)
            .arg(Arg::new("FILTER").index(1))
            .arg(Arg::new("exact").long("exact").num_args(0))
            .ignore_errors(true);

        let matches = cmd.get_matches_from(&self.bench_args);

        let filter = matches.get_one::<String>("FILTER").cloned();
        let exact = matches.get_flag("exact");

        // Rebuild remaining args: strip --exact and the filter value
        let mut remaining = Vec::new();
        let mut filter_removed = false;
        for arg in &self.bench_args {
            if arg == "--exact" {
                continue;
            }
            // Remove the first non-flag arg that matches the parsed filter
            if !filter_removed {
                if let Some(f) = &filter {
                    if arg == f {
                        filter_removed = true;
                        continue;
                    }
                }
            }
            remaining.push(arg.clone());
        }

        (filter, exact, remaining)
    }

    /// Determine the maximum number of parallel benchmark jobs.
    fn max_jobs(&self) -> usize {
        match self.jobs {
            0 => PerformanceCoresPool::num_cores(),
            j => j,
        }
    }

    /// Run criterion benchmarks in parallel.
    ///
    /// This is an async method—bring your own tokio runtime.
    pub async fn run(&self) -> Result<()> {
        let (filter, exact, remaining_bench_args) = self.parse_bench_args();

        let discovery = BenchmarkDiscovery::new(
            &self.build_args,
            &remaining_bench_args,
            filter.as_deref(),
            exact,
        );

        let binaries = if self.binaries.is_empty() {
            discovery.build().await?
        } else {
            self.binaries.clone()
        };

        let mut benches = Vec::new();
        for binary in &binaries {
            for name in discovery.list(binary).await? {
                benches.push((binary.clone(), name));
            }
        }

        if benches.is_empty() {
            anyhow::bail!("No benchmarks found");
        }

        let max_jobs = self.max_jobs();

        let output: Arc<dyn Reporter> = match &self.output {
            Some(o) => o.clone(),
            None => Arc::new(output::ProgressReporter::default()),
        };

        let runner = BenchRunner::new(max_jobs, &remaining_bench_args, output);
        runner.run(&benches).await?;

        Ok(())
    }
}
