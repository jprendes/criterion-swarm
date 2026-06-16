# criterion-swarm

Parallel [Criterion](https://github.com/bheisler/criterion.rs) benchmark runner with CPU pinning and progress reporting.

## Features

- **Parallel execution** — Run multiple criterion benchmarks simultaneously, each pinned to a dedicated performance core.
- **Live progress** — Progress bars and spinners show real-time build and benchmark status.
- **Pluggable reporting** — Implement the `Reporter` trait to customize how results are displayed.
- **Automatic discovery** — Builds and lists criterion benchmarks from your workspace automatically.
- **Filtering** — Supports `--exact` and filter patterns to run subsets of benchmarks.
- **Two-phase execution** — Inspect discovered benchmarks before running them.

## Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
criterion-swarm = "0.1"
```

Run all benchmarks in parallel:

```rust
use criterion_swarm::CriterionSwarm;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CriterionSwarm::builder()
        .jobs(4)
        .run()
        .await?;
    Ok(())
}
```

## Two-Phase Execution

Use `prepare()` to build and discover benchmarks, then inspect before running:

```rust
let swarm = CriterionSwarm::builder()
    .jobs(4)
    .prepare()
    .await?;

println!("Found {} benchmarks, using {} CPUs", swarm.benchmarks().len(), swarm.jobs());
swarm.run().await?;
```

## Builder API

```rust
CriterionSwarm::builder()
    .jobs(4)                          // Number of parallel jobs (0 = all P-cores)
    .binary("target/release/bench")   // Use a pre-built binary
    .build_args(["--features", "x"])  // Extra cargo build arguments
    .bench_args(["--quick"])          // Extra criterion arguments
    .output(ProgressReporter::new())  // Custom reporter
    .run()
    .await?;
```

## Custom Reporters

Implement the `Reporter` trait to handle build and benchmark events:

```rust
use criterion_swarm::Reporter;

struct MyReporter;

impl Reporter for MyReporter {
    fn on_build_start(&self) {
        println!("Building benchmarks...");
    }

    fn on_build_output(&self, line: &str) {
        println!("  {line}");
    }

    fn on_build_complete(&self, output_lines: &[String]) {
        println!("Build done!");
    }

    fn on_build_failed(&self, output_lines: &[String]) {
        println!("Build failed!");
        for line in output_lines {
            eprintln!("  {line}");
        }
    }

    fn on_run_start(&self, benchmarks: &[String], jobs: usize) {
        println!("Running {} benchmarks on {} cores", benchmarks.len(), jobs);
    }

    fn on_benchmark_complete(&self, benchmark: &str, output_lines: &[String]) {
        println!("✓ {benchmark}");
    }

    fn on_run_complete(&self) {
        println!("All done!");
    }
}
```

Built-in reporters:
- `ProgressReporter` — Progress bars and spinners with live build/benchmark output (default)
- `NoopReporter` — Discards all output

## License

Apache-2.0
