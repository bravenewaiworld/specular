//! Spike S0: does a Rust binary built with our cross toolchain run on the Brick? Checks threads,
//! loading TrimUI's EGL library at runtime, and panic backtraces.

fn main() {
    let worker = std::thread::spawn(|| (1..=10u64).product::<u64>());
    println!("thread: 10! = {}", worker.join().unwrap());

    match unsafe { libloading::Library::new("libEGL.so.1") } {
        Ok(lib) => {
            let f = unsafe { lib.get::<unsafe extern "C" fn()>(b"eglGetDisplay\0") };
            println!("libEGL.so.1: loaded, eglGetDisplay {}", if f.is_ok() { "found" } else { "missing" });
        }
        Err(e) => println!("libEGL.so.1: {e}"),
    }

    let r = std::panic::catch_unwind(|| panic!("test panic"));
    println!("panic caught: {}", r.is_err());
}
