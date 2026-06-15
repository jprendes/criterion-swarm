//! Output handlers and formatting utilities for benchmark reporting.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::ops::Range;
use std::sync::Mutex;

use ansi_replace::replacer::Writable;
use ansi_replace::AnsiExt as _;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::Reporter;

/// A no-op output handler that silently discards all events.
pub struct NoopReporter;

impl Reporter for NoopReporter {}

/// Inner mutable state for the default output handler.
struct DefaultOutputState {
    multi: MultiProgress,
    overall: ProgressBar,
    spinners: HashMap<String, ProgressBar>,
    pending: HashSet<String>,
    show_output: bool,
    enabled: bool,
    total: usize,
    done_count: usize,
}

/// Default output handler that displays progress bars and spinners.
///
/// This is used when no custom [`Reporter`] is provided.
pub struct ProgressReporter {
    progress: bool,
    output: bool,
    state: Mutex<Option<DefaultOutputState>>,
}

impl Default for ProgressReporter {
    fn default() -> Self {
        use std::io::IsTerminal;
        Self {
            progress: std::io::stderr().is_terminal(),
            output: true,
            state: Mutex::new(None),
        }
    }
}

impl ProgressReporter {
    /// Create a new default output handler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Show progress bars and spinners (default: true if stderr is a TTY).
    pub fn progress(mut self, show: bool) -> Self {
        self.progress = show;
        self
    }

    /// Show per-benchmark output lines on completion (default: true).
    pub fn output(mut self, show: bool) -> Self {
        self.output = show;
        self
    }
}

impl DefaultOutputState {
    fn println(&self, msg: &str) {
        if self.enabled {
            let _ = self.multi.println(msg);
        } else {
            eprintln!("{msg}");
        }
    }
}

impl Reporter for ProgressReporter {
    fn on_run_start(&self, benchmarks: &[String], _jobs: usize) {
        let total = benchmarks.len();
        let use_progress = self.progress;
        let multi = MultiProgress::new();
        let overall = if use_progress {
            let bar = multi.add(ProgressBar::new(total as u64));
            bar.set_style(
                ProgressStyle::with_template("{prefix} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                    .unwrap()
                    .progress_chars("━━─"),
            );
            bar.set_prefix("Benchmarks");
            bar
        } else {
            ProgressBar::hidden()
        };

        *self.state.lock().unwrap() = Some(DefaultOutputState {
            multi,
            overall,
            spinners: HashMap::new(),
            pending: HashSet::new(),
            show_output: self.output,
            enabled: use_progress,
            total,
            done_count: 0,
        });
    }

    fn on_benchmark_started(&self, benchmark: &str) {
        let mut guard = self.state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        if !state.enabled {
            return;
        }
        state.pending.insert(benchmark.to_string());
    }

    fn on_output_line(&self, benchmark: &str, line: &str) {
        let mut guard = self.state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };

        // If this is a pending benchmark, create and show its spinner now
        if state.pending.remove(benchmark) {
            let bar = state
                .multi
                .insert_before(&state.overall, ProgressBar::new_spinner());
            bar.set_style(
                ProgressStyle::with_template("  {spinner:.green} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            bar.enable_steady_tick(std::time::Duration::from_millis(100));
            state.spinners.insert(benchmark.to_string(), bar);
        }

        let Some(spinner) = state.spinners.get(benchmark) else {
            return;
        };
        if !line.is_empty() {
            spinner.set_message(format!("\x1b[1;32m{benchmark}\x1b[0m: {line}"));
        }
    }

    fn on_benchmark_complete(&self, benchmark: &str, output_lines: &[String]) {
        let mut guard = self.state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };

        // Finish spinner
        if let Some(bar) = state.spinners.remove(benchmark) {
            bar.finish_and_clear();
            state.multi.remove(&bar);
        }

        state.done_count += 1;
        state.overall.set_position(state.done_count as u64);

        let total = state.total;
        let done_count = state.done_count;
        state.println(&format!(
            "[{done_count}/{total}] \x1b[1;32m{benchmark}\x1b[0m ... done"
        ));

        if state.show_output {
            for line in output_lines {
                if !line.starts_with("Benchmarking") {
                    state.println(line);
                }
            }
            state.println("");
        }
    }

    fn on_benchmark_failed(&self, benchmark: &str, output_lines: &[String], error: &anyhow::Error) {
        let mut guard = self.state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };

        // Finish spinner
        if let Some(bar) = state.spinners.remove(benchmark) {
            bar.finish_and_clear();
            state.multi.remove(&bar);
        }

        state.done_count += 1;
        state.overall.set_position(state.done_count as u64);

        let total = state.total;
        let done_count = state.done_count;
        state.println(&format!(
            "[{done_count}/{total}] \x1b[1;32m{benchmark}\x1b[0m ... FAILED"
        ));

        if state.show_output {
            for line in output_lines {
                if !line.starts_with("Benchmarking") {
                    state.println(line);
                }
            }
            state.println("");
        }

        state.println(&format!("  error: {error}"));
    }

    fn on_run_complete(&self) {
        let guard = self.state.lock().unwrap();
        let Some(state) = guard.as_ref() else { return };
        state.overall.finish_and_clear();
    }
}

/// Returns true if an output line is build noise that should be suppressed.
pub(crate) fn is_noisy_line(line: &str) -> bool {
    line.contains("waiting for file lock on")
        || line.contains("Gnuplot not found")
        || line.contains("`bench` profile [optimized]")
}

/// Strip the bench name from an output line.
///
/// Strategy:
/// - If the line starts with the bench name, replace it with spaces to preserve alignment
/// - Any other appearance of the bench name and surrounding whitespace are removed entirely
/// - ANSI codes are preserved in all cases
pub(crate) fn strip_bench_prefix(line: &str, bench: &str) -> String {
    let escaped = regex::escape(bench);
    let pattern = regex::Regex::new(&format!(r" ?{escaped}")).unwrap();

    let result = line.ansi_replace(&pattern, |m: &str, i: Range<usize>, dst: &mut Writable| {
        if i.start == 0 && m == bench {
            write!(dst, "{:n$}", " ", n = m.len())?;
        }
        Ok(())
    });

    if result.ansi_strip().trim().is_empty() {
        return String::new();
    }

    result
}
