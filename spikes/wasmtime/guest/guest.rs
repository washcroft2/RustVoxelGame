//! Throwaway guest for the wasmtime spike. Built with plain `rustc` for
//! wasm32-unknown-unknown, so no wasm toolchain crate is needed. The host
//! import and exports stand in for a mod calling the game once per block.
#![no_std]

unsafe extern "C" {
    fn host_get_block(x: i32, y: i32, z: i32) -> i32;
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

/// Calls the host `n` times.
#[unsafe(no_mangle)]
pub extern "C" fn with_host(n: i32) -> i32 {
    let mut sum = 0i32;
    for i in 0..n {
        // SAFETY: the host links `host_get_block` before instantiation.
        sum = sum.wrapping_add(unsafe { host_get_block(i, i, i) });
    }
    sum
}

/// Same loop shape as `with_host` with the call replaced by local arithmetic,
/// so the difference between the two is the cost of crossing the boundary.
#[unsafe(no_mangle)]
pub extern "C" fn without_host(n: i32) -> i32 {
    let mut sum = 0i32;
    for i in 0..n {
        sum = sum.wrapping_add(core::hint::black_box(i ^ (i >> 1)));
    }
    sum
}

/// Does nothing; measures the host-to-guest call cost.
#[unsafe(no_mangle)]
pub extern "C" fn noop() {}

/// Never returns, to test epoch interruption.
#[unsafe(no_mangle)]
pub extern "C" fn spin() -> i32 {
    let mut i = 0u32;
    loop {
        i = core::hint::black_box(i.wrapping_add(1));
    }
}
