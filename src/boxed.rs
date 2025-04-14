use crate::alloc::{AllocError, Allocator, AllocatorExtensions};
use crate::ExtAlloc;

use core::alloc::Layout;
use core::marker::PhantomData;
use core::ptr::NonNull;

/// An instance of `T` stored using a given allocator
pub struct RawBox<'alloc, T: ?Sized> {
    ptr: NonNull<T>,
    _phantom: PhantomData<(&'alloc (), T)>,
}

impl<'alloc, T: ?Sized> RawBox<'alloc, T> {
    /// Construct a new `RawBox` with the given value
    pub fn try_new<A: Allocator>(x: T, alloc: &'alloc A) -> Result<Self, AllocError>
    where
        T: Sized,
    {
        let ptr = alloc.allocate_one()?;
        unsafe { ptr.write(x) };
        Ok(Self {
            ptr,
            _phantom: PhantomData,
        })
    }
}

impl<'alloc, T: ?Sized> core::ops::Deref for RawBox<'alloc, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl<'alloc, T: ?Sized> core::ops::DerefMut for RawBox<'alloc, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

unsafe impl<'alloc, T: ?Sized> ExtAlloc for RawBox<'alloc, T> {
    unsafe fn drop_and_dealloc<A: Allocator>(this: &mut Self, alloc: A) {
        unsafe {
            // `Layout::for_value_raw` would be preferred here
            let layout = Layout::for_value(this.ptr.as_ref());
            core::ptr::drop_in_place(this.ptr.as_ptr());
            alloc.deallocate(this.ptr.cast(), layout);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::alloc::DebugAllocator;

    #[test]
    fn raw_box_works() {
        let alloc = DebugAllocator::new();
        let mut boxed = RawBox::try_new(1, &alloc).unwrap();
        assert_eq!(*boxed, 1);
        unsafe { RawBox::drop_and_dealloc(&mut boxed, &alloc) };
    }
}
