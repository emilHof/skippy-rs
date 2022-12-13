extern crate alloc;

use alloc::alloc::{alloc, dealloc, handle_alloc_error};

use core::{
    alloc::Layout,
    borrow::Borrow,
    fmt::{Debug, Display},
    mem,
    ops::{Index, IndexMut},
    ptr::{self, NonNull},
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

const HEIGHT_BITS: usize = 5;

const HEIGHT: usize = 1 << HEIGHT_BITS;

/// Head stores the first pointer tower at the beginning of the list. It is always of maximum
#[repr(C)]
struct Head<K, V> {
    key: K,
    val: V,
    height: usize,
    levels: Levels<K, V>,
}

impl<K, V> Head<K, V> {
    fn new() -> NonNull<Self> {
        let head_ptr = unsafe { Node::<K, V>::alloc(HEIGHT).cast() };

        if let Some(head) = NonNull::new(head_ptr) {
            head
        } else {
            panic!()
        }
    }

    unsafe fn drop(ptr: NonNull<Self>) {
        Node::<K, V>::dealloc(ptr.as_ptr().cast());
    }
}

#[repr(C)]
struct Levels<K, V> {
    pointers: [[AtomicPtr<Node<K, V>>; 2]; 1],
}

impl<K, V> Levels<K, V> {
    fn get_size(height: usize) -> usize {
        assert!(height <= HEIGHT && height > 0);

        mem::size_of::<Self>() * (height - 1)
    }
}

impl<K, V> Index<usize> for Levels<K, V> {
    type Output = [AtomicPtr<Node<K, V>>; 2];

    fn index(&self, index: usize) -> &Self::Output {
        unsafe { self.pointers.get_unchecked(index) }
    }
}

impl<K, V> IndexMut<usize> for Levels<K, V> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        unsafe { self.pointers.get_unchecked_mut(index) }
    }
}

#[repr(C)]
struct Node<K, V> {
    key: K,
    val: V,
    height: usize,
    levels: Levels<K, V>,
}

impl<K, V> Node<K, V> {
    fn new(key: K, val: V, height: usize) -> *mut Self {
        unsafe {
            let node = Self::alloc(height);

            ptr::write(&mut (*node).key, key);
            ptr::write(&mut (*node).val, val);
            node
        }
    }

    fn new_rand_height(key: K, val: V, list: &SkipList<K, V>) -> *mut Self {
        // construct the base nod
        Self::new(key, val, list.gen_height())
    }

    unsafe fn alloc(height: usize) -> *mut Self {
        let layout = Self::get_layout(height);

        let ptr = alloc(layout).cast::<Self>();

        if ptr.is_null() {
            handle_alloc_error(layout);
        }

        ptr::write(&mut (*ptr).height, height);

        ptr::write_bytes((*ptr).levels.pointers.as_mut_ptr(), 0, height);

        ptr
    }

    unsafe fn dealloc(ptr: *mut Self) {
        let height = (*ptr).height;

        let layout = Self::get_layout(height);

        dealloc(ptr.cast(), layout);
    }

    unsafe fn get_layout(height: usize) -> Layout {
        let size_self = mem::size_of::<Self>();
        let align = mem::align_of::<Self>();
        let size_levels = Levels::<K, V>::get_size(height);

        Layout::from_size_align_unchecked(size_self + size_levels, align)
    }

    unsafe fn drop(ptr: *mut Self) {
        ptr::drop_in_place(&mut (*ptr).key);
        ptr::drop_in_place(&mut (*ptr).val);

        Self::dealloc(ptr);
    }
}

pub struct SkipList<K, V> {
    head: NonNull<Head<K, V>>,
    state: ListState,
}

impl<K, V> SkipList<K, V> {
    /// Instantiates a new, empty [SkipList](SkipList).
    pub fn new() -> Self {
        SkipList {
            head: Head::new(),
            state: ListState {
                len: AtomicUsize::new(0),
                max_height: AtomicUsize::new(1),
                seed: AtomicUsize::new(rand::random()),
            },
        }
    }

    /// Gets the length of the [SkipList](SkipList).
    pub fn len(&self) -> usize {
        self.state.len.load(Ordering::Relaxed)
    }

    pub fn is_empty(&self) -> bool {
        self.state.len.load(Ordering::Relaxed) < 1
    }

    /// Randomly generates a height that is within the right parameters.
    /// Prevents the hight from getting unnecessarily large by making it
    /// at most one level higher then the previously largest height in the
    /// list.
    fn gen_height(&self) -> usize {
        let mut seed = self.state.seed.load(Ordering::Relaxed);
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;

        self.state.seed.store(seed, Ordering::Relaxed);

        let mut height = std::cmp::min(HEIGHT, seed.trailing_zeros() as usize + 1);

        let head = unsafe { &(*self.head.as_ptr()) };

        while height >= 4 && head.levels[height - 2][1].load(Ordering::Relaxed).is_null() {
            height -= 1;
        }

        if height > self.state.max_height.load(Ordering::Relaxed) {
            self.state.max_height.store(height, Ordering::Relaxed);
        }

        height
    }
}

impl<K, V> SkipList<K, V>
where
    K: Ord,
{
    /// Inserts a value in the list given a key.
    pub fn insert(&self, key: K, mut val: V) -> Option<V> {
        // After this check, whether we are holding the head or a regular Node will
        // not impact the operation.
        let insertion_point = unsafe {
            let insertion_point = self.find(&key);

            if let Ok(insertion_point) = insertion_point {
                if (*insertion_point).key == key {
                    std::mem::swap(&mut (*insertion_point).val, &mut val);
                    return Some(val);
                }

                // We have a regular Node
                insertion_point
            } else {
                // We are dealing with the head of the list
                insertion_point.unwrap_err()
            }
        };

        let new_node = Node::new_rand_height(key, val, self);

        let link_node = insertion_point;

        unsafe { Self::link_nodes(new_node, link_node) };

        self.state.len.fetch_add(1, Ordering::Relaxed);

        None
    }

    /// This function is unsafe, as it does not check whether new_node or link node are valid
    /// pointers.
    /// To call this function safely:
    /// - new_node cannot be null
    /// - link_node cannot be null
    /// - no pointer tower along the path can have a null pointer pointing backwards
    /// - a tower of sufficient height must eventually be reached, the list head can be this tower
    unsafe fn link_nodes(new_node: *mut Node<K, V>, mut link_node: *mut Node<K, V>) {
        // iterate over all the levels in the new nodes pointer tower
        for level in 0..((*new_node).height) {
            // move backwards until a pointer tower of sufficient hight is reached
            while (*link_node).height <= level {
                link_node = (*link_node).levels[level - 1][0].load(Ordering::Relaxed);
            }

            // perform the re-linking
            (*new_node).levels[level][0].store(link_node, Ordering::Relaxed);
            (*new_node).levels[level][1].store(
                (*link_node).levels[level][1].load(Ordering::Relaxed),
                Ordering::Relaxed,
            );
            let old_right = (*link_node).levels[level][1].load(Ordering::Relaxed);
            if !old_right.is_null() {
                (*old_right).levels[level][0].store(new_node, Ordering::Relaxed);
            }
            (*link_node).levels[level][1].store(new_node, Ordering::Relaxed);
        }
    }

    pub fn remove(&self, key: &K) -> Option<(K, V)> {
        if self.is_empty() {
            return None;
        }

        let target = unsafe {
            match self.find(key) {
                Err(_) => return None,
                Ok(target) => {
                    if target.is_null() || (*target).key != *key {
                        return None;
                    }
                    target
                }
            }
        };

        self.unlink(target);

        let (key, val) = unsafe {
            let key = core::ptr::read(&(*target).key);
            let val = core::ptr::read(&(*target).val);
            Node::<K, V>::dealloc(target);

            (key, val)
        };

        self.state.len.fetch_sub(1, Ordering::Relaxed);

        Some((key, val))
    }

    /// Logically removes the node from the list by linking its adjacent nodes to one-another.
    fn unlink(&self, node: *mut Node<K, V>) {
        // safety check against UB caused by unlinking the head
        if self.is_head(node) {
            panic!()
        }

        unsafe {
            for level in (0..(*node).height).rev() {
                let [ref left, ref right] = (*node).levels[level];
                (*left.load(Ordering::Relaxed)).levels[level][1]
                    .store(right.load(Ordering::Relaxed), Ordering::Relaxed);
                let right_ptr = right.load(Ordering::Relaxed);

                if !right_ptr.is_null() {
                    (*right_ptr).levels[level][0]
                        .store(left.load(Ordering::Relaxed), Ordering::Relaxed);
                }
            }
        }
    }

    /// This method is `unsafe` as it may return the head typecast as a Node, which can
    /// cause UB if not handled appropriately. If the return value is Ok(...) then it is a
    /// regular Node. If it is Err(...) then it is the head.
    unsafe fn find(&self, key: &K) -> Result<*mut Node<K, V>, *mut Node<K, V>> {
        let mut level = self.state.max_height.load(Ordering::Relaxed);
        let head = unsafe { &(*self.head.as_ptr()) };

        let mut prev = [&head.levels; HEIGHT];

        // find the first and highest node tower
        while level > 1 && head.levels[level - 1][1].load(Ordering::Relaxed).is_null() {
            level -= 1;
        }

        let mut curr = self.head.as_ptr() as *const Node<K, V>;

        unsafe {
            'search: loop {
                while level > 1
                    && ((*curr).levels[level - 1][1]
                        .load(Ordering::Relaxed)
                        .is_null()
                        || (*(*curr).levels[level - 1][1].load(Ordering::Relaxed)).key > *key)
                {
                    prev[level - 1] = &(*curr).levels;
                    level -= 1;
                }

                if !(*curr).levels[level - 1][1]
                    .load(Ordering::Relaxed)
                    .is_null()
                    && (*(*curr).levels[level - 1][1].load(Ordering::Relaxed)).key <= *key
                {
                    curr = (*curr).levels[level - 1][1].load(Ordering::Relaxed);
                } else {
                    (0..level).for_each(|i| prev[i] = &(*curr).levels);
                    break 'search;
                }
            }
        }

        if self.is_head(curr) {
            return Err(curr as *mut _);
        }

        Ok(curr as *mut _)
    }

    pub fn get<'a>(&self, key: &K) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        // Perform safety check for whether we are dealing with the head.
        let target = unsafe {
            let target = self.find(key);

            if let Ok(target) = target {
                target
            } else {
                return None;
            }
        };

        unsafe {
            if (*target).key == *key {
                let target = &(*target);
                return Some(Entry {
                    key: &target.key,
                    val: &target.val,
                });
            }
        }

        None
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

impl<K, V> Drop for SkipList<K, V> {
    fn drop(&mut self) {
        let mut node = unsafe { (*self.head.as_ptr()).levels[0][1].load(Ordering::Relaxed) };

        while !node.is_null() {
            unsafe {
                let temp = node;
                node = (*temp).levels[0][1].load(Ordering::Relaxed);
                Node::<K, V>::drop(temp);
            }
        }

        unsafe { Head::<K, V>::drop(self.head) };
    }
}

impl<K, V> Default for SkipList<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> super::skiplist::SkipList<K, V> for SkipList<K, V>
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

pub struct SearchResult<'a, K, V> {
    head: bool,
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

struct ListState {
    len: AtomicUsize,
    max_height: AtomicUsize,
    seed: AtomicUsize,
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
        let mut list = SkipList::new();
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
        assert!(list.remove(&6).is_none());
        assert!(list.remove(&1).is_none());
        assert!(list.remove(&5).is_some());
        assert!(list.remove(&2).is_some());

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
    }
}
