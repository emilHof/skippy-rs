use crate::internal::utils::Node;

use super::{Entry, SkipList};
use core::iter::{FromIterator, IntoIterator, Iterator};

pub struct Iter<'a, K, V> {
    list: &'a SkipList<'a, K, V>,
    next: Option<Entry<'a, K, V>>,
}

impl<'a, K, V> Iter<'a, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    pub fn from_list(list: &'a SkipList<'a, K, V>) -> Self {
        Self {
            list,
            next: list.get_first(),
        }
    }
}

impl<'a, K, V> core::iter::Iterator for Iter<'a, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    type Item = Entry<'a, K, V>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(next) = self.next.take() {
            self.next = self.list.next_node(&next);
            return Some(next);
        }

        None
    }
}

impl<'a, K, V> IntoIterator for SkipList<'a, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;
    fn into_iter(self) -> Self::IntoIter {
        IntoIter::from_list(self)
    }
}

impl<'a, K, V> FromIterator<(K, V)> for SkipList<'a, K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        let list = Self::new();
        for (k, v) in iter {
            list.insert(k, v);
        }

        list
    }
}

pub struct IntoIter<K, V> {
    next: *mut Node<K, V>,
}

impl<K, V> IntoIter<K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    pub fn from_list<'a>(mut list: SkipList<'a, K, V>) -> Self {
        unsafe {
            let next = list.head.as_ref().levels[0].load_ptr();
            for level in list.head.as_mut().levels.pointers.iter_mut() {
                level.store_ptr(core::ptr::null_mut());
            }

            IntoIter { next }
        }
    }
}

impl<K, V> core::iter::Iterator for IntoIter<K, V>
where
    K: Ord + Send + Sync,
    V: Send + Sync,
{
    type Item = (K, V);
    fn next(&mut self) -> Option<Self::Item> {
        if self.next.is_null() {
            return None;
        }

        let next = self.next;

        self.next = unsafe { (*next).levels[0].load_ptr() };

        let (key, val) = unsafe { (core::ptr::read(&(*next).key), core::ptr::read(&(*next).val)) };

        unsafe {
            Node::dealloc(next);
        }

        (key, val).into()
    }
}
