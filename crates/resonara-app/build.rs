fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("scarlet") {
        return;
    }
    // The resident /bin/scarlet-ld provides the four dl* imports. LLD requires
    // explicit PIE/eager binding and the CLAP crate's symbol-free DSO link seed
    // to retain them as genuine dynamic imports rather than zero direct calls.
    // Every deployable artifact MUST pass audit-scarlet-host.py after linking:
    // it rejects any other unresolved symbol or unsupported ELF feature.
    for argument in [
        "-pie",
        "--dynamic-linker=/bin/scarlet-ld",
        "--export-dynamic",
        "--unresolved-symbols=ignore-all",
        "-z",
        "now",
    ] {
        println!("cargo:rustc-link-arg-bin=resonara={argument}");
        println!("cargo:rustc-link-arg-examples={argument}");
    }
}
