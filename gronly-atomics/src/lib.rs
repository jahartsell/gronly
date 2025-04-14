#![doc = include_str!("../README.md")]
#![no_std]

#[cfg(feature = "macros")]
pub use gronly_atomics_macros::*;

cfg_if::cfg_if! {
    if #[cfg(all(loom, shuttle))] {
        compile_error!("Only one of cfg 'loom' and cfg 'shuttle' may be selected");
    } else if #[cfg(loom)] {
        pub use ::loom::*;
    } else if #[cfg(shuttle)] {
        pub use ::shuttle::*;
    } else if #[cfg(feature = "std")] {
        mod route_std;
        pub use route_std::*;
    } else if #[cfg(feature = "alloc")] {
        mod route_alloc;
        mod route_core;
        pub use route_alloc::*;
        pub use route_core::*;
    } else {
        mod route_core;
        pub use route_core::*;
    }
}

use crate::sync::atomic::Ordering;

/// Provides a common interface for methods that differ between backends
///
/// Loom even tracks atomic uses in exclusive contexts, meaning there is no `get_mut(&mut self)`.
/// Instead it offers `with_mut(&mut self, callback_fn)`. To bridge the gap, this API offers
/// `load_mut`, `store_mut` and `swap_mut`.
pub trait AtomicCompat<T> {
    /// Unsynchronized load in exclusive contexts
    fn load_mut(&mut self) -> T;

    /// Unsynchronized store in exclusive contexts
    fn store_mut(&mut self, val: T);

    /// Unsynchronized swap in exclusive contexts
    fn swap_mut(&mut self, val: T) -> T;

    /// `compare_exchange_weak` for native atomics, `compare_exchange` for modeled atomics
    ///
    /// Using `compare_exchange_weak` in modeled contexts can have surprising results. It can cause
    /// normally flaky tests to consistenly fail, and sometimes cause loom to combinatorially
    /// explode.
    ///
    /// ```ignore
    /// struct AtomicBox<T>(AtomicPtr<T>);
    ///
    /// impl<T> AtomicBox<T> {
    ///     const LOCKED: *mut T = core::ptr::null_mut();
    ///
    ///     fn try_lock(&self) -> Result<BoxGuard<'a, T>, BoxError> {
    ///         let mut current = self.load(Relaxed);
    ///         if current == LOCKED { return Err(BoxError); }
    ///         match self.cmpxchg(current, LOCKED, Acquire, Relaxed) {
    ///             Ok(ptr) => Ok(BoxGuard::new(...)),
    ///             Err(_) => Err(BoxError),
    /// i       }
    ///     }
    /// }
    /// ```
    fn cmpxchg(&self, current: T, new: T, success: Ordering, failure: Ordering) -> Result<T, T>;

    /// Unsynchronized load
    ///
    /// # Safety
    ///
    /// This should be treaded as a non-atomic load. The exact behavior depends on the backend,
    /// currently loom models the load (see `unsync_load`) and shuttle does not (`raw_load`).
    #[cfg(any(loom, shuttle))]
    unsafe fn debug_load(&self) -> T;
}

macro_rules! impl_compat {
    ($name:ident, $atom:ty $(, $generic:ident)? ) => {
        impl< $($generic)? > AtomicCompat< $atom > for crate :: sync :: atomic :: $name < $($generic)? > {
            fn load_mut(&mut self) -> $atom {
                #[cfg(loom)]
                return self.with_mut(|v| *v);

                #[cfg(not(loom))]
                return *self.get_mut();
            }

            fn store_mut(&mut self, val: $atom) {
                #[cfg(loom)]
                self.with_mut(|v| { *v = val; });

                #[cfg(not(loom))]
                { *self.get_mut() = val; }
            }

            fn swap_mut(&mut self, val: $atom) -> $atom {
                #[cfg(loom)]
                return self.with_mut(|v| { ::core::mem::replace(v, val) });

                #[cfg(not(loom))]
                return ::core::mem::replace(self.get_mut(), val);
            }

            fn cmpxchg(
                &self,
                current: $atom,
                new: $atom,
                success: Ordering,
                failure: Ordering,
            ) -> Result< $atom , $atom > {
                #[cfg(any(loom, shuttle))]
                return self.compare_exchange(current, new, success, failure);

                #[cfg(not(any(loom, shuttle)))]
                return self.compare_exchange_weak(current, new, success, failure);
            }

            #[cfg(any(loom, shuttle))]
            unsafe fn debug_load(&self) -> $atom {
                #[cfg(loom)]
                return unsafe { self.unsync_load() };

                #[cfg(shuttle)]
                return unsafe { self.raw_load() };
            }
        }
    }
}

impl_compat!(AtomicPtr, *mut T, T);
impl_compat!(AtomicU8, u8);
impl_compat!(AtomicU16, u16);
impl_compat!(AtomicU32, u32);
impl_compat!(AtomicU64, u64);
impl_compat!(AtomicUsize, usize);
impl_compat!(AtomicI8, i8);
impl_compat!(AtomicI16, i16);
impl_compat!(AtomicI32, i32);
impl_compat!(AtomicI64, i64);
impl_compat!(AtomicIsize, isize);

#[cfg(loom)]
use sync::atomic::Ordering::Relaxed;

// loom's AtomicBool doesn't have `with_mut`, fall back to relaxed ops
impl AtomicCompat<bool> for sync::atomic::AtomicBool {
    fn load_mut(&mut self) -> bool {
        #[cfg(loom)]
        return self.load(Relaxed);

        #[cfg(not(loom))]
        return *self.get_mut();
    }

    fn store_mut(&mut self, val: bool) {
        #[cfg(loom)]
        return self.store(val, Relaxed);

        #[cfg(not(loom))]
        {
            *self.get_mut() = val;
        }
    }

    fn swap_mut(&mut self, val: bool) -> bool {
        #[cfg(loom)]
        return self.swap(val, Relaxed);

        #[cfg(not(loom))]
        return ::core::mem::replace(self.get_mut(), val);
    }

    fn cmpxchg(
        &self,
        current: bool,
        new: bool,
        success: Ordering,
        failure: Ordering,
    ) -> Result<bool, bool> {
        #[cfg(any(loom, shuttle))]
        return self.compare_exchange(current, new, success, failure);
        
        #[cfg(not(any(loom, shuttle)))]
        return self.compare_exchange_weak(current, new, success, failure);
    }

    #[cfg(any(loom, shuttle))]
    unsafe fn debug_load(&self) -> bool {
        #[cfg(loom)]
        return unsafe { self.unsync_load() };

        #[cfg(shuttle)]
        return unsafe { self.raw_load() };
    }
}
