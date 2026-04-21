use criterion::{Criterion, black_box, criterion_group, criterion_main};
use roko_primitives::HdcVector;

fn bench_bind(c: &mut Criterion) {
    let a = HdcVector::from_seed(b"bind-left");
    let b = HdcVector::from_seed(b"bind-right");
    c.bench_function("hdc_bind", |bencher| {
        bencher.iter(|| black_box(a.bind(&b)));
    });
}

fn bench_bundle_3(c: &mut Criterion) {
    let vectors: Vec<HdcVector> = (0..3)
        .map(|i| HdcVector::from_seed(format!("bundle-{i}").as_bytes()))
        .collect();
    let refs: Vec<&HdcVector> = vectors.iter().collect();
    c.bench_function("hdc_bundle_3", |bencher| {
        bencher.iter(|| black_box(HdcVector::bundle(&refs)));
    });
}

fn bench_bundle_16(c: &mut Criterion) {
    let vectors: Vec<HdcVector> = (0..16)
        .map(|i| HdcVector::from_seed(format!("bundle-{i}").as_bytes()))
        .collect();
    let refs: Vec<&HdcVector> = vectors.iter().collect();
    c.bench_function("hdc_bundle_16", |bencher| {
        bencher.iter(|| black_box(HdcVector::bundle(&refs)));
    });
}

fn bench_similarity(c: &mut Criterion) {
    let a = HdcVector::from_seed(b"similarity-a");
    let b = HdcVector::from_seed(b"similarity-b");
    c.bench_function("hdc_similarity", |bencher| {
        bencher.iter(|| black_box(a.similarity(&b)));
    });
}

fn bench_from_seed(c: &mut Criterion) {
    c.bench_function("hdc_from_seed", |bencher| {
        bencher.iter(|| black_box(HdcVector::from_seed(b"benchmark-seed-input")));
    });
}

fn bench_permute(c: &mut Criterion) {
    let v = HdcVector::from_seed(b"permute-input");
    c.bench_function("hdc_permute", |bencher| {
        bencher.iter(|| black_box(v.permute(42)));
    });
}

fn bench_bytes_roundtrip(c: &mut Criterion) {
    let v = HdcVector::from_seed(b"bytes-roundtrip");
    c.bench_function("hdc_bytes_roundtrip", |bencher| {
        bencher.iter(|| {
            let bytes = v.to_bytes();
            black_box(HdcVector::from_bytes(&bytes))
        });
    });
}

criterion_group!(
    benches,
    bench_bind,
    bench_bundle_3,
    bench_bundle_16,
    bench_similarity,
    bench_from_seed,
    bench_permute,
    bench_bytes_roundtrip,
);
criterion_main!(benches);
