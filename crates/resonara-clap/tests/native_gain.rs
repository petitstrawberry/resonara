//! Real native shared-library integration; run after building the bundled effect:
//! cargo build --release --manifest-path plugins/resonara-gain/Cargo.toml
//! cargo test -p resonara-clap --test native_gain -- --ignored
use resonara_clap::{HostPlugin, ProcessError, ProcessStatus, discover};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    path::PathBuf,
};

#[test]
#[ignore = "requires native gain fixture"]
fn audio_only_plugin_has_no_native_gui_capability() {
    let host = load();
    assert_eq!(
        host.gui_support(c"cocoa").unwrap(),
        resonara_clap::GuiSupport::default()
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires native gain fixture"]
fn macos_bundle_uses_declared_executable_and_shares_binary_lease() {
    let path = std::env::temp_dir().join(format!("resonara-bundle-{}.clap", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
    // Name deliberately differs from the bundle; guessing its stem would fail.
    let binary = path.join("Contents/MacOS/ActualGain");
    std::fs::copy(gain_path(), &binary).unwrap();
    std::fs::write(
        path.join("Contents/Info.plist"),
        br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleExecutable</key><string>ActualGain</string>
<key>CFBundleIdentifier</key><string>org.resonara.test.bundle</string>
<key>CFBundlePackageType</key><string>BNDL</string></dict></plist>"#,
    )
    .unwrap();
    let mut bundle = HostPlugin::load(&path, Some("org.resonara.gain")).unwrap();
    let binary_host = HostPlugin::load(&binary, Some("org.resonara.gain")).unwrap();
    bundle.set_parameter(0, 0.75).unwrap();
    assert_eq!(binary_host.parameter_value(0).unwrap(), 1.);
    assert_eq!(bundle.parameter_value(0).unwrap(), 0.75);
    drop(bundle);
    assert_eq!(binary_host.parameter_value(0).unwrap(), 1.);
    drop(binary_host);
    std::fs::remove_dir_all(path).unwrap();
}

struct CountingAllocator;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                ALLOCATIONS.with(|v| v.set(v.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                ALLOCATIONS.with(|v| v.set(v.get() + 1));
            }
        });
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                ALLOCATIONS.with(|v| v.set(v.get() + 1));
            }
        });
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let _ = COUNTING.try_with(|on| {
            if on.get() {
                DEALLOCATIONS.with(|v| v.set(v.get() + 1));
            }
        });
        unsafe { System.dealloc(ptr, layout) }
    }
}
fn gain_path() -> PathBuf {
    std::env::var_os("RESONARA_TEST_CLAP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let filename = if cfg!(target_os = "macos") {
                "libresonara_gain.dylib"
            } else {
                "libresonara_gain.so"
            };
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../plugins/resonara-gain/target/release")
                .join(filename)
        })
}
fn load() -> HostPlugin {
    HostPlugin::load(&gain_path(), Some("org.resonara.gain"))
        .expect("Build plugins/resonara-gain first or set RESONARA_TEST_CLAP")
}

#[test]
#[ignore = "requires the built native gain shared library"]
fn real_gain_discovery_parameters_state_and_audio() {
    let descriptors = discover(&gain_path()).unwrap();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].id, "org.resonara.gain");
    let mut plugin = load();
    assert_eq!(plugin.parameters().len(), 1);
    assert_eq!(plugin.parameters()[0].name, "Gain");
    assert_eq!(plugin.parameter_value(0).unwrap(), 1.);
    assert!(plugin.parameter_text(0, 0.5).unwrap().contains("0.5"));
    for value in [f64::NAN, f64::INFINITY, -0.01, 2.01] {
        assert!(plugin.set_parameter(0, value).is_err());
    }
    assert!(plugin.set_parameter(77, 1.).is_err());
    plugin.set_parameter(0, 0.5).unwrap();
    let state = plugin.save_state().unwrap();
    assert_eq!(state.len(), 16);
    assert_eq!(&state[..4], b"RSGN");
    plugin.set_parameter(0, 1.5).unwrap();
    plugin.load_state(&state).unwrap();
    assert_eq!(plugin.parameter_value(0).unwrap(), 0.5);
    assert!(plugin.load_state(b"bad state").is_err());
    assert_eq!(plugin.parameter_value(0).unwrap(), 0.5);
    let (owner, mut runtime) = plugin.activate(48000., 128).unwrap();
    assert!(owner.realtime_alive());
    for frames in [1, 3, 64, 128] {
        let mut audio = vec![[0.5, -0.25]; frames];
        assert_eq!(
            runtime.process(&mut audio).unwrap(),
            ProcessStatus::Continue
        );
        assert!(audio.iter().all(|f| *f == [0.25, -0.125]));
    }
    let mut oversized = vec![[0.5, -0.25]; 129];
    assert_eq!(
        runtime.process(&mut oversized),
        Err(ProcessError::InvalidBlock)
    );
    assert!(oversized.iter().all(|f| *f == [0.5, -0.25]));
    let mut bad = [[f32::NAN, 0.]];
    assert_eq!(runtime.process(&mut bad), Err(ProcessError::InvalidBlock));
    drop(runtime);
    assert!(!owner.realtime_alive());
    let mut inactive = owner.deactivate().unwrap();
    assert_eq!(inactive.save_state().unwrap(), state);
}

#[test]
#[ignore = "requires the built native gain shared library"]
fn real_gain_cross_thread_realtime_allocates_and_frees_nothing() {
    let mut plugin = load();
    plugin.set_parameter(0, 0.5).unwrap();
    let (owner, mut runtime) = plugin.activate(48000., 128).unwrap();
    std::thread::spawn(move || {
        let mut audio = [[0.5, -0.25]; 128];
        // Count the first start/process/stop too, including host thread-check.
        ALLOCATIONS.with(|v| v.set(0));
        DEALLOCATIONS.with(|v| v.set(0));
        COUNTING.with(|v| v.set(true));
        let result = runtime.process(&mut audio);
        for _ in 0..1000 {
            audio.fill([0.5, -0.25]);
            runtime.process(&mut audio).unwrap();
        }
        COUNTING.with(|v| v.set(false));
        assert!(result.is_ok());
        assert_eq!(ALLOCATIONS.with(Cell::get), 0);
        assert_eq!(DEALLOCATIONS.with(Cell::get), 0);
        assert_eq!(audio[0], [0.25, -0.125]);
        // Simulates CPAL dropping its closure on its backend thread.
        drop(runtime);
    })
    .join()
    .unwrap();
    let mut inactive = owner.deactivate().unwrap();
    assert_eq!(inactive.parameter_value(0).unwrap(), 0.5);
    assert_eq!(inactive.save_state().unwrap().len(), 16);
}

#[test]
#[ignore = "requires the built native gain shared library"]
fn real_gain_instances_independent_and_owner_teardown_reclaims_slots() {
    let quarantined = resonara_clap::quarantined_instance_count();
    let mut first = load();
    let mut second = load();
    first.set_parameter(0, 0.25).unwrap();
    second.set_parameter(0, 1.5).unwrap();
    let (a_owner, mut a) = first.activate(44100., 16).unwrap();
    let (b_owner, mut b) = second.activate(44100., 16).unwrap();
    let mut block = [[0.5, -0.5]; 16];
    a.process(&mut block).unwrap();
    assert_eq!(block[0], [0.125, -0.125]);
    drop(a);
    drop(a_owner);
    block.fill([0.5, -0.5]);
    b.process(&mut block).unwrap();
    assert_eq!(block[0], [0.75, -0.75]);
    // Keep one lease alive so repeated create/destroy cannot hide a slot leak
    // behind library unloading and resetting its static pool of 64 slots.
    for _ in 0..100 {
        let (owner, mut rt) = load().activate(48000., 8).unwrap();
        std::thread::spawn(move || {
            let mut b = [[0.1, 0.2]; 8];
            rt.process(&mut b).unwrap();
            drop(rt);
        })
        .join()
        .unwrap();
        drop(owner);
    }
    drop(b);
    drop(b_owner);
    assert_eq!(resonara_clap::quarantined_instance_count(), quarantined);
}

#[test]
fn missing_plugin_is_a_recoverable_error() {
    assert!(
        discover(std::path::Path::new(
            "/definitely/not/a/resonara-plugin.clap"
        ))
        .is_err()
    );
    assert!(
        HostPlugin::load(
            std::path::Path::new("/definitely/not/a/resonara-plugin.clap"),
            None
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires the built native gain shared library"]
fn real_gain_nonfinite_output_fails_without_overwriting_dry_input() {
    let mut plugin = load();
    plugin.set_parameter(0, 2.).unwrap();
    let (owner, mut runtime) = plugin.activate(48000., 8).unwrap();
    let mut audio = [[f32::MAX, -f32::MAX]; 8];
    assert_eq!(
        runtime.process(&mut audio),
        Err(ProcessError::NonFiniteOutput)
    );
    assert_eq!(audio, [[f32::MAX, -f32::MAX]; 8]);
    assert_eq!(runtime.failure(), Some(ProcessError::NonFiniteOutput));
    audio.fill([0.1, 0.2]);
    assert_eq!(
        runtime.process(&mut audio),
        Err(ProcessError::NonFiniteOutput)
    );
    assert_eq!(audio, [[0.1, 0.2]; 8]);
    drop(runtime);
    drop(owner);
}
