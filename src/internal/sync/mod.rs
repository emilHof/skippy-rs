use core::borrow::Borrow;
use core::fmt::Debug;
use core::marker::Sync;
use core::ptr::NonNull;
use core::sync::atomic::AtomicPtr;
use core::sync::atomic::Ordering;

use haphazard::raw::Pointer;
use haphazard::{Global, HazardPointer, Domain};

use crate::{
    internal::utils::{skiplist_basics, GeneratesHeight, Node, HEIGHT},
    internal::sync::iter::{ Iter, IntoIter },
    skiplist,
};

pub(crate) mod tagged;
pub mod iter;

skiplist_basics!(SkipList);

impl<'a, K, V> Debug for SkipList<'a, K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkipList").field("head", &self.head.as_ptr()).finish()
    }
}

impl<'domain, K, V> SkipList<'domain, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    /// Inserts a value in the list given a key.
    pub fn insert(&self, key: K, val: V) -> Option<(K, V)> {
        // After this check, whether we are holding the head or a regular Node will
        // not impact the operation.
        let mut insertion_point = self.find(&key, false);
        let mut existing = None;

        while let Some(target) = insertion_point.target.take() {
            existing = target.try_remove_and_tag().ok().map(|temp| {
                unsafe {
                    let _ = self.unlink(&target, target.height(), &insertion_point.prev);
                }
                insertion_point = self.find(&key, false);
                temp
            });
        };
        
        let mut prev = insertion_point.prev;

        let new_node_raw = Node::new_rand_height(key, val, self);

        // Protects the new_node so concurrent removals do not invalidate our pointer.
        let new_node = NodeRef::from_raw(new_node_raw);

        let mut starting_height = 0;

        // The node should not be in build stage!
        // assert!(new_node.set_build_begin().is_ok());
        //

        self.state.len.fetch_add(1, Ordering::AcqRel);

        unsafe {
            while let Err(starting) =
                self.link_nodes(&new_node, prev, starting_height)
            {
                let mut search = self.find(&new_node.key, false);
                
                while let Some(target) = search.target.take() {
                    if core::ptr::eq(target.as_ptr(), new_node.as_ptr()) {
                        break;
                    }

                    existing = target.try_remove_and_tag().ok().map(|temp| {
                        let _ = self.unlink(&target, target.height(), &search.prev);
                        search = self.find(&new_node.key, false);
                        temp
                    });
                };

                (starting_height, prev) = (starting, search.prev);
            }
        }

        existing
    }

    /// This function is unsafe, as it does not check whether new_node or link node are valid
    /// pointers.
    ///
    /// # Safety
    ///
    /// 1. `new_node` cannot be null
    /// 2. A tower of sufficient height must eventually be reached, the list head can be this tower
    unsafe fn link_nodes<'a>(
        &self,
        new_node: &'a NodeRef<'a, K, V>,
        previous_nodes: [(NodeRef<'a, K, V>, Option<NodeRef<'a, K, V>>); HEIGHT],
        start_height: usize,
    ) -> Result<(), usize> {
        // iterate over all the levels in the new nodes pointer tower
        for i in start_height..new_node.height() {
            let (prev, next) = &previous_nodes[i];
            let next_ptr = next.as_ref().map_or(core::ptr::null_mut(), |n| n.as_ptr());

            if new_node.removed() {
                break;
            }

            // we check if the next node is actually lower in key than our current node.
            if next.as_ref()
                .and_then(|n| if n.key <= new_node.key && !new_node.removed() {
                    Some(())
                } else {
                    None
                }).is_some()
            {
                return Err(i);
            }
            
            // Swap the previous' next node into the new_node's level
            // It could be the case that we link ourselves to the previous node, but just as we do
            // this `next` attempts to unlink itself and fails. So while we succeeded, `next`
            // repeats its search and finds that we are the next
            new_node.levels[i].store_ptr(next_ptr);

            if i == 0 {
                new_node.add_ref();
            } else if new_node.try_add_ref().is_err() {
                break;
            }

            // Swap the new_node into the previous' level. If the previous' level has changed since
            // the search, we repeat the search from this level.
            if let Err((_other, _tag)) = prev.levels[i].compare_exchange(
                next_ptr, 
                new_node.as_ptr()
            ) {
                new_node.sub_ref();
                return Err(i);
            }

        }
        Ok(())
    }

    #[allow(unused_assignments)]
    pub fn remove(&self, key: &K) -> Option<(K, V)>
    where
        K: Send,
        V: Send,
    {
    match self.find(key, false) {
        SearchResult {
                target: Some(target),
                prev,
            } => {

                // Set the target state to being removed
                // If this errors, it is already being removed by someone else
                // and thus we exit early.
                if target.set_removed().is_err() {
                    return None;
                }

                // # Safety:
                // 1. `key` and `val` will not be tempered with.
                // TODO This works for now, yet once `Atomic` is used
                // this may need to change.
                let (key, val, height) = unsafe {
                    (
                        core::ptr::read(&target.key),
                        core::ptr::read(&target.val),
                        target.height()
                    )
                };

                if let Err(_) = target.tag_levels(1) {
                    panic!("SHOULD NOT BE TAGGED!")
                };

                // #Safety:
                // 1. The height we got from the `node` guarantees it is a valid height for levels.
                unsafe {
                    if self.unlink(&target, height, &prev).is_err() {
                        self.find(&key, false);
                    }
                }


                Some((key, val))
            }
            _ => None,
        }
    }

    /// Logically removes the node from the list by linking its adjacent nodes to one-another.
    ///
    /// # Safety
    /// 1. All indices in [0, height) are valid indices for `node.levels`.
    unsafe fn unlink<'a>(
        &self,
        node: &'a NodeRef<'a, K, V>,
        height: usize,
        previous_nodes: &[(NodeRef<'a, K, V>, Option<NodeRef<'a, K, V>>); HEIGHT],
    ) -> Result<(), usize> {
        // safety check against UB caused by unlinking the head
        if self.is_head(node.as_ptr()) {
            panic!()
        }

        // # Safety
        //
        // 1.-3. Some as method and covered by method caller.
        // 4. We are not unlinking the head. - Covered by previous safety check.
        for (i, (prev, next)) in previous_nodes.iter().enumerate().take(height).rev() {
            let (new_next, _tag) = node.levels[i].load_decomposed();
            let _next_ptr = next.as_ref().map_or(core::ptr::null_mut(), |n| n.as_ptr());

            // We check if the previous node is being removed after we have already unlinked
            // from it as the prev nodes expects us to do this.
            // We still need to stop the unlink here, as we will have to relink to the actual,
            // lively previous node at this level as well.

            // Performs a compare_exchange, expecting the old value of the pointer to be the current
            // node. If it is not, we cannot make any reasonable progress, so we search again.
            if let Err((_other, _tag)) = prev.levels[i].compare_exchange(
                node.as_ptr(),
                new_next,
            ) {
                return Err(i + 1);
            }

            node.as_ref().try_sub_ref().expect("not to underflow");
        }

        self.state.len.fetch_sub(1, Ordering::AcqRel);

        // Since all levels were unlinked we can safely retire the node.
        self.retire_node(node.as_ptr());

        // we see if we can drop some pointers in the list.
        self.garbage.domain.eager_reclaim();
        Ok(())
    }

    /// Unlink [Node](Node) `curr` at the given level of [Node](Node) `prev` by exchanging the pointer for `next`.
    ///
    /// # Safety
    ///
    /// 1. `prev`, `curr`, are protected accesses.
    #[allow(unused)]
    unsafe fn unlink_level<'a>(
        &'a self,
        prev: &NodeRef<'a, K, V>,
        curr: NodeRef<'a, K, V>,
        next: Option<NodeRef<'a, K, V>>,
        level: usize,
    ) -> Result<Option<NodeRef<'a, K, V>>, ()> {
        // The pointer to `next` is tagged to signal unlinking. 
        let next_ptr = next.as_ref().map_or(core::ptr::null_mut(), |n| n.as_ptr());

        if let Ok(_) = prev.levels[level].compare_exchange(curr.as_ptr(), next_ptr) {
            if curr.try_sub_ref().expect("not to underflow") == 0 {
                self.state.len.fetch_sub(1, Ordering::AcqRel);

                self.retire_node(curr.as_ptr());
            }
            Ok(next)
        } else {
            Err(())
        }
    }

    fn retire_node(&self, node_ptr: *mut Node<K, V>) {
        unsafe {
            self.garbage
                .domain
                .retire_ptr::<Node<K, V>, DeallocOnDrop<K, V>>(node_ptr)
                /*
                .retire_ptr_with(node_ptr, |ptr: *mut dyn Reclaim| {
                    Node::<K, V>::dealloc(ptr as *mut Node<K, V>);
                })
                */
        };
    }

    fn retire_node_empty(&self, node_ptr: *mut Node<K, V>) {
        unsafe {
            self.garbage
                .domain
                .retire_ptr::<Node<K, V>, DeallocOnDrop<K, V>>(node_ptr)
                /*
                .retire_ptr_with(node_ptr, |ptr: *mut dyn haphazard::raw::Reclaim| {
                    Node::<K, V>::dealloc(ptr as *mut Node<K, V>);
                })
                */
        };
    }

    fn find<'a>(&'a self, key: &K, search_closest: bool) -> SearchResult<'a, K, V> {
        let head = unsafe { &(*self.head.as_ptr()) };

        let mut prev = unsafe {
            let mut prev: [core::mem::MaybeUninit<(NodeRef<'a, K, V>, Option<NodeRef<'a, K, V>>)>; HEIGHT] 
                = core::mem::MaybeUninit::uninit().assume_init();

            for (i, level) in prev.iter_mut().enumerate() {
                core::ptr::write(
                    level.as_mut_ptr(), 
                    (NodeRef::from_raw(self.head.cast::<Node<K, V>>().as_ptr()), NodeRef::from_maybe_tagged(&self.head.as_ref().levels[i]))
                )
            }

            core::mem::transmute::<_, [(NodeRef<'a, K, V>, Option<NodeRef<'a, K, V>>); HEIGHT]>(prev)
        };


        '_search: loop {
            let mut level = self.state.max_height.load(Ordering::Relaxed);
            // Find the first and highest node tower
            while level > 1 && head.levels[level - 1].load_ptr().is_null() {
                level -= 1;
            }

            // We need not protect the head, as it will always be valid, as long as we are in a sane
            // state.
            let mut curr = NodeRef::from_raw(self.head.as_ptr().cast::<Node<K, V>>());

            // steps:
            // 1. Go through each level until we reach a node with a key GEQ to ours or that is null
            //     1.1 If we are equal, then the node must either be marked as removed or removed nodes
            //       are allowed in this search.
            //       Should this be the case, then we drop down a level while also protecting a pointer
            //       to the current node, in order to keep the `Level` valid in our `prev` array.
            //     1.2 If we the `next` node is less or equal but removed and removed nodes are
            //       disallowed, then we set our current node to the next node.
            while level > 0 {
                let next = unsafe {
                    let mut next = NodeRef::from_maybe_tagged(&curr.levels[level - 1]);
                    loop {
                        if next.is_none() {
                            break next;
                        }

                        if let Some(n) = next.as_ref() {
                            if n.levels[level - 1].load_tag() == 0 {
                                break next;
                            }
                        }

                        let n = next.unwrap();

                        let new_next = NodeRef::from_maybe_tagged(&n.levels[level - 1]);

                        let Ok(n) = self.unlink_level(&curr, n, new_next, level - 1) else {
                            continue '_search;
                        };

                        next = n

                    }
                };

                match next {
                    Some(next) 
                        // This check should ensure that we always get a non-removed node, if there
                        // is one, of our target key, as long as allow removed is set to false.
                        if (*next).key < *key => {

                        // If the current node is being removed, we try to help unlinking it at this level.
                        // Update previous_nodes.
                        prev[level - 1] = (curr, Some(next.clone()));

                        curr = next;
                    },
                    next => {
                        // Update previous_nodes.
                        prev[level - 1] = (curr.clone(), next);

                        level -= 1;
                    }
                }
            }

            unsafe {
                return if search_closest {
                    let mut next = NodeRef::from_maybe_tagged(&curr.levels[level - 1]);
                    loop {
                        if next.is_none() {
                            break;
                        }

                        if let Some(n) = next.as_ref() {
                            if n.levels[level - 1].load_tag() == 0 {
                                break;
                            }
                        }

                        let n = next.unwrap();

                        let new_next = NodeRef::from_maybe_tagged(&n.levels[level - 1]);

                        let Ok(n) = self.unlink_level(&curr, n, new_next, level - 1) else {
                            continue '_search;
                        };

                        next = n
                    }

                    SearchResult { prev, target: next }
                } else {
                    match NodeRef::from_maybe_tagged(&prev[0].0.as_ref().levels[0]) {
                        Some(next) if next.key == *key && !next.removed() => SearchResult { prev, target: Some(next) },
                        _ => SearchResult { prev, target: None }
                    }
                }
            }
        }
    }

    pub fn get<'a>(&'a self, key: &K) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        // Perform safety check for whether we are dealing with the head.
        match self.find(key, false) {
            SearchResult {
                target: Some(target),
                ..
            } => Some(Entry::from(target)),
            _ => None,
        }
    }

    fn is_head(&self, ptr: *const Node<K, V>) -> bool {
        std::ptr::eq(ptr, self.head.as_ptr().cast())
    }

    fn next_node<'a>(&'a self, node: &Entry<'a, K, V>) -> Option<Entry<'a, K, V>> {
        let node: &NodeRef<'_, _, _> = unsafe { core::mem::transmute(node) };

        // This means we have a stale node and cannot return a sane answer!
        if node.levels[0].load_tag() == 1 {
            return self.find(&node.key, true).target.map(|t| t.into())
        };

        let mut next = NodeRef::from_maybe_tagged(&node.levels[0])?;
        
        // Unlink and skip all removed `Node`s we may encounter.
        while next.levels[0].load_tag() == 1 {
            let new = NodeRef::from_maybe_tagged(&next.levels[0]);
            next = unsafe {
                self.unlink_level(&node, next, new, 0)
                    .ok()
                    .unwrap_or_else(|| self.find(&node.key, true).target)?
            };
        }

        Some(next.into())
    }

    pub fn get_first<'a>(&'a self) -> Option<Entry<'a, K, V>> {
        if self.is_empty() {
            return None;
        }

        let curr = NodeRef::from_raw(self.head.as_ptr().cast::<Node<K, V>>());

        self.next_node(&curr.into())
    }

    pub fn get_last<'a>(&'a self) -> Option<Entry<'a, K, V>> {
        let mut curr = self.get_first()?;

        while let Some(next) = self.next_node(&curr) {
            curr = next;
        }

        return Some(curr.into())
    }

    fn traverse_with<F>(&self, mut f: F) where F: FnMut(&K, &V) {
        let mut curr = self.get_first();

        while let Some(c) = curr {
            let k = c.key();
            let v = c.val();
            f(&k, &v);

            curr = self.next_node(&c);
        }
    }

    pub fn iter<'a>(&'a self) -> Iter<'a, K, V> {
        Iter::from_list(self)
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

unsafe impl<'domain, K, V> Send for SkipList<'domain, K, V>
where
    K: Send + Sync,
    V: Send + Sync,
{
}

unsafe impl<'domain, K, V> Sync for SkipList<'domain, K, V>
where
    K: Send + Sync,
    V: Send + Sync,
{
}

// TODO Make sure this is sound!
impl<'domain, K, V> From<super::skiplist::SkipList<'domain, K, V>> for SkipList<'domain, K, V>
where
    K: Sync,
    V: Sync,
{
    fn from(list: super::skiplist::SkipList<'domain, K, V>) -> Self {
        unsafe { core::mem::transmute(list) }
    }
}


#[allow(dead_code)]
pub struct Entry<'a, K: 'a, V: 'a> {
    node: core::ptr::NonNull<Node<K, V>>,
    _hazard: haphazard::HazardPointer<'a, Global>,
}

impl<'a, K, V> Entry<'a, K, V> {
    pub fn val(&self) -> &V {
        // #Safety
        //
        // Our `HazardPointer` ensures that our pointers is valid.
        unsafe { &self.node.as_ref().val }
    }

    pub fn key(&self) -> &K {
        // #Safety
        //
        // Our `HazardPointer` ensures that our pointers is valid.
        unsafe { &self.node.as_ref().key }
    }

    pub fn remove(self) -> Option<(K, V)> {
        unsafe {
            self.node.as_ref().set_removed().ok()?;

            let (key, val) = (
                core::ptr::read(&self.node.as_ref().key),
                core::ptr::read(&self.node.as_ref().val),
            );

            self.node.as_ref().tag_levels(1).expect("no tags to exists");

            (key, val).into()
            
        }
    }
}

struct SearchResult<'a, K, V> {
    prev: [(NodeRef<'a, K, V>, Option<NodeRef<'a, K, V>>); HEIGHT],
    target: Option<NodeRef<'a, K, V>>,
}

impl<'a, K, V> Debug for SearchResult<'a, K, V>
where
    K: Debug + Default,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchResult")
            .field("target", &self.target)
            .finish()
    }
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

#[allow(dead_code)]
struct NodeRef<'a, K, V> {
    node: NonNull<Node<K, V>>,
    _hazard: HazardPointer<'a>
}

impl<'a, K, V> NodeRef<'a, K, V> {
    fn from_raw_in(ptr: *mut Node<K, V>, domain: &'a Domain<Global>) -> Self {
        let mut _hazard = HazardPointer::new_in_domain(domain);
        _hazard.protect_raw(ptr);
        unsafe {
            NodeRef { node: NonNull::new_unchecked(ptr), _hazard }
        }
    }

    fn from_raw(ptr: *mut Node<K, V>) -> Self {
        Self::from_raw_in(ptr, Domain::global())
    }

    fn from_ptr_in(ptr: &AtomicPtr<Node<K, V>>, domain: &'a Domain<Global>) -> Option<Self> {
        let mut _hazard = HazardPointer::new_in_domain(domain);
        let node = _hazard.protect_ptr(ptr)?.0;

        Some(NodeRef { node, _hazard })
    }

    fn from_ptr(ptr: &AtomicPtr<Node<K, V>>) -> Option<Self> {
        Self::from_ptr_in(ptr, Domain::global())
    }

    fn as_ptr(&self) -> *mut Node<K, V> {
        self.node.as_ptr()
    }
}

impl<'a, K, V> AsRef<Node<K, V>> for NodeRef<'a, K, V> {
    fn as_ref(&self) -> &Node<K, V> {
        unsafe { &(*self.as_ptr()) }
    }
}

impl<'a, K, V> core::ops::Deref for NodeRef<'a, K, V> {
    type Target = Node<K, V>;
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'a, K, V> core::ops::DerefMut for NodeRef<'a, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut (*self.as_ptr()) }
    }
}

impl<'a, K, V> core::fmt::Debug for NodeRef<'a, K, V> 
where 
    K: Debug, 
    V: Debug 
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            f.debug_struct("NodeRef").field("node", self.node.as_ref()).finish()
        }
    }
}

impl<'a, K, V> From<NodeRef<'a, K, V>> for Entry<'a, K, V> {
    fn from(value: NodeRef<'a, K, V>) -> Self {
        unsafe { core::mem::transmute(value) }
    }
}

impl<'a, K, V> Clone for NodeRef<'a, K, V> {
    fn clone(&self) -> Self {
        let mut _hazard = HazardPointer::new();
        _hazard.protect_raw(self.node.as_ptr());

        NodeRef { node: self.node.clone(), _hazard }
    }
}

impl<'a, K, V> core::cmp::PartialEq for NodeRef<'a, K, V> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.node.as_ptr(), other.node.as_ptr())
    }
}

impl<'a, K, V> core::cmp::Eq for NodeRef<'a, K, V> {}

#[repr(transparent)]
struct DeallocOnDrop<K, V>(*mut Node<K, V>);

unsafe impl<K, V> Send for DeallocOnDrop<K, V> 
where K: Send + Sync,
      V: Send + Sync
{
}

unsafe impl<K, V> Sync for DeallocOnDrop<K, V> 
where K: Send + Sync,
      V: Send + Sync
{
}

impl<K, V> From<*mut Node<K, V>> for DeallocOnDrop<K, V> {
    fn from(node: *mut Node<K, V>) -> Self {
        DeallocOnDrop(node)
    }
}

impl<K, V> Drop for DeallocOnDrop<K, V> {
    fn drop(&mut self) {
        unsafe {
            Node::dealloc(self.0)
        }
    }
}

unsafe impl<K, V> Pointer<Node<K, V>> for DeallocOnDrop<K, V> {
    fn into_raw(self) -> *mut Node<K, V> {
        self.0
    }

    unsafe fn from_raw(ptr: *mut Node<K, V>) -> Self {
        DeallocOnDrop::from(ptr)
    }
}

impl<K, V> core::ops::Deref for DeallocOnDrop<K, V> {
    type Target = Node<K, V>;

    fn deref(&self) -> &Self::Target {
        unsafe { &(*self.0) }
    }
}

impl<K, V> core::ops::DerefMut for DeallocOnDrop<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {&mut (*self.0)}
    }
}

#[cfg(test)]
mod sync_test {
    use rand::Rng;

    use super::*;

    #[test]
    fn test_new_node_sync() {
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
    fn test_new_list_sync() {
        let _: SkipList<'_, usize, usize> = SkipList::new();
    }

    #[test]
    fn test_insert_sync() {
        let list = SkipList::new();
        let mut rng: u16 = rand::random();

        for _ in 0..10_000 {
            rng ^= rng << 3;
            rng ^= rng >> 12;
            rng ^= rng << 7;
            list.insert(rng, "hello there!");
        }
    }

    #[test]
    fn test_rand_height_sync() {
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
    fn test_insert_verbose_sync() {
        let list = SkipList::new();

        list.insert(1, 1);

        list.iter().for_each(|n| println!("k: {},", n.key()));

        list.insert(2, 2);

        list.iter().for_each(|n| println!("k: {},", n.key()));

        list.insert(5, 3);

        list.iter().for_each(|n| println!("k: {},", n.key()));
    }

    #[test]
    fn test_remove() {
        let list = SkipList::new();
        let mut rng: u16 = rand::random();

        for _ in 0..10_000 {
            rng ^= rng << 3;
            rng ^= rng >> 12;
            rng ^= rng << 7;
            list.insert(rng, "hello there!");
        }
        for _ in 0..10_000 {
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

        list.iter().for_each(|n| println!("k: {},", n.key()));

        assert!(list.remove(&1).is_some());

        list.iter().for_each(|n| println!("k: {},", n.key()));

        println!("removing 6");
        assert!(list.remove(&6).is_none());
        println!("removing 1");
        assert!(list.remove(&1).is_none());
        println!("removing 5");
        assert!(list.remove(&5).is_some());
        println!("removing 2");
        assert!(list.remove(&2).is_some());

        list.iter().for_each(|n| println!("k: {},", n.key()));

        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_find_removed() {
        let list = SkipList::new();

        list.insert(3, ());

        list.insert(4, ());

        list.insert(5, ());

        assert!(list.find(&3, false).target.is_some());
        assert!(list.find(&4, false).target.is_some());

        // manually get reference to the nodes
        let node_3 = unsafe { &mut (*(*list.head.as_ptr()).levels[0].load_ptr()) };
        let node_4 =
            unsafe { &mut (*(*(*list.head.as_ptr()).levels[0].load_ptr()).levels[0].load_ptr()) };
        let node_5 = unsafe {
            &mut (*(*(*(*list.head.as_ptr()).levels[0].load_ptr()).levels[0].load_ptr()).levels[0]
                .load_ptr())
        };

        // make sure it is the right node
        assert_eq!(node_3.key, 3);
        println!("{:?}", node_3);
        assert_eq!(node_4.key, 4);
        println!("{:?}", node_4);
        assert_eq!(node_5.key, 5);
        println!("{:?}", node_5);

        // remove the node logically
        let _ = node_4.set_removed();

        assert!(list.find(&4, false).target.is_none());

        println!("{:?}", list.find(&3, false));

        assert!(!node_3.removed());

        assert!(list.remove(&4).is_none());

        // remove the node logically
        node_4.height_and_removed.store(
            node_4.height_and_removed.load(Ordering::SeqCst) & (usize::MAX >> 1),
            Ordering::SeqCst,
        );

        assert!(!node_4.removed());

        assert!(list.remove(&4).is_some());
    }

    #[test]
    fn test_sync_remove() {
        use std::sync::Arc;
        let list = Arc::new(SkipList::new());
        let mut rng = rand::thread_rng();

        for _ in 0..10_000 {
            list.insert(rng.gen::<u16>(), ());
        }
        let threads = (0..20)
            .map(|_| {
                let list = list.clone();
                std::thread::spawn(move || {
                    let mut rng = rand::thread_rng();
                    for _ in 0..1_000 {
                        let target = &rng.gen::<u16>();
                        list.remove(&target);
                    }
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap()
        }

        list.traverse_with(|k, _| println!("key: {}", k));
    }

    #[test]
    fn test_sync_insert() {
        use std::sync::Arc;
        let list = Arc::new(SkipList::new());

        let threads = (0..20)
            .map(|_| {
                let list = list.clone();
                std::thread::spawn(move || {
                    let mut rng = rand::thread_rng();
                    for _ in 0..1_000 {
                        let target = rng.gen::<u8>();

                        list.insert(target, ());
                    }
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap()
        }

        list.traverse_with(|k, _| println!("key: {}", k));
    }

    #[test]
    fn test_sync_inmove() {
        use std::sync::Arc;
        let list = Arc::new(SkipList::new());

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

        list.traverse_with(|k, _| println!("key: {}", k));
    }

    #[test]
    fn test_sync_iterate() {
        use std::sync::Arc;
        let list = Arc::new(SkipList::new());

        let threads = (0..20)
            .map(|_| {
                let list = list.clone();
                std::thread::spawn(move || {
                    let mut rng = rand::thread_rng();
                    for _ in 0..1_000 {
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

        for _ in 0..5 {
            list.iter().for_each(|e| println!("key: {}", e.key()));
        }

        for thread in threads {
            thread.join().unwrap()
        }

        let list = Arc::<SkipList<'_, u8, ()>>::try_unwrap(list).unwrap();

        list.into_iter().for_each(|(k, _)| println!("key: {}", k))
    }
}
