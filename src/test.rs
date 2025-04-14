extern crate alloc;

// This uses a mix of modeled and unmodeled atomics.
// modeled atomics are imported as `ModeledXXX`
// unmodeled atomics use their standard name

use alloc::sync::Arc;
use core::fmt::{Formatter, Debug};
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicUsize};
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use gronly_atomics::sync::atomic::AtomicUsize as ModeledUsize;
use papaya::{HashMap, HashSet};

use crate::alloc::{AllocError, Allocator, Layout};

struct TestAllocatorState {
    leases: HashMap<NonNull<u8>, Layout>,
    fail_counter: ModeledUsize,
    assert_on_drop: AtomicBool,
}

impl Drop for TestAllocatorState {
    fn drop(&mut self) {
        if *self.assert_on_drop.get_mut() {
            assert!(self.leases.is_empty(), "Leaks: {:?}", self.leases);
        }
    }
}

impl Default for TestAllocatorState {
    fn default() -> Self {
        Self {
            leases: HashMap::new(),
            fail_counter: ModeledUsize::new(0),
            assert_on_drop: AtomicBool::new(true),
        }
    }
}

unsafe impl Send for TestAllocatorState {}
unsafe impl Sync for TestAllocatorState {}

/// Allocator for use in tests
///
/// This asserts that every deallocation properly matches a current allocation and asserts on drop
/// that every allocation was deallocated. In addition, it can be told to fail the n'th next
/// allocation at any time.
///
/// This wraps `alloc::alloc::alloc` / `dealloc` so that tests are feature-gate independent.
#[derive(Clone, Default)]
#[repr(transparent)]
pub struct TestAllocator(Arc<TestAllocatorState>);

impl TestAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the n'th next allocation to fail, overrides a previous mark
    pub fn fail_nth(&self, n: usize) {
        self.0.fail_counter.store(n, Release);
    }
}

unsafe impl Allocator for TestAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let mut count = self.0.fail_counter.load(Relaxed);
        loop {
            // Use strong compare_exchange for predictable behavior in loom tests

            match count {
                // No fail mark set
                0 => break,

                // Next should fail, fail the allocation if we decement the count
                1 => match self.0.fail_counter.compare_exchange(
                    1,
                    0,
                    Acquire,
                    Relaxed,
                ) {
                    Ok(_) => return Err(AllocError),
                    Err(new_count) => count = new_count,
                },

                // succeed the allocation if we decement the count
                _ => match self.0.fail_counter.compare_exchange(
                    count,
                    count - 1,
                    Acquire,
                    Relaxed,
                ) {
                    Ok(_) => break,
                    Err(new_count) => count = new_count,
                },
            }

            core::hint::spin_loop();
        }

        let ptr = unsafe { alloc::alloc::alloc(layout) };
        let ptr = NonNull::new(ptr).ok_or(AllocError)?;

        self.0.leases.pin().insert(ptr, layout);

        Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        match self.0.leases.pin().remove_if(&ptr, |_, l| l == &layout) {
            Ok(Some(_)) => {} // Success, `layout` matches the one stored in `leases`
            Ok(None) => {
                self.0.assert_on_drop.store(false, Relaxed);
                panic!("No lease for: {:?}", ptr)
            }
            Err((_, l)) => {
                self.0.assert_on_drop.store(false, Relaxed);
                panic!(
                    "Incorrect layout for {:?}, given: {:?}, expected: {:?}",
                    ptr, layout, l
                )
            }
        }

        unsafe { alloc::alloc::dealloc(ptr.as_ptr(), layout) }
    }
}

struct TestDropState {
    ids: HashSet<usize>,
    next_id: AtomicUsize,
    assert_on_drop: AtomicBool,
}

impl Default for TestDropState {
    fn default() -> Self {
        Self {
            ids: HashSet::new(),
            next_id: AtomicUsize::new(0),
            assert_on_drop: AtomicBool::new(true),
        }
    }
}

/// Object factory that asserts produced objects are properly dropped
#[derive(Clone, Default)]
pub struct TestDrop(Arc<TestDropState>);

impl TestDrop {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create<T>(&self, element: T) -> DropMe<T> {
        let id = self.0.next_id.fetch_add(1, Relaxed);
        self.0.ids.pin().insert(id);
        let parent = ManuallyDrop::new(self.0.clone());
        DropMe {
            id,
            parent,
            element,
        }
    }
}

impl Drop for TestDrop {
    fn drop(&mut self) {
        if self.0.assert_on_drop.load(Relaxed) {
            assert!(self.0.ids.is_empty(), "Undropped ids: {:?}", self.0.ids)
        }
    }
}

pub struct DropMe<T> {
    id: usize,
    parent: ManuallyDrop<Arc<TestDropState>>,
    element: T,
}

impl<T> Deref for DropMe<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.element
    }
}

impl<T> DerefMut for DropMe<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.element
    }
}

impl<T> Drop for DropMe<T> {
    fn drop(&mut self) {
        // On the first drop, reading parent is fine
        // On the second drop, our `Arc` is already dropped and the parent may be deleted
        // In typical usage, `TestDrop` outlives all its children so parent will still live
        if !self.parent.ids.pin().remove(&self.id) {
            self.parent.assert_on_drop.store(false, Relaxed);
            panic!("Repeated drop for id: {:?}", self.id);
        }

        // Make sure that even if we are dropped multiple times, we only drop the parent `Arc`
        // once. Repeatedly dropping `parent` will drop/deallocate it early and cause memory errors
        unsafe { ManuallyDrop::drop(&mut self.parent) };
    }
}

impl<T: Debug> Debug for DropMe<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("DropMe")
            .field(&self.element)
            .finish()
    }
}

impl<T, U> core::cmp::PartialEq<U> for DropMe<T>
where 
T: PartialEq<U>,
{
    fn eq(&self, other: &U) -> bool {
        self.element.eq(other)
    }
}

#[cfg(test)]
mod test {
    extern crate std;

    use crate::alloc::AllocatorExtensions;

    use super::*;

    use gronly_atomics::{modeled_test, unmodeled_test, thread};
    use std::vec::Vec;

    #[unmodeled_test]
    fn allocator_fails_nth() {
        let alloc = TestAllocator::new();
        alloc.fail_nth(3);

        let layout = Layout::new::<u64>();
        let a = alloc.allocate(layout).unwrap();
        let b = alloc.allocate(layout).unwrap();
        assert!(alloc.allocate(layout).is_err());
        let d = alloc.allocate(layout).unwrap();

        unsafe {
            alloc.deallocate(a.cast(), layout);
            alloc.deallocate(b.cast(), layout);
            alloc.deallocate(d.cast(), layout);
        }
    }

    #[unmodeled_test]
    #[should_panic]
    #[cfg(not(miri))]
    fn allocator_deallocate_panics_for_incorrect_layout() {
        // List test inentionally leaks, miri obv doesn't like that
        let alloc = TestAllocator::new();

        let layout = Layout::new::<u64>();
        let p = alloc.allocate(layout).unwrap();

        let layout = Layout::new::<u32>();
        unsafe { alloc.deallocate(p.cast(), layout) };
    }

    #[unmodeled_test]
    #[should_panic]
    fn allocator_deallocate_panics_for_invalid_pointer() {
        let alloc = TestAllocator::new();

        let layout = Layout::new::<u64>();
        let p = NonNull::<u64>::dangling();

        unsafe { alloc.deallocate(p.cast(), layout) };
    }

    #[modeled_test]
    fn allocator_works_concurrently() {
        let alloc = TestAllocator::new();

        let threads: Vec<_> = (0..2).map(|_| {
            let alloc = alloc.clone();
            thread::spawn(move || {
                let p = alloc.allocate_one::<u64>().unwrap();
                unsafe { alloc.deallocate_one(p) };
            })
        }).collect();

        for t in threads {
            t.join().unwrap();
        }
    }

    #[unmodeled_test]
    fn test_drop_succeeds_when_all_children_dropped() {
        let testdrop = TestDrop::new();

        let _a = testdrop.create(3);
        let _b = testdrop.create(4);
        let _c = testdrop.create(5);
    }

    #[unmodeled_test]
    #[should_panic]
    #[cfg(not(miri))]
    fn test_drop_panics_when_children_leaked() {
        let testdrop = TestDrop::new();

        let a = testdrop.create(3);
        core::mem::forget(a);
    }

    #[unmodeled_test]
    #[should_panic]
    fn test_drop_panics_when_children_double_dropped() {
        let testdrop = TestDrop::new();

        let mut a = ManuallyDrop::new(testdrop.create(3));
        unsafe {
            ManuallyDrop::drop(&mut a);
            ManuallyDrop::drop(&mut a);
        }
    }

    #[modeled_test]
    fn test_drop_works_concurrently() {
        let testdrop = TestDrop::new();

        let threads: Vec<_> = (0..2).map(|_| {
            let testdrop = testdrop.clone();
            thread::spawn(move || {
                let _obj = testdrop.create(5);
            })
        }).collect();

        for t in threads {
            t.join().unwrap();
        }
    }
}
