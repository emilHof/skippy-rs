extern crate alloc;

use alloc::alloc::{alloc, dealloc, handle_alloc_error, Layout};

use core::{
    fmt::Debug,
    mem,
    ops::Index,
    ptr::{self, NonNull},
    sync::atomic::AtomicPtr,
};

pub(crate) const HEIGHT_BITS: usize = 5;

pub(crate) const HEIGHT: usize = 1 << HEIGHT_BITS;

/// Head stores the first pointer tower at the beginning of the list. It is always of maximum
#[repr(C)]
pub(crate) struct Head<K, V> {
    pub(crate) key: K,
    pub(crate) val: V,
    pub(crate) height: usize,
    pub(crate) levels: Levels<K, V>,
}

impl<K, V> Head<K, V> {
    pub(crate) fn new() -> NonNull<Self> {
        let head_ptr = unsafe { Node::<K, V>::alloc(HEIGHT).cast() };

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
    pub(crate) pointers: [[AtomicPtr<Node<K, V>>; 2]; 1],
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

impl<K, V> Debug for Levels<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:?}, {:?}]",
            self.pointers[0][0].load(std::sync::atomic::Ordering::Relaxed),
            self.pointers[0][1].load(std::sync::atomic::Ordering::Relaxed)
        )
    }
}

#[repr(C)]
pub(crate) struct Node<K, V> {
    pub(crate) key: K,
    pub(crate) val: V,
    pub(crate) height: usize,
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

        ptr::write(&mut (*ptr).height, height);

        ptr::write_bytes((*ptr).levels.pointers.as_mut_ptr(), 0, height);

        ptr
    }

    pub(crate) unsafe fn dealloc(ptr: *mut Self) {
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

    pub(crate) unsafe fn drop(ptr: *mut Self) {
        ptr::drop_in_place(&mut (*ptr).key);
        ptr::drop_in_place(&mut (*ptr).val);

        Self::dealloc(ptr);
    }
}

pub(crate) trait GeneratesHeight {
    fn gen_height(&self) -> usize;
}

macro_rules! skiplist_basics {
    ($my_list: ident) => {
        pub struct $my_list<K, V> {
            head: core::ptr::NonNull<crate::internal::skiplist::Head<K, V>>,
            state: crate::internal::skiplist::ListState,
        }

        impl<K, V> $my_list<K, V> {
            pub fn new() -> Self {
                $my_list {
                    head: crate::internal::skiplist::Head::new(),
                    state: crate::internal::skiplist::ListState::new(),
                }
            }

            pub fn len(&self) -> usize {
                self.state.len.load(Ordering::Relaxed)
            }

            pub fn is_empty(&self) -> bool {
                self.state.len.load(Ordering::Relaxed) < 1
            }

            fn gen_height(&self) -> usize {
                let mut seed = self.state.seed.load(Ordering::Relaxed);
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;

                self.state.seed.store(seed, Ordering::Relaxed);

                let mut height = std::cmp::min(
                    crate::internal::utils::HEIGHT,
                    seed.trailing_zeros() as usize + 1,
                );

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

        impl<K, V> GeneratesHeight for $my_list<K, V> {
            fn gen_height(&self) -> usize {
                self.gen_height()
            }
        }

        impl<K, V> Drop for $my_list<K, V> {
            fn drop(&mut self) {
                let mut node =
                    unsafe { (*self.head.as_ptr()).levels[0][1].load(Ordering::Relaxed) };

                while !node.is_null() {
                    unsafe {
                        let temp = node;
                        node = (*temp).levels[0][1].load(Ordering::Relaxed);
                        crate::internal::skiplist::Node::<K, V>::drop(temp);
                    }
                }

                unsafe { crate::internal::skiplist::Head::<K, V>::drop(self.head) };
            }
        }
    };
}

pub(crate) use skiplist_basics;
