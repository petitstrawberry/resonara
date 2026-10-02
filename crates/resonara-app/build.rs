fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("scarlet") {
        bundle_freeverb();
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

fn bundle_freeverb() {
    use std::{env, fs, path::PathBuf, process::Command};
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    if !matches!(target_os.as_str(), "macos" | "linux") {
        return;
    }
    let plugin = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../plugins/resonara-freeverb");
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "src",
        "vendor/clap-sys",
    ] {
        println!("cargo:rerun-if-changed={}", plugin.join(name).display());
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    // A distinct target directory and no inherited jobserver tokens avoid
    // recursively waiting on the application's Cargo build lock or jobs.
    let build = out.join("freeverb-target");
    let status = Command::new(env::var_os("CARGO").unwrap())
        .args([
            "build",
            "--locked",
            "--release",
            "--jobs",
            "2",
            "--target",
            &target,
            "--manifest-path",
        ])
        .arg(plugin.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&build)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_MAKEFLAGS")
        .status()
        .expect("build bundled Freeverb");
    assert!(status.success(), "Bundled Freeverb build failed");
    let file = if target_os == "macos" {
        "libresonara_freeverb.dylib"
    } else {
        "libresonara_freeverb.so"
    };
    let profile = out.ancestors().nth(3).expect("Cargo profile directory");
    let directory = profile.join("plugins");
    fs::create_dir_all(&directory).unwrap();
    let temporary = directory.join("resonara-freeverb.clap.tmp");
    fs::copy(build.join(&target).join("release").join(file), &temporary).unwrap();
    fs::rename(temporary, directory.join("resonara-freeverb.clap")).unwrap();
    let mut notices = String::new();
    for name in [
        "LICENSE",
        "vendor/freeverb/LICENSE",
        "vendor/clap-sys/LICENSE-MIT",
        "vendor/clap-sys/LICENSE-CLAP",
    ] {
        println!("cargo:rerun-if-changed={}", plugin.join(name).display());
        notices.push_str(&format!(
            "=== {name} ===\n\n{}\n",
            fs::read_to_string(plugin.join(name)).unwrap()
        ));
    }
    fs::write(directory.join("resonara-freeverb.LICENSE.txt"), notices).unwrap();
}
