//! The arena, a fast but limited type of allocator.
//!
//! Arenas are a type of allocator that destroy the objects within,
//! all at once, once the arena itself is destroyed.
//! They do not support deallocation of individual objects while the arena itself is still alive.
//! The benefit of an arena is very fast allocation; just a vector push.
//!
//! This is an equivalent of
//! [`arena::TypedArena`](http://doc.rust-lang.org/arena/struct.TypedArena.html)
//! distributed with rustc, but is available of Rust beta/stable.
//!
//! It is slightly less efficient, but simpler internally and uses much less unsafe code.
//! It is based on a `Vec<Vec<T>>` instead of raw pointers and manual drops.

// Potential optimizations:
// 1) add and stabilize a method for in-place reallocation of vecs.
// 2) add and stabilize placement new.
// 3) use an iterator. This may add far too much unsafe code.

use std::cell::RefCell;
use std::cmp;
use std::iter;
use std::mem;

// Initial size in bytes.
const INITIAL_SIZE: usize = 1024;
// Minimum capacity. Must be larger than 0.
const MIN_CAPACITY: usize = 1;

#[derive(Debug)]
pub struct Arena<T> {
    chunks: RefCell<ChunkList<T>>,
}

#[derive(Debug)]
struct ChunkList<T> {
    current: Vec<T>,
    rest: Vec<Vec<T>>,
}

impl<T> Arena<T> {
    pub fn new() -> Arena<T> {
        let size = cmp::max(1, mem::size_of::<T>());
        Arena::with_capacity(INITIAL_SIZE / size)
    }

    pub fn with_capacity(n: usize) -> Arena<T> {
        let n = cmp::max(MIN_CAPACITY, n);
        Arena {
            chunks: RefCell::new(ChunkList {
                current: Vec::with_capacity(n),
                rest: vec![],
            }),
        }
    }

    pub fn alloc(&self, value: T) -> &mut T {
        &mut self.extend(iter::once(value).into_iter())[0]
    }

    pub fn extend<I>(&self, iter: I) -> &mut [T]
        where I: Iterator<Item = T> + ExactSizeIterator
    {
        let mut chunks = self.chunks.borrow_mut();

        if chunks.current.len() + iter.len() > chunks.current.capacity() {
            chunks.reserve(iter.len());
        }

        // At this point, the current chunk must have free capacity.
        let next_item_index = chunks.current.len();
        chunks.current.extend(iter);
        let new_slice_ref = {
            let new_slice_ref = &mut chunks.current[next_item_index..];

            // Extend the lifetime from that of `chunks_borrow` to that of `self`.
            // This is OK because we’re careful to never move items
            // by never pushing to inner `Vec`s beyond their initial capacity.
            // The returned reference is unique (`&mut`):
            // the `Arena` never gives away references to existing items.
            unsafe { mem::transmute::<&mut [T], &mut [T]>(new_slice_ref) }
        };

        new_slice_ref
    }

    #[inline]
    pub fn alloc_uninit(&self, num: usize) -> *mut [T] {
        let mut chunks = self.chunks.borrow_mut();

        if chunks.current.len() + num > chunks.current.capacity() {
            chunks.reserve(num);
        }

        // At this point, the current chunk must have free capacity.
        let next_item_index = chunks.current.len();
        unsafe {
            chunks.current.set_len(next_item_index + num);
        }
        // Extend the lifetime...
        &mut chunks.current[next_item_index..] as *mut _
    }

    pub fn remaining_capacity(&self) -> usize {
        let chunks = self.chunks.borrow();
        chunks.current.capacity() - chunks.current.len()
    }
}

impl<T> ChunkList<T> {
    #[inline(never)]
    #[cold]
    fn reserve(&mut self, additional: usize) {
        let double_cap = self.current.capacity().checked_mul(2).expect("capacity overflow");
        let required_cap = additional.checked_next_power_of_two().expect("capacity overflow");
        let new_capacity = cmp::max(double_cap, required_cap);
        let chunk = mem::replace(&mut self.current, Vec::with_capacity(new_capacity));
        self.rest.push(chunk);
    }
}


#[test]
fn it_works() {
    use std::cell::Cell;

    struct DropTracker<'a>(&'a Cell<u32>);
    impl<'a> Drop for DropTracker<'a> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    struct Node<'a, 'b: 'a>(Option<&'a Node<'a, 'b>>, u32, DropTracker<'b>);
    let drop_counter = Cell::new(0);
    {
        let arena = Arena::with_capacity(2);

        let mut node: &Node = arena.alloc(Node(None, 1, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 0);

        node = arena.alloc(Node(Some(node), 2, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 1);

        node = arena.alloc(Node(Some(node), 3, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 1);

        node = arena.alloc(Node(Some(node), 4, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 1);

        assert_eq!(node.1, 4);
        assert_eq!(node.0.unwrap().1, 3);
        assert_eq!(node.0.unwrap().0.unwrap().1, 2);
        assert_eq!(node.0.unwrap().0.unwrap().0.unwrap().1, 1);
        assert!(node.0.unwrap().0.unwrap().0.unwrap().0.is_none());

        mem::drop(node);
        assert_eq!(drop_counter.get(), 0);

        let mut node: &Node = arena.alloc(Node(None, 5, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 1);

        node = arena.alloc(Node(Some(node), 6, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 2);

        node = arena.alloc(Node(Some(node), 7, DropTracker(&drop_counter)));
        assert_eq!(arena.chunks.borrow().rest.len(), 2);

        assert_eq!(drop_counter.get(), 0);

        assert_eq!(node.1, 7);
        assert_eq!(node.0.unwrap().1, 6);
        assert_eq!(node.0.unwrap().0.unwrap().1, 5);
        assert!(node.0.unwrap().0.unwrap().0.is_none());

        assert_eq!(drop_counter.get(), 0);
    }
    assert_eq!(drop_counter.get(), 7);
}
