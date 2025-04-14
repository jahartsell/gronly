//! Memory allocation utilities
//!
//! This primarily serves to re-export things from `std`, `alloc`, or `allocator-api2` depending
//! on the features selected.

use core::ptr::NonNull;

pub use allocator_api2::alloc::{AllocError, Allocator};
pub use core::alloc::Layout;

#[cfg(feature = "alloc")]
pub use allocator_api2::alloc::Global;

/// Extensions for conveniently allocating types and arrays
#[allow(dead_code)]
pub(crate) trait AllocatorExtensions: Allocator {
    /// Allocate memory suitable for a single instance of `T`
    #[inline(always)]
    fn allocate_one<T>(&self) -> Result<NonNull<T>, AllocError> {
        let layout = Layout::new::<T>();
        self.allocate(layout).map(|ptr| ptr.cast())
    }

    /// Allocate memory suitable for an instance of `[T; n]`
    #[inline(always)]
    fn allocate_array<T>(&self, n: usize) -> Result<NonNull<[T]>, AllocError> {
        let layout = Layout::array::<T>(n).map_err(|_| AllocError)?;
        let ptr = self.allocate(layout)?;
        Ok(NonNull::slice_from_raw_parts(ptr.cast(), n))
    }

    /// Deallocates `ptr` using the layout of `T`
    /// 
    /// # Safety
    /// 
    ///  * `ptr` must be currently allocated with this allocator
    ///  * `Layout` for  `T` must fit
    #[inline(always)]
    unsafe fn deallocate_one<T>(&self, ptr: NonNull<T>) {
        let layout = Layout::new::<T>();

        // Safety: Caller ensures `ptr` and `layout` are valid
        unsafe { self.deallocate(ptr.cast(), layout) };
    }

    /// Deallocates `ptr` using the layout of `[T; n]`
    /// 
    /// # Safety
    /// 
    ///  * `ptr` must be currently allocated with this allocator
    ///  * `Layout` for  `[T; N]` with `N` computed from `ptr` metatdata must fit
    #[inline(always)]
    unsafe fn deallocate_array<T>(&self, ptr: NonNull<[T]>) {
        // Safety: This is arguably UB; however, this is what rust itself used internally prior to
        // the unstable `Layout::for_value_raw` and still uses in many places.
        let layout = unsafe { Layout::for_value(ptr.as_ref()) };

        // Safety: Caller ensures `ptr` and `layout` are valid
        unsafe { self.deallocate(ptr.cast(), layout) };
    }
}

impl<T: Allocator> AllocatorExtensions for T {}

#[cfg(test)]
mod test {
    use super::*;

    use gronly_atomics::unmodeled_test;

    use crate::test::TestAllocator;

    #[unmodeled_test]
    fn allocate_one_works() {
        let alloc = TestAllocator::new();
        let a = alloc.allocate_one::<u32>().unwrap();

        // Ensure the returned memory may be used
        unsafe { a.write(4) };

        // `TestAllocator` ensures `deallocate_one` properly matches the allocate call
        unsafe { alloc.deallocate_one(a) };
    }

    #[unmodeled_test]
    fn allocate_array_works() {
        let alloc = TestAllocator::new();
        let mut a = alloc.allocate_array::<u32>(5).unwrap();

        // Ensure the returned memory may be used
        unsafe { a.as_mut().fill(2) };
        
        // `TestAllocator` ensures `deallocate_one` properly matches the allocate call
        unsafe { alloc.deallocate_array(a) };
    }
}