#![feature(test)]
use crossbeam_skiplist::SkipMap;
use skippy::sync_skiplist::SkipList;

extern crate test;

use test::Bencher;

#[bench]
fn skippy_insert(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();

    b.iter(|| {
        let mut list = SkipList::new();

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(seed, "Hello There!");
        }
    })
}

#[bench]
fn crossbeam_insert(b: &mut Bencher) {
    let upper = test::black_box(1_000);
    let mut seed: u16 = rand::random();

    b.iter(|| {
        let list = SkipMap::new();

        for _ in 0..upper {
            seed ^= seed << 6;
            seed ^= seed >> 11;
            seed ^= seed << 5;
            list.insert(seed, "Hello There!");
        }
    })
}
