// The core log's C shim (src/log.c). Cross builds give cc the device's compiler as CC_<target>.
fn main() {
    println!("cargo:rerun-if-changed=src/log.c");
    cc::Build::new().file("src/log.c").compile("specular_log");
}
