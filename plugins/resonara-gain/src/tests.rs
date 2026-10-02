use super::*;
use clap_sys::audio_buffer::clap_audio_buffer;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    mem::{align_of, offset_of, size_of},
    sync::Mutex,
};

static SERIAL: Mutex<()> = Mutex::new(());
thread_local! {
    static AUDIO: Cell<bool> = const { Cell::new(false) };
    static COUNT_ALLOC: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}
struct CountingAllocator;
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOC.try_with(Cell::get).unwrap_or(false) {
            ALLOCS.with(|c| c.set(c.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNT_ALLOC.try_with(Cell::get).unwrap_or(false) {
            ALLOCS.with(|c| c.set(c.get() + 1));
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}
unsafe extern "C" fn is_main(_host: *const clap_host) -> bool {
    !AUDIO.with(Cell::get)
}
unsafe extern "C" fn is_audio(_host: *const clap_host) -> bool {
    AUDIO.with(Cell::get)
}
static THREADS: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(is_main),
    is_audio_thread: Some(is_audio),
};
unsafe extern "C" fn host_extension(_host: *const clap_host, id: *const c_char) -> *const c_void {
    if unsafe { is_id(id, CLAP_EXT_THREAD_CHECK) } {
        (&THREADS as *const clap_host_thread_check).cast()
    } else {
        ptr::null()
    }
}
static HOST: clap_host = clap_host {
    clap_version: CLAP_VERSION,
    host_data: ptr::null_mut(),
    name: c"Tests".as_ptr(),
    vendor: c"Resonara".as_ptr(),
    url: c"".as_ptr(),
    version: c"1".as_ptr(),
    get_extension: Some(host_extension),
    request_restart: None,
    request_process: None,
    request_callback: None,
};
struct Instance(*const clap_plugin);
impl Instance {
    fn new() -> Self {
        unsafe {
            assert!(entry_init(c"test".as_ptr()));
            let p = create(&FACTORY, &HOST, PLUGIN_ID.as_ptr());
            assert!(!p.is_null());
            assert!(plugin_init(p));
            Self(p)
        }
    }
    fn value(&self) -> f64 {
        let mut n = -1.;
        unsafe {
            assert!(param_value(self.0, 0, &mut n));
        }
        n
    }
    fn activate(&self) {
        unsafe {
            assert!(activate(self.0, 48000., 1, 1024));
        }
    }
    fn start(&self) {
        AUDIO.with(|a| a.set(true));
        unsafe {
            assert!(start(self.0));
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let state = slot(self.0).unwrap().lifecycle.load(Ordering::Acquire);
            AUDIO.with(|a| a.set(true));
            if state == PROCESSING {
                stop(self.0);
            }
            AUDIO.with(|a| a.set(false));
            if state >= ACTIVE {
                deactivate(self.0);
            }
            destroy(self.0);
            entry_deinit();
        }
    }
}
fn event(time: u32, value: f64) -> clap_event_param_value {
    clap_event_param_value {
        header: clap_event_header {
            size: size_of::<clap_event_param_value>() as u32,
            time,
            space_id: 0,
            type_: CLAP_EVENT_PARAM_VALUE,
            flags: 0,
        },
        param_id: 0,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    }
}
struct Events(Vec<clap_event_param_value>);
impl Events {
    fn list(&self) -> clap_input_events {
        clap_input_events {
            ctx: (self as *const Self).cast_mut().cast(),
            size: Some(events_size),
            get: Some(events_get),
        }
    }
}
unsafe extern "C" fn events_size(list: *const clap_input_events) -> u32 {
    unsafe { (&*(*list).ctx.cast::<Events>()).0.len() as u32 }
}
unsafe extern "C" fn events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    unsafe { &(&*(*list).ctx.cast::<Events>()).0[index as usize].header }
}
fn audio_buffer(channels: &mut [*mut f32; 2]) -> clap_audio_buffer {
    clap_audio_buffer {
        data32: channels.as_mut_ptr(),
        data64: ptr::null_mut(),
        channel_count: 2,
        latency: 0,
        constant_mask: 0,
    }
}
fn block(
    input: &clap_audio_buffer,
    output: &mut clap_audio_buffer,
    events: &clap_input_events,
) -> clap_process {
    clap_process {
        steady_time: -1,
        frames_count: 8,
        transport: ptr::null(),
        audio_inputs: input,
        audio_outputs: output,
        audio_inputs_count: 1,
        audio_outputs_count: 1,
        in_events: events,
        out_events: ptr::null(),
    }
}

#[test]
fn abi_matches_registry_bindings() {
    macro_rules! same {
        ($module:ident::$ty:ident, $($field:ident),+ $(,)?) => {{
            type Local = clap_sys::$module::$ty;
            type Original = clap_sys_upstream::$module::$ty;
            assert_eq!(size_of::<Local>(), size_of::<Original>());
            assert_eq!(align_of::<Local>(), align_of::<Original>());
            $(assert_eq!(offset_of!(Local, $field), offset_of!(Original, $field));)+
        }};
    }
    same!(
        entry::clap_plugin_entry,
        clap_version,
        init,
        deinit,
        get_factory
    );
    same!(
        plugin::clap_plugin_descriptor,
        clap_version,
        id,
        name,
        vendor,
        url,
        manual_url,
        support_url,
        version,
        description,
        features
    );
    same!(
        plugin::clap_plugin,
        desc,
        plugin_data,
        init,
        destroy,
        activate,
        deactivate,
        start_processing,
        stop_processing,
        reset,
        process,
        get_extension,
        on_main_thread
    );
    same!(
        host::clap_host,
        clap_version,
        host_data,
        name,
        vendor,
        url,
        version,
        get_extension,
        request_restart,
        request_process,
        request_callback
    );
    same!(
        process::clap_process,
        steady_time,
        frames_count,
        transport,
        audio_inputs,
        audio_outputs,
        audio_inputs_count,
        audio_outputs_count,
        in_events,
        out_events
    );
    same!(
        audio_buffer::clap_audio_buffer,
        data32,
        data64,
        channel_count,
        latency,
        constant_mask
    );
    same!(
        events::clap_event_param_value,
        header,
        param_id,
        cookie,
        note_id,
        port_index,
        channel,
        key,
        value
    );
    same!(events::clap_input_events, ctx, size, get);
    same!(events::clap_output_events, ctx, try_push);
    same!(stream::clap_istream, ctx, read);
    same!(stream::clap_ostream, ctx, write);
    assert_eq!(
        size_of::<clap_param_info>(),
        size_of::<clap_sys_upstream::ext::params::clap_param_info>()
    );
    assert_eq!(
        size_of::<clap_audio_port_info>(),
        size_of::<clap_sys_upstream::ext::audio_ports::clap_audio_port_info>()
    );
}

#[test]
fn lifecycle_enforces_order_and_threads() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    unsafe {
        assert!(!plugin_init(p.0));
        assert!(!start(p.0));
        assert_eq!(process(p.0, ptr::null()), CLAP_PROCESS_ERROR);
        for (rate, min, max) in [(0., 1, 8), (f64::NAN, 1, 8), (48000., 0, 8), (48000., 9, 8)] {
            assert!(!activate(p.0, rate, min, max));
        }
        AUDIO.with(|a| a.set(true));
        assert!(!activate(p.0, 48000., 1, 8));
        AUDIO.with(|a| a.set(false));
        p.activate();
        assert!(!start(p.0));
        p.start();
        assert!(!start(p.0));
        // A wrong-thread stop must not silently terminate the running instance.
        AUDIO.with(|a| a.set(false));
        stop(p.0);
        assert_eq!(
            slot(p.0).unwrap().lifecycle.load(Ordering::Acquire),
            PROCESSING
        );
    }
}

#[test]
fn metadata_and_text_are_bounded() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    unsafe {
        assert_eq!(plugin_count(&FACTORY), 1);
        assert!(descriptor(&FACTORY, 1).is_null());
        assert!(extension(p.0, c"clap.gui".as_ptr()).is_null());
        assert!(!extension(p.0, CLAP_EXT_STATE.as_ptr()).is_null());
        let mut port = core::mem::zeroed();
        assert!(port_info(p.0, 0, true, &mut port));
        assert_eq!(port.channel_count, 2);
        assert_eq!(port.in_place_pair, 0);
        assert_eq!(CStr::from_ptr(port.port_type), CLAP_PORT_STEREO);
        assert!(!port_info(p.0, 1, true, &mut port));
        let mut info = core::mem::zeroed();
        assert!(param_info(p.0, 0, &mut info));
        assert_eq!(
            (info.id, info.min_value, info.max_value, info.default_value),
            (0, 0., 2., 1.)
        );
        let mut buf = [42; 8];
        assert!(!value_to_text(p.0, 0, 1., buf.as_mut_ptr(), 5));
        assert_eq!(buf, [42; 8]);
        assert!(value_to_text(p.0, 0, 1.25, buf.as_mut_ptr(), 8));
        assert_eq!(CStr::from_ptr(buf.as_ptr()), c"1.250");
        let mut n = -1.;
        assert!(text_to_value(p.0, 0, c"0.125".as_ptr(), &mut n));
        assert!((n - 0.125).abs() < 1e-12);
        for text in [c"", c".", c"NaN", c"-1", c"3", c"1x", c"1.2.3"] {
            assert!(!text_to_value(p.0, 0, text.as_ptr(), &mut n));
        }
    }
}

#[test]
fn processing_applies_sample_offsets_and_supports_in_place() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    p.activate();
    p.start();
    let mut left = [0.25f32; 8];
    let mut right = [-0.5f32; 8];
    let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
    let input = audio_buffer(&mut channels);
    let mut output = audio_buffer(&mut channels);
    let events = Events(vec![
        event(0, 0.5),
        event(3, 2.),
        event(8, 0.),
        event(7, f64::NAN),
    ]);
    let list = events.list();
    let call = block(&input, &mut output, &list);
    unsafe {
        assert_eq!(process(p.0, &call), CLAP_PROCESS_CONTINUE);
    }
    assert_eq!(left, [0.125, 0.125, 0.125, 0.5, 0.5, 0.5, 0.5, 0.5]);
    assert_eq!(right, [-0.25, -0.25, -0.25, -1., -1., -1., -1., -1.]);
    assert_eq!(p.value(), 2.);
    assert_eq!(output.constant_mask, 0);
    unsafe {
        reset(p.0);
    }
    assert_eq!(p.value(), 2.);
}

#[test]
fn flush_rejects_wrong_targets_and_clamps_finite_values() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    let mut wrong = event(0, 0.);
    wrong.param_id = 99;
    let mut note = event(0, 0.);
    note.note_id = 1;
    let mut small = event(0, 0.);
    small.header.size = 1;
    let mut foreign = event(0, 0.);
    foreign.header.space_id = 5;
    let events = Events(vec![
        event(0, 0.25),
        wrong,
        note,
        small,
        foreign,
        event(0, f64::INFINITY),
    ]);
    unsafe {
        flush(p.0, &events.list(), ptr::null());
    }
    assert_eq!(p.value(), 0.25);
    let events = Events(vec![event(0, 10.)]);
    unsafe {
        flush(p.0, &events.list(), ptr::null());
    }
    assert_eq!(p.value(), 2.);
    p.activate(); // Active flush is audio-thread-only.
    let events = Events(vec![event(0, 0.)]);
    unsafe {
        flush(p.0, &events.list(), ptr::null());
    }
    assert_eq!(p.value(), 2.);
    AUDIO.with(|a| a.set(true));
    unsafe {
        flush(p.0, &events.list(), ptr::null());
    }
    assert_eq!(p.value(), 0.);
}

struct Bytes {
    data: Vec<u8>,
    offset: usize,
    chunk: usize,
}
unsafe extern "C" fn write_bytes(
    stream: *const clap_ostream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    let data = unsafe { &mut *(*stream).ctx.cast::<Bytes>() };
    let count = (size as usize).min(data.chunk);
    data.data
        .extend_from_slice(unsafe { core::slice::from_raw_parts(buffer.cast(), count) });
    count as i64
}
unsafe extern "C" fn read_bytes(
    stream: *const clap_istream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    let data = unsafe { &mut *(*stream).ctx.cast::<Bytes>() };
    let count = (size as usize)
        .min(data.chunk)
        .min(data.data.len() - data.offset);
    unsafe {
        ptr::copy_nonoverlapping(data.data.as_ptr().add(data.offset), buffer.cast(), count);
    }
    data.offset += count;
    count as i64
}
fn restore(p: &Instance, bytes: Vec<u8>, chunk: usize) -> bool {
    let mut data = Bytes {
        data: bytes,
        offset: 0,
        chunk,
    };
    let input = clap_istream {
        ctx: (&mut data as *mut Bytes).cast(),
        read: Some(read_bytes),
    };
    unsafe { load(p.0, &input) }
}
#[test]
fn versioned_state_handles_partial_io_and_rejects_corruption_atomically() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    let events = Events(vec![event(0, 0.375)]);
    unsafe {
        flush(p.0, &events.list(), ptr::null());
    }
    let mut data = Bytes {
        data: vec![],
        offset: 0,
        chunk: 3,
    };
    let output = clap_ostream {
        ctx: (&mut data as *mut Bytes).cast(),
        write: Some(write_bytes),
    };
    unsafe {
        assert!(save(p.0, &output));
    }
    assert_eq!(data.data.len(), 16);
    assert_eq!(&data.data[..8], b"RSGN\x01\0\0\0");
    assert_eq!(&data.data[8..], &0.375f64.to_le_bytes());
    let second = Instance::new();
    assert!(restore(&second, data.data.clone(), 1));
    assert_eq!(second.value(), 0.375);
    for len in 0..16 {
        assert!(!restore(&second, data.data[..len].to_vec(), 2));
    }
    let mut corrupt = data.data.clone();
    corrupt[0] = b'?';
    assert!(!restore(&second, corrupt, 16));
    let mut corrupt = data.data.clone();
    corrupt[4] = 2;
    assert!(!restore(&second, corrupt, 16));
    for bad in [f64::NAN, f64::INFINITY, -0.5, 2.01] {
        let mut corrupt = data.data.clone();
        corrupt[8..].copy_from_slice(&bad.to_le_bytes());
        assert!(!restore(&second, corrupt, 16));
    }
    assert!(!restore(&second, data.data.clone(), 0));
    assert_eq!(second.value(), 0.375);
    data.chunk = 0;
    unsafe {
        assert!(!save(p.0, &output));
    }
}

#[test]
fn pool_is_independent_bounded_and_reusable() {
    let _lock = SERIAL.lock().unwrap();
    let instances: Vec<_> = (0..CAPACITY).map(|_| Instance::new()).collect();
    unsafe {
        assert!(create(&FACTORY, &HOST, PLUGIN_ID.as_ptr()).is_null());
    }
    for (index, p) in instances.iter().enumerate() {
        let values = Events(vec![event(0, index as f64 / 64.)]);
        unsafe {
            flush(p.0, &values.list(), ptr::null());
        }
    }
    for (index, p) in instances.iter().enumerate() {
        assert_eq!(p.value(), index as f64 / 64.);
    }
    drop(instances);
    for _ in 0..CAPACITY * 2 {
        let p = Instance::new();
        assert_eq!(p.value(), 1.);
    }
    assert_eq!(ENTRY_USERS.load(Ordering::Acquire), 0);
}

#[test]
fn entry_refcount_and_concurrent_factory_admission() {
    let _lock = SERIAL.lock().unwrap();
    unsafe {
        assert!(get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()).is_null());
        assert!(entry_init(ptr::null()));
        assert!(entry_init(ptr::null()));
        entry_deinit();
        assert!(!get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()).is_null());
        entry_deinit();
        assert!(get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()).is_null());
        // Defensive underflow prevention; valid hosts balance calls exactly.
        entry_deinit();
        assert_eq!(ENTRY_USERS.load(Ordering::Acquire), 0);
    }
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..100 {
                    let p = Instance::new();
                    assert_eq!(p.value(), 1.);
                }
            });
        }
    });
    assert_eq!(ENTRY_USERS.load(Ordering::Acquire), 0);
    assert!(SLOTS.iter().all(|s| !s.taken.load(Ordering::Acquire)));
}

#[test]
fn thousand_process_blocks_allocate_and_free_nothing() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    p.activate();
    p.start();
    let mut left = [0.25f32; 8];
    let mut right = [-0.5f32; 8];
    let mut out_left = [0f32; 8];
    let mut out_right = [0f32; 8];
    let mut inputs = [left.as_mut_ptr(), right.as_mut_ptr()];
    let mut outputs = [out_left.as_mut_ptr(), out_right.as_mut_ptr()];
    let input = audio_buffer(&mut inputs);
    let mut output = audio_buffer(&mut outputs);
    let events = Events(vec![event(0, 0.5), event(4, 1.)]);
    let list = events.list();
    let call = block(&input, &mut output, &list);
    ALLOCS.with(|c| c.set(0));
    COUNT_ALLOC.with(|c| c.set(true));
    for _ in 0..1000 {
        unsafe {
            assert_eq!(process(p.0, &call), CLAP_PROCESS_CONTINUE);
        }
    }
    COUNT_ALLOC.with(|c| c.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(
        out_left,
        [0.125, 0.125, 0.125, 0.125, 0.25, 0.25, 0.25, 0.25]
    );
}

#[test]
fn malformed_process_calls_fail_without_writing_audio() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    p.activate();
    p.start();
    let mut left = [0.25f32; 8];
    let mut right = [-0.5f32; 8];
    let mut out_left = [7f32; 8];
    let mut out_right = [8f32; 8];
    let mut inputs = [left.as_mut_ptr(), right.as_mut_ptr()];
    let mut outputs = [out_left.as_mut_ptr(), out_right.as_mut_ptr()];
    let input = audio_buffer(&mut inputs);
    let mut output = audio_buffer(&mut outputs);
    let events = Events(vec![]);
    let list = events.list();
    let valid = block(&input, &mut output, &list);
    let mut bad = valid;
    bad.frames_count = 1025;
    unsafe {
        assert_eq!(process(p.0, &bad), CLAP_PROCESS_ERROR);
    }
    bad = valid;
    bad.audio_outputs_count = 0;
    unsafe {
        assert_eq!(process(p.0, &bad), CLAP_PROCESS_ERROR);
    }
    bad = valid;
    bad.audio_inputs = ptr::null();
    unsafe {
        assert_eq!(process(p.0, &bad), CLAP_PROCESS_ERROR);
    }
    let mut wrong_input = input;
    wrong_input.channel_count = 1;
    bad = valid;
    bad.audio_inputs = &wrong_input;
    unsafe {
        assert_eq!(process(p.0, &bad), CLAP_PROCESS_ERROR);
    }
    wrong_input = input;
    wrong_input.data32 = ptr::null_mut();
    bad.audio_inputs = &wrong_input;
    unsafe {
        assert_eq!(process(p.0, &bad), CLAP_PROCESS_ERROR);
    }
    AUDIO.with(|a| a.set(false));
    unsafe {
        assert_eq!(process(p.0, &valid), CLAP_PROCESS_ERROR);
    }
    assert_eq!(out_left, [7.; 8]);
    assert_eq!(out_right, [8.; 8]);
}
