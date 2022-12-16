use core::{borrow::Borrow, marker::Sync, sync::atomic::Ordering};
use std::ptr::NonNull;

use haphazard::{raw::Reclaim, Global};

use crate::internal::utils::{skiplist_basics, GeneratesHeight, Levels, Node, HEIGHT};

skiplist_basics!(SkipList);

impl<'domain, K, V> SkipList<'domain, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    /// Inserts a value in the list given a key.
    pub fn insert(&self, key: K, mut val: V) -> Option<V> {
        // After this check, whether we are holding the head or a regular Node will
        // not impact the operation.
        unsafe {
            let insertion_point = self.find(&key);

            match insertion_point {
                SearchResult {
                    target: Some((target, hazard)),
                    ..
                } => {
                    std::mem::swap(&mut (*target.as_ptr()).val, &mut val);
                    drop(hazard);
                    Some(val)
                }
                SearchResult {
                    prev,
                    level_hazards,
                    ..
                } => {
                    let new_node = Node::new_rand_height(key, val, self);

                    self.link_nodes(new_node, prev);

                    self.state.len.fetch_add(1, Ordering::Relaxed);

                    drop(level_hazards);

                    None
                }
            }
        }
    }

    /// This function is unsafe, as it does not check whether new_node or link node are valid
    /// pointers.
    ///
    /// # Safety
    ///
    /// 1. `new_node` cannot be null
    /// 2. `link_node` cannot be null
    /// 3. No pointer tower along the path can have a null pointer pointing backwards
    /// 4. A tower of sufficient height must eventually be reached, the list head can be this tower
    unsafe fn link_nodes(&self, new_node: *mut Node<K, V>, prev: [&Levels<K, V>; HEIGHT]) {
        // iterate over all the levels in the new nodes pointer tower
        for (i, levels) in prev.iter().enumerate().take((*new_node).height()) {
            // move backwards until a pointer tower of sufficient hight is reached
            unsafe {
                (*new_node).levels[i].store_ptr(levels[i].load_ptr());
                levels[i].store_ptr(new_node);
            }
        }
    }

    pub fn remove(&self, key: &K) -> Option<(K, V)>
    where
        K: Send,
        V: Send,
    {
        if self.is_empty() {
            return None;
        }

        unsafe {
            match self.find(key) {
                SearchResult {
                    target: Some((target, hazard)),
                    prev,
                    level_hazards,
                } => {
                    let target = target.as_ptr();

                    // Set the target state to being removed
                    // If this errors, it is already being removed by someone else
                    // and thus we exit early.
                    if (*target).set_removed().is_err() {
                        return None;
                    }

                    //
                    let key = core::ptr::read(&(*target).key);
                    let val = core::ptr::read(&(*target).val);

                    self.unlink(target, prev);
                    self.garbage
                        .domain
                        .retire_ptr_with(target, |ptr: *mut dyn Reclaim| {
                            Node::<K, V>::dealloc(ptr as *mut Node<K, V>)
                        });

                    // We see if we can drop some pointers in the list.
                    self.garbage.domain.eager_reclaim();

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

    unsafe fn find<'a>(&'a self, key: &K) -> SearchResult<'a, K, V> {
        let mut level_hazards: haphazard::HazardPointerArray<'a, haphazard::Global, HEIGHT> =
            haphazard::HazardPointer::many_in_domain(self.garbage.domain);

        let mut curr_hazard = haphazard::HazardPointer::new_in_domain(self.garbage.domain);
        let mut next_hazard = haphazard::HazardPointer::new_in_domain(self.garbage.domain);

        'search: loop {
            let mut level = self.state.max_height.load(Ordering::Relaxed);
            let head = unsafe { &(*self.head.as_ptr()) };

            let mut prev = [&head.levels; HEIGHT];

            // find the first and highest node tower
            while level > 1 && head.levels[level - 1].load_ptr().is_null() {
                level -= 1;
            }

            // TODO this should be protected!!
            let mut curr = self.head.as_ptr().cast::<Node<K, V>>();

            // steps:
            // 1. Go through each level until we reach a node with a key greater than ours
            //     1.1 If
            while level > 0 {
                let next = unsafe { next_hazard.protect_ptr((*curr).levels[level - 1].as_std()) };
                match next {
                    Some((next, _))
                        if unsafe {
                            (*next.as_ptr()).key >= *key && !(*next.as_ptr()).removed()
                        } =>
                    {
                        // TODO
                        // we need to ensure that we are getting protected access to the levels
                        level_hazards.as_refs()[level - 1]
                            .protect_raw((curr as *const Node<K, V>).cast_mut());
                        prev[level - 1] = &(*curr).levels;
                        level -= 1;
                    }
                    None => {
                        // Same as the previous match arm, yet I am struggling to find a way of
                        // combining them efficiently
                        // TODO Improve matching
                        level_hazards.as_refs()[level - 1]
                            .protect_raw((curr as *const Node<K, V>).cast_mut());
                        prev[level - 1] = &(*curr).levels;
                        level -= 1;
                    }
                    Some((next, _)) => {
                        // Otherwise, it is either smaller, or being removed.
                        // Either way, we protect move to this next node.
                        curr = next.as_ptr();
                        curr_hazard.protect_raw((curr as *const Node<K, V>).cast_mut());

                        // Reset protection for the next pointer
                        next_hazard.reset_protection();
                    }
                };
            }

            return match next_hazard.protect_ptr((*curr).levels[level].as_std()) {
                Some((next, _))
                    if unsafe { (*next.as_ptr()).key == *key && !(*next.as_ptr()).removed() } =>
                {
                    SearchResult {
                        prev,
                        level_hazards,
                        target: Some((next, next_hazard)),
                    }
                }
                _ => SearchResult {
                    prev,
                    level_hazards,
                    target: None,
                },
            };
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
                    target: Some((node, hazard)),
                    ..
                } => Some(Entry { node, hazard }),
                _ => None,
            }
        }
    }

    fn is_head(&self, ptr: *const Node<K, V>) -> bool {
        std::ptr::eq(ptr, self.head.as_ptr().cast())
    }

    pub fn get_first<'a>(&'a self) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        let mut hazard = haphazard::HazardPointer::new_in_domain(self.garbage.domain);
        let mut next_hazard = haphazard::HazardPointer::new_in_domain(self.garbage.domain);
        let mut curr = unsafe {
            hazard
                .protect_ptr((*self.head.as_ptr()).levels[0].as_std())?
                .0
                .as_ptr()
        };

        while let Some((next, _)) = unsafe { next_hazard.protect_ptr((*curr).levels[0].as_std()) } {
            curr = next.as_ptr();
            hazard.protect_raw(curr);
            if unsafe { !(*curr).removed() } {
                unsafe {
                    return Some(Entry {
                        node: NonNull::new_unchecked(curr),
                        hazard,
                    });
                }
            }
        }

        None
    }

    pub fn get_last<'a>(&'a self) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        'last: loop {
            let mut hazard = haphazard::HazardPointer::new_in_domain(self.garbage.domain);
            let mut next_hazard = haphazard::HazardPointer::new_in_domain(self.garbage.domain);
            let mut curr = unsafe {
                hazard
                    .protect_ptr((*self.head.as_ptr()).levels[0].as_std())?
                    .0
                    .as_ptr()
            };

            while let Some((next, _)) =
                unsafe { next_hazard.protect_ptr((*curr).levels[0].as_std()) }
            {
                curr = next.as_ptr();
                hazard.protect_raw(curr);
            }

            if unsafe { (*curr).removed() } {
                hazard.reset_protection();
                next_hazard.reset_protection();
                continue 'last;
            }

            unsafe {
                return Some(Entry {
                    node: NonNull::new_unchecked(curr),
                    hazard,
                });
            }
        }
    }
}

impl<'domain, K, V> Default for SkipList<'domain, K, V>
where
    K: Sync,
    V: Sync,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'domain, K, V> crate::skiplist::SkipList<K, V> for SkipList<'domain, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    type Entry<'a> = Entry<'a, K, V> where K: 'a, V: 'a, Self: 'a;

    fn new() -> Self {
        SkipList::new()
    }

    fn insert(&self, key: K, value: V) -> Option<V> {
        self.insert(key, value)
    }

    fn remove(&self, key: &K) -> Option<(K, V)> {
        self.remove(key)
    }

    fn get<'a>(&'a self, key: &K) -> Option<Self::Entry<'a>> {
        self.get(key)
    }

    fn last<'a>(&'a self) -> Option<Self::Entry<'a>> {
        self.get_first()
    }

    fn front<'a>(&'a self) -> Option<Self::Entry<'a>> {
        self.get_last()
    }

    fn len(&self) -> usize {
        self.len()
    }
}

pub struct Entry<'a, K: 'a, V: 'a> {
    node: core::ptr::NonNull<Node<K, V>>,
    hazard: haphazard::HazardPointer<'a, Global>,
}

struct SearchResult<'a, K, V> {
    prev: [&'a Levels<K, V>; HEIGHT],
    level_hazards: haphazard::HazardPointerArray<'a, haphazard::Global, HEIGHT>,
    target: Option<(NonNull<Node<K, V>>, haphazard::HazardPointer<'a, Global>)>,
}

impl<'a, K, V> Borrow<K> for Entry<'a, K, V> {
    fn borrow(&self) -> &K {
        unsafe { &self.node.as_ref().key }
    }
}

impl<'a, K, V> AsRef<V> for Entry<'a, K, V> {
    fn as_ref(&self) -> &V {
        unsafe { &self.node.as_ref().val }
    }
}
#[cfg(test)]
mod sync_test {
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
        let list = SkipList::new();

        list.insert(1, 1);
        unsafe {
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
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
                    node,
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
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
                    node,
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
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
            let mut node = &(*list.head.as_ptr()).levels[0];
            while !node.as_std().load(Ordering::Relaxed).is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
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
                    node,
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
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
                    node,
                    (*node.as_std().load(Ordering::Relaxed)).key,
                    (*node.as_std().load(Ordering::Relaxed)).key
                );
                print!("                                ");
                for level in 0..(*node.as_std().load(Ordering::Relaxed)).height() {
                    let ref left = (*node.as_std().load(Ordering::Relaxed)).levels[level];
                    print!("{:?} | ", left);
                }
                println!();
                node = &(*node.as_std().load(Ordering::Relaxed)).levels[0];
            }
        }

        assert_eq!(list.len(), 0);
    }
}
