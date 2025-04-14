#![doc = include_str!("../README.md")]
#![no_std]

pub mod alloc;
mod owned;
pub mod sllist;
#[cfg(test)]
mod test;

use alloc::Allocator;

pub use sllist::{RawSLList, SLList};

/// Trait for types which require explicit deallocation
///
/// It is expected that containers in this library are used with custom allocators, as such always
/// storing the allocator internally can result in significant overhead when composing them.
/// Therefore, raw containers in this library *do not* internally store their allocators.
///
/// # Safety
///
/// Types implementing this trait must conceptually have a single "active allocator". All
/// allocations managed by the type must use the currently active allocator.
pub unsafe trait ExternalAllocator {
    /// Perform any necessary drops and release all allocations
    ///
    /// # Safety
    ///
    /// `alloc` must be the currently active allocator
    /// Using `this` after the call completes is UB unless the implementing type specifies otherwise
    unsafe fn drop_and_dealloc<A: Allocator>(this: &mut Self, alloc: A);
}
