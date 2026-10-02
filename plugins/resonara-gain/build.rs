fn main() {
    // A Linux loadable image needs no CRT constructor/destructor or libc startup.
    // Native Scarlet uses its own isolated target/linker settings in build.py.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-cdylib=-nostartfiles");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-Bsymbolic");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,defs");
    }
}
