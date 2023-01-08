use crate::internal::skiplist;
use crate::internal::skiplist::SkipList;
use crate::internal::sync;
use crate::internal::sync::SkipList as SyncSkipList;

/// [PriorityQueue](PriorityQueue) is implemented using a [SkipList](crate::skiplist::SkipList) and is available as both
/// a non-thread safe, but faster, and a thread-safe, yet slower, variation.
pub struct PriorityQueue<L> {
    queue: L,
}

impl<'domain> PriorityQueue<()> {
    pub fn new<V: Sync>() -> PriorityQueue<SkipList<'domain, V, ()>> {
        PriorityQueue {
            queue: SkipList::new(),
        }
    }
    pub fn new_sync<V: Sync>() -> PriorityQueue<SyncSkipList<'domain, V, ()>> {
        PriorityQueue {
            queue: SyncSkipList::new(),
        }
    }
}

unsafe impl<L> Send for PriorityQueue<L> where L: Send + Sync {}

unsafe impl<L> Sync for PriorityQueue<L> where L: Send + Sync {}

impl<'a, V> PriorityQueue<SkipList<'a, V, ()>>
where
    V: Ord,
{
    pub fn push(&mut self, value: V) {
        self.queue.insert(value, ());
    }

    pub fn peek(&'a self) -> Option<&V> {
        self.queue.get_first()?.key().into()
    }

    pub fn pop(&mut self) -> Option<V> {
        self.queue.remove_first().map(|(v, ..)| v)
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl<'a, V> PriorityQueue<SyncSkipList<'a, V, ()>>
where
    V: Ord + Send + Sync + 'a,
{
    pub fn push(&self, value: V) {
        self.queue.insert(value, ());
    }

    pub fn peek(&'a self) -> Option<sync::Entry<'a, V, ()>> {
        self.queue.get_first()
    }

    pub fn pop(&'a self) -> Option<sync::Entry<'a, V, ()>> {
        let first = self.queue.get_first()?;

        first.remove()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

mod iter {
    use super::*;

    impl<'a, V: Ord> PriorityQueue<SkipList<'a, V, ()>> {
        pub fn iter(&'a self) -> skiplist::iter::Iter<'a, V, ()> {
            self.queue.iter()
        }

        pub fn iter_mut(&'a mut self) -> skiplist::iter::IterMut<'a, V, ()> {
            self.queue.iter_mut()
        }
    }

    impl<'a, V> IntoIterator for PriorityQueue<SkipList<'a, V, ()>>
    where
        V: Ord,
    {
        type Item = <SkipList<'a, V, ()> as IntoIterator>::Item;
        type IntoIter = <SkipList<'a, V, ()> as IntoIterator>::IntoIter;

        fn into_iter(self) -> Self::IntoIter {
            self.queue.into_iter()
        }
    }

    impl<'a, V> PriorityQueue<SyncSkipList<'a, V, ()>>
    where
        V: Ord + Send + Sync,
    {
        pub fn iter(&'a self) -> sync::iter::Iter<'a, V, ()> {
            self.queue.iter()
        }
    }

    impl<'a, V> IntoIterator for PriorityQueue<SyncSkipList<'a, V, ()>>
    where
        V: Ord + Send + Sync,
    {
        type Item = <SyncSkipList<'a, V, ()> as IntoIterator>::Item;
        type IntoIter = <SyncSkipList<'a, V, ()> as IntoIterator>::IntoIter;

        fn into_iter(self) -> Self::IntoIter {
            self.queue.into_iter()
        }
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
        use std::cmp::Reverse;
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
                    assert_eq!(sq.pop().map(|r: Reverse<u8>| r.0), queue.pop());
                }
                _ => {
                    sq.push(Reverse(seed));
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
