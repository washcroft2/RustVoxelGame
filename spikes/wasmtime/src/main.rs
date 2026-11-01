//! Phase 0 wasmtime spike (plan sections 10 and 13). Throwaway.
//!
//! Measures, on whatever machine runs it: module compile and instantiate time,
//! host-call overhead, host-to-guest call overhead, the cost of epoch checks,
//! and how quickly epoch interruption stops a runaway guest.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use wasmtime::{Config, Engine, Linker, Module, Result, Store, Trap, TypedFunc};

const HOST_CALLS: i32 = 100_000;
const REPEATS: usize = 15;

fn main() -> Result<()> {
    let wasm = build_guest()?;
    println!(
        "os={} arch={} guest={} bytes",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::fs::metadata(&wasm)?.len()
    );

    for epoch in [false, true] {
        println!(
            "\n== epoch interruption {}",
            if epoch { "ON" } else { "OFF" }
        );
        let engine = engine(epoch)?;
        let t = Instant::now();
        let module = Module::from_file(&engine, &wasm)?;
        println!("compile: {:?}", t.elapsed());

        let mut linker = Linker::new(&engine);
        linker.func_wrap("env", "host_get_block", |x: i32, y: i32, z: i32| {
            x.wrapping_add(y).wrapping_add(z) & 0xff
        })?;

        let t = Instant::now();
        let mut store = Store::new(&engine, ());
        if epoch {
            store.set_epoch_deadline(1 << 40);
        }
        let instance = linker.instantiate(&mut store, &module)?;
        println!("instantiate (incl. store): {:?}", t.elapsed());

        let with_host: TypedFunc<i32, i32> = instance.get_typed_func(&mut store, "with_host")?;
        let without_host: TypedFunc<i32, i32> =
            instance.get_typed_func(&mut store, "without_host")?;
        let noop: TypedFunc<(), ()> = instance.get_typed_func(&mut store, "noop")?;

        let a = median(|| time(|| with_host.call(&mut store, HOST_CALLS).map(drop)))?;
        let b = median(|| time(|| without_host.call(&mut store, HOST_CALLS).map(drop)))?;
        let per_call = a.as_nanos().saturating_sub(b.as_nanos()) as f64 / f64::from(HOST_CALLS);
        println!(
            "guest->host call: {per_call:.1} ns/call  (loop with host {a:?}, without {b:?}, n={HOST_CALLS})"
        );

        let n = 100_000u32;
        let c = median(|| {
            time(|| {
                for _ in 0..n {
                    noop.call(&mut store, ())?;
                }
                Ok(())
            })
        })?;
        println!(
            "host->guest call: {:.1} ns/call",
            c.as_nanos() as f64 / f64::from(n)
        );

        // Pure guest compute, for the epoch-check overhead comparison.
        let d = median(|| time(|| without_host.call(&mut store, 200_000_000).map(drop)))?;
        println!("pure guest loop, 200M iterations: {d:?}");
    }

    interrupt_latency(&wasm)?;
    Ok(())
}

fn engine(epoch: bool) -> Result<Engine> {
    let mut config = Config::new();
    config.epoch_interruption(epoch);
    Engine::new(&config)
}

/// A ticker thread bumps the epoch every millisecond, the way the game would
/// once per frame slice; the guest gets a deadline of one tick.
fn interrupt_latency(wasm: &PathBuf) -> Result<()> {
    println!("\n== epoch interrupt latency (tick = 1 ms, deadline = 1 tick)");
    let engine = engine(true)?;
    let module = Module::from_file(&engine, wasm)?;
    let mut linker = Linker::new(&engine);
    linker.func_wrap("env", "host_get_block", |_: i32, _: i32, _: i32| 0i32)?;

    let stop = Arc::new(AtomicBool::new(false));
    let ticker = {
        let (engine, stop) = (engine.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
                engine.increment_epoch();
            }
        })
    };

    let mut samples = Vec::new();
    for _ in 0..30 {
        let mut store = Store::new(&engine, ());
        store.set_epoch_deadline(1);
        store.epoch_deadline_trap();
        let instance = linker.instantiate(&mut store, &module)?;
        let spin: TypedFunc<(), i32> = instance.get_typed_func(&mut store, "spin")?;
        let t = Instant::now();
        let err = spin.call(&mut store, ()).expect_err("spin must not return");
        samples.push(t.elapsed());
        assert_eq!(
            err.downcast_ref::<Trap>(),
            Some(&Trap::Interrupt),
            "{err:?}"
        );
    }
    stop.store(true, Ordering::Relaxed);
    ticker.join().expect("ticker thread panicked");

    samples.sort();
    println!(
        "time until trap: min {:?}  median {:?}  max {:?}  (30 runs)",
        samples[0],
        samples[samples.len() / 2],
        samples[samples.len() - 1]
    );
    Ok(())
}

fn time(f: impl FnOnce() -> Result<()>) -> Result<Duration> {
    let t = Instant::now();
    f()?;
    Ok(t.elapsed())
}

fn median(mut f: impl FnMut() -> Result<Duration>) -> Result<Duration> {
    let mut v = (0..REPEATS).map(|_| f()).collect::<Result<Vec<_>>>()?;
    v.sort();
    Ok(v[v.len() / 2])
}

fn build_guest() -> Result<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("../../target/spike-wasmtime");
    std::fs::create_dir_all(&out)?;
    let wasm = out.join("guest.wasm");
    let status = Command::new("rustc")
        .args(["--edition", "2024", "--target", "wasm32-unknown-unknown"])
        .args([
            "--crate-type",
            "cdylib",
            "-C",
            "opt-level=3",
            "-C",
            "panic=abort",
        ])
        .args(["-C", "link-arg=--allow-undefined"])
        .arg(root.join("guest/guest.rs"))
        .arg("-o")
        .arg(&wasm)
        .status()?;
    wasmtime::ensure!(status.success(), "rustc failed building the guest");
    Ok(wasm)
}
