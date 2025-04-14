use core::fmt::Debug;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use gronly_atomics::AtomicCompat;
use gronly_atomics::sync::atomic::{AtomicPtr, Ordering};

/// A `NonNull` wrapper indicating ownership of the pointee
///
/// This is identical in concept to `::core::ptr::Unique`, but varies in practice because
/// `Owned` requires the type system to enforce the aliasing guarantees. This means constructing an
/// `Owned` is unsafe, there must not be any existing references to the pointee and all future
/// references must be obtained through the `Owned`. In return, dereferencing an `Owned` is always
/// safe.
///
/// This type intentionally leaks. In order to drop and/or deallocate it must be converted into
/// a pointer type.
pub struct Owned<T: ?Sized> {
    ptr: NonNull<T>,
    _phantom: PhantomData<T>,
}

impl<T: ?Sized> Owned<T> {
    /// Create a new `Owned`
    ///
    /// # Safety
    ///
    ///  * `ptr` must be convertible to a `&mut` for the lifetime of this `Owned`
    ///  * All references to the pointee must be derived from this `Owned`
    pub unsafe fn new(ptr: NonNull<T>) -> Self {
        Self {
            ptr,
            _phantom: PhantomData,
        }
    }

    /// Return the underlying `*const` pointer
    ///
    /// This always points to a valid object, but may *not* be dereferenced until the `Owned` is
    /// dropped.
    pub fn as_ptr(this: &Self) -> *const T {
        this.ptr.as_ptr()
    }

    /// Return the underlying `*mut` pointer
    ///
    /// This always points to a valid object, but may *not* be dereferenced until the `Owned` is
    /// dropped.
    pub fn as_mut_ptr(this: &Self) -> *mut T {
        this.ptr.as_ptr()
    }

    /// Return the underlying `NonNull` pointer
    ///
    /// This always points to a valid object, but may *not* be dereferenced until the `Owned` is
    /// dropped.
    pub fn as_non_null(this: &Self) -> NonNull<T> {
        this.ptr
    }
}

impl<T: ?Sized + Debug> Debug for Owned<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let value: &T = &self;
        f.debug_tuple("Owned").field(&value).finish()
    }
}

impl<T: ?Sized> Deref for Owned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // Safety: This is the whole guarantee of the type
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: ?Sized> DerefMut for Owned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: This is the whole guarantee of the type
        unsafe { self.ptr.as_mut() }
    }
}

// Safety: `Owned` upholds aliasing guarantes and acts as a `T`
unsafe impl<T: Send + ?Sized> Send for Owned<T> {}

// Safety: `Owned` upholds aliasing guarantes and acts as a `T`
unsafe impl<T: Sync + ?Sized> Sync for Owned<T> {}

impl<T: ?Sized> From<Owned<T>> for NonNull<T> {
    fn from(value: Owned<T>) -> Self {
        value.ptr
    }
}

impl<T: ?Sized> From<Owned<T>> for *mut T {
    fn from(value: Owned<T>) -> Self {
        value.ptr.as_ptr()
    }
}

/// An `AtomicPtr` wrapper indicating ownership of the pointee
///
/// This type is conceptually an atomic `Option<Owned>`; however, the aliasing guarantees are
/// upheld by the user rather than the type system. In this sense it is closer to
/// `::core::ptr::Unique`.
///
/// If the inner pointer is non-null, it must point to a valid object.
pub struct AtomicOwned<T>(AtomicPtr<T>);

#[allow(dead_code)]
impl<T> AtomicOwned<T> {
    /// Create a new `AtomicOwned`
    pub fn new(ptr: Owned<T>) -> Self {
        Self(AtomicPtr::new(ptr.into()))
    }

    /// Create a new null `AtomicOwned`
    pub fn new_null() -> Self {
        Self(AtomicPtr::new(core::ptr::null_mut()))
    }

    /// Create a new `AtomicOwned` with the same pointee as `self`
    pub fn clone(&self, order: Ordering) -> Self {
        Self(AtomicPtr::new(self.0.load(order)))
    }

    /// Create a new `AtomicOwned` with the same pointee as `self`
    pub fn clone_mut(&mut self) -> Self {
        Self(AtomicPtr::new(self.0.load_mut()))
    }

    /// Atomically load and dereference the underlying pointer
    ///
    /// # Safety
    ///
    ///  * The pointee must be uniquely owned by this `AtomicOwned`
    ///  * The pointee must be valid for the chosen lifetime
    pub unsafe fn load<'a>(&self, order: Ordering) -> Option<&'a T> {
        // Caller guarantees `ptr` is safe to deref
        NonNull::new(self.0.load(order)).map(|ptr| unsafe { ptr.as_ref() })
    }

    /// Load and dereference the underlying pointer
    ///
    /// # Safety
    ///
    /// * The pointee must be uniquely owned by this `AtomicOwned`
    /// * The pointee must be valid for the chosen lifetime
    pub unsafe fn load_mut<'a>(&mut self) -> Option<&'a mut T> {
        // Caller guarantees `ptr` is safe to deref
        NonNull::new(self.0.load_mut()).map(|mut ptr| unsafe { ptr.as_mut() })
    }

    /// Atomically load the underlying pointer
    pub fn load_ptr(&self, order: Ordering) -> *mut T {
        self.0.load(order)
    }

    /// Load the underlying pointer
    pub fn load_ptr_mut(&mut self) -> *mut T {
        self.0.load_mut()
    }

    /// Atomically store the underlying pointer, potentially leaking the previous pointee
    pub fn store(&self, value: Option<&mut T>, order: Ordering) {
        let ptr = value.map_or(core::ptr::null_mut(), |val| val as *mut T);
        self.0.store(ptr, order);
    }

    /// Store the underlying pointer, potentially leaking the previous pointee
    pub fn store_mut(&mut self, value: Option<&mut T>) {
        let ptr = value.map_or(core::ptr::null_mut(), |val| val as *mut T);
        self.0.store_mut(ptr);
    }

    /// Atomically store the underlying pointer, potentially leaking the previous pointee
    pub fn store_ptr(&self, value: *mut T, order: Ordering) {
        self.0.store(value, order);
    }

    /// Store the underlying pointer, potentially leaking the previous pointee
    pub fn store_ptr_mut(&mut self, value: *mut T) {
        self.0.store_mut(value);
    }

    /// Store the value `new` if the current value is the same as `current`
    pub fn cmpxchg(
        &self,
        current: *mut T,
        new: *mut T,
        success: Ordering,
        failure: Ordering,
    ) -> Result<*mut T, *mut T> {
        self.0.cmpxchg(current, new, success, failure)
    }
}

impl<T> Default for AtomicOwned<T> {
    /// Create a new null `AtomicOwned`
    fn default() -> Self {
        Self::new_null()
    }
}