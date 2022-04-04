//! A library for catching panic.

#![feature(alloc_error_hook)]
#![feature(allocator_api)]

use std::alloc::Layout;
use std::cell::Cell;
use std::error::Error;
use std::fmt;
use std::panic::{PanicInfo, UnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

/// The error type for allocation failure.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct AllocError(Layout);

impl AllocError {
    /// Creates a new `AllocError`.
    #[must_use]
    #[inline]
    pub const fn new(layout: Layout) -> Self {
        AllocError(layout)
    }

    /// Returns the memory layout of the `AllocError`.
    #[must_use]
    #[inline]
    pub const fn layout(self) -> Layout {
        self.0
    }
}

impl fmt::Debug for AllocError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AllocError")
            .field("size", &self.0.size())
            .field("align", &self.0.align())
            .finish()
    }
}

impl fmt::Display for AllocError {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to allocate memory by required layout {{size: {}, align: {}}}",
            self.0.size(),
            self.0.align()
        )
    }
}

impl Error for AllocError {}

thread_local! {
    static THREAD_ALLOC_ERROR: Cell<Option<AllocError>> = Cell::new(None);
}

struct ThreadAllocError;

impl ThreadAllocError {
    /// Injects alloc error to current thread.
    #[inline]
    fn inject(e: AllocError) {
        debug_assert!(!ThreadAllocError::has_error());
        THREAD_ALLOC_ERROR.with(|error| {
            error.set(Some(e));
        })
    }

    /// Checks if has alloc error in current thread.
    #[inline]
    fn has_error() -> bool {
        THREAD_ALLOC_ERROR.with(|error| error.get().is_some())
    }

    /// Takes alloc error from current thread
    #[inline]
    fn take() -> Option<AllocError> {
        THREAD_ALLOC_ERROR.with(|error| error.take())
    }

    /// Clears alloc error in current thread
    #[inline]
    fn clear() {
        let _ = ThreadAllocError::take();
    }
}

fn oom_hook(layout: Layout) {
    ThreadAllocError::inject(AllocError(layout));
    panic!("memory allocation of {} bytes failed", layout.size());
}

/// Invokes a closure, capturing the out-of-memory panic if one occurs.
///
/// This function will return `Ok` with the closure's result if the closure
/// does not panic, and will return `AllocError` if allocation error occurs. The
/// process will abort if other panics occur.
#[inline]
pub fn catch_oom<F: FnOnce() -> R + UnwindSafe, R>(f: F) -> Result<R, AllocError> {
    type Hook = Box<dyn Fn(&PanicInfo<'_>) + 'static + Sync + Send>;

    fn panic_hook(_: &PanicInfo<'_>) {
        // panic abort except alloc error
        if !ThreadAllocError::has_error() {
            std::process::abort();
        }
    }

    static SET_HOOK: AtomicBool = AtomicBool::new(false);
    if !SET_HOOK.load(Ordering::Acquire) {
        let hook: Hook = Box::try_new(panic_hook).map_err(|_| AllocError::new(Layout::new::<Hook>()))?;
        std::panic::set_hook(hook);
        std::alloc::set_alloc_error_hook(oom_hook);
        SET_HOOK.store(true, Ordering::Release);
    }

    ThreadAllocError::clear();
    let result = std::panic::catch_unwind(f);
    match result {
        Ok(r) => Ok(r),
        Err(_) => match ThreadAllocError::take() {
            None => {
                unreachable!()
            }
            Some(e) => Err(e),
        },
    }
}
