use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn bench_fib(c: &mut Criterion) {
    c.bench_function("fib 10", |b| b.iter(|| fibonacci(black_box(10))));
    c.bench_function("fib 20", |b| b.iter(|| fibonacci(black_box(20))));
    c.bench_function("fib 25", |b| b.iter(|| fibonacci(black_box(25))));
}

criterion_group!(benches, bench_fib);
criterion_main!(benches);
