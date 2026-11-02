//! Profiling spans that cost nothing unless the `profile` feature is on.
//!
//! A disabled `tracing` span still costs a check, so spans in hot code go through this macro,
//! which expands to nothing in normal builds. Which version is used is decided by this crate's
//! `profile` feature, which the game's own `profile` feature switches on for the whole build.

/// Opens a profiling span with a literal name until the end of the enclosing block.
///
/// ```
/// fn work() {
///     voxel_core::span!("work");
///     // ... timed in profile builds, free otherwise
/// }
/// # work();
/// ```
#[cfg(feature = "profile")]
#[macro_export]
macro_rules! span {
    ($name:literal) => {
        let _profile_span = $crate::__tracing::trace_span!($name).entered();
    };
}

/// Opens a profiling span with a literal name until the end of the enclosing block.
#[cfg(not(feature = "profile"))]
#[macro_export]
macro_rules! span {
    ($name:literal) => {};
}
