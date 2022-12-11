use std::{
    fmt::{Debug, Display},
    ptr,
};

const HEIGHT_BITS: usize = 5;

const HEIGHT: usize = 1 << HEIGHT_BITS;

/// Head stores the first pointer tower at the beginning of the list. It is always of maximum
#[repr(C)]
struct Head<K, V> {
    pointers: Vec<[*mut Node<K, V>; 2]>,
}

impl<K, V> Head<K, V> {
    fn new() -> Self {
        Head {
            pointers: vec![[std::ptr::null_mut(); 2]; HEIGHT],
        }
    }
}

pub struct SkipList<K, V> {
    head: Head<K, V>,
    state: ListState,
}

impl<K, V> SkipList<K, V> {
    pub fn new() -> Self {
        SkipList {
            head: Head::new(),
            state: ListState {
                len: 0,
                max_height: 1,
                seed: rand::random(),
            },
        }
    }

    pub fn len(&self) -> usize {
        self.state.len
    }
    fn gen_height(&mut self) -> usize {
        let seed = &mut self.state.seed;
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;

        let mut height = std::cmp::min(HEIGHT, seed.trailing_zeros() as usize + 1);

        while height >= 4 && self.head.pointers[height - 2][1].is_null() {
            height -= 1;
        }

        if height > self.state.max_height {
            self.state.max_height = height;
        }

        height
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

            if !self.is_head(insertion_point) && (*insertion_point).key == key {
                // if so, replace the value
                let _ = std::mem::replace(&mut (*insertion_point).val, val);
                self.state.len += 1;
                return true;
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

            self.state.len += 1;
            true
        }
    }

    fn internal_find(&self, key: &K) -> *mut Node<K, V> {
        let mut level = HEIGHT;

        // find the first and highest node tower
        while level > 1 && self.head.pointers[level - 1][1].is_null() {
            level -= 1;
        }

        let mut curr = &self.head as *const Head<K, V> as *const Node<K, V>;

        unsafe {
            'search: loop {
                while level > 1
                    && ((*curr).pointers[level - 1][1].is_null()
                        || (*(*curr).pointers[level - 1][1]).key > *key)
                {
                    level -= 1;
                }

                if !(*curr).pointers[level - 1][1].is_null()
                    && (*(*curr).pointers[level - 1][1]).key <= *key
                {
                    curr = (*curr).pointers[level - 1][1];
                } else {
                    break 'search;
                }
            }
        }

        curr as *mut _
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        if self.len() < 1 {
            return None;
        }

        let target = self.internal_find(key);

        if self.is_head(target) {
            return None;
        }

        unsafe {
            if (*target).key == *key {
                return Some(&(*target).val);
            }
        }

        None
    }

    fn is_head(&self, ptr: *const Node<K, V>) -> bool {
        std::ptr::eq(ptr, &self.head as *const _ as *const Node<K, V>)
    }
}

#[repr(C)]
pub struct Node<K, V> {
    pointers: Vec<[*mut Node<K, V>; 2]>,
    key: K,
    val: V,
}

impl<K, V> Node<K, V> {
    pub fn new(key: K, val: V, height: usize) -> *mut Self {
        Box::into_raw(Box::new(Node {
            pointers: vec![[ptr::null_mut(); 2]; height],
            key,
            val,
        }))
    }

    fn new_rand_height<'a>(key: K, val: V, list: &'a mut SkipList<K, V>) -> *mut Self {
        // construct the base nod
        let seed = &mut list.state.seed;
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;

        Self::new(key, val, seed.trailing_zeros() as usize + 1)
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

    fn ne(&self, other: &Self) -> bool {
        self.key != other.key && self.val != other.val
    }
}

impl<K, V> Debug for Node<K, V>
where
    K: Debug,
    V: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Node {{ key:  {:?}, val: {:?}, height: {}}}",
            self.key,
            self.val,
            self.pointers.len()
        )
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
    seed: usize,
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
    fn test_new_list() {
        let _: SkipList<i32, i32> = SkipList::new();
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
        let mut node = list.head.pointers[0][0];
        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key,
                    (*node).key
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
        let mut node = list.head.pointers[0][0];
        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key,
                    (*node).key
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
        let mut node = list.head.pointers[0][0];

        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key,
                    (*node).key
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
        let mut node = list.head.pointers[0][0];

        unsafe {
            while !node.is_null() {
                println!(
                    "{:?}-key: {:?}, val: {:?}----------------------------------------------",
                    node,
                    (*node).key,
                    (*node).key
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
