use criterion::{Criterion, black_box, criterion_group, criterion_main};
use roko_core::{Body, ContentHash, Engram, Kind, Score};

fn bench_engram_build(c: &mut Criterion) {
    c.bench_function("engram_build", |bencher| {
        bencher.iter(|| {
            black_box(
                Engram::builder(Kind::Task)
                    .body(Body::text("implement the login feature"))
                    .score(Score::new(0.9, 0.5, 1.0, 1.5))
                    .created_at_ms(1_000_000)
                    .tag("plan_id", "plan-42")
                    .tag("priority", "high")
                    .build(),
            )
        });
    });
}

fn bench_engram_build_minimal(c: &mut Criterion) {
    c.bench_function("engram_build_minimal", |bencher| {
        bencher.iter(|| {
            black_box(
                Engram::builder(Kind::Episode)
                    .created_at_ms(1_000_000)
                    .build(),
            )
        });
    });
}

fn bench_content_hash_of(c: &mut Criterion) {
    let data = b"benchmark content hash computation over this payload";
    c.bench_function("content_hash_of", |bencher| {
        bencher.iter(|| black_box(ContentHash::of(data)));
    });
}

fn bench_content_hash_large(c: &mut Criterion) {
    let data = vec![0xABu8; 4096];
    c.bench_function("content_hash_of_4k", |bencher| {
        bencher.iter(|| black_box(ContentHash::of(&data)));
    });
}

fn bench_engram_content_hash(c: &mut Criterion) {
    let engram = Engram::builder(Kind::Task)
        .body(Body::text(
            "implement the login feature with OAuth2 support",
        ))
        .score(Score::new(0.9, 0.5, 1.0, 1.5))
        .created_at_ms(1_000_000)
        .tag("plan_id", "plan-42")
        .tag("priority", "high")
        .tag("agent", "claude-opus")
        .build();
    c.bench_function("engram_content_hash", |bencher| {
        bencher.iter(|| black_box(engram.content_hash()));
    });
}

fn bench_score_effective(c: &mut Criterion) {
    let score = Score::new_extended(0.85, 0.6, 2.0, 1.5, 0.7, 0.8, 0.9);
    c.bench_function("score_effective", |bencher| {
        bencher.iter(|| black_box(score.effective()));
    });
}

fn bench_engram_serde_roundtrip(c: &mut Criterion) {
    let engram = Engram::builder(Kind::Task)
        .body(Body::text("serde benchmark payload"))
        .score(Score::new(0.9, 0.5, 1.0, 1.5))
        .created_at_ms(1_000_000)
        .tag("plan_id", "plan-42")
        .build();
    c.bench_function("engram_serde_roundtrip", |bencher| {
        bencher.iter(|| {
            let json = serde_json::to_string(&engram).expect("serialize");
            black_box(serde_json::from_str::<Engram>(&json).expect("deserialize"))
        });
    });
}

criterion_group!(
    benches,
    bench_engram_build,
    bench_engram_build_minimal,
    bench_content_hash_of,
    bench_content_hash_large,
    bench_engram_content_hash,
    bench_score_effective,
    bench_engram_serde_roundtrip,
);
criterion_main!(benches);
