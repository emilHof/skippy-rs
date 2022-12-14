use core::{
    borrow::Borrow,
    fmt::{Debug, Display},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::internal::utils::{skiplist_basics, GeneratesHeight, Head, Levels, Node, HEIGHT};

skiplist_basics!(SkipList);

impl<K, V> SkipList<K, V>
where
    K: Ord,
{
    /// Inserts a value in the list given a key.
    pub fn insert(&self, key: K, mut val: V) -> Option<V> {
        // After this check, whether we are holding the head or a regular Node will
        // not impact the operation.
        unsafe {
            let insertion_point = self.find(&key);

            match insertion_point {
                SearchResult {
                    target: Some(target),
                    ..
                } => {
                    std::mem::swap(&mut (*target.as_ptr()).val, &mut val);
                    Some(val)
                }
                SearchResult { prev, .. } => {
                    let new_node = Node::new_rand_height(key, val, self);

                    Self::link_nodes(new_node, prev);

                    self.state.len.fetch_add(1, Ordering::Relaxed);

                    None
                }
            }
        }
    }

    /// This function is unsafe, as it does not check whether new_node or link node are valid
    /// pointers.
    /// To call this function safely:
    /// - new_node cannot be null
    /// - link_node cannot be null
    /// - no pointer tower along the path can have a null pointer pointing backwards
    /// - a tower of sufficient height must eventually be reached, the list head can be this tower
    unsafe fn link_nodes(new_node: *mut Node<K, V>, prev: [&Levels<K, V>; HEIGHT]) {
        // iterate over all the levels in the new nodes pointer tower
        for (i, levels) in prev.iter().enumerate().take((*new_node).height) {
            // move backwards until a pointer tower of sufficient hight is reached
            (*new_node).levels[i][1].store(levels[i][1].load(Ordering::Relaxed), Ordering::Relaxed);
            levels[i][1].store(new_node, Ordering::Relaxed);
        }
    }

    pub fn remove(&self, key: &K) -> Option<(K, V)> {
        if self.is_empty() {
            return None;
        }

        unsafe {
            match self.find(key) {
                SearchResult {
                    target: Some(target),
                    prev,
                } => {
                    let target = target.as_ptr();
                    let key = core::ptr::read(&(*target).key);
                    let val = core::ptr::read(&(*target).val);

                    self.unlink(target, prev);
                    Node::<K, V>::dealloc(target);
                    self.state.len.fetch_sub(1, Ordering::Relaxed);

                    Some((key, val))
                }
                _ => None,
            }
        }
    }

    /// Logically removes the node from the list by linking its adjacent nodes to one-another.
    fn unlink(&self, node: *mut Node<K, V>, prev: [&Levels<K, V>; HEIGHT]) {
        // safety check against UB caused by unlinking the head
        if self.is_head(node) {
            panic!()
        }
        unsafe {
            for (i, levels) in prev.iter().enumerate().take((*node).height) {
                levels[i][1].store(
                    (*node).levels[i][1].load(Ordering::Relaxed),
                    Ordering::Relaxed,
                );
            }
        }
    }

    /// This method is `unsafe` as it may return the head typecast as a Node, which can
    /// cause UB if not handled appropriately. If the return value is Ok(...) then it is a
    /// regular Node. If it is Err(...) then it is the head.
    unsafe fn find<'a>(&'a self, key: &K) -> SearchResult<'a, K, V> {
        let mut level = self.state.max_height.load(Ordering::Relaxed);
        let head = unsafe { &(*self.head.as_ptr()) };

        let mut prev = [&head.levels; HEIGHT];

        // find the first and highest node tower
        while level > 1 && head.levels[level - 1][1].load(Ordering::Relaxed).is_null() {
            level -= 1;
        }

        let mut curr = self.head.as_ptr().cast::<Node<K, V>>();
        prev[level - 1] = &(*curr).levels;

        unsafe {
            while level > 0 {
                if (*curr).levels[level - 1][1]
                    .load(Ordering::Relaxed)
                    .is_null()
                    || (*(*curr).levels[level - 1][1].load(Ordering::Relaxed)).key >= *key
                {
                    prev[level - 1] = &(*curr).levels;
                    level -= 1;
                } else {
                    curr = (*curr).levels[level - 1][1].load(Ordering::Relaxed)
                }
            }
        }

        let next = (*curr).levels[level][1].load(Ordering::Relaxed);

        if !next.is_null() && &(*next).key == key {
            SearchResult {
                prev,
                target: unsafe { Some(NonNull::new_unchecked(next)) },
            }
        } else {
            SearchResult { prev, target: None }
        }
    }

    pub fn get<'a>(&self, key: &K) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        // Perform safety check for whether we are dealing with the head.
        unsafe {
            match self.find(key) {
                SearchResult {
                    target: Some(ptr), ..
                } => Some(Entry {
                    key: &ptr.as_ref().key,
                    val: &ptr.as_ref().val,
                }),
                _ => None,
            }
        }
    }

    fn is_head(&self, ptr: *const Node<K, V>) -> bool {
        std::ptr::eq(ptr, self.head.as_ptr().cast())
    }

    pub fn get_first<'a>(&self) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        let first = unsafe { (*self.head.as_ptr()).levels[0][1].load(Ordering::Relaxed) };

        unsafe {
            if !first.is_null() {
                let first = &(*first);
                return Some(Entry {
                    key: &first.key,
                    val: &first.val,
                });
            }
        }

        None
    }

    pub fn get_last<'a>(&self) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        let curr = unsafe { (*self.head.as_ptr()).levels[0][1].load(Ordering::Relaxed) };

        unsafe {
            if curr.is_null() {
                return None;
            }

            let mut curr = &(*curr);

            while !curr.levels[0][1].load(Ordering::Relaxed).is_null() {
                curr = &(*curr.levels[0][1].load(Ordering::Relaxed));
            }

            Some(Entry {
                key: &curr.key,
                val: &curr.val,
            })
        }
    }
}

impl<K, V> Default for SkipList<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> crate::skiplist::SkipList<K, V> for SkipList<K, V>
where
    K: Ord,
{
    type Entry<'a> = Entry<'a, K, V> where K: 'a, V: 'a;

    fn new() -> Self {
        SkipList::new()
    }

    fn insert(&self, key: K, value: V) -> Option<V> {
        self.insert(key, value)
    }

    fn remove(&self, key: &K) -> Option<(K, V)> {
        self.remove(key)
    }

    fn get<'a>(&self, key: &K) -> Option<Self::Entry<'a>> {
        self.get(key)
    }

    fn last<'a>(&self) -> Option<Self::Entry<'a>> {
        self.get_first()
    }

    fn front<'a>(&self) -> Option<Self::Entry<'a>> {
        self.get_last()
    }

    fn len(&self) -> usize {
        self.len()
    }
}

pub struct Entry<'a, K, V> {
    key: &'a K,
    val: &'a V,
}

impl<'a, K, V> Borrow<K> for Entry<'a, K, V> {
    fn borrow(&self) -> &K {
        self.key
    }
}

impl<'a, K, V> AsRef<V> for Entry<'a, K, V> {
    fn as_ref(&self) -> &V {
        self.val
    }
}

#[derive(Debug)]
pub struct SearchResult<'a, K, V> {
    prev: [&'a Levels<K, V>; HEIGHT],
    target: Option<NonNull<Node<K, V>>>,
}

impl<K, V> PartialEq for Node<K, V>
where
    K: PartialEq,
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.val == other.val
    }
}

impl<K, V> Debug for Node<K, V>
where
    K: Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Node {{ key:  {:?}, val: {:?}, height: {}, levels: [{}]}}",
            self.key,
            self.val,
            self.height,
            (0..self.height).fold(String::new(), |acc, level| {
                format!("{}{:?}, ", acc, self.levels[level])
            })
        )
    }
}

impl<K, V> Display for Node<K, V>
where
    K: Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (1..=self.levels.pointers.len()).try_for_each(|level| {
            writeln!(
                f,
                "[key:  {:?}, val: {:?}, level: {}]",
                self.key, self.val, level,
            )
        })
    }
}

pub(crate) struct ListState {
    len: AtomicUsize,
    max_height: AtomicUsize,
    seed: AtomicUsize,
}

impl ListState {
    pub(crate) fn new() -> Self {
        ListState {
            len: AtomicUsize::new(0),
            max_height: AtomicUsize::new(1),
            seed: AtomicUsize::new(rand::random()),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_new_node() {
        let node = Node::new(100, "hello", 1);
        let other = Node::new(100, "hello", 1);
        unsafe { println!("node 1: {:?},", *node) };
        unsafe { println!("node 2: {:?},", *other) };
        let other = unsafe {
            let node = Node::alloc(1);
            core::ptr::write(&mut (*node).key, 100);
            core::ptr::write(&mut (*node).val, "hello");
            node
        };

        unsafe { println!("node 1: {:?}, node 2: {:?}", *node, *other) };

        unsafe { assert_eq!(*node, *other) };
    }

    #[test]
    fn test_new_list() {
        let _: SkipList<usize, usize> = SkipList::new();
    }

    #[test]
    fn test_insert() {
        let list = SkipList::new();
        let mut rng: u16 = rand::random();

        for _ in 0..100_000 {
            rng ^= rng << 3;
            rng ^= rng >> 12;
            rng ^= rng << 7;
            list.insert(rng, "hello there!");
        }
    }

    #[test]
    fn test_rand_height() {
        let mut list: SkipList<i32, i32> = SkipList::new();
        let node = Node::new_rand_height("Hello", "There!", &mut list);

        assert!(!node.is_null());
        let height = unsafe { (*node).levels.pointers.len() };

        println!("height: {}", height);

        unsafe {
            println!("{}", *node);
        }

        unsafe {
            let _ = Box::from_raw(node);
        }
    }

    #[test]
    fn test_insert_verbose() {
        let list = SkipList::new();

        list.insert(1, 1);
        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0][1];
            while !node.load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.load(Ordering::Relaxed)).key,
                    (*node.load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [ref left, _] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [_, ref right] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", right);
                }
                println!();
                node = &(*node.load(Ordering::Relaxed)).levels[0][1];
            }
        }

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        list.insert(2, 2);

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0][1];
            while !node.load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.load(Ordering::Relaxed)).key,
                    (*node.load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [ref left, _] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [_, ref right] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", right);
                }
                println!();
                node = &(*node.load(Ordering::Relaxed)).levels[0][1];
            }
        }

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        list.insert(5, 3);

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0][1];
            while !node.load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.load(Ordering::Relaxed)).key,
                    (*node.load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [ref left, _] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [_, ref right] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", right);
                }
                println!();
                node = &(*node.load(Ordering::Relaxed)).levels[0][1];
            }
        }

        println!("trying to drop");
    }

    #[test]
    fn test_remove() {
        let list = SkipList::new();
        let mut rng: u16 = rand::random();

        for _ in 0..100_000 {
            rng ^= rng << 3;
            rng ^= rng >> 12;
            rng ^= rng << 7;
            list.insert(rng, "hello there!");
        }
        for _ in 0..100_000 {
            rng ^= rng << 3;
            rng ^= rng >> 12;
            rng ^= rng << 7;
            list.remove(&rng);
        }
    }

    #[test]
    fn test_verbose_remove() {
        let list = SkipList::new();

        list.insert(1, 1);
        list.insert(2, 2);
        list.insert(2, 2);
        list.insert(5, 3);

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0][1];
            while !node.load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.load(Ordering::Relaxed)).key,
                    (*node.load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [ref left, _] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [_, ref right] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", right);
                }
                println!();
                node = &(*node.load(Ordering::Relaxed)).levels[0][1];
            }
        }

        assert!(list.remove(&1).is_some());

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0][1];
            while !node.load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.load(Ordering::Relaxed)).key,
                    (*node.load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [ref left, _] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [_, ref right] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", right);
                }
                println!();
                node = &(*node.load(Ordering::Relaxed)).levels[0][1];
            }
        }

        println!("removing 6");
        assert!(list.remove(&6).is_none());
        println!("removing 1");
        assert!(list.remove(&1).is_none());
        println!("removing 5");
        assert!(list.remove(&5).is_some());
        println!("removing 2");
        assert!(list.remove(&2).is_some());
        //list.remove(&2);

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0][1];
            while !node.load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.load(Ordering::Relaxed)).key,
                    (*node.load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [ref left, _] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                print!("                                ");
                for level in 0..(*node.load(Ordering::Relaxed)).height {
                    let [_, ref right] = (*node.load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", right);
                }
                println!();
                node = &(*node.load(Ordering::Relaxed)).levels[0][1];
            }
        }

        assert_eq!(list.len(), 0);
    }
}
