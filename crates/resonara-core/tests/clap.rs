//! Session/graph tests for CLAP. Missing identities never load external code.
use resonara_core::{graph::NodeId, *};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    path::PathBuf,
    sync::{Arc, OnceLock, atomic::Ordering},
};

thread_local! {
    static AUDITING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static FREES: Cell<usize> = const { Cell::new(0) };
}
struct Audit;
fn allocation() {
    AUDITING.with(|guard| {
        if guard.get() {
            ALLOCS.with(|count| count.set(count.get() + 1));
        }
    });
}
fn deallocation() {
    AUDITING.with(|guard| {
        if guard.get() {
            FREES.with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for Audit {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        deallocation();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        deallocation();
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Audit = Audit;

fn saved_gain(value: f64) -> ClapInsert {
    let mut state = b"RSGN".to_vec();
    state.extend_from_slice(&1u32.to_le_bytes());
    state.extend_from_slice(&value.to_le_bytes());
    ClapInsert {
        library: plugins::BUNDLED_GAIN_LIBRARY.into(),
        plugin_id: plugins::BUNDLED_GAIN_ID.into(),
        name: "Resonara Gain".into(),
        state,
        parameters: vec![ClapParameter {
            id: 0,
            name: "Gain".into(),
            min: 0.,
            max: 2.,
            value,
            stepped: false,
            read_only: false,
            hidden: false,
        }],
    }
}
fn missing() -> ClapInsert {
    ClapInsert {
        library: "unavailable.clap".into(),
        plugin_id: "org.example.absent".into(),
        ..saved_gain(1.5)
    }
}
fn project(plugin: ClapInsert, bypass: bool) -> Project {
    Project {
        tracks: vec![Track {
            name: "CLAP source".into(),
            clips: vec![Clip {
                source_channels: 2,
                edit: Default::default(),
                start: 0,
                source_offset: 0,
                frames: 512,
                samples: Arc::new(vec![[0.125, -0.25]; 512]),
            }],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: ChannelRouting {
                inserts: vec![Insert {
                    kind: InsertKind::Clap { plugin },
                    bypass,
                }],
                ..Default::default()
            },
        }],
        ..Project::default()
    }
}
fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("resonara-clap-{}-{name}", std::process::id()))
}

#[test]
#[ignore = "requires resonara-gain.clap in CLAP_PATH with no RESONARA_CLAP_LIBRARY override"]
fn standard_effect_uses_the_installation_catalog_without_an_override() {
    assert!(std::env::var_os("RESONARA_CLAP_LIBRARY").is_none());
    let insert = plugins::load_bundled_gain().unwrap();
    assert!(insert.is_bundled_gain());
    let edited = plugins::set_parameter(&insert, 0, 0.5).unwrap();
    let loaded = project(edited, false);
    let mut engine = Engine::try_new(&loaded, Arc::new(Controls::new(&loaded)), 48000, 0).unwrap();
    assert_eq!(engine.graph_info().unavailable_plugins, 0);
    let mut output = [0.; 64];
    engine.render(&mut output, 2);
    assert_eq!(output, [0.0625, -0.125].repeat(32).as_slice());
}

#[test]
#[ignore = "requires external-gain.clap in CLAP_PATH (a copy of the native gain fixture)"]
fn installed_effect_discovery_state_reopen_render_and_export() {
    let catalog = plugins::scan_installed();
    let choice = catalog
        .effects
        .iter()
        .find(|c| c.library == "external-gain.clap")
        .unwrap_or_else(|| panic!("External fixture missing: {:?}", catalog.warnings));
    let default = plugins::load_installed(choice).unwrap();
    assert_eq!(default.library, "external-gain.clap");
    assert!(plugins::is_available(&default));
    let edited = plugins::set_parameters(&default, &[(0, 0.5)]).unwrap();
    assert_eq!(edited.library, default.library);
    assert!(plugins::set_parameters(&default, &[(0, 0.5), (99, 1.)]).is_err());
    assert_eq!(default.parameters[0].value, 1.);
    let path = temp("external-session.json");
    project(edited, false).save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let mut engine = Engine::try_new(&loaded, Arc::new(Controls::new(&loaded)), 48000, 0).unwrap();
    assert_eq!(engine.graph_info().unavailable_plugins, 0);
    let mut output = [0.; 64];
    ALLOCS.with(|n| n.set(0));
    FREES.with(|n| n.set(0));
    AUDITING.with(|a| a.set(true));
    engine.render(&mut output, 2);
    AUDITING.with(|a| a.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(FREES.with(Cell::get), 0);
    assert_eq!(output, [0.0625, -0.125].repeat(32).as_slice());
    let wav = temp("external-export.wav");
    loaded.export_wav(&wav).unwrap();
    let samples = hound::WavReader::open(&wav)
        .unwrap()
        .into_samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(samples, [0.0625, -0.125].repeat(512));
    std::fs::remove_file(wav).unwrap();
}

#[test]
fn old_parameter_metadata_defaults_to_editable_continuous_visible() {
    let parameter: ClapParameter =
        serde_json::from_str(r#"{"id":0,"name":"Gain","min":0,"max":2,"value":1}"#).unwrap();
    assert!(!parameter.stepped && !parameter.read_only && !parameter.hidden);
}

#[test]
fn unknown_plugin_state_and_cached_parameters_survive_save_load() {
    let p = project(missing(), false);
    let path = temp("missing.resonara.json");
    p.save(&path).unwrap();
    let loaded = Project::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(loaded.tracks[0].routing, p.tracks[0].routing);
    let controls = Arc::new(Controls::new(&loaded));
    let mut engine = Engine::try_new(&loaded, controls.clone(), 48000, 0).unwrap();
    assert_eq!(controls.unavailable_plugins.load(Ordering::Relaxed), 1);
    assert_eq!(engine.graph_info().unavailable_plugins, 1);
    let mut output = [0f32; 64];
    engine.render(&mut output, 2);
    assert_eq!(output, [0.125, -0.25].repeat(32).as_slice());
    assert!(!controls.error.load(Ordering::Relaxed));
}

#[test]
fn bypassed_missing_plugin_is_not_required_or_counted() {
    let p = project(missing(), true);
    let mut engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), 48000, 0).unwrap();
    assert_eq!(engine.graph_info().unavailable_plugins, 0);
    assert!(engine.take_plugin_owners().is_empty());
    let path = temp("bypassed.wav");
    p.export_wav(&path).unwrap();
    assert!(path.is_file());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn missing_insert_bypass_updates_warning_without_replacing_engine() {
    let mut p = project(missing(), true);
    let (mut playback, mut renderer) = live::Playback::new(&p, 48000, 0, false).unwrap();
    let controls = playback.controls.clone();
    for bypass in [false, true, false, true] {
        p.tracks[0].routing.inserts[0].bypass = bypass;
        playback.update(&p).unwrap();
        assert!(Arc::ptr_eq(&controls, &playback.controls));
        assert_eq!(
            controls.unavailable_plugins.load(Ordering::Relaxed),
            u32::from(!bypass)
        );
        let mut output = [0.; 2];
        renderer.render(&mut output, 2);
        assert_eq!(output, [0.125, -0.25]);
        assert!(!controls.error.load(Ordering::Relaxed));
    }
    drop(renderer);
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn initially_bypassed_clap_is_loaded_and_toggles_without_replacement() {
    require_effect();
    let mut p = project(saved_gain(0.5), true);
    let mut engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), 48000, 0).unwrap();
    let owners = engine.take_plugin_owners();
    assert_eq!(
        owners.len(),
        1,
        "Bypassed GUI needs the same live plugin owner"
    );
    drop(engine);
    drop(owners);
    let (mut playback, mut renderer) = live::Playback::new(&p, 48000, 0, false).unwrap();
    let controls = playback.controls.clone();
    for bypass in [true, false, true, false] {
        p.tracks[0].routing.inserts[0].bypass = bypass;
        playback.update(&p).unwrap();
        assert!(Arc::ptr_eq(&controls, &playback.controls));
        let mut output = [0.; 2];
        ALLOCS.with(|n| n.set(0));
        FREES.with(|n| n.set(0));
        AUDITING.with(|a| a.set(true));
        renderer.render(&mut output, 2);
        AUDITING.with(|a| a.set(false));
        assert_eq!(
            output,
            if bypass {
                [0.125, -0.25]
            } else {
                [0.0625, -0.125]
            }
        );
        assert_eq!(ALLOCS.with(Cell::get), 0);
        assert_eq!(FREES.with(Cell::get), 0);
        assert!(controls.playing.load(Ordering::Relaxed));
    }
    assert_eq!(controls.position.load(Ordering::Relaxed), 4);
    drop(renderer);
}

#[test]
fn missing_plugin_export_refuses_before_creating_or_truncating_destination() {
    let p = project(missing(), false);
    let absent = temp("must-not-create.wav");
    let _ = std::fs::remove_file(&absent);
    assert!(p.export_wav(&absent).is_err());
    assert!(!absent.exists());
    let existing = temp("must-not-truncate.wav");
    std::fs::write(&existing, b"keep this file").unwrap();
    assert!(p.export_wav(&existing).is_err());
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep this file");
    std::fs::remove_file(existing).unwrap();
}

#[test]
fn saved_library_paths_and_unknown_ids_are_never_loader_input() {
    for (library, id) in [
        ("../resonara-gain.clap", plugins::BUNDLED_GAIN_ID),
        ("/tmp/resonara-gain.clap", plugins::BUNDLED_GAIN_ID),
        ("plugins/resonara-gain.clap", plugins::BUNDLED_GAIN_ID),
        (plugins::BUNDLED_GAIN_LIBRARY, "org.example.other"),
    ] {
        let plugin = ClapInsert {
            library: library.into(),
            plugin_id: id.into(),
            ..saved_gain(1.)
        };
        assert!(!plugins::is_available(&plugin));
        assert!(plugins::set_parameter(&plugin, 0, 0.5).is_err());
        let p = project(plugin, false);
        p.validate().unwrap(); // Kept as an unavailable, recoverable project entry.
        let engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), 48000, 0).unwrap();
        assert_eq!(engine.graph_info().unavailable_plugins, 1);
    }
}

#[test]
fn clap_storage_and_parameter_metadata_are_bounded_even_when_bypassed() {
    let mut plugin = saved_gain(1.);
    plugin.state = vec![0; plugins::MAX_STATE_BYTES + 1];
    assert!(project(plugin.clone(), true).validate().is_err());
    let json = serde_json::to_string(&plugin).unwrap();
    assert!(serde_json::from_str::<ClapInsert>(&json).is_err());
    for value in [f64::NAN, f64::INFINITY, -1., 3.] {
        let mut plugin = saved_gain(1.);
        plugin.parameters[0].value = value;
        assert!(project(plugin, false).validate().is_err());
    }
    let mut plugin = saved_gain(1.);
    plugin.parameters.push(plugin.parameters[0].clone());
    assert!(project(plugin, false).validate().is_err());
    let mut plugin = saved_gain(1.);
    plugin.parameters = (0..=plugins::MAX_PARAMETERS)
        .map(|id| ClapParameter {
            id: id as u32,
            name: "P".into(),
            min: 0.,
            max: 1.,
            value: 0.,
            stepped: false,
            read_only: false,
            hidden: false,
        })
        .collect();
    assert!(plugin.validate().is_err());
    assert!(serde_json::from_str::<ClapInsert>(&serde_json::to_string(&plugin).unwrap()).is_err());
    let mut p = project(missing(), false);
    let mut plugin = missing();
    plugin.state = vec![0; plugins::MAX_STATE_BYTES];
    p.tracks[0].routing.inserts = vec![
        Insert {
            kind: InsertKind::Clap { plugin },
            bypass: true
        };
        17
    ];
    assert!(p.validate().is_err());
}

// Actual bundled-effect tests require the artifact built by
// python3 plugins/resonara-gain/build.py --arch linux --output <directory>.
// Running without the deployment fixture is an explicit ignored test rather
// than silently passing a placeholder as successful plug-in processing.
fn require_effect() {
    static CHECKED: OnceLock<()> = OnceLock::new();
    CHECKED.get_or_init(|| {
        assert!(plugins::is_available(&saved_gain(1.)), "Build/install the bundled effect or set RESONARA_CLAP_LIBRARY to its absolute .clap file");
    });
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn live_clap_bypass_parameter_and_removal_keep_transport_and_retire_off_callback() {
    require_effect();
    let mut p = project(saved_gain(0.5), false);
    let (mut playback, mut renderer) = live::Playback::new(&p, 48000, 0, false).unwrap();
    let mut output = [0.; 2];
    renderer.render(&mut output, 2);
    assert_eq!(output, [0.0625, -0.125]);
    for (bypass, gain) in [(true, 0.5), (false, 0.5), (false, 2.), (true, 2.)] {
        p.tracks[0].routing.inserts[0] = Insert {
            kind: InsertKind::Clap {
                plugin: saved_gain(gain),
            },
            bypass,
        };
        playback.update(&p).unwrap();
        ALLOCS.with(|n| n.set(0));
        FREES.with(|n| n.set(0));
        AUDITING.with(|a| a.set(true));
        renderer.render(&mut output, 2);
        AUDITING.with(|a| a.set(false));
        let gain = if bypass { 1. } else { gain as f32 };
        assert_eq!(output, [0.125 * gain, -0.25 * gain]);
        assert_eq!(ALLOCS.with(Cell::get), 0);
        assert_eq!(FREES.with(Cell::get), 0);
        assert!(playback.controls.playing.load(Ordering::Relaxed));
        assert_eq!(
            playback
                .controls
                .unavailable_plugins
                .load(Ordering::Relaxed),
            0
        );
    }
    p.tracks[0].routing.inserts.clear();
    playback.update(&p).unwrap();
    renderer.render(&mut output, 2);
    assert_eq!(output, [0.125, -0.25]);
    assert_eq!(playback.controls.position.load(Ordering::Relaxed), 6);
    // The backend destroys its render endpoint before the plugin lifecycle owner.
    drop(renderer);
    playback.collect_retired();
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn bundled_gain_parameter_and_opaque_state_roundtrip_render_and_export() {
    require_effect();
    let default = plugins::load_bundled_gain().unwrap();
    assert_eq!(default.plugin_id, plugins::BUNDLED_GAIN_ID);
    assert_eq!(default.library, plugins::BUNDLED_GAIN_LIBRARY);
    assert_eq!(default.parameters[0].value, 1.);
    let updated = plugins::set_parameter(&default, 0, 0.5).unwrap();
    assert_eq!(updated.parameters[0].value, 0.5);
    assert_eq!(updated.state, saved_gain(0.5).state);
    assert_eq!(default.parameters[0].value, 1.); // Editing a snapshot is nonmutating.
    assert!(plugins::set_parameter(&updated, 0, 3.).is_err());
    assert!(plugins::set_parameter(&updated, 99, 1.).is_err());
    let p = project(updated, false);
    let path = temp("gain.resonara.json");
    p.save(&path).unwrap();
    let p = Project::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let controls = Arc::new(Controls::new(&p));
    let mut engine = Engine::try_new(&p, controls.clone(), 44100, 0).unwrap();
    assert_eq!(engine.graph_info().unavailable_plugins, 0);
    let mut output = [0.; 64];
    engine.render(&mut output, 2);
    assert_eq!(output, [0.0625, -0.125].repeat(32).as_slice());
    assert!(!controls.error.load(Ordering::Relaxed));
    let path = temp("gain.wav");
    p.export_wav(&path).unwrap();
    let samples = hound::WavReader::open(&path)
        .unwrap()
        .into_samples::<f32>()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(samples, [0.0625, -0.125].repeat(512));
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn shared_clap_insert_processes_once_per_quantum_and_callback_never_allocates() {
    require_effect();
    let plugin = plugins::set_parameter(&plugins::load_bundled_gain().unwrap(), 0, 0.5).unwrap();
    let mut p = project(plugin, false);
    p.tracks[0].clips[0].frames = 257 * 1001;
    p.tracks[0].clips[0].samples = Arc::new(vec![[0.125, -0.25]; 257 * 1001]);
    let aux = p.add_bus("Parallel", BusKind::Aux);
    p.tracks[0].routing.sends.push(Send {
        target: aux,
        gain: 1.,
        pre_fader: true,
        enabled: true,
    });
    let graph = p.routing_graph().unwrap();
    let plugin_id = graph
        .nodes
        .iter()
        .find(|node| matches!(node.processor, graph::Processor::Clap { .. }))
        .unwrap()
        .id;
    let mut engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), 48000, 0).unwrap();
    assert_eq!(engine.graph_info().unavailable_plugins, 0);
    let mut output = [0.; 514];
    ALLOCS.with(|n| n.set(0));
    FREES.with(|n| n.set(0));
    AUDITING.with(|a| a.set(true));
    engine.render(&mut output, 2);
    AUDITING.with(|a| a.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "audio callback allocated");
    assert_eq!(FREES.with(Cell::get), 0, "audio callback freed storage");
    assert_eq!(output, [0.125, -0.25].repeat(257).as_slice());
    assert_eq!(engine.render_counts().quanta, 3);
    assert_eq!(engine.node_process_count(NodeId(plugin_id.0)), Some(3));
    AUDITING.with(|a| a.set(true));
    for i in 0..1000 {
        engine.controls.send_gains[0].store(
            (if i % 2 == 0 { 0.5f32 } else { 1.0f32 }).to_bits(),
            Ordering::Relaxed,
        );
        engine.render(&mut output, 2);
    }
    AUDITING.with(|a| a.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0, "repeated callbacks allocated");
    assert_eq!(FREES.with(Cell::get), 0, "repeated callbacks freed storage");
    assert_eq!(output, [0.125, -0.25].repeat(257).as_slice());
    assert_eq!(engine.render_counts().quanta, 3003);
    assert_eq!(engine.node_process_count(NodeId(plugin_id.0)), Some(3003));
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn invalid_state_becomes_placeholder_and_owner_guards_return_to_main_thread() {
    require_effect();
    for state in [Vec::new(), b"invalid opaque state".to_vec()] {
        let mut invalid = plugins::load_bundled_gain().unwrap();
        invalid.state = state;
        let p = project(invalid, false);
        let engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), 48000, 0).unwrap();
        assert_eq!(engine.graph_info().unavailable_plugins, 1);
        drop(engine);
    }
    let p = project(plugins::load_bundled_gain().unwrap(), false);
    let mut engine = Engine::try_new(&p, Arc::new(Controls::new(&p)), 48000, 0).unwrap();
    let owners = engine.take_plugin_owners();
    assert_eq!(owners.len(), 1);
    std::thread::spawn(move || {
        let mut output = [0.; 64];
        engine.render(&mut output, 2);
        assert_eq!(output, [0.125, -0.25].repeat(32).as_slice());
        drop(engine);
    })
    .join()
    .unwrap();
    for owner in owners {
        assert!(!owner.realtime_alive());
        // Explicit deactivation verifies lifecycle retirement succeeded rather
        // than relying on the host's quarantine/leak fallback for API misuse.
        let mut inactive = owner.deactivate().unwrap();
        assert_eq!(inactive.save_state().unwrap(), saved_gain(1.).state);
        drop(inactive); // Destroy on the original construction thread.
    }
}

#[test]
#[ignore = "requires built bundled CLAP effect; run with RESONARA_CLAP_LIBRARY and --include-ignored"]
fn plugin_process_failure_latches_silent_without_callback_allocations() {
    require_effect();
    let plugin = plugins::set_parameter(&plugins::load_bundled_gain().unwrap(), 0, 2.).unwrap();
    let mut p = project(plugin, false);
    p.tracks[0].clips[0].samples = Arc::new(vec![[f32::MAX, -f32::MAX]; 512]);
    p.validate().unwrap(); // Finite input; the external gain overflows its output.
    let controls = Arc::new(Controls::new(&p));
    let mut engine = Engine::try_new(&p, controls.clone(), 48000, 0).unwrap();
    assert_eq!(engine.graph_info().unavailable_plugins, 0);
    let mut output = [1.; 128];
    ALLOCS.with(|n| n.set(0));
    FREES.with(|n| n.set(0));
    AUDITING.with(|a| a.set(true));
    engine.render(&mut output, 2);
    AUDITING.with(|a| a.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(FREES.with(Cell::get), 0);
    assert_eq!(output, [0.; 128]);
    assert!(controls.error.load(Ordering::Relaxed));
    assert!(!controls.playing.load(Ordering::Relaxed));
    let counts = engine.render_counts();
    let source_calls = engine.node_process_count(NodeId(0));
    controls.playing.store(true, Ordering::Relaxed);
    output.fill(1.);
    engine.render(&mut output, 2);
    assert_eq!(output, [0.; 128]);
    assert_eq!(engine.render_counts(), counts);
    assert_eq!(engine.node_process_count(NodeId(0)), source_calls);
    assert!(!controls.playing.load(Ordering::Relaxed));
}
