//! Lock-free singly-linked list
//!
//!  * A `Link` points to either null or a valid `Node`
//!  * A `Link` logically owns its pointee `Node`
//!  * A `Node` is "published" if there may be public references to its value
//!  * A `Node` is "free" if it is not reachable from the list root
//!  * A free `Node` may not be published, and its link does not logically own its pointee
//!  * All public references borrow directly from `SLList`, not `Node` or `Link`
//!  * Ownership of a `Node` may be transferred between `Links` even while published
//!
//! In a shared context `&list`
//!
//!  * Published `Node`s are never removed, there is no memory management to keep exposed refs alive
//!
//! In an exclusive context `&mut list`
//!
//!  * No nodes are published, because all public references borrow from the list
//!  * It is safe to remove `Nodes` that are not published

use core::fmt::{Debug, Formatter};
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::ptr::NonNull;

use gronly_atomics::sync::atomic::Ordering::Relaxed;

use crate::alloc::{Allocator, AllocatorExtensions};
use crate::owned::{AtomicOwned, Owned};
use crate::ExternalAllocator;

type Link<T> = AtomicOwned<Node<T>>;

struct Node<T> {
    next: Link<T>,
    element: T,
}

impl<T> Node<T> {
    /// Allocate and initialize a node with the given link and element.
    ///
    /// On failure, return the element that was not stored.
    fn allocate<A: Allocator>(next: Link<T>, element: T, alloc: A) -> Result<Owned<Self>, T> {
        match alloc.allocate_one::<Self>() {
            Ok(ptr) => {
                // Safety: `ptr` is freshly allocated for `Self`
                unsafe { ptr.write(Self { next, element }) };

                // Safety: `ptr` has been initialized and is unexposed
                unsafe { Ok(Owned::new(ptr)) }
            }
            Err(_) => Err(element),
        }
    }

    /// Move the `Node` out of the `Owned` and deallocate the memory
    ///
    /// # Safety
    ///
    ///  * `this` must have been allocated with `alloc` using `Node::new`
    unsafe fn deallocate<A: Allocator>(this: Owned<Self>, alloc: A) -> Self {
        let ptr: NonNull<_> = this.into();

        // Safety: `Owned` guarantees the pointee is valid
        let node = unsafe { ptr.read() };

        // Safety: Caller upholds
        unsafe { alloc.deallocate_one(ptr) };

        node
    }
}

/// Singly-linked list with an external allocator, see [`SLList`]
pub struct RawSLList<'alloc, T> {
    head: Link<T>,
    _phantom: PhantomData<(T, &'alloc ())>,
}

impl<'alloc, T> RawSLList<'alloc, T> {
    /// Construct a new empty list
    pub fn new() -> Self {
        Self {
            head: Link::new_null(),
            _phantom: PhantomData,
        }
    }

    /// Construct a new empty list with an allocator
    ///
    /// This does not allocate at all, it only serves to infer the lifetime from the allocator
    pub fn new_in<A: Allocator>(_alloc: &'alloc A) -> Self {
        Self::new()
    }

    /// Return `true` if the list is empty, otherwise `false`
    pub fn is_empty(&self) -> bool {
        self.head.load_ptr(Relaxed).is_null()
    }
}

unsafe impl<'alloc, T> ExternalAllocator for RawSLList<'alloc, T> {
    unsafe fn drop_and_dealloc<A: Allocator>(this: &mut Self, alloc: A) {
        let mut next = this.head.load_ptr_mut();

        while let Some(node) = NonNull::new(next) {
            // Safety: Unpublished non-null node, we have exclusive ownership
            let mut owned = unsafe { Owned::new(node) };

            next = owned.next.load_ptr_mut();

            // Safety: `node` was allocated with `Node::allocate`
            unsafe { Node::deallocate(owned, &alloc) };
        }
    }
}

/// Lock-free singly linked list
///
/// Iteration and insertion are supported in shared / concurrent contexts. Removal is only
/// supported in exclusive contexts.
pub struct SLList<
    T,
    #[cfg(feature = "alloc")] A: Allocator = crate::alloc::Global,
    #[cfg(not(feature = "alloc"))] A: Allocator,
> {
    raw: RawSLList<'static, T>,
    alloc: A,
}

impl<T, A: Allocator> SLList<T, A> {
    /// Construct a new empty singly-linked list
    pub fn new() -> Self
    where
        A: Default,
    {
        Self {
            raw: RawSLList::new(),
            alloc: A::default(),
        }
    }

    /// Construct a new empty singly-linked list with the given allocator
    pub fn new_in(alloc: A) -> Self {
        Self {
            raw: RawSLList::new(),
            alloc,
        }
    }

    /// Returns a reference to the underlying allocator
    pub fn allocator(&self) -> &A {
        &self.alloc
    }

    /// Return `true` if the list is empty, otherwise `false`
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Return a [`Cursor`] to the front of the list
    pub fn iter(&self) -> Cursor<'_, T, A> {
        Cursor {
            next: &self.raw.head,
            alloc: &self.alloc,
        }
    }

    /// Return a [`Cursor`] to the front of the list
    pub fn iter_mut(&mut self) -> CursorMut<'_, T, A> {
        CursorMut {
            next: &mut self.raw.head,
            alloc: &self.alloc,
        }
    }

    /// Pop a value from the front of the list
    pub fn pop(&mut self) -> Option<T> {
        self.iter_mut().remove()
    }

    /// Push a value to the front of the list
    pub fn push(&self, value: T) -> &T {
        self.iter().insert(value)
    }

    /// Push a value to the front of the list
    pub fn try_push(&self, value: T) -> Result<&T, T> {
        self.iter().try_insert(value)
    }

    /// Push a value to the front of the list
    pub fn push_mut(&mut self, value: T) -> &mut T {
        self.iter_mut().insert(value)
    }

    /// Push a value to the front of the list
    pub fn try_push_mut(&mut self, value: T) -> Result<&mut T, T> {
        self.iter_mut().try_insert(value)
    }
}

impl<T, A: Allocator + Default> Default for SLList<T, A> {
    fn default() -> Self {
        Self {
            raw: RawSLList::new(),
            alloc: A::default(),
        }
    }
}

impl<T, A: Allocator> Debug for SLList<T, A>
where
    T: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T, A: Allocator> PartialEq for SLList<T, A>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl<T, A: Allocator> Eq for SLList<T, A> where T: Eq {}

impl<'list, T, A: Allocator> IntoIterator for &'list SLList<T, A> {
    type Item = &'list T;
    type IntoIter = Cursor<'list, T, A>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// TODO: #[may_dangle] T
impl<T, A: Allocator> Drop for SLList<T, A> {
    fn drop(&mut self) {
        // Safety: `self` is currently valid, and will not be used after
        unsafe { RawSLList::drop_and_dealloc(&mut self.raw, &self.alloc) };
    }
}

/// Cursor for walking and inserting into [`SLList`]
///
/// A cursor is a fused `Iterator`, and can be used to concurrently insert into the list.
pub struct Cursor<'list, T, A: Allocator> {
    next: &'list Link<T>,
    alloc: &'list A,
}

impl<'list, T, A: Allocator> Cursor<'list, T, A> {
    /// Insert an item before the cursor
    ///
    /// # Panics
    ///
    /// If allocating the node fails
    ///
    /// # Examples
    ///
    /// In the simple case, a cursor points to a location between elements of the list and items
    /// are inserted behind the cursor.
    ///
    /// ```ignore
    /// let list = SLList::new();   // List representation, cursor position marked with ^
    /// let cursor = list.iter();   // ^ - null
    /// cursor.insert(1);           // 1 - ^ - null
    /// cursor.insert(2);           // 1 - 2 - ^ - null
    /// ```
    pub fn insert(&mut self, value: T) -> &'list T {
        self.try_insert(value)
            .unwrap_or_else(|_| panic!("Failed to allocate node"))
    }

    /// Insert an item before the cursor
    ///
    /// If allocating a new node fails, this returns the `value` that was not inserted
    pub fn try_insert(&mut self, value: T) -> Result<&'list T, T> {
        let link = self.next.clone(Relaxed);
        let mut node = Node::allocate(link, value, &self.alloc)?;
        let node_ptr = Owned::as_mut_ptr(&node);

        // Publish the Node
        //
        // cmpxchg is only required to be atomic, so `Relaxed, Relaxed` is fine here.
        loop {
            match self.next.cmpxchg(
                node.next.load_ptr_mut(),
                node_ptr,
                Relaxed,
                Relaxed,
            ) {
                Ok(_) => break,
                Err(new_next) => {
                    node.next.store_ptr_mut(new_next);
                }
            }

            core::hint::spin_loop();
        }

        // Safety: Node is published, it lives for 'list now
        let node = unsafe { &*node_ptr };
        self.next = &node.next;
        Ok(&node.element)
    }

    /// Return the next value without advancing the cursor
    ///
    /// # Examples
    ///
    /// Note it is possible for another cursor to insert new elements such that this returns
    /// different values when repeatedly called
    ///
    /// ```ignore
    /// // Inserting through `cur_B` affects `cur_A`
    /// let (list, cur_a, cur_b) = ...; // 1 - 2 - A, B - 3 - null
    /// assert_eq!(cur_a.peek(), Some(&3));
    /// assert_eq!(cur_b.peek(), Some(&3));
    /// cur_b.insert(4);                // 1 - 2 - A - 4 - B - 3 - null
    /// assert_eq!(cur_a.peek(), Some(&4));
    /// assert_eq!(cur_b.peek(), Some(&3));
    /// ```
    pub fn peek(&self) -> Option<&'list T> {
        // Safety: Cursor points to a published node, `load` guaranteed safe with lifetime 'list
        unsafe { self.next.load(Relaxed).map(|node| &node.element) }
    }
}

impl<'list, T, A: Allocator> Iterator for Cursor<'list, T, A> {
    type Item = &'list T;

    fn next(&mut self) -> Option<Self::Item> {
        // Safety: Cursor points to a published node, `load` guaranteed safe with lifetime 'list
        // Relaxed is fine since nodes are never removed in shared contexts.
        unsafe { self.next.load(Relaxed) }.map(|node| {
            self.next = &node.next;
            &node.element
        })
    }
}

impl<'list, T, A: Allocator> core::iter::FusedIterator for Cursor<'list, T, A> {}

impl<'list, T, A: Allocator> Clone for Cursor<'list, T, A> {
    fn clone(&self) -> Self {
        Self {
            next: &self.next,
            alloc: &self.alloc,
        }
    }
}

impl<'list, T, A: Allocator> Debug for Cursor<'list, T, A>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Cursor").field(&self.peek()).finish()
    }
}

/// Cursor for mutably walking, inserting, and removing elements in a [`SLList`]
pub struct CursorMut<'list, T, A: Allocator> {
    next: &'list mut Link<T>,
    alloc: &'list A,
}

impl<'list, T, A: Allocator> CursorMut<'list, T, A> {
    /// Insert an item before the cursor
    ///
    /// # Panics
    ///
    /// If allocating the node fails
    pub fn insert(&mut self, value: T) -> &'list mut T {
        self.try_insert(value)
            .unwrap_or_else(|_| panic!("Failed to allocate node"))
    }

    /// Insert an item before the cursor
    ///
    /// If allocating a new node fails, this returns the `value` that was not inserted
    pub fn try_insert(&mut self, value: T) -> Result<&'list mut T, T> {
        let node = Node::allocate(self.next.clone_mut(), value, &self.alloc)?;
        let node_ptr = Owned::as_mut_ptr(&node);

        // Publish the Node
        self.next.store_ptr_mut(node_ptr);

        // Safety: node is published, it lives for 'list now
        let node = unsafe { &mut *node_ptr };
        self.next = &mut node.next;
        Ok(&mut node.element)
    }

    /// Remove the next item the cursor will yield
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let (list, cur) = ...; // 1 - 2 - A - 3 - null
    /// assert_eq!(cur.peek(), Some(&3));
    /// assert_eq!(cur.remove(), Some(3))
    /// ```
    pub fn remove(&mut self) -> Option<T> {
        let node_ptr = self.next.load_ptr_mut();
        if let Some(node) = NonNull::new(node_ptr) {
            // Safety: `&mut self` implies no other references to `node` exist
            let mut node = unsafe { Owned::new(node) };
            self.next.store_ptr_mut(node.next.load_ptr_mut());

            // Safety: `node` was allocated with `Node::new` using `self.alloc`
            let node = unsafe { Node::deallocate(node, self.alloc) };
            Some(node.element)
        } else {
            None
        }
    }

    /// Return the next value without advancing the cursor
    pub fn peek(&mut self) -> Option<&'list mut T> {
        // Safety: `&mut self` implies no other references to `node` exist
        unsafe { self.next.load_mut() }.map(|node| &mut node.element)
    }
}

impl<'list, T, A: Allocator> Iterator for CursorMut<'list, T, A> {
    type Item = &'list mut T;

    fn next(&mut self) -> Option<Self::Item> {
        // Safety: `&mut self` implies no other references to `node` exist
        unsafe { self.next.load_mut() }.map(|node| {
            self.next = &mut node.next;
            &mut node.element
        })
    }
}

impl<'list, T, A: Allocator> FusedIterator for CursorMut<'list, T, A> {}

impl<'list, T, A: Allocator> Debug for CursorMut<'list, T, A>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        // Safety: `CursorMut` mutably borrows the list, we own the node
        let value = unsafe { self.next.load(Relaxed) }.map(|node| &node.element);
        f.debug_tuple("Cursor").field(&value).finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use gronly_atomics::{modeled_test, unmodeled_test, thread};
    use gronly_atomics::sync::Arc;

    use crate::test::{TestAllocator, TestDrop};

    type SLList<T> = super::SLList<T, TestAllocator>;

    #[allow(edition_2024_expr_fragment_specifier)]
    macro_rules! sllist {
        ( $( $x:expr ),* ) => {
            {
                let list = SLList::new();
                #[allow(unused_mut)]
                #[allow(unused_variables)]
                let mut cur = list.iter();
                $(
                    cur.insert( $x );
                )*
                list
            }
        };
    }

    #[unmodeled_test]
    fn node_allocates_and_deallocates() {
        let testdrop = TestDrop::new();
        let alloc = TestAllocator::new();

        let link_ptr = NonNull::dangling();
        let link = unsafe { AtomicOwned::new(Owned::new(link_ptr)) };
        let element = 3;

        let node = Node::allocate(link, testdrop.create(element), &alloc).unwrap();
        assert_eq!(node.element, 3);
        unsafe { Node::deallocate(node, &alloc) };
    }

    #[unmodeled_test]
    fn doctest_cursor_insert() {
        let list = SLList::new(); // List representation, cursor position marked with ^
        let mut cursor = list.iter(); // ^ - null
        assert_eq!(list, sllist![]);
        assert_eq!(cursor.peek(), None);

        cursor.insert(1); // 1 - ^ - null
        assert_eq!(list, sllist![1]);
        assert_eq!(cursor.peek(), None);

        cursor.insert(2); // 1 - 2 - ^ - null
        assert_eq!(list, sllist![1, 2]);
        assert_eq!(cursor.peek(), None);
    }

    #[unmodeled_test]
    fn doctest_cursor_peek() {
        // Inserting through `cur_B` affects `cur_A`
        let list = sllist![1, 2, 3];
        let mut cur_a = list.iter();
        cur_a.next();
        cur_a.next();

        let mut cur_b = cur_a.clone();

        // 1 - 2 - A, B - 3 - null
        assert_eq!(cur_a.peek(), Some(&3));
        assert_eq!(cur_b.peek(), Some(&3));
        cur_b.insert(4); // 1 - 2 - A - 4 - B - 3 - null
        assert_eq!(cur_a.peek(), Some(&4));
        assert_eq!(cur_b.peek(), Some(&3));
    }

    #[unmodeled_test]
    fn doctest_cursor_mut_remove() {
        let mut list = sllist![1, 2, 3];
        let mut cur = list.iter_mut();
        cur.next();
        cur.next();
        assert_eq!(cur.peek(), Some(&mut 3));
        assert_eq!(cur.remove(), Some(3))
    }

    #[unmodeled_test]
    fn cursor_insert_returns_value_on_failure() {
        let list = sllist![1, 2, 3];
        let mut cur = list.iter();

        list.alloc.fail_nth(1);

        assert_eq!(cur.try_insert(4), Err(4));
        assert_eq!(list, sllist![1, 2, 3]);
    }

    #[unmodeled_test]
    fn sllist_push_inserts_at_the_front() {
        let list = sllist![1, 2, 3];
        list.push(4);
        assert_eq!(list, sllist![4, 1, 2, 3]);
    }

    #[unmodeled_test]
    fn sllist_push_mut_inserts_at_the_front() {
        let mut list = sllist![1, 2, 3];
        list.push_mut(4);
        assert_eq!(list, sllist![4, 1, 2, 3]);
    }

    #[unmodeled_test]
    fn sllist_drops_contents() {
        let testdrop = TestDrop::new();

        let _list = sllist![testdrop.create(1), testdrop.create(2), testdrop.create(3)];
    }

    #[modeled_test]
    fn sllist_concurrently_inserts() {
        let list = Arc::new(sllist![1, 2, 3]);

        let t1 = {
            let list = list.clone();

            thread::spawn(move || {
                let mut cur = list.iter();
                cur.next();
                cur.insert(4);
            })
        };

        let t2 = {
            let list = list.clone();

            thread::spawn(move || {
                let mut cur = list.iter();
                cur.next();
                cur.insert(5);
            })
        };

        t1.join().unwrap();
        t2.join().unwrap();

        let list_ref: &SLList<_> = &list;
        let expected = (list_ref == &sllist![1, 4, 5, 2, 3])
                    || (list_ref == &sllist![1, 5, 4, 2, 3]);
        assert!(expected, "{:?}", list);
    }
}