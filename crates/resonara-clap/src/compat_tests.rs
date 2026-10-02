//! A native gain instance wrapped in a test vtable requiring a silent sidechain,
//! empty MIDI input, and a single continuous processing lifecycle.
use super::*;

#[repr(C)]
struct Probe {
    api: clap_plugin,
    original: clap_plugin,
    starts: AtomicU32,
    stops: AtomicU32,
}
unsafe fn probe<'a>(p: *const clap_plugin) -> &'a Probe {
    unsafe { &*p.cast::<Probe>() }
}
unsafe extern "C" fn start(p: *const clap_plugin) -> bool {
    let probe = unsafe { probe(p) };
    probe.starts.fetch_add(1, Ordering::Relaxed);
    unsafe { probe.original.start_processing.unwrap()(p) }
}
unsafe extern "C" fn stop(p: *const clap_plugin) {
    let probe = unsafe { probe(p) };
    probe.stops.fetch_add(1, Ordering::Relaxed);
    unsafe { probe.original.stop_processing.unwrap()(p) };
}
unsafe extern "C" fn process(
    p: *const clap_plugin,
    audio: *const clap_process,
) -> clap_process_status {
    let audio = unsafe { &*audio };
    if audio.audio_inputs_count != 2 || audio.audio_outputs_count != 1 {
        return CLAP_PROCESS_ERROR;
    }
    let side = unsafe { &*audio.audio_inputs.add(1) };
    if side.channel_count != 2 || side.data32.is_null() {
        return CLAP_PROCESS_ERROR;
    }
    for channel in 0..2 {
        let ptr = unsafe { *side.data32.add(channel) };
        if ptr.is_null()
            || unsafe { std::slice::from_raw_parts(ptr, audio.frames_count as usize) }
                .iter()
                .any(|v| *v != 0.)
        {
            return CLAP_PROCESS_ERROR;
        }
    }
    if audio.in_events.is_null()
        || unsafe { (*audio.in_events).size.unwrap()(audio.in_events) } != 0
    {
        return CLAP_PROCESS_ERROR;
    }
    let mut main = *audio;
    main.audio_inputs_count = 1;
    unsafe { probe(p).original.process.unwrap()(p, &main) }
}
unsafe extern "C" fn count(_: *const clap_plugin, input: bool) -> u32 {
    if input { 2 } else { 1 }
}
unsafe extern "C" fn get(
    p: *const clap_plugin,
    index: u32,
    input: bool,
    out: *mut clap_audio_port_info,
) -> bool {
    if input && index == 1 {
        let out = unsafe { &mut *out };
        out.id = 100;
        out.channel_count = 2;
        out.flags = 0;
        out.port_type = CLAP_PORT_STEREO.as_ptr();
        out.in_place_pair = CLAP_INVALID_ID;
        true
    } else {
        let extension =
            unsafe { probe(p).original.get_extension.unwrap()(p, CLAP_EXT_AUDIO_PORTS.as_ptr()) }
                .cast::<clap_plugin_audio_ports>();
        unsafe { (*extension).get.unwrap()(p, index, input, out) }
    }
}
unsafe extern "C" fn note_count(_: *const clap_plugin, input: bool) -> u32 {
    u32::from(input)
}
static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(count),
    get: Some(get),
};
static NOTES: clap_plugin_note_ports = clap_plugin_note_ports {
    count: Some(note_count),
    get: None,
};
unsafe extern "C" fn extension(p: *const clap_plugin, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_AUDIO_PORTS {
        (&PORTS as *const clap_plugin_audio_ports).cast()
    } else if id == CLAP_EXT_NOTE_PORTS {
        (&NOTES as *const clap_plugin_note_ports).cast()
    } else {
        unsafe { probe(p).original.get_extension.unwrap()(p, id.as_ptr()) }
    }
}

#[test]
#[ignore = "requires RESONARA_TEST_CLAP native gain fixture"]
fn disconnected_sidechain_midi_and_continuous_start_stop() {
    let path = std::env::var_os("RESONARA_TEST_CLAP").expect("Set RESONARA_TEST_CLAP");
    let mut host = HostPlugin::load(Path::new(&path), Some("org.resonara.gain")).unwrap();
    let original = unsafe { *host.instance().plugin };
    let mut fixture = Box::new(Probe {
        api: original,
        original,
        starts: AtomicU32::new(0),
        stops: AtomicU32::new(0),
    });
    fixture.api.start_processing = Some(start);
    fixture.api.stop_processing = Some(stop);
    fixture.api.process = Some(process);
    fixture.api.get_extension = Some(extension);
    // This wrapper preserves the gain's plugin_data and forwards its other
    // callbacks unchanged. The Box outlives every controller and audio proxy.
    host.instance_mut().plugin = &fixture.api;
    let (owner, mut runtime) = host.activate(48000., 128).unwrap();
    runtime.process(&mut []).unwrap();
    assert_eq!(fixture.starts.load(Ordering::Relaxed), 0);
    for size in [1, 128, 17, 64] {
        let mut audio = vec![[0.125, -0.25]; size];
        runtime.process(&mut audio).unwrap();
        assert!(audio.iter().all(|frame| *frame == [0.125, -0.25]));
    }
    assert_eq!(fixture.starts.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.stops.load(Ordering::Relaxed), 0);
    drop(runtime);
    assert_eq!(fixture.stops.load(Ordering::Relaxed), 1);
    let inactive = owner.deactivate().unwrap();
    assert_eq!(inactive.parameter_value(0).unwrap(), 1.);
    drop(inactive);
}

#[test]
#[ignore = "requires RESONARA_TEST_CLAP native gain fixture"]
fn paused_flush_and_seek_do_not_stop_or_destroy_the_processing_instance() {
    let path = std::env::var_os("RESONARA_TEST_CLAP").expect("Set RESONARA_TEST_CLAP");
    let mut host = HostPlugin::load(Path::new(&path), Some("org.resonara.gain")).unwrap();
    let original = unsafe { *host.instance().plugin };
    let mut fixture = Box::new(Probe {
        api: original,
        original,
        starts: AtomicU32::new(0),
        stops: AtomicU32::new(0),
    });
    fixture.api.start_processing = Some(start);
    fixture.api.stop_processing = Some(stop);
    host.instance_mut().plugin = &fixture.api;
    let (owner, mut runtime) = host.activate(48000., 128).unwrap();
    // Active-but-not-started is a legal CLAP flush phase for a stopped editor.
    owner
        .instance()
        .context
        .flush
        .store(true, Ordering::Relaxed);
    runtime.flush_pending().unwrap();
    assert_eq!(fixture.starts.load(Ordering::Relaxed), 0);
    for _ in 0..4 {
        let mut audio = [[0.125, -0.25]; 64];
        runtime.process(&mut audio).unwrap();
        assert_eq!(audio[0], [0.125, -0.25]);
        owner
            .instance()
            .context
            .flush
            .store(true, Ordering::Relaxed);
        runtime.flush_pending().unwrap();
        runtime.reset_transport();
        owner.service_main_thread().unwrap();
        assert_eq!(fixture.starts.load(Ordering::Relaxed), 1);
        assert_eq!(fixture.stops.load(Ordering::Relaxed), 0);
    }
    drop(runtime);
    assert_eq!(fixture.stops.load(Ordering::Relaxed), 1);
    drop(owner);
}
