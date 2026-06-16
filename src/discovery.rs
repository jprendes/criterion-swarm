//! Benchmark binary discovery and enumeration.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::Reporter;

/// Discovers available benchmarks by querying the benchmark binary.
pub struct BenchmarkDiscovery {
    build_args: Vec<String>,
    bench_args: Vec<String>,
    filter: Option<String>,
    exact: bool,
}

impl BenchmarkDiscovery {
    /// Create a new discovery instance with the given parameters.
    pub fn new(
        build_args: &[String],
        bench_args: &[String],
        filter: Option<&str>,
        exact: bool,
    ) -> Self {
        Self {
            build_args: build_args.to_vec(),
            bench_args: bench_args.to_vec(),
            filter: filter.map(|s| s.to_string()),
            exact,
        }
    }

    /// Build all benchmark binaries and return their paths.
    pub async fn build(&self, reporter: &Arc<dyn Reporter>) -> Result<Vec<PathBuf>> {
        let mut cmd = Command::new("cargo");
        cmd.args([
            "build",
            "--release",
            "--benches",
            "--message-format=json",
            "--color=always",
        ]);
        cmd.args(&self.build_args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("Failed to run cargo build for benchmarks")?;

        reporter.on_build_start();

        // Stream stderr lines to the reporter
        let stderr = child.stderr.take().unwrap();
        let stderr_handle = {
            let reporter = reporter.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                let mut collected = Vec::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    reporter.on_build_output(&line);
                    collected.push(line);
                }
                collected
            })
        };

        let output = child
            .wait_with_output()
            .await
            .context("Failed to run cargo build for benchmarks")?;

        let build_lines = stderr_handle.await.unwrap_or_default();

        if !output.status.success() {
            reporter.on_build_failed(&build_lines);
            bail!("Failed to build benchmarks");
        }

        reporter.on_build_complete(&build_lines);

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut binaries = Vec::new();

        // Parse cargo's JSON output to find all benchmark binary paths
        for line in stdout.lines() {
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-artifact") {
                continue;
            }
            let is_bench = msg
                .get("target")
                .and_then(|t| t.get("kind"))
                .and_then(|k| k.as_array())
                .is_some_and(|kinds| kinds.iter().any(|k| k.as_str() == Some("bench")));
            if !is_bench {
                continue;
            }
            if let Some(filenames) = msg.get("filenames").and_then(|f| f.as_array()) {
                for f in filenames {
                    if let Some(path) = f.as_str() {
                        // Skip non-executable artifacts:
                        //   .d    = dep-info files (all platforms)
                        //   .pdb  = debug symbols (Windows)
                        //   .dSYM = debug symbol bundles (macOS)
                        //   .dwp  = DWARF packages (Linux, split-debuginfo)
                        //   .lib  = import libraries (Windows)
                        //   .exp  = export files (Windows)
                        let dominated = [".d", ".pdb", ".dSYM", ".dwp", ".lib", ".exp"];
                        if dominated.iter().any(|ext| path.ends_with(ext)) {
                            continue;
                        }
                        binaries.push(PathBuf::from(path));
                    }
                }
            }
        }

        if binaries.is_empty() {
            bail!("No benchmark binaries found in cargo build output");
        }

        Ok(binaries)
    }

    /// List all benchmark names matching the configured filter.
    pub async fn list(&self, binary: &Path) -> Result<Vec<String>> {
        let mut cmd = Command::new(binary);
        cmd.args(["--bench", "--list"]);
        if self.exact {
            cmd.arg("--exact");
        }
        if let Some(filter) = &self.filter {
            cmd.arg(filter);
        }
        cmd.args(&self.bench_args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());

        let output = cmd
            .output()
            .await
            .with_context(|| format!("Failed to run {} --bench --list", binary.display()))?;

        if !output.status.success() {
            bail!(
                "Failed to list benchmarks from {}: exit code {}",
                binary.display(),
                output.status,
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let names: Vec<String> = stdout
            .lines()
            .filter_map(|line| line.strip_suffix(": benchmark"))
            .map(|s| s.to_string())
            .collect();

        Ok(names)
    }
}
