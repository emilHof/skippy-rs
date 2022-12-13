pub trait SkipList<K, V> {
    type Entry<'a>
    where
        K: 'a,
        V: 'a;

    fn new() -> Self;

    fn insert(&mut self, key: K, value: V) -> Option<V>;

    fn get<'a>(&self, key: &K) -> Option<Self::Entry<'a>>;

    fn remove(&mut self, key: &K) -> Option<(K, V)>;

    fn front<'a>(&self) -> Option<Self::Entry<'a>>;

    fn last<'a>(&self) -> Option<Self::Entry<'a>>;

    fn len(&self) -> usize;
}
