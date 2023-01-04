#![feature(test)]
use crossbeam_skiplist::SkipMap;
use skippy::{SSkipList, SkipList};
use std::sync::{atomic::AtomicUsize, Arc};

extern crate test;

use test::Bencher;

struct CountOnCmp<K> {
    key: K,
    counter: Arc<AtomicUsize>,
}

impl<K> PartialEq for CountOnCmp<K>
where
    K: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.key.eq(&other.key)
    }
}

impl<K> Eq for CountOnCmp<K> where K: Eq {}

impl<K> PartialOrd for CountOnCmp<K>
where
    K: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(self.key.cmp(&other.key))
    }
}

impl<K> Ord for CountOnCmp<K>
where
    K: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.key.cmp(&other.key)
    }
}

#[bench]
fn insert_skippy(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        let mut list = SkipList::new();

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(
                CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                },
                "Hello There!",
            );
        }
    });

    println!(
        "cmp count for insert skippy: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn insert_sync_skippy(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        let list = SSkipList::new();

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(
                CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                },
                "Hello There!",
            );
        }
    });

    println!(
        "cmp count for insert skippy: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn insert_crossbeam(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        let list = SkipMap::new();

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(
                CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                },
                "Hello There!",
            );
        }
    });

    println!(
        "cmp count for insert crossbeam: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn get_skippy(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let mut list: SkipList<CountOnCmp<u16>, u8> = SkipList::new();

    let counter = Arc::new(AtomicUsize::new(0));

    for _ in 0..upper {
        seed ^= seed << 6;
        seed ^= seed >> 11;
        seed ^= seed << 5;
        list.insert(
            CountOnCmp {
                key: seed,
                counter: counter.clone(),
            },
            0,
        );
    }

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.get(&CountOnCmp {
                key: seed,
                counter: counter.clone(),
            });
        }
    });

    println!(
        "cmp count for get skippy: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn get_skippy_sync(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let list = SSkipList::new();

    let counter = Arc::new(AtomicUsize::new(0));

    for _ in 0..upper {
        seed ^= seed << 6;
        seed ^= seed >> 11;
        seed ^= seed << 5;
        list.insert(
            CountOnCmp {
                key: seed,
                counter: counter.clone(),
            },
            0,
        );
    }

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.get(&CountOnCmp {
                key: seed,
                counter: counter.clone(),
            });
        }
    });

    println!(
        "cmp count for get skippy_sync: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn get_crossbeam(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let list = SkipMap::new();

    let counter = Arc::new(AtomicUsize::new(0));

    for _ in 0..upper {
        seed ^= seed << 6;
        seed ^= seed >> 11;
        seed ^= seed << 5;
        list.insert(
            CountOnCmp {
                key: seed,
                counter: counter.clone(),
            },
            "Hello There!",
        );
    }

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.get(&CountOnCmp {
                key: seed,
                counter: counter.clone(),
            });
        }
    });

    println!(
        "cmp count for get crossbeam: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn remove_skippy(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let mut list: SkipList<CountOnCmp<u16>, u8> = SkipList::new();

    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(
                CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                },
                0,
            );
        }

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.remove(&CountOnCmp {
                key: seed,
                counter: counter.clone(),
            });
        }
    });

    println!(
        "cmp count for remove skippy: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn remove_skippy_sync(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let list = SSkipList::new();

    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(
                CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                },
                0,
            );
        }

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.remove(&CountOnCmp {
                key: seed,
                counter: counter.clone(),
            });
        }
    });

    println!(
        "cmp count for remove skippy_sync: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn remove_crossbeam(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let list = SkipMap::new();

    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(
                CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                },
                "Hello There!",
            );
        }

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.remove(&CountOnCmp {
                key: seed,
                counter: counter.clone(),
            });
        }
    });

    println!(
        "cmp count for remove crossbeam: {}m",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000
    );
}

#[bench]
fn inmove_skippy(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let mut seed2: u8 = rand::random();
    let mut list: SkipList<CountOnCmp<u16>, u8> = SkipList::new();

    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            seed2 ^= seed2 << 3;
            seed2 ^= seed2 >> 5;
            seed2 ^= seed2 << 2;
            if seed2 % 5 == 0 {
                list.remove(&CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                });
            } else {
                list.insert(
                    CountOnCmp {
                        key: seed,
                        counter: counter.clone(),
                    },
                    0,
                );
            }
        }
    });

    println!(
        "cmp count for inmove skippy: {}m; len: {}",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000,
        list.len()
    );
}

#[bench]
fn inmove_skippy_sync(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let mut seed2: u8 = rand::random();
    let list = SSkipList::new();

    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            seed2 ^= seed2 << 3;
            seed2 ^= seed2 >> 5;
            seed2 ^= seed2 << 2;
            if seed2 % 5 == 0 {
                list.remove(&CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                });
            } else {
                list.insert(
                    CountOnCmp {
                        key: seed,
                        counter: counter.clone(),
                    },
                    0,
                );
            }
        }
    });

    println!(
        "cmp count for inmove skippy_sync: {}m; len: {}",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000,
        list.len()
    );
}

#[bench]
fn inmove_crossbeam(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();
    let mut seed2: u8 = rand::random();
    let list = SkipMap::new();

    let counter = Arc::new(AtomicUsize::new(0));

    b.iter(|| {
        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            seed2 ^= seed2 << 3;
            seed2 ^= seed2 >> 5;
            seed2 ^= seed2 << 2;
            if seed2 % 5 == 0 {
                list.remove(&CountOnCmp {
                    key: seed,
                    counter: counter.clone(),
                });
            } else {
                list.insert(
                    CountOnCmp {
                        key: seed,
                        counter: counter.clone(),
                    },
                    0,
                );
            }
        }
    });

    println!(
        "cmp count for inmove crossbeam: {}m; len: {}",
        counter.load(std::sync::atomic::Ordering::Acquire) / 1_000_000,
        list.len()
    );
}
