extern crate alloc;

use alloc::alloc::{alloc, dealloc, handle_alloc_error, Layout};
use haphazard::AtomicPtr;

const REMOVED_MASK: u32 = (1 as u32) << 31;

use core::{
    fmt::Debug,
    fmt::Display,
    mem,
    ops::Index,
    ptr::{self, NonNull},
    sync::atomic::AtomicU32,
    sync::atomic::Ordering,
};

/// Head stores the first pointer tower at the beginning of the list. It is always of maximum
#[repr(C)]
pub(crate) struct Head<K, V> {
    pub(crate) key: K,
    pub(crate) val: V,
    pub(crate) height_and_removed: AtomicU32,
    pub(crate) levels: Levels<K, V>,
}

impl<K, V> Head<K, V> {
    pub(crate) fn new() -> NonNull<Self> {
        let head_ptr = unsafe { Node::<K, V>::alloc(super::HEIGHT).cast() };

        if let Some(head) = NonNull::new(head_ptr) {
            head
        } else {
            panic!()
        }
    }

    pub(crate) unsafe fn drop(ptr: NonNull<Self>) {
        Node::<K, V>::dealloc(ptr.as_ptr().cast());
    }
}

#[repr(C)]
pub(crate) struct Levels<K, V> {
    pub(crate) pointers: [AtomicPtr<Node<K, V>>; 1],
}

impl<K, V> Levels<K, V> {
    fn get_size(height: usize) -> usize {
        assert!(height <= super::HEIGHT && height > 0);

        mem::size_of::<Self>() * (height - 1)
    }
}

impl<K, V> Index<usize> for Levels<K, V> {
    type Output = AtomicPtr<Node<K, V>>;

    fn index(&self, index: usize) -> &Self::Output {
        unsafe { self.pointers.get_unchecked(index) }
    }
}

#[repr(C)]
pub(crate) struct Node<K, V> {
    pub(crate) key: K,
    pub(crate) val: V,
    pub(crate) height_and_removed: AtomicU32,
    pub(crate) levels: Levels<K, V>,
}

impl<K, V> Node<K, V> {
    pub(crate) fn new(key: K, val: V, height: usize) -> *mut Self {
        unsafe {
            let node = Self::alloc(height);

            ptr::write(&mut (*node).key, key);
            ptr::write(&mut (*node).val, val);
            node
        }
    }

    pub(crate) fn new_rand_height(
        key: K,
        val: V,
        list: &impl crate::internal::utils::GeneratesHeight,
    ) -> *mut Self {
        // construct the base nod
        Self::new(key, val, list.gen_height())
    }

    pub(crate) unsafe fn alloc(height: usize) -> *mut Self {
        let layout = Self::get_layout(height);

        let ptr = alloc(layout).cast::<Self>();

        if ptr.is_null() {
            handle_alloc_error(layout);
        }

        ptr::write(
            &mut (*ptr).height_and_removed,
            AtomicU32::new(height as u32),
        );

        ptr::write_bytes((*ptr).levels.pointers.as_mut_ptr(), 0, height);

        ptr
    }

    pub(crate) unsafe fn dealloc(ptr: *mut Self) {
        let height = (*ptr).height();

        let layout = Self::get_layout(height);
        println!("deallocating {:?}", ptr);

        dealloc(ptr.cast(), layout);
    }

    unsafe fn get_layout(height: usize) -> Layout {
        let size_self = mem::size_of::<Self>();
        let align = mem::align_of::<Self>();
        let size_levels = Levels::<K, V>::get_size(height);

        Layout::from_size_align_unchecked(size_self + size_levels, align)
    }

    pub(crate) unsafe fn drop(ptr: *mut Self) {
        ptr::drop_in_place(&mut (*ptr).key);
        ptr::drop_in_place(&mut (*ptr).val);

        Self::dealloc(ptr);
    }

    pub(crate) fn height(&self) -> usize {
        (self.height_and_removed.load(Ordering::Relaxed) & (u32::MAX ^ REMOVED_MASK)) as usize
    }

    pub(crate) fn removed(&self) -> bool {
        self.height_and_removed
            .load(Ordering::SeqCst)
            .leading_zeros()
            == 0
    }

    pub(crate) fn set_removed(&self) -> Result<u32, ()> {
        let height_and_removed = self.height_and_removed.load(Ordering::SeqCst);
        // if removed is set then someone else is already removing the node
        if height_and_removed.leading_zeros() == 0 || self.removed() {
            return Err(());
        }

        // set removed
        let new_height_and_removed = height_and_removed | REMOVED_MASK;

        // try to exchange
        self.height_and_removed
            .compare_exchange(
                height_and_removed,
                new_height_and_removed,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .map_err(|_| ())
    }

    pub(crate) fn set_height(&self, height: usize) {
        assert!(height <= self.height());

        let old_height_and_removed = self.height_and_removed.load(Ordering::SeqCst);

        let new_height_and_removed = (old_height_and_removed & REMOVED_MASK) | height as u32;

        while let Err(other) = self.height_and_removed.compare_exchange(
            old_height_and_removed,
            new_height_and_removed,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            if (other << 1 >> 1) <= height as u32 {
                break;
            }
        }
    }
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
        f.debug_struct("Node")
            .field("key", &self.key)
            .field("val", &self.val)
            .field("height", &self.height())
            .field(
                "levels",
                &(0..self.height()).fold(String::new(), |acc, level| {
                    format!("{}{:?}, ", acc, self.levels[level])
                }),
            )
            .finish()
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

#[cfg(test)]
mod node_test {
    use super::*;

    #[test]
    fn test_set_height() {
        let node = unsafe { &mut (*Node::new(3, (), 5)) };

        assert_eq!(node.height(), 5);

        assert!(!node.removed());

        assert!(node.set_removed().is_ok());

        assert!(node.removed());

        assert_eq!(node.height(), 5);

        node.set_height(3);

        assert_eq!(node.height(), 3);

        assert!(node.removed());
    }
}
