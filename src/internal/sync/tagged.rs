use super::NodeRef;
use crate::internal::utils::Node;
use haphazard::{AtomicPtr, HazardPointer};

pub(crate) struct MaybeTagged<T>(AtomicPtr<T>);

impl<T> MaybeTagged<T> {
    pub(crate) fn load_ptr(&self) -> *mut T {
        self.load_decomposed().0
    }
    pub(crate) fn load_decomposed(&self) -> (*mut T, usize) {
        let raw = unsafe { self.0.as_std().load(std::sync::atomic::Ordering::Acquire) };
        Self::decompose_raw(raw)
    }

    #[inline]
    fn decompose_raw(raw: *mut T) -> (*mut T, usize) {
        (
            (raw as usize & !unused_bits::<T>()) as *mut T,
            raw as usize & unused_bits::<T>(),
        )
    }

    pub(crate) fn store_composed(&self, ptr: *mut T, tag: usize) {
        let tagged = Self::compose_raw(ptr, tag);

        unsafe {
            self.0
                .as_std()
                .store(tagged, std::sync::atomic::Ordering::Release);
        }
    }

    #[inline]
    fn compose_raw(ptr: *mut T, tag: usize) -> *mut T {
        ((ptr as usize & !unused_bits::<T>()) | (tag & unused_bits::<T>())) as *mut T
    }

    pub(crate) fn store_ptr(&self, ptr: *mut T) {
        self.store_composed(ptr, 0);
    }

    pub(crate) fn compare_exchange(
        &self,
        expected: *mut T,
        new: *mut T,
    ) -> Result<(*mut T, usize), (*mut T, usize)> {
        self.compare_exchange_with_tag(expected, 0, new, 0)
    }

    pub(crate) fn compare_exchange_with_tag(
        &self,
        expected: *mut T,
        e_tag: usize,
        new: *mut T,
        n_tag: usize,
    ) -> Result<(*mut T, usize), (*mut T, usize)> {
        unsafe {
            match self.0.as_std().compare_exchange(
                Self::compose_raw(expected, e_tag),
                Self::compose_raw(new, n_tag),
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            ) {
                Ok(new) => Ok(Self::decompose_raw(new)),
                Err(other) => Err(Self::decompose_raw(other)),
            }
        }
    }

    pub(crate) fn tag(&self, tag: usize) {
        let (mut old_ptr, mut old_tag) = self.load_decomposed();

        while let Err((other_ptr, other_tag)) =
            self.compare_exchange_with_tag(old_ptr, old_tag, old_ptr, tag)
        {
            (old_ptr, old_tag) = (other_ptr, other_tag);
        }
    }

    pub(crate) fn try_tag(&self, expected: *mut T, tag: usize) -> Result<*mut T, *mut T> {
        let old_tag = self.load_tag();
        self.compare_exchange_with_tag(expected, old_tag, expected, tag)
            .map(|s| s.0)
            .map_err(|e| e.0)
    }

    pub(crate) fn compare_exchange_tag(&self, e_tag: usize, tag: usize) -> Result<usize, usize> {
        let mut ptr = self.load_ptr();
        while let Err((other_ptr, other_tag)) = self.compare_exchange_with_tag(ptr, e_tag, ptr, tag)
        {
            if other_tag != e_tag {
                return Err(other_tag);
            }

            ptr = other_ptr;
        }

        Ok(tag)
    }

    pub(crate) fn load_tag(&self) -> usize {
        self.load_decomposed().1
    }

    pub(crate) fn as_std(&self) -> &core::sync::atomic::AtomicPtr<T> {
        unsafe { self.0.as_std() }
    }

    pub(crate) fn as_hpz(&self) -> &AtomicPtr<T> {
        &self.0
    }
}

const fn align<T>() -> usize {
    core::mem::align_of::<T>()
}

const fn unused_bits<T>() -> usize {
    (1 << align::<T>().trailing_zeros()) - 1
}

impl<'a, K, V> NodeRef<'a, K, V> {
    pub(crate) fn from_maybe_tagged(maybe_tagged: &MaybeTagged<Node<K, V>>) -> Option<Self> {
        let mut _hazard = HazardPointer::new();
        let mut ptr = maybe_tagged.load_ptr();

        _hazard.protect_raw(ptr);

        let mut v_ptr = maybe_tagged.load_ptr();

        while !core::ptr::eq(ptr, v_ptr) {
            ptr = v_ptr;
            _hazard.protect_raw(ptr);

            v_ptr = maybe_tagged.load_ptr();
        }

        if ptr.is_null() {
            None
        } else {
            unsafe {
                Some(NodeRef {
                    node: core::ptr::NonNull::new_unchecked(ptr),
                    _hazard,
                })
            }
        }
    }
}
