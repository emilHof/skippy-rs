use crate::internal::skiplist;
use crate::internal::sync;
use crate::skiplist::Entry;
use std::marker::PhantomData;

/// [PriorityQueue](PriorityQueue) is implemented using a [SkipList](crate::skiplist::SkipList) and is available as both
/// a non-thread safe, but faster, and a thread-safe, yet slower, variation.
pub struct PriorityQueue<V, L> {
    queue: L,
    _phantom: PhantomData<V>,
}

impl<'domain, V> PriorityQueue<V, ()>
where
    V: Sync,
{
    pub fn new() -> PriorityQueue<V, skiplist::SkipList<'domain, V, ()>> {
        PriorityQueue {
            queue: skiplist::SkipList::new(),
            _phantom: PhantomData,
        }
    }
    pub fn new_sync() -> PriorityQueue<V, sync::SkipList<'domain, V, ()>> {
        PriorityQueue {
            queue: sync::SkipList::new(),
            _phantom: PhantomData,
        }
    }
}

unsafe impl<V, L> Send for PriorityQueue<V, L>
where
    V: Send + Sync,
    L: Send + Sync,
{
}

unsafe impl<V, L> Sync for PriorityQueue<V, L>
where
    V: Send + Sync,
    L: Send + Sync,
{
}

impl<'a, V, L> PriorityQueue<V, L>
where
    L: crate::skiplist::SkipList<V, ()> + 'a,
    V: Ord + 'a,
{
    pub fn push(&self, value: V) {
        self.queue.insert(value, ());
    }

    pub fn peek(&'a self) -> Option<L::Entry<'a>> {
        self.queue.front()
    }

    pub fn pop(&'a self) -> Option<V> {
        match self.queue.front() {
            Some(e) => self.queue.remove(e.key()).map(|(v, _)| v),
            None => None,
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod pq_test {
    use std::collections::BinaryHeap;

    use super::*;

    #[test]
    fn test_push() {
        let queue = PriorityQueue::new();
        let mut rng: u16 = rand::random();

        for _ in 0..10_000 {
            rng ^= rng << 3;

            queue.push(rng)
        }
    }

    #[test]
    fn test_pop() {
        let queue = PriorityQueue::new();
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
        let queue = PriorityQueue::new();

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
        let queue = PriorityQueue::new();
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

    #[test]
    fn test_sync_push() {
        let n = 1_000;
        let mut seed: u32 = rand::random();
        let queue = PriorityQueue::new_sync();

        for _ in 0..n {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 7;

            queue.push(seed);
        }

        assert!(queue.len() > 0);
    }
}
