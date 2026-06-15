use criterion_swarm::{CriterionSwarm, ProgressReporter, Reporter};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // Run all benchmarks with the default progress output (quiet mode):
    CriterionSwarm::builder()
        .jobs(2)
        .bench_arg("--quick")
        .output(ProgressReporter::new())
        .run()
        .await?;

    // Run a filtered benchmark with a custom output handler:
    CriterionSwarm::builder()
        .jobs(1)
        .bench_args(["--exact", "fib 10", "--quick"])
        .output(SummaryOutput::new())
        .run()
        .await?;

    Ok(())
}

/// A custom output handler that prints a summary at the end.
struct SummaryOutput {
    results: std::sync::Mutex<Vec<(String, bool)>>,
}

impl SummaryOutput {
    fn new() -> Self {
        Self {
            results: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl Reporter for SummaryOutput {
    fn on_run_start(&self, benchmarks: &[String], jobs: usize) {
        let n = benchmarks.len();
        eprintln!("Will run {n} benchmark(s) on {jobs} CPU(s)");
    }

    fn on_benchmark_started(&self, benchmark: &str) {
        eprintln!("  → starting: {benchmark}");
    }

    fn on_benchmark_complete(&self, benchmark: &str, _lines: &[String]) {
        let mut results = self.results.lock().unwrap();
        results.push((benchmark.to_string(), true));
    }

    fn on_benchmark_failed(&self, benchmark: &str, _lines: &[String], _error: &anyhow::Error) {
        let mut results = self.results.lock().unwrap();
        results.push((benchmark.to_string(), false));
    }

    fn on_run_complete(&self) {
        let results = self.results.lock().unwrap();
        let passed = results.iter().filter(|(_, ok)| *ok).count();
        let failed = results.iter().filter(|(_, ok)| !ok).count();
        eprintln!("\nSummary: {passed} passed, {failed} failed");
    }
}
