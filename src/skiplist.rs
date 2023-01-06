pub trait SkipList<K, V> {
    type Entry<'a>: Entry<'a, K, V>
    where
        Self: 'a;

    fn new() -> Self;

    fn insert(&self, key: K, value: V) -> Option<V>;

    fn get<'a>(&'a self, key: &K) -> Option<Self::Entry<'a>>;

    fn remove(&self, key: &K) -> Option<(K, V)>;

    fn front<'a>(&'a self) -> Option<Self::Entry<'a>>;

    fn last<'a>(&'a self) -> Option<Self::Entry<'a>>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() < 1
    }
}

pub trait Entry<'a, K, V> {
    fn val(&self) -> &V;
    fn key(&self) -> &K;
}
