use std::{
    fmt::{Debug, Display},
    ptr,
};

use rand::{rngs::ThreadRng, RngCore};

const HEIGHT_BITS: usize = 5;

const HEIGHT: usize = 1 << HEIGHT_BITS;

pub struct SkipList<K, V> {
    head: Node<K, V>,
    state: ListState,
}

impl<K, V> SkipList<K, V> {
    pub fn new() -> Self {
        SkipList {
            head: unsafe { *Box::from_raw(Node::null_full_height()) },
            state: ListState {
                len: 0,
                height: HEIGHT,
                seed: rand::thread_rng(),
            },
        }
    }
}

impl<K, V> SkipList<K, V>
where
    K: Ord,
{
    pub fn insert(&mut self, key: K, val: V) -> bool {
        let insertion_point = self.internal_find(&key);

        unsafe {
            // check if the insertion_point is of the same key
            if (*insertion_point)
                .key
                .as_ref()
                .map(|other| *other == key)
                .unwrap_or(false)
            {
                // if so, replace the value
                return (*insertion_point).val.replace(val).is_some();
            }

            let new_node = Node::new_rand_height(key, val, self);

            let mut shared_height = 0;
            for ([new_left, new_right], [_, old_right]) in (*new_node)
                .pointers
                .iter_mut()
                .zip((*insertion_point).pointers.iter_mut())
            {
                *new_left = insertion_point;
                *new_right = *old_right;
                *old_right = new_node;
                shared_height += 1;
            }

            let mut link_node = insertion_point;

            for level in shared_height..((*new_node).pointers.len()) {
                while (*link_node).pointers.len() <= level {
                    link_node = (*link_node).pointers[level - 1][0];
                }

                let ([new_left, new_right], [_, old_right]) = (
                    &mut (*new_node).pointers[level],
                    &mut (*link_node).pointers[level],
                );

                *new_left = link_node;
                *new_right = *old_right;
                *old_right = new_node;
            }

            true
        }
    }

    fn internal_find(&self, key: &K) -> *mut Node<K, V> {
        let mut level = HEIGHT;

        // find the first and highest node tower
        while level > 1 && self.head.pointers[level - 1][1].is_null() {
            level -= 1;
        }

        let mut curr = &self.head as *const Node<K, V>;

        unsafe {
            'search: loop {
                while level > 1
                    && ((*curr).pointers[level - 1][1].is_null()
                        || (*(*curr).pointers[level - 1][1])
                            .key
                            .as_ref()
                            .map(|other| other > key)
                            .unwrap_or(false))
                {
                    level -= 1;
                }

                if !(*curr).pointers[level - 1][1].is_null()
                    && (*(*curr).pointers[level - 1][1])
                        .key
                        .as_ref()
                        .map(|other| other <= key)
                        .unwrap_or(false)
                {
                    curr = (*curr).pointers[level - 1][1];
                } else {
                    break 'search;
                }
            }
        }

        curr as *mut _
    }
}

pub struct Node<K, V> {
    key: Option<K>,
    val: Option<V>,
    pointers: Vec<[*mut Node<K, V>; 2]>,
}

impl<K, V> Node<K, V> {
    pub fn new(key: Option<K>, val: Option<V>, height: usize) -> *mut Self {
        Box::into_raw(Box::new(Node {
            key,
            val,
            pointers: vec![[ptr::null_mut(); 2]; height],
        }))
    }

    /// Constructs a full Node tower with null keys and values
    fn null_full_height() -> *mut Self {
        Self::new(None, None, HEIGHT)
    }

    fn new_rand_height<'a>(key: K, val: V, list: &'a mut SkipList<K, V>) -> *mut Self {
        // construct the base nod
        let height =
            match (2..HEIGHT).try_fold(1, |prev, height| match list.state.seed.next_u32() % 2 {
                0 => Err(prev),
                _ => Ok(height),
            }) {
                Ok(res) | Err(res) => res,
            };
        Self::new(Some(key), Some(val), height)
    }
}

impl<K, V> PartialEq for Node<K, V>
where
    K: PartialEq,
    V: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        match (
            self.key.as_ref(),
            self.val.as_ref(),
            other.key.as_ref(),
            other.val.as_ref(),
        ) {
            (Some(key), Some(val), Some(other_key), Some(other_val)) => {
                key == other_key && val == other_val
            }
            _ => false,
        }
    }

    fn ne(&self, other: &Self) -> bool {
        match (
            self.key.as_ref(),
            self.val.as_ref(),
            other.key.as_ref(),
            other.val.as_ref(),
        ) {
            (Some(key), Some(val), Some(other_key), Some(other_val)) => {
                key != other_key || val != other_val
            }
            _ => true,
        }
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
                "Node {{ key:  {:?}, val: {:?}, height: {}}}",
                self.key,
                self.val,
                self.pointers.len()
            )
        }
    }
}

impl<K, V> Display for Node<K, V>
where
    K: Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (1..=self.pointers.len()).try_for_each(|level| {
            write!(
                f,
                "[key:  {:?}, val: {:?}, level: {}]\n",
                self.key, self.val, level,
            )
        })
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
        let node = unsafe { Box::from_raw(Node::new(Some(100), Some("hello"), 1)) };
        assert_eq!(
            Node {
                key: Some(100),
                val: Some("hello"),
                pointers: vec![[ptr::null_mut(); 2]; 1]
            },
            *node
        );
    }

    #[test]
    fn test_insert() {
        let mut list = SkipList::new();
        let mut rng: u16 = rand::random();

        for _ in 0..100_000 {
            rng ^= rng << 3;
            rng ^= rng >> 12;
            rng ^= rng << 7;
            list.insert(rng, "hello there!");
        }
        let mut node = &mut list.head as *mut Node<u16, &str>;
        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key.unwrap_or(0),
                    (*node).key.unwrap_or(0)
                );
                print!("                                ");
                for (_, [left, _]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", left);
                }
                print!("\n");
                print!("                                ");
                for (_, [_, right]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", right);
                }
                print!("\n");
                node = (*node).pointers[0][1];
            }
        }
    }

    #[test]
    fn test_rand_height() {
        let mut list = SkipList::new();
        let mut node = Node::new_rand_height("Hello", "There!", &mut list);

        assert!(!node.is_null());
        let mut height = unsafe { (*node).pointers.len() };

        println!("height: {}", height);

        unsafe {
            println!("{}", *node);
        }

        unsafe {
            let _ = Box::from_raw(node);
        }
    }

    // #[test]
    fn test_insert_verbose() {
        let mut list = SkipList::new();

        list.insert(1, 1);
        let mut node = &mut list.head as *mut Node<i32, i32>;
        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key.unwrap_or(0),
                    (*node).key.unwrap_or(0)
                );
                print!("                                ");
                for (_, [left, _]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", left);
                }
                print!("\n");
                print!("                                ");
                for (_, [_, right]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", right);
                }
                print!("\n");
                node = (*node).pointers[0][1];
            }
        }

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        list.insert(2, 2);
        let mut node = &mut list.head as *mut Node<i32, i32>;

        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key.unwrap_or(0),
                    (*node).key.unwrap_or(0)
                );
                print!("                                ");
                for (_, [left, _]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", left);
                }
                print!("\n");
                print!("                                ");
                for (_, [_, right]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", right);
                }
                print!("\n");
                node = (*node).pointers[0][1];
            }
        }

        println!("/////////////////////////////////////////////////////////////////////////");
        println!("/////////////////////////////////////////////////////////////////////////");

        list.insert(5, 3);
        let mut node = &mut list.head as *mut Node<i32, i32>;

        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key.unwrap_or(0),
                    (*node).key.unwrap_or(0)
                );
                print!("                                ");
                for (_, [left, _]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", left);
                }
                print!("\n");
                print!("                                ");
                for (_, [_, right]) in (0..6).zip((*node).pointers.iter()) {
                    print!("{:?} | ", right);
                }
                print!("\n");
                node = (*node).pointers[0][1];
            }
        }
    }
}
