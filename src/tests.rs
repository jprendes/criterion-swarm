use std::sync::Mutex;

use crate::output::{is_noisy_line, strip_bench_prefix};
use crate::{CriterionSwarm, NoopReporter, OutputMode, ProgressReporter, Reporter};

// --- Builder tests ---

#[test]
fn builder_defaults() {
    let swarm = CriterionSwarm::builder();
    assert_eq!(swarm.jobs, 0);
    assert!(swarm.binaries.is_empty());
    assert!(swarm.build_args.is_empty());
    assert!(swarm.bench_args.is_empty());
    assert!(swarm.output.is_none());
}

#[test]
fn builder_jobs() {
    let swarm = CriterionSwarm::builder().jobs(4);
    assert_eq!(swarm.jobs, 4);
}

#[test]
fn builder_binaries() {
    let swarm = CriterionSwarm::builder().binaries(["/tmp/a", "/tmp/b", "/tmp/c"]);
    assert_eq!(swarm.binaries.len(), 3);
}

#[test]
fn builder_build_args() {
    let swarm = CriterionSwarm::builder()
        .build_arg("--release")
        .build_args(["--features", "unstable"]);
    assert_eq!(
        swarm.build_args,
        vec!["--release", "--features", "unstable"]
    );
}

#[test]
fn builder_bench_args() {
    let swarm = CriterionSwarm::builder()
        .bench_arg("--warm-up-time")
        .bench_args(["5", "--sample-size", "50"]);
    assert_eq!(
        swarm.bench_args,
        vec!["--warm-up-time", "5", "--sample-size", "50"]
    );
}

#[test]
fn builder_output_sets_reporter() {
    let swarm = CriterionSwarm::builder().output(NoopReporter);
    assert!(swarm.output.is_some());
}

// --- parse_bench_args tests ---

#[test]
fn parse_bench_args_empty() {
    let swarm = CriterionSwarm::builder();
    let (filter, exact, remaining) = swarm.parse_bench_args();
    assert_eq!(filter, None);
    assert!(!exact);
    assert!(remaining.is_empty());
}

#[test]
fn parse_bench_args_filter_only() {
    let swarm = CriterionSwarm::builder().bench_args(["my_bench"]);
    let (filter, exact, remaining) = swarm.parse_bench_args();
    assert_eq!(filter, Some("my_bench".to_string()));
    assert!(!exact);
    assert!(remaining.is_empty());
}

#[test]
fn parse_bench_args_exact_and_filter() {
    let swarm = CriterionSwarm::builder().bench_args(["--exact", "fib 10"]);
    let (filter, exact, remaining) = swarm.parse_bench_args();
    assert_eq!(filter, Some("fib 10".to_string()));
    assert!(exact);
    assert!(remaining.is_empty());
}

#[test]
fn parse_bench_args_preserves_remaining() {
    let swarm = CriterionSwarm::builder().bench_args([
        "--exact",
        "fib 10",
        "--warm-up-time",
        "5",
        "--sample-size",
        "50",
    ]);
    let (filter, exact, remaining) = swarm.parse_bench_args();
    assert_eq!(filter, Some("fib 10".to_string()));
    assert!(exact);
    assert_eq!(
        remaining,
        vec!["--warm-up-time", "5", "--sample-size", "50"]
    );
}

#[test]
fn parse_bench_args_filter_with_extra_args() {
    let swarm = CriterionSwarm::builder().bench_args(["my_bench", "--sample-size", "100"]);
    let (filter, exact, remaining) = swarm.parse_bench_args();
    assert_eq!(filter, Some("my_bench".to_string()));
    assert!(!exact);
    assert_eq!(remaining, vec!["--sample-size", "100"]);
}

// --- is_noisy_line tests ---

#[test]
fn noisy_line_file_lock() {
    assert!(is_noisy_line(
        "    Blocking waiting for file lock on build directory"
    ));
}

#[test]
fn noisy_line_gnuplot() {
    assert!(is_noisy_line("Gnuplot not found, using plotters backend"));
}

#[test]
fn noisy_line_bench_profile() {
    assert!(is_noisy_line(
        "   Compiling foo v0.1.0 `bench` profile [optimized]"
    ));
}

#[test]
fn noisy_line_normal_output() {
    assert!(!is_noisy_line("time:   [94.573 ns 94.599 ns 94.630 ns]"));
}

// --- strip_bench_prefix tests ---

#[test]
fn strip_prefix_at_start() {
    let result = strip_bench_prefix("fib 10: Benchmarking", "fib 10");
    // The bench name at the start is replaced with spaces
    assert!(result.starts_with("      "));
    assert!(result.contains("Benchmarking"));
}

#[test]
fn strip_prefix_not_present() {
    let result = strip_bench_prefix("time: [94ns 95ns 96ns]", "fib 10");
    assert_eq!(result, "time: [94ns 95ns 96ns]");
}

#[test]
fn strip_prefix_empty_after_strip() {
    let result = strip_bench_prefix("fib 10", "fib 10");
    assert_eq!(result, "");
}

// --- Reporter trait tests ---

#[test]
fn noop_reporter_implements_trait() {
    let reporter: &dyn Reporter = &NoopReporter;
    // All methods are no-ops; just verify they don't panic
    reporter.on_build_start();
    reporter.on_build_output("Compiling foo v0.1.0");
    reporter.on_build_complete(&["Compiling foo v0.1.0".to_string()]);
    reporter.on_build_failed(&["error[E0308]: mismatched types".to_string()]);
    reporter.on_run_start(&["bench1".to_string()], 2);
    reporter.on_benchmark_started("bench1");
    reporter.on_output_line("bench1", "some output");
    reporter.on_benchmark_complete("bench1", &["line1".to_string()]);
    reporter.on_run_complete();
}

#[test]
fn custom_reporter_receives_events() {
    struct TestReporter {
        events: Mutex<Vec<String>>,
    }

    impl Reporter for TestReporter {
        fn on_run_start(&self, benchmarks: &[String], jobs: usize) {
            self.events
                .lock()
                .unwrap()
                .push(format!("start:{}:{}", benchmarks.len(), jobs));
        }
        fn on_benchmark_started(&self, benchmark: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("started:{benchmark}"));
        }
        fn on_output_line(&self, benchmark: &str, line: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("output:{benchmark}:{line}"));
        }
        fn on_benchmark_complete(&self, benchmark: &str, _output_lines: &[String]) {
            self.events
                .lock()
                .unwrap()
                .push(format!("complete:{benchmark}"));
        }
        fn on_benchmark_failed(
            &self,
            benchmark: &str,
            _output_lines: &[String],
            error: &anyhow::Error,
        ) {
            self.events
                .lock()
                .unwrap()
                .push(format!("failed:{benchmark}:{error}"));
        }
        fn on_run_complete(&self) {
            self.events.lock().unwrap().push("done".to_string());
        }
    }

    let reporter = TestReporter {
        events: Mutex::new(Vec::new()),
    };

    reporter.on_run_start(&["a".to_string(), "b".to_string()], 4);
    reporter.on_benchmark_started("a");
    reporter.on_output_line("a", "hello");
    reporter.on_benchmark_complete("a", &["hello".to_string()]);
    reporter.on_run_complete();

    let events = reporter.events.lock().unwrap();
    assert_eq!(
        *events,
        vec![
            "start:2:4",
            "started:a",
            "output:a:hello",
            "complete:a",
            "done"
        ]
    );
}

// --- ProgressReporter config tests ---

#[test]
fn progress_reporter_builder_methods_chainable() {
    let _reporter = ProgressReporter::new()
        .build(OutputMode::SILENT)
        .benchmarks(OutputMode::SILENT);
}

// --- Build hook tests ---

#[test]
fn custom_reporter_full_lifecycle() {
    struct LifecycleReporter {
        events: Mutex<Vec<String>>,
    }

    impl Reporter for LifecycleReporter {
        fn on_build_start(&self) {
            self.events.lock().unwrap().push("build_start".to_string());
        }
        fn on_build_output(&self, line: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_output:{line}"));
        }
        fn on_build_complete(&self, output_lines: &[String]) {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_complete:{}", output_lines.len()));
        }
        fn on_run_start(&self, benchmarks: &[String], jobs: usize) {
            self.events
                .lock()
                .unwrap()
                .push(format!("run_start:{}:{}", benchmarks.len(), jobs));
        }
        fn on_benchmark_started(&self, benchmark: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("started:{benchmark}"));
        }
        fn on_benchmark_complete(&self, benchmark: &str, _output_lines: &[String]) {
            self.events
                .lock()
                .unwrap()
                .push(format!("complete:{benchmark}"));
        }
        fn on_run_complete(&self) {
            self.events.lock().unwrap().push("run_complete".to_string());
        }
    }

    let reporter = LifecycleReporter {
        events: Mutex::new(Vec::new()),
    };

    reporter.on_build_start();
    reporter.on_build_output("Compiling foo");
    reporter.on_build_complete(&["Compiling foo".to_string()]);
    reporter.on_run_start(&["bench_a".to_string()], 2);
    reporter.on_benchmark_started("bench_a");
    reporter.on_benchmark_complete("bench_a", &[]);
    reporter.on_run_complete();

    let events = reporter.events.lock().unwrap();
    assert_eq!(
        *events,
        vec![
            "build_start",
            "build_output:Compiling foo",
            "build_complete:1",
            "run_start:1:2",
            "started:bench_a",
            "complete:bench_a",
            "run_complete",
        ]
    );
}
