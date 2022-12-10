use std::{
    fmt::{Debug, Display},
    ptr,
};

use rand::{rngs::ThreadRng, RngCore};

const HEIGHT: usize = 1 << 4;

pub struct SkipList<K, V> {
    head: Node<K, V>,
    state: ListState,
}

impl<K, V> SkipList<K, V> {
    pub fn new() -> Self {
        SkipList {
            head: Node::null_full_height(),
            state: ListState {
                len: 0,
                height: HEIGHT,
                seed: rand::thread_rng(),
            },
        }
    }
}

pub struct Node<K, V> {
    level: usize,
    left: *mut Node<K, V>,
    right: *mut Node<K, V>,
    top: *mut Node<K, V>,
    down: *mut Node<K, V>,
    refd: RefdData<K, V>,
    owned: Option<RefdData<K, V>>,
}

impl<K, V> Node<K, V> {
    pub fn new(key: K, val: V, level: usize) -> Self {
        let refd = RefdData {
            key: Box::into_raw(Box::new(key)),
            val: Box::into_raw(Box::new(val)),
            height: Box::into_raw(Box::new(HEIGHT)),
        };

        Node {
            level,
            left: ptr::null_mut(),
            right: ptr::null_mut(),
            top: ptr::null_mut(),
            down: ptr::null_mut(),
            refd: refd.clone(),
            owned: Some(refd),
        }
    }

    fn new_refd(refd: RefdData<K, V>, level: usize) -> Self {
        Node {
            level,
            left: ptr::null_mut(),
            right: ptr::null_mut(),
            top: ptr::null_mut(),
            down: ptr::null_mut(),
            refd,
            owned: None,
        }
    }

    /// Constructs a full Node tower with null keys and values
    fn null_full_height() -> Self {
        let refd = RefdData {
            key: ptr::null_mut(),
            val: ptr::null_mut(),
            height: Box::into_raw(Box::new(HEIGHT)),
        };

        (1..HEIGHT)
            .rev()
            .fold(Node::new_refd(refd.clone(), HEIGHT), |top, level| {
                // construct a new Node with reference to the key and value
                let mut node = Node::new_refd(refd.clone(), level);
                // assign prev node to top
                node.top = Box::into_raw(Box::new(top));
                node
            })
    }

    fn new_rand_height<'a>(key: K, val: V, list: &'a mut SkipList<K, V>) -> Self {
        // construct the base node
        let mut base_node = Node::new(key, val, 1);

        // get mutable reference to the keys
        let refd: RefdData<K, V> = base_node.refd.clone();

        // construct towering nodes to probabilistic height
        let last_node = match (2..HEIGHT).try_fold(&mut base_node as *mut _, |down, level| {
            match list.state.seed.next_u32() % 2 {
                0 => Err(down),
                _ => {
                    let mut node = Node::new_refd(refd.clone(), level);
                    node.down = down;
                    let node_ptr = Box::into_raw(Box::new(node));
                    unsafe { (*down).top = node_ptr };
                    Ok(node_ptr)
                }
            }
        }) {
            Err(node) => node,
            Ok(node) => node,
        };

        unsafe {
            let height = (*last_node).level;
            *(*last_node).refd.height = height;
        }

        base_node
    }
}

impl<K, V> PartialEq for Node<K, V>
where
    K: PartialEq,
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        unsafe { *self.refd.key == *other.refd.key && *self.refd.val == *other.refd.val }
    }

    fn ne(&self, other: &Self) -> bool {
        unsafe { *self.refd.key != *other.refd.key || *self.refd.val != *other.refd.val }
    }
}

impl<K, V> Debug for Node<K, V>
where
    K: Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            write!(
                f,
                "Node {{ key:  {:?}, val: {:?}, level: {}}}",
                *self.refd.key, *self.refd.val, self.level
            )
        }
    }
}

impl<K, V> Display for Node<K, V>
where
    K: Display,
    V: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe {
            write!(
                f,
                "
                [key:  {}, val: {}, level: {}]\n
                {}
                ",
                *self.refd.key,
                *self.refd.val,
                self.level,
                if !self.top.is_null() {
                    format!("{}", *self.top)
                } else {
                    "".to_owned()
                }
            )
        }
    }
}

struct ListState {
    len: usize,
    height: usize,
    seed: ThreadRng,
}

struct RefdData<K, V> {
    key: *mut K,
    val: *mut V,
    height: *mut usize,
}

impl<K, V> Clone for RefdData<K, V> {
    fn clone(&self) -> Self {
        RefdData {
            key: self.key,
            val: self.val,
            height: self.height,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_new_node() {
        let node = Node::new(100, "hello", 1);
        let refd = RefdData {
            key: Box::into_raw(Box::new(100)),
            val: Box::into_raw(Box::new("hello")),
            height: Box::into_raw(Box::new(1)),
        };
        assert_eq!(
            Node {
                level: 1,
                left: ptr::null_mut(),
                right: ptr::null_mut(),
                top: ptr::null_mut(),
                down: ptr::null_mut(),
                refd: refd.clone(),
                owned: Some(refd)
            },
            node
        );
    }

    #[test]
    fn test_rand_height() {
        let mut list = SkipList::new();
        let mut node = Node::new_rand_height("Hello", "There!", &mut list);
        assert!(node.owned.is_some());
        let mut height = unsafe { *node.owned.as_ref().unwrap().height };
        println!("height: {}", height);
        println!("{}", node);

        let mut node = &mut node as *mut Node<_, _>;

        while !node.is_null() {
            height -= 1;
            unsafe {
                println!("level: {}", (*node).level);
                node = (*node).top
            }
        }

        assert_eq!(height, 0);
    }
}
