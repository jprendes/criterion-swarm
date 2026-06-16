//! Output handlers and formatting utilities for benchmark reporting.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write;
use std::ops::{BitOr, BitOrAssign, Range};
use std::sync::Mutex;
use std::time::Instant;

use ansi_replace::replacer::Writable;
use ansi_replace::AnsiExt as _;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

use crate::Reporter;

/// Controls how output is displayed for a phase (build or benchmarks).
///
/// Flags can be combined with `|` to enable multiple behaviors:
///
/// ```
/// use criterion_swarm::OutputMode;
///
/// let mode = OutputMode::SPINNER | OutputMode::STREAM | OutputMode::SUMMARY;
/// ```
///
/// ## Flags
///
/// - [`SPINNER`](Self::SPINNER) — Show a live spinner with recent output.
/// - [`STREAM`](Self::STREAM) — Print output as it arrives.
/// - [`SUMMARY`](Self::SUMMARY) — Print a completion summary.
/// - [`SILENT`](Self::SILENT) — No output at all.
///
/// ## Build context
///
/// For builds, `SPINNER` and `STREAM` both display cargo output in real time.
/// `SPINNER` shows a compact rolling display; `STREAM` prints every line.
/// If both are set, `STREAM` takes priority.
///
/// ## Benchmark context
///
/// For benchmarks, `SPINNER` shows live progress during execution while
/// `STREAM` prints each benchmark's results on completion. Both can be
/// active simultaneously.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct OutputMode {
    pub(crate) spinner: bool,
    pub(crate) stream: bool,
    pub(crate) summary: bool,
}

impl OutputMode {
    /// No output at all.
    pub const SILENT: Self = Self {
        spinner: false,
        stream: false,
        summary: false,
    };

    /// Show a live spinner with recent output lines.
    pub const SPINNER: Self = Self {
        spinner: true,
        stream: false,
        summary: false,
    };

    /// Print output as it arrives (build lines, or benchmark results on completion).
    pub const STREAM: Self = Self {
        spinner: false,
        stream: true,
        summary: false,
    };

    /// Print a completion summary.
    pub const SUMMARY: Self = Self {
        spinner: false,
        stream: false,
        summary: true,
    };
}

impl BitOr for OutputMode {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self {
            spinner: self.spinner || rhs.spinner,
            stream: self.stream || rhs.stream,
            summary: self.summary || rhs.summary,
        }
    }
}

impl BitOrAssign for OutputMode {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

/// A no-op output handler that silently discards all events.
pub struct NoopReporter;

impl Reporter for NoopReporter {}

/// Inner mutable state for the default output handler.
struct DefaultOutputState {
    multi: MultiProgress,
    overall: ProgressBar,
    spinners: HashMap<String, ProgressBar>,
    pending: HashSet<String>,
    mode: OutputMode,
    total: usize,
    done_count: usize,
    start_time: Instant,
}

/// State for the build spinner showing recent output lines.
struct BuildSpinnerState {
    spinner: ProgressBar,
    lines: VecDeque<String>,
}

/// Default output handler that displays progress bars and spinners.
///
/// This is used when no custom [`Reporter`] is provided.
pub struct ProgressReporter {
    build: OutputMode,
    benchmarks: OutputMode,
    build_state: Mutex<Option<BuildSpinnerState>>,
    state: Mutex<Option<DefaultOutputState>>,
}

impl Default for ProgressReporter {
    fn default() -> Self {
        use std::io::IsTerminal;
        let tty = std::io::stderr().is_terminal();
        Self {
            build: if tty {
                OutputMode::SPINNER | OutputMode::SUMMARY
            } else {
                OutputMode::STREAM | OutputMode::SUMMARY
            },
            benchmarks: if tty {
                OutputMode::SPINNER | OutputMode::STREAM | OutputMode::SUMMARY
            } else {
                OutputMode::STREAM | OutputMode::SUMMARY
            },
            build_state: Mutex::new(None),
            state: Mutex::new(None),
        }
    }
}

impl ProgressReporter {
    /// Create a new default output handler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure how build output is displayed.
    ///
    /// - [`OutputMode::SPINNER`] — compact rolling display of recent lines.
    /// - [`OutputMode::STREAM`] — print every line as it arrives.
    /// - [`OutputMode::SUMMARY`] — print "Build complete" when done.
    ///
    /// If both `SPINNER` and `STREAM` are set, `STREAM` takes priority.
    ///
    /// Default: `SPINNER | SUMMARY` in a TTY, `STREAM | SUMMARY` otherwise.
    pub fn build(mut self, mode: OutputMode) -> Self {
        self.build = mode;
        self
    }

    /// Configure how benchmark output is displayed.
    ///
    /// - [`OutputMode::SPINNER`] — live progress bar and per-benchmark spinners.
    /// - [`OutputMode::STREAM`] — print each benchmark's results on completion.
    /// - [`OutputMode::SUMMARY`] — print a summary line when all benchmarks finish.
    ///
    /// `SPINNER` and `STREAM` can be active simultaneously.
    ///
    /// Default: `SPINNER | STREAM | SUMMARY` in a TTY, `STREAM | SUMMARY` otherwise.
    pub fn benchmarks(mut self, mode: OutputMode) -> Self {
        self.benchmarks = mode;
        self
    }
}

impl DefaultOutputState {
    fn println(&self, msg: &str) {
        if self.mode.spinner {
            let _ = self.multi.println(msg);
        } else {
            eprintln!("{msg}");
        }
    }
}

impl Reporter for ProgressReporter {
    fn on_build_start(&self) {
        if self.build.spinner && !self.build.stream {
            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::with_template("{spinner:.green} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));
            spinner.set_message("Building benchmarks...");
            *self.build_state.lock().unwrap() = Some(BuildSpinnerState {
                spinner,
                lines: VecDeque::with_capacity(5),
            });
        }
    }

    fn on_build_output(&self, line: &str) {
        if line.is_empty() {
            return;
        }
        // Stream mode (or spinner|stream): print every line
        if self.build.stream {
            eprintln!("{line}");
            return;
        }
        // Spinner mode: update rolling display
        let mut guard = self.build_state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        state.lines.push_back(line.to_string());
        if state.lines.len() > 5 {
            state.lines.pop_front();
        }
        let msg = state
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| if i == 0 { l.clone() } else { format!("  {l}") })
            .collect::<Vec<_>>()
            .join("\n");
        state.spinner.set_message(msg);
    }

    fn on_build_complete(&self, _output_lines: &[String]) {
        let mut guard = self.build_state.lock().unwrap();
        if let Some(state) = guard.take() {
            state.spinner.finish_and_clear();
        }
        if self.build.summary {
            eprintln!("Build complete");
        }
    }

    fn on_build_failed(&self, output_lines: &[String]) {
        let mut guard = self.build_state.lock().unwrap();
        if let Some(state) = guard.take() {
            state.spinner.finish_and_clear();
        }
        // Always print build output on failure so errors are visible
        if !self.build.stream {
            for line in output_lines {
                eprintln!("{line}");
            }
        }
    }

    fn on_run_start(&self, benchmarks: &[String], _jobs: usize) {
        let total = benchmarks.len();
        let mode = self.benchmarks;
        let multi = MultiProgress::new();
        let overall = if mode.spinner {
            let bar = multi.add(ProgressBar::new(total as u64));
            bar.set_style(
                ProgressStyle::with_template(
                    "{prefix} [{bar:40.cyan/blue}] {pos}/{len} ({elapsed})",
                )
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
            mode,
            total,
            done_count: 0,
            start_time: Instant::now(),
        });
    }

    fn on_benchmark_started(&self, benchmark: &str) {
        let mut guard = self.state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        if !state.mode.spinner {
            return;
        }
        state.pending.insert(benchmark.to_string());
    }

    fn on_output_line(&self, benchmark: &str, line: &str) {
        let mut guard = self.state.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        if !state.mode.spinner {
            return;
        }

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

        if state.mode.stream {
            let total = state.total;
            let done_count = state.done_count;
            state.println(&format!(
                "[{done_count}/{total}] \x1b[1;32m{benchmark}\x1b[0m ... done"
            ));

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

        if state.mode.stream {
            let total = state.total;
            let done_count = state.done_count;
            state.println(&format!(
                "[{done_count}/{total}] \x1b[1;32m{benchmark}\x1b[0m ... FAILED"
            ));

            for line in output_lines {
                if !line.starts_with("Benchmarking") {
                    state.println(line);
                }
            }
            state.println("");

            state.println(&format!("  error: {error}"));
        }
    }

    fn on_run_complete(&self) {
        let guard = self.state.lock().unwrap();
        let Some(state) = guard.as_ref() else { return };
        state.overall.finish_and_clear();
        if state.mode.summary {
            let elapsed = state.start_time.elapsed();
            let secs = elapsed.as_secs();
            let time_str = if secs >= 60 {
                format!("{}m {:02}s", secs / 60, secs % 60)
            } else {
                format!("{:.1}s", elapsed.as_secs_f64())
            };
            state.println(&format!(
                "{} benchmarks complete in {time_str}",
                state.total
            ));
        }
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
