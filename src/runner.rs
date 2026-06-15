//! Orchestrates parallel benchmark execution, wiring together process spawning
//! and progress reporting.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Result};
use cpu_pin::CpuInfo;

use crate::cpu::PerformanceCoresPool;
use crate::output::{is_noisy_line, strip_bench_prefix};
use crate::process::{self, ProcessOutput};
use crate::Reporter;

/// Events sent from benchmark tasks to the orchestration loop.
enum BenchEvent {
    /// The benchmark has started running.
    Started { bench: String },
    /// An output line was produced by the given benchmark.
    OutputLine { bench: String, line: String },
    /// The benchmark has completed.
    Done(BenchResult),
}

/// Result of a single benchmark run, combining identity with output.
struct BenchResult {
    bench: String,
    output_lines: Vec<String>,
    success: Result<()>,
}

/// Orchestrates parallel benchmark execution with progress reporting.
pub struct BenchRunner {
    max_jobs: usize,
    bench_args: Vec<String>,
    output: Arc<dyn Reporter>,
}

impl BenchRunner {
    /// Create a new runner with the given configuration.
    pub fn new(max_jobs: usize, bench_args: &[String], output: Arc<dyn Reporter>) -> Self {
        Self {
            max_jobs,
            bench_args: bench_args.to_vec(),
            output,
        }
    }

    /// Run all benchmarks in parallel.
    ///
    /// Each entry is a (binary_path, benchmark_name) pair.
    pub async fn run(&self, benches: &[(PathBuf, String)]) -> Result<()> {
        let bench_names: Vec<String> = benches.iter().map(|(_, name)| name.clone()).collect();
        self.output.on_run_start(&bench_names, self.max_jobs);

        if self.max_jobs > PerformanceCoresPool::num_cores() {
            bail!(
                "Requested number of jobs {} exceeds available performance cores {}, use --jobs=0 or --quick to use all available performance cores.",
                self.max_jobs,
                PerformanceCoresPool::num_cores(),
            );
        }

        let pool = PerformanceCoresPool::new(self.max_jobs)?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<BenchEvent>();

        // Spawn all benchmarks (they'll wait on the semaphore internally)
        for (binary, bench) in benches {
            let bench = bench.clone();
            let binary = binary.clone();
            let tx = tx.clone();
            let pool = pool.clone();
            let bench_args = self.bench_args.clone();

            tokio::spawn(async move {
                let core = pool.get().await;
                Self::run_one(&bench, &binary, core, &tx, &bench_args).await;
            });
        }

        // Drop our sender so rx closes when all tasks finish
        drop(tx);

        // Process events as they arrive
        let mut failed = Vec::new();

        while let Some(event) = rx.recv().await {
            match event {
                BenchEvent::Started { bench } => {
                    self.output.on_benchmark_started(&bench);
                }
                BenchEvent::OutputLine { bench, line } => {
                    if is_noisy_line(&line) {
                        continue;
                    }
                    let line = strip_bench_prefix(&line, &bench);
                    if !line.is_empty() {
                        self.output.on_output_line(&bench, &line);
                    }
                }
                BenchEvent::Done(result) => {
                    let output_lines: Vec<String> = result
                        .output_lines
                        .iter()
                        .filter(|l| !is_noisy_line(l))
                        .map(|l| strip_bench_prefix(l, &result.bench))
                        .filter(|l| !l.is_empty())
                        .collect();

                    match &result.success {
                        Ok(()) => self
                            .output
                            .on_benchmark_complete(&result.bench, &output_lines),
                        Err(e) => self
                            .output
                            .on_benchmark_failed(&result.bench, &output_lines, e),
                    }

                    if result.success.is_err() {
                        failed.push(result.bench);
                    }
                }
            }
        }

        self.output.on_run_complete();

        if !failed.is_empty() {
            anyhow::bail!(
                "{} benchmark(s) failed: {}",
                failed.len(),
                failed.join(", ")
            );
        }

        Ok(())
    }

    /// Run a single benchmark, streaming output events and sending the final result.
    async fn run_one(
        bench: &str,
        binary: &Path,
        core: impl Deref<Target = CpuInfo>,
        event_tx: &tokio::sync::mpsc::UnboundedSender<BenchEvent>,
        bench_args: &[String],
    ) {
        // Create a channel for output lines from the process
        let (output_tx, mut output_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let bench_name = bench.to_string();
        let event_tx_clone = event_tx.clone();

        // Forward output lines as events
        let forwarder = tokio::spawn(async move {
            while let Some(line) = output_rx.recv().await {
                let _ = event_tx_clone.send(BenchEvent::OutputLine {
                    bench: bench_name.clone(),
                    line,
                });
            }
        });

        // Signal that this benchmark is starting
        let _ = event_tx.send(BenchEvent::Started {
            bench: bench.to_string(),
        });

        let result = match process::run(bench, binary, core, &output_tx, bench_args).await {
            Ok(ProcessOutput { output_lines }) => BenchResult {
                bench: bench.to_string(),
                output_lines,
                success: Ok(()),
            },
            Err(e) => BenchResult {
                bench: bench.to_string(),
                output_lines: vec![],
                success: Err(e),
            },
        };

        // Ensure all output forwarding completes before sending Done
        drop(output_tx);
        let _ = forwarder.await;

        let _ = event_tx.send(BenchEvent::Done(result));
    }
}
