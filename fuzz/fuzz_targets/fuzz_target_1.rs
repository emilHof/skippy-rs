#![no_main]

use libfuzzer_sys::fuzz_target;
use rand::Rng;
use skippy_rs::SyncSkipList;
use std::sync::Arc;

fuzz_target!(|data: &[u8]| {
    let list = Arc::new(SyncSkipList::new());

    let threads = (0..20)
        .map(|_| {
            let list = list.clone();
            std::thread::spawn(move || {
                let mut rng = rand::thread_rng();
                for _ in 0..5_000 {
                    let target = rng.gen::<u8>();
                    if rng.gen::<u8>() % 5 == 0 {
                        list.remove(&target);
                    } else {
                        list.insert(target, ());
                    }
                }
            })
        })
        .collect::<Vec<_>>();

    for thread in threads {
        thread.join().unwrap()
    }
});
