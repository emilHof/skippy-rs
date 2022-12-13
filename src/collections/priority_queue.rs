use crate::skiplist;
use crate::sync_skiplist;
use std::borrow::Borrow;
use std::marker::PhantomData;

pub struct PriorityQueue<L, V> {
    queue: L,
    _phantom: PhantomData<V>,
}

impl<V> PriorityQueue<sync_skiplist::SkipList<V, ()>, V> {
    pub fn new() -> PriorityQueue<sync_skiplist::SkipList<V, ()>, V> {
        PriorityQueue {
            queue: sync_skiplist::SkipList::new(),
            _phantom: PhantomData,
        }
    }
}

impl<L, V> PriorityQueue<L, V>
where
    L: skiplist::SkipList<V, ()> + Send + Sync,
{
    pub fn new_shared() -> Self {
        PriorityQueue {
            queue: L::new(),
            _phantom: PhantomData,
        }
    }
}

impl<'a, L, V> PriorityQueue<L, V>
where
    L: skiplist::SkipList<V, ()> + 'a,
    V: Ord + 'a,
    L::Entry<'a>: Borrow<V>,
{
    pub fn push(&mut self, value: V) {
        self.queue.insert(value, ());
    }

    pub fn peek(&'a self) -> Option<L::Entry<'a>> {
        self.queue.front()
    }

    pub fn pop(&mut self) -> Option<V> {
        match self.queue.front() {
            Some(e) => self.queue.remove(e.borrow()).map(|(v, _)| v),
            None => None,
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod pq_test {
    use std::collections::BinaryHeap;

    use super::*;

    #[test]
    fn test_push() {
        let mut queue = PriorityQueue::new();
        let mut rng: u16 = rand::random();

        for _ in 0..10_000 {
            rng ^= rng << 6;
            rng ^= rng >> 9;
            rng ^= rng << 3;

            queue.push(rng)
        }
    }

    #[test]
    fn test_pop() {
        let mut queue = PriorityQueue::new();
        let mut rng: u16 = rand::random();

        for _ in 0..10_000 {
            rng ^= rng << 6;
            rng ^= rng >> 9;
            rng ^= rng << 3;

            queue.push(rng)
        }

        for _ in 0..10_000 {
            queue.pop();
        }

        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_push_pop() {
        let n = 1_000;
        let mut seed: u32 = rand::random();
        let mut queue = PriorityQueue::new();

        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            match seed % 5 {
                0 => {
                    queue.pop();
                }
                _ => {
                    queue.push(seed);
                }
            }
        }

        assert!(queue.len() > 0);
    }

    #[test]
    fn test_with_std() {
        let n = 100_000;
        let mut seed: u8 = rand::random();
        let mut queue = PriorityQueue::new();
        let mut sq = BinaryHeap::new();

        for _ in 0..n {
            seed ^= seed << 4;
            seed ^= seed >> 3;
            seed ^= seed << 5;

            match seed % 5 {
                0 => {
                    assert_eq!(sq.pop(), queue.pop());
                }
                _ => {
                    sq.push(seed);
                    queue.push(seed);
                }
            }
        }
    }
}
