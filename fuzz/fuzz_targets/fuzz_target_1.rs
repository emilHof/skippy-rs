#![no_main]

use afl;
use libfuzzer_sys::fuzz_target;
use rand::Rng;
use skippy_rs::SyncSkipList;
use std::sync::Arc;

const fn randomize(mut seed: usize) -> usize {
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^ seed << 5
}

fuzz_target!(|seeds: Vec<usize>| {
    let list = Arc::new(SyncSkipList::new());

    let threads = seeds
        .into_iter()
        .map(|mut seed| {
            let list = list.clone();
            unsafe {
                std::thread::spawn(move || {
                    let mut seed = seed;
                    for _ in 0..5_000 {
                        seed = randomize(seed);

                        if seed % 5 == 0 {
                            seed = randomize(seed);
                            list.remove(&(*(&seed as *const usize as *const u8)));
                        } else {
                            seed = randomize(seed);
                            list.insert(*(&seed as *const usize as *const u8), ());
                        }
                    }
                })
            }
        })
        .collect::<Vec<_>>();

    for thread in threads {
        thread.join().unwrap()
    }
});
