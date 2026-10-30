// Copyright (C) 2026 Jeffrey Stedfast. MIT; see LICENSE.

use std::time::{Duration, Instant};

pub fn measure(name: &str, bytes: usize, mut run: impl FnMut()) {
    for _ in 0..3 {
        run();
    }
    let mut samples = [0.0f64; 5];
    for sample in &mut samples {
        let start = Instant::now();
        let mut iterations = 0;
        while start.elapsed() < Duration::from_millis(100) {
            run();
            iterations += 1;
        }
        *sample = iterations as f64 / start.elapsed().as_secs_f64();
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:65} {:10.1} MiB/s {:12.1} operations/s",
        samples[2] * bytes as f64 / 1048576.0,
        samples[2],
    );
}

pub fn heading() {
    println!(
        "{}-{}, SIMD feature: {}; median of 5 warmed-up samples >= 100 ms",
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(feature = "simd"),
    );
    println!("Fixture loading, generation, and correctness assertions are outside timing.");
}
