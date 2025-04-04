use std::collections::HashMap;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nohash_hasher::BuildNoHashHasher;
use overlaymap::OverlayMap;

type Hasher = BuildNoHashHasher<u64>;

fn overlaymap(c: &mut Criterion) {
    let mut g = c.benchmark_group("overlaymap");

    g.bench_function("get", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = OverlayMap::<u64, u64, Hasher>::new();
                map.insert(key, key);
                (map, key)
            },
            |(map, key)| {
                black_box(map.get(black_box(&key)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("new_insert", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let map = OverlayMap::<u64, u64, Hasher>::new();
                (map, key)
            },
            |(mut map, key)| {
                black_box(map.insert(black_box(key), black_box(key)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("swap_insert", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = OverlayMap::<u64, u64, Hasher>::new();
                map.insert(key, key);
                (map, key)
            },
            |(mut map, key)| {
                black_box(map.insert(black_box(key), black_box(key + 1)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("extend_swap", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = OverlayMap::<u64, u64, Hasher>::new();
                map.insert(key, key);
                let mut other = HashMap::<u64, u64, Hasher>::with_hasher(Hasher::default());
                other.insert(key, key);
                (map, other)
            },
            |(mut map, other)| {
                black_box(map.extend(black_box(other)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("try_swap", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = OverlayMap::<u64, u64, Hasher>::new();
                map.insert(key, key);
                (map, key)
            },
            |(mut map, key)| {
                black_box(map.try_swap(black_box(&key), black_box(|old: &u64| Some(old + 1))));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.finish();
}

fn baseline(c: &mut Criterion) {
    let mut g = c.benchmark_group("baseline");

    g.bench_function("get", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = HashMap::<u64, u64, Hasher>::with_hasher(Hasher::default());
                map.insert(key, key);
                (map, key)
            },
            |(map, key)| {
                black_box(map.get(black_box(&key)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("new_insert", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let map = HashMap::<u64, u64, Hasher>::with_hasher(Hasher::default());
                (map, key)
            },
            |(mut map, key)| {
                black_box(map.insert(black_box(key), black_box(key)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("swap_insert", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = HashMap::<u64, u64, Hasher>::with_hasher(Hasher::default());
                map.insert(key, key);
                (map, key)
            },
            |(mut map, key)| {
                black_box(map.insert(black_box(key), black_box(key + 1)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.bench_function("extend_swap", |b| {
        let mut i = 0;
        b.iter_batched(
            || {
                let key = i;
                i += 1;
                let mut map = HashMap::<u64, u64, Hasher>::with_hasher(Hasher::default());
                map.insert(key, key);
                let mut other = HashMap::<u64, u64, Hasher>::with_hasher(Hasher::default());
                other.insert(key, key);
                (map, other)
            },
            |(mut map, other)| {
                black_box(map.extend(black_box(other)));
            },
            criterion::BatchSize::SmallInput,
        );
    });

    g.finish();
}

criterion_group!(benches, overlaymap, baseline);
criterion_main!(benches);
