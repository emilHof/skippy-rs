use core::{borrow::Borrow, fmt::Debug, marker::Sync, sync::atomic::Ordering};
use std::ptr::NonNull;

use haphazard::{raw::Reclaim, Global, HazardPointer, HazardPointerArray};

use crate::{
    internal::utils::{skiplist_basics, GeneratesHeight, Levels, Node, HEIGHT},
    skiplist,
};

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
            let insertion_point = self.find(&key, false);

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
                    mut prev,
                    level_hazards: mut _level_hazards,
                    ..
                } => {
                    let new_node = Node::new_rand_height(key, val, self);

                    // Protects the new_node so concurrent removals do not invalidate our pointer.
                    let mut hazard = HazardPointer::new_in_domain(&self.garbage.domain);
                    hazard.protect_raw(new_node);

                    let mut starting_height = 0;

                    while let Err(starting) =
                        self.link_nodes(new_node, prev, starting_height)
                    {
                        // println!("error, retrying...");
                        // println!("failed to build full height {:?} at height: {}", new_node, starting);
                        (prev, starting_height, _level_hazards) = {
                            let SearchResult {
                                prev,
                                level_hazards,
                                target,
                            } = self.find(&(*new_node).key, false);
                            if let Some(target) = target {
                                if starting == 0 {
                                    // println!("target: {:?}, new: {:?}", target.0.as_ptr(), new_node);
                                    std::mem::swap(
                                        &mut (*target.0.as_ptr()).val,
                                        &mut (*new_node).val,
                                    );

                                    self.retire_node(new_node);

                                    return None;
                                }
                                /*
                                println!(
                                    "node already exists! t: {:?}, new: {:?}",
                                    target.0, new_node
                                );
                                */
                            }
                            (prev, starting, level_hazards)
                        };
                    }

                    self.state.len.fetch_add(1, Ordering::Relaxed);

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
    unsafe fn link_nodes(
        &self,
        new_node: *mut Node<K, V>,
        previous_nodes: [(NonNull<Node<K, V>>, *mut Node<K, V>); HEIGHT],
        start_height: usize,
    ) -> Result<(), usize> {
        // iterate over all the levels in the new nodes pointer tower
        for i in start_height..(*new_node).height() {
            /*
            println!("accessing new {:?} at level {}", new_node, i);
            if (*new_node).height() > 31 {
                println!("height: {}, of {:?}", (*new_node).height(), new_node);
            }
            */
            let (prev, next) = previous_nodes[i];

            // we check if the next node is actually lower in key than our current node.
            if (!next.is_null()
                && (*next).key <= (*new_node).key
                 && !(*new_node).removed())
                || prev.as_ref().removed()
            {
                /*
                println!("{} or {}",!next.is_null()
                && (*next).key <= (*new_node).key
                && !(*next).removed(), prev.as_ref().removed() );
                println!("prev: {:?}, new: {:?}, next: {:?}, level: {}", prev.as_ptr(), new_node, next, i);
                if !next.is_null() && !self.is_head(prev.as_ptr()) {
                    println!(
                        "prev: {:?}, new: {:?}, next: {:?}, level: {}", 
                        (*(prev.as_ptr() as *mut Node<u8, ()>)).key, 
                        (*(new_node as *mut Node<u8, ()>)).key, 
                        (*(next as *mut Node<u8, ()>)).key, i
                    );
                }
                */
                return Err(i);
            }
            
            // Swap the previous' next node into the new_node's level
            // It could be the case that we link ourselves to the previous node, but just as we do
            // this `next` attempts to unlink itself and fails. So while we succeeded, `next`
            // repeats its search and finds that we are the next
            (*new_node).levels[i].as_std().store(next, Ordering::Release);

            // Swap the new_node into the previous' level. If the previous' level has changed since
            // the search, we repeat the search from this level.
            if let Err(_other) = prev.as_ref().levels[i].as_std().compare_exchange(
                next,
            //  ^^^^------------- Ensure that next is still the same.
                new_node,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                /*
                if !_other.is_null() && !next.is_null() {
                    println!(
                        "failed to swap in {:?} as expected was {:?} and other was {:?}!",
                        (*(new_node as *mut Node<u8, ()>)).key,
                        (*(next as *mut Node<u8, ()>)).key,
                        (*(_other as *mut Node<u8, ()>)).key,
                    );
                }
                */
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
        unsafe {
            match self.find(key, false) {
                SearchResult {
                    target: Some((target, mut _hazard)),
                    mut prev,
                    mut level_hazards,
                } => {
                    let mut target = target.as_ptr();
                    // println!("found target: {:?}", target);

                    // Set the target state to being removed
                    // If this errors, it is already being removed by someone else
                    // and thus we exit early.
                    // println!("checking if {:?} has been removed", target);

                    if (*target).set_removed().is_err() {
                        return None;
                    }

                    // println!("{:?} has not been removed yet", target);

                    //
                    let key = core::ptr::read(&(*target).key);
                    let val = core::ptr::read(&(*target).val);
                    let mut height = (*target).height();

                    'unlink: while let Err(new_height) = self.unlink(target, height, prev, level_hazards) {
                        (target, height, prev, _hazard, level_hazards) = {
                            loop {
                                if let SearchResult {
                                    target: Some((new_target, hazard)),
                                    prev,
                                    level_hazards,
                                } = self.find(&key, true)
                                {
                                    if !core::ptr::eq(new_target.as_ptr(), target) {
                                        continue;
                                    }
                                    // println!("retried search for: {:?}, prev is now {:?}", target.as_ptr(), &prev[0..target.as_ref().height()]);
                                    break (new_target.as_ptr(), new_height, prev, hazard, level_hazards)
                                } else {
                                    break 'unlink;
                                }
                            }
                        };
                    }

                    // TODO Ensure the safety of this!
                    // #Safety:
                    //
                    // we are the only thread that has permission to drop this node.
                    self.retire_node(target);

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
    ///
    /// # Safety
    /// 1. `node` is a protected pointer.
    /// 2. All pointers in `previous_nodes` are protected.
    /// 3. All indices in [0, height) are valid indices for `node.levels`.
    unsafe fn unlink(
        &self,
        node: *mut Node<K, V>,
        height: usize,
        previous_nodes: [(NonNull<Node<K, V>>, *mut Node<K, V>); HEIGHT],
        _level_hazards: HazardPointerArray<'domain, Global, HEIGHT>,
    ) -> Result<(), usize> {
        // safety check against UB caused by unlinking the head
        if self.is_head(node) {
            panic!()
        }

        // # Safety
        //
        // 1.-3. Some as method and covered by method caller.
        // 4. We are not unlinking the head. - Covered by previous safety check.
        unsafe {
            for (i, &(prev, next)) in previous_nodes.iter().enumerate().take(height).rev() {
                let new_next = (*node).levels[i].as_std().load(Ordering::SeqCst);

                // If someone has already unlinked the node at this level, we simply continue.
                if core::ptr::eq(prev.as_ref().levels[i].load_ptr(), new_next) {
                    continue;
                } 

                // Performs a compare_exchange, expecting the old value of the pointer to be the current
                // node. If it is not, we cannot make any reasonable progress, so we search again.
                if let Err(_) = prev.as_ref().levels[i].as_std().compare_exchange(
                    node,
                    new_next,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    return Err(i + 1);
                }

                if !core::ptr::eq(node, next) {
                    return Err(i + 1);
                }

                // We check if the previous node is being removed after we have already unlinked
                // from it as the prev nodes expects us to do this.
                // We still need to stop the unlink here, as we will have to relink to the actual,
                // lively previous node at this level as well.
                if prev.as_ref().removed() {
                    return Err(i + 1);
                }
            }
        }

        Ok(())
    }

    /// Unlink [Node](Node) `curr` at the given level of [Node](Node) `prev` by exchanging the pointer for `next`.
    ///
    /// # Safety
    ///
    /// 1. `prev`, `curr`, and `next`, are all protected accesses.
    unsafe fn unlink_level(
        prev: *mut Node<K, V>,
        curr: *mut Node<K, V>,
        next: *mut Node<K, V>,
        level: usize,
    ) -> Result<*mut Node<K, V>, ()> {
        (*prev).levels[level].as_std().compare_exchange(curr, next, Ordering::SeqCst, Ordering::SeqCst).map_err(|_| ())
    }


    fn retire_node(&self, node_ptr: *mut Node<K, V>) {
        unsafe {
            self.garbage
                .domain
                .retire_ptr_with(node_ptr, |ptr: *mut dyn Reclaim| {
                    Node::<K, V>::dealloc(ptr as *mut Node<K, V>);
                });
        }
    }

    unsafe fn find<'a>(&'a self, key: &K, allow_removed: bool) -> SearchResult<'a, K, V> {
        let mut level_hazards: HazardPointerArray<'a, Global, HEIGHT> =
            HazardPointer::many_in_domain(self.garbage.domain);

        let mut curr_hazard = HazardPointer::new_in_domain(self.garbage.domain);
        let mut next_hazard = HazardPointer::new_in_domain(self.garbage.domain);

        let head = unsafe { &(*self.head.as_ptr()) };

        let mut prev = [(self.head.cast::<Node<K, V>>(), core::ptr::null_mut()); HEIGHT];

        unsafe {
            for i in 0..HEIGHT {
                prev[i].1 = self.head.as_ref().levels[i].load_ptr();
            }
        }

        'search: loop {
            let mut level = self.state.max_height.load(Ordering::Relaxed);
            // Find the first and highest node tower
            while level > 1 && head.levels[level - 1].load_ptr().is_null() {
                level_hazards.as_refs()[level - 1].protect_raw(self.head.as_ptr());
                level -= 1;
            }
            // We need not protect the head, as it will always be valid, as long as we are in a sane
            // state.
            let mut curr = self.head.as_ptr().cast::<Node<K, V>>();

            // steps:
            // 1. Go through each level until we reach a node with a key GEQ to ours or that is null
            //     1.1 If we are equal, then the node must either be marked as removed or removed nodes
            //       are allowed in this search.
            //       Should this be the case, then we drop down a level while also protecting a pointer
            //       to the current node, in order to keep the `Level` valid in our `prev` array.
            //     1.2 If we the `next` node is less or equal but removed and removed nodes are
            //       disallowed, then we set our current node to the next node.
            while level > 0 {
                let next = unsafe { next_hazard.protect_ptr((*curr).levels[level - 1].as_std()).map(|n| n.0.as_ptr()) };

                unsafe {
                    /*
                    next.as_ref().map(|n| println!(
                            "accessing next: {:?}({}), from: {:?}({}) at level: {}, while searching for {}", 
                            n,
                            (*(*n as *mut Node<u8, ()>)).key, 
                            curr,
                            (*(curr as *mut Node<u8, ()>)).key, 
                            level - 1,
                            *(key as *const _ as *const u8)
                        ));
                            "accessing next: {:?}, from: {:?} at level: {}, while searching for {}", 
                            (*(*n as *mut Node<u8, ()>)).key, 
                            (*(curr as *mut Node<u8, ()>)).key, 
                            level - 1,
                            *(&key as *const _ as *const u8)
                    */

                    match next {
                        Some(next) 
                            // This check should ensure that we always get a non-removed node, if there
                            // is one, of our target key, as long as allow removed is set to false.
                            if (*next).key < *key => {
                            // If the current node is being removed, we try to help unlinking it at this level.
                            /*
                            if (*curr).removed() {
                                if let Err(_) = Self::unlink_level(prev[level - 1].0.as_ptr(), curr, next, level - 1) {
                                    continue 'search;
                                }
                                println!("successfully unlinked at level");
                            }
                            */
                            
                            // Update previous_nodes.
                            prev[level - 1] = (NonNull::new_unchecked(curr), next);
                            level_hazards.as_refs()[level - 1]
                                .protect_raw((curr as *const Node<K, V>).cast_mut());

                            curr = next;
                            curr_hazard.protect_raw((curr as *const Node<K, V>).cast_mut());
                        },
                        opt => {
                            let next = opt.unwrap_or(core::ptr::null_mut());
                            /*
                            if (*curr).removed() {
                                if let Err(_) = Self::unlink_level(prev[level - 1].0.as_ptr(), curr, next, level - 1) {
                                    continue 'search;
                                }
                                println!("helped unlink at level");
                            }
                            */
                            // Update previous_nodes.
                            prev[level - 1] = (NonNull::new_unchecked(curr), next);
                            level_hazards.as_refs()[level - 1]
                                .protect_raw((curr as *const Node<K, V>).cast_mut());

                            level -= 1;
                        }
                    };
                }
            }

            let next = next_hazard.protect_ptr(prev[0].0.as_ref().levels[0].as_std()).map_or(core::ptr::null_mut(), |next| next.0.as_ptr());

            unsafe {
                return if core::ptr::eq(next, prev[0].1)
                    && !next.is_null()
                    && (*next).key == *key
                    && (allow_removed || !(*next).removed()) {
                    SearchResult {
                        prev,
                        level_hazards,
                        target: Some((NonNull::new_unchecked(next), next_hazard)),
                    }
                } else {
                    SearchResult {
                        prev,
                        level_hazards,
                        target: None,
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
        unsafe {
            match self.find(key, false) {
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

        let mut hazard = HazardPointer::new_in_domain(self.garbage.domain);
        let mut next_hazard = HazardPointer::new_in_domain(self.garbage.domain);
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

impl<'domain, K, V> skiplist::SkipList<K, V> for SkipList<'domain, K, V>
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

// TODO Make sure this is sound!
impl<'domain, K, V> From<super::skiplist::SkipList<'domain, K, V>> for SkipList<'domain, K, V>
where
    K: Sync,
    V: Sync,
{
    fn from(list: super::skiplist::SkipList<'domain, K, V>) -> Self {
        let new = unsafe { core::ptr::read(&list as *const _ as *const Self) };
        core::mem::forget(list);
        new
    }
}

pub struct Entry<'a, K: 'a, V: 'a> {
    node: core::ptr::NonNull<Node<K, V>>,
    hazard: haphazard::HazardPointer<'a, Global>,
}

impl<'a, K, V> skiplist::Entry<'a, K, V> for Entry<'a, K, V> {
    fn val(&self) -> &V {
        // #Safety
        //
        // Our `HazardPointer` ensures that our pointers is valid.
        unsafe { &self.node.as_ref().val }
    }

    fn key(&self) -> &K {
        // #Safety
        //
        // Our `HazardPointer` ensures that our pointers is valid.
        unsafe { &self.node.as_ref().key }
    }
}

struct SearchResult<'a, K, V> {
    prev: [(NonNull<Node<K, V>>, *mut Node<K, V>); HEIGHT],
    level_hazards: haphazard::HazardPointerArray<'a, haphazard::Global, HEIGHT>,
    target: Option<(NonNull<Node<K, V>>, haphazard::HazardPointer<'a, Global>)>,
}

impl<'a, K, V> Debug for SearchResult<'a, K, V>
where
    K: Debug + Default,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            f.debug_struct("SearchResult")
                .field("target", &self.target.as_ref().map(|t| t.0.as_ref()))
                .finish()
        }
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

        for _ in 0..100_000 {
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

    #[test]
    fn test_find_removed() {
        let list = SkipList::new();

        list.insert(3, ());

        list.insert(4, ());

        list.insert(5, ());

        unsafe {
            assert!(list.find(&3, false).target.is_some());
            assert!(list.find(&4, false).target.is_some());
        }

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
        node_4.set_removed();

        unsafe {
            assert!(list.find(&4, false).target.is_none());

            assert!(list.find(&4, true).target.is_some());

            println!("{:?}", list.find(&3, false));
        }

        assert!(!node_3.removed());

        assert!(list.remove(&4).is_none());

        // remove the node logically
        node_4.height_and_removed.store(
            node_4.height_and_removed.load(Ordering::SeqCst) ^ (1 as u32) << 31,
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

        for _ in 0..1_000 {
            list.insert(rng.gen::<u8>(), ());
        }
        let threads = (0..30)
            .map(|_| {
                let list = list.clone();
                std::thread::spawn(move || {
                    let mut rng = rand::thread_rng();
                    for _ in 0..10_000 {
                        let target = &rng.gen::<u8>();
                        let success = list.remove(&target);
                        if success.is_some() {
                            println!("{:?}", success);
                        }
                    }
                    println!("all done!");
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap()
        }

        unsafe {
            let mut node = (*list.head.as_ptr()).levels[0].load_ptr();
            while !node.is_null() {
                println!("{:?}", *node);
                node = (*node).levels[0].load_ptr();
            }
        }
    }

    #[test]
    fn test_sync_insert() {
        use std::sync::Arc;
        let list = Arc::new(SkipList::new());

        let threads = (0..30)
            .map(|_| {
                let list = list.clone();
                std::thread::spawn(move || {
                    let mut rng = rand::thread_rng();
                    for _ in 0..10_000 {
                        let target = rng.gen::<u8>();

                        list.insert(target, ());
                    }
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap()
        }

        unsafe {
            let mut node = (*list.head.as_ptr()).levels[0].load_ptr();
            while !node.is_null() {
                println!("{:?} - {:?}", node, *node);
                node = (*node).levels[0].load_ptr();
            }
        }
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
                    for _ in 0..1_000 {
                        let target = rng.gen::<u8>();
                        if rng.gen::<u8>() % 5 == 0 {
                            list.remove(&target);
                        } else {
                            // println!("inserting: {}", target);
                            list.insert(target, ());
                        }
                    }
                    // println!("all done!");
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap()
        }

        unsafe {
            let mut node = (*list.head.as_ptr()).levels[0].load_ptr();
            while !node.is_null() {
                println!("{:?} - {:?}", node, *node);
                node = (*node).levels[0].load_ptr();
            }
        }
    }
}
