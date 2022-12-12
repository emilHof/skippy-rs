#![feature(test)]
use crossbeam_skiplist::SkipSet;
use skippy::priority_queue::PriorityQueue;
use std::collections::BinaryHeap;

extern crate test;

use test::Bencher;

#[bench]
fn bench_push_skippy(b: &mut Bencher) {
    let n = test::black_box(1_000);
    let mut seed: u32 = rand::random();
    let mut queue = PriorityQueue::new();

    b.iter(|| {
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            queue.push(seed);
        }
    });

    println!("skippy len: {}", queue.len());
}

#[bench]
fn bench_push_std(b: &mut Bencher) {
    let n = test::black_box(1_000);
    let mut seed: u32 = rand::random();
    let mut queue = BinaryHeap::new();

    b.iter(|| {
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            queue.push(seed);
        }
    });

    println!("std len: {}", queue.len());
}

#[bench]
fn bench_push_crossbeam(b: &mut Bencher) {
    let n = test::black_box(1_000);
    let mut seed: u32 = rand::random();
    let queue = SkipSet::new();

    b.iter(|| {
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            queue.insert(seed);
        }
    });

    println!("cb len: {}", queue.len());
}

#[bench]
fn bench_push_pop_skippy(b: &mut Bencher) {
    let n = test::black_box(100_000);
    let mut seed: u32 = rand::random();
    let mut queue = PriorityQueue::new();

    b.iter(|| {
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            match seed % 5 {
                0 => {
                    queue.push(seed);
                }
                _ => {
                    queue.pop();
                }
            }
        }
    });

    println!("skippy len: {}", queue.len());
}

#[bench]
fn bench_push_pop_std(b: &mut Bencher) {
    let n = test::black_box(100_000);
    let mut seed: u32 = rand::random();
    let mut queue = BinaryHeap::new();

    b.iter(|| {
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            match seed % 5 {
                0 => {
                    queue.push(seed);
                }
                _ => {
                    queue.pop();
                }
            }
        }
    });

    println!("std len: {}", queue.len());
}

#[bench]
fn bench_push_pop_crossbeam(b: &mut Bencher) {
    let n = test::black_box(100_000);
    let mut seed: u32 = rand::random();
    let queue = SkipSet::new();

    b.iter(|| {
        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            match seed % 5 {
                0 => {
                    queue.insert(seed);
                }
                _ => {
                    queue.pop_front();
                }
            }
        }
    });

    println!("cb len: {}", queue.len());
}
