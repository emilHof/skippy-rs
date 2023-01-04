use core::{ptr::NonNull, sync::atomic::Ordering};

use crate::internal::utils::{skiplist_basics, GeneratesHeight, Levels, Node, HEIGHT};

skiplist_basics!(SkipList);

impl<'domain, K, V> SkipList<'domain, K, V>
where
    K: Ord,
{
    /// Inserts a value in the list given a key.
    pub fn insert(&mut self, key: K, val: V) -> Option<V> {
        self.internal_insert(key, val, true)
    }

    pub fn insert_adjacent(&mut self, key: K, val: V) -> Option<V> {
        self.internal_insert(key, val, false)
    }

    fn internal_insert(&self, key: K, mut val: V, replace: bool) -> Option<V> {
        // After this check, whether we are holding the head or a regular Node will
        // not impact the operation.
        unsafe {
            let insertion_point = self.find(&key);

            match insertion_point {
                SearchResult {
                    target: Some(target),
                    ..
                } if replace => {
                    std::mem::swap(&mut (*target.as_ptr()).val, &mut val);
                    Some(val)
                }
                SearchResult { prev, .. } => {
                    let new_node = Node::new_rand_height(key, val, self);

                    self.link_nodes(new_node, prev);

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
    unsafe fn link_nodes(&self, new_node: *mut Node<K, V>, prev: [&Levels<K, V>; HEIGHT]) {
        // iterate over all the levels in the new nodes pointer tower
        for (i, levels) in prev.iter().enumerate().take((*new_node).height()) {
            // move backwards until a pointer tower of sufficient hight is reached
            unsafe {
                (*new_node).levels[i].store_ptr(levels[i].load_ptr());
                levels[i].store_ptr(new_node);
                (*new_node).add_ref();
            }
        }
    }

    pub fn remove(&mut self, key: &K) -> Option<(K, V)> {
        self.internal_remove(key)
    }

    pub fn remove_first(&mut self) -> Option<(K, V)> {
        if self.is_empty() {
            return None;
        }

        unsafe {
            let key = &(*self.head.as_ref().levels[0].load_ptr()).key;
            self.internal_remove(key)
        }
    }

    fn internal_remove(&self, key: &K) -> Option<(K, V)> {
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
            for (i, levels) in prev.iter().enumerate().take((*node).height()) {
                levels[i].store_ptr((*node).levels[i].load_ptr());
            }
        }
    }

    unsafe fn unlink_level(
        prev: *mut Node<K, V>,
        curr: *mut Node<K, V>,
        level: usize,
    ) -> *mut Node<K, V> {
        let next = (*curr).levels[level].load_ptr();

        if (*curr).sub_ref() == 0 {
            Node::<K, V>::dealloc(curr);
        }

        (*prev).levels[level].store_ptr(next);
        next
    }

    /// This method is `unsafe` as it may return the head typecast as a Node, which can
    /// cause UB if not handled appropriately. If the return value is Ok(...) then it is a
    /// regular Node. If it is Err(...) then it is the head.
    unsafe fn find<'a>(&'a self, key: &K) -> SearchResult<'a, K, V> {
        let mut level = self.state.max_height.load(Ordering::Relaxed);
        let head = unsafe { &(*self.head.as_ptr()) };

        let mut prev = [&head.levels; HEIGHT];

        // find the first and highest node tower
        while level > 1 && head.levels[level - 1].load_ptr().is_null() {
            level -= 1;
        }

        let mut curr = self.head.as_ptr().cast::<Node<K, V>>();
        prev[level - 1] = &(*curr).levels;

        unsafe {
            while level > 0 {
                let mut next = (*curr).levels[level - 1].load_ptr();

                if !next.is_null() && (*next).levels[level - 1].load_tag() == 1 {
                    next = Self::unlink_level(curr, next, level - 1);
                }

                if next.is_null() || (*next).key >= *key {
                    prev[level - 1] = &(*curr).levels;
                    level -= 1;
                } else {
                    curr = next;
                }
            }
        }

        let next = (*curr).levels[level].load_ptr();

        if !next.is_null() && &(*next).key == key {
            SearchResult {
                prev,
                target: unsafe { Some(NonNull::new_unchecked(next)) },
            }
        } else {
            SearchResult { prev, target: None }
        }
    }

    pub fn get<'a>(&'a self, key: &K) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        // Perform safety check for whether we are dealing with the head.
        unsafe {
            match self.find(key) {
                SearchResult {
                    target: Some(ptr), ..
                } => Some(Entry { node: ptr.as_ref() }),
                _ => None,
            }
        }
    }

    fn is_head(&self, ptr: *const Node<K, V>) -> bool {
        std::ptr::eq(ptr, self.head.as_ptr().cast())
    }

    fn next_node<'a>(&'a self, entry: &Entry<'a, K, V>) -> Option<Entry<'a, K, V>> {
        let node = entry.node;

        if node.levels[0].load_tag() == 1 {
            return None;
        }

        let mut next = node.levels[0].load_ptr();

        unsafe {
            while !next.is_null() && (*next).levels[0].load_tag() == 1 {
                next = Self::unlink_level(node as *const _ as *mut Node<K, V>, next, 0);
            }
        }

        if next.is_null() {
            return None;
        }

        unsafe { Some(Entry { node: &(*next) }) }
    }

    pub fn get_first<'a>(&'a self) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        unsafe {
            self.next_node(&Entry {
                node: &(*self.head.as_ptr().cast()),
            })
        }
    }

    pub fn get_last<'a>(&'a self) -> Option<Entry<'a, K, V>> {
        let mut curr = self.get_first()?;

        while let Some(next) = self.next_node(&curr) {
            curr = next;
        }

        Some(curr)
    }

    fn traverse_with<F>(&self, mut f: F)
    where
        F: FnMut(&K, &V),
    {
        let mut curr = unsafe { self.head.as_ref().levels[0].load_ptr() };

        unsafe {
            while !curr.is_null() {
                if !(*curr).removed() {
                    let key = &(*curr).key;
                    let val = &(*curr).val;

                    f(key, val);
                }

                curr = (*curr).levels[0].load_ptr();
            }
        }
    }

    pub fn entry<'a: 'domain>(&'a mut self, key: K) -> Option<MutEntry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        unsafe {
            match self.find(&key) {
                SearchResult {
                    prev: _,
                    target: Some(mut target),
                } => MutEntry {
                    node: target.as_mut(),
                    list: self,
                    key,
                }
                .into(),
                _ => None,
            }
        }
    }
}

pub struct Entry<'a, K, V> {
    node: &'a Node<K, V>,
}

impl<'a, K, V> Entry<'a, K, V> {
    pub fn val(&self) -> &'a V {
        &self.node.val
    }

    pub fn key(&self) -> &'a K {
        &self.node.key
    }
}

pub struct MutEntry<'a, K, V> {
    list: &'a mut SkipList<'a, K, V>,
    node: &'a mut Node<K, V>,
    key: K,
}

impl<'a, K, V> MutEntry<'a, K, V> {
    pub fn val(&self) -> &V {
        &self.node.val
    }

    pub fn key(&self) -> &K {
        &self.node.key
    }
}

impl<'a, K: Ord, V> MutEntry<'a, K, V> {
    pub fn remove(self) -> Option<(K, V)> {
        self.list.remove(&self.key)
    }
}

struct SearchResult<'a, K, V> {
    prev: [&'a Levels<K, V>; HEIGHT],
    target: Option<NonNull<Node<K, V>>>,
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
        let _: SkipList<'_, usize, usize> = SkipList::new();
    }

    #[test]
    fn test_insert() {
        let mut list = SkipList::new();
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
        let mut list: SkipList<'_, i32, i32> = SkipList::new();
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
        let mut list = SkipList::new();

        list.insert(1, 1);
        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node.as_std(),
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left.as_std());
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
            }
        }

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        list.insert(2, 2);

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node.as_std(),
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left.as_std());
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
            }
        }

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        list.insert(5, 3);

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node.as_std(),
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left.as_std());
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
            }
        }

        println!("trying to drop");
    }

    #[test]
    fn test_remove() {
        let mut list = SkipList::new();
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
        let mut list = SkipList::new();

        list.insert(1, 1);
        list.insert(2, 2);
        list.insert(2, 2);
        list.insert(5, 3);

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node.as_std(),
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left.as_std());
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
            }
        }

        assert!(list.remove(&1).is_some());

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node.as_std(),
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left.as_std());
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
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
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node.as_std(),
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left.as_std());
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
            }
        }

        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_traverse() {
        let mut list = SkipList::new();
        for _ in 0..100 {
            list.insert(rand::random::<u8>(), ());
        }

        let mut prev = list.get_first().unwrap().key().clone();

        list.traverse_with(|k, _| {
            println!("key: {:?}", k);
            assert!(*k >= prev);
            prev = k.clone();
        })
    }

    #[test]
    fn test_get_last() {
        let mut list = SkipList::new();
        for _ in 0..100 {
            list.insert(rand::random::<u8>(), ());
        }

        assert!(list.get_last().is_some());

        println!("{}", list.get_last().unwrap().key())
    }
}
