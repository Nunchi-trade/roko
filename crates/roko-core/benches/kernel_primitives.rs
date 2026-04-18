use std::hint::black_box;
use std::time::Instant;

use roko_core::{Body, Decay, Engram, Kind, Score};

fn bench_score_effective(iterations: u64) -> u128 {
    let score = Score::new_extended(0.82, 0.35, 1.4, 1.1, 0.7, 0.6, 0.75);
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(score).effective();
    }
    start.elapsed().as_nanos()
}

fn bench_decay_apply(iterations: u64) -> u128 {
    let decay = Decay::Ebbinghaus {
        strength: 1.2,
        scale_ms: 86_400_000,
    };
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(decay).apply(black_box(3_600_000));
    }
    start.elapsed().as_nanos()
}

fn bench_engram_content_hash(iterations: u64) -> u128 {
    let engram = Engram::builder(Kind::Compound(vec![Kind::Task, Kind::Prompt]))
        .body(Body::text("benchmark payload"))
        .tag("scope", "parity")
        .build();
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(&engram).content_hash();
    }
    start.elapsed().as_nanos()
}

fn main() {
    let iterations = 10_000;
    let score_ns = bench_score_effective(iterations);
    let decay_ns = bench_decay_apply(iterations);
    let hash_ns = bench_engram_content_hash(iterations);

    println!("score_effective_ns={score_ns}");
    println!("decay_apply_ns={decay_ns}");
    println!("engram_content_hash_ns={hash_ns}");
}
