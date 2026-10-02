use super::*;
use clap_sys::audio_buffer::clap_audio_buffer;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    mem::size_of,
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
fn port_matches_upstream_freeverb_sample_for_sample() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    let s = unsafe { slot(p.0).unwrap() };
    // Use static DSP storage, not the test/audio thread stack.
    let dsp = unsafe { &mut *s.dsp.get() };
    for rate in [8000, 44100, 48000, 96000] {
        for values in [[0.3, 1., 0.5, 0.5, 1.], [1., 0., 1., 0., 0.25]] {
            dsp.configure(rate);
            let mut reference = freeverb_reference::Freeverb::new(rate);
            for (id, v) in values.into_iter().enumerate() {
                s.set(id, v);
            }
            dsp.set_wet(values[0]);
            dsp.set_dry(values[1]);
            dsp.set_room_size(values[2]);
            dsp.set_dampening(values[3]);
            dsp.set_width(values[4]);
            reference.set_wet(values[0]);
            reference.set_dry(values[1]);
            reference.set_room_size(values[2]);
            reference.set_dampening(values[3]);
            reference.set_width(values[4]);
            for frame in 0..30000 {
                let input = if frame == 0 {
                    (1.0, 0.5)
                } else if frame < 1000 {
                    ((frame % 17) as f64 * 0.002, -(frame % 13) as f64 * 0.003)
                } else {
                    (0.0, 0.0)
                };
                let a = dsp.tick(input);
                let b = reference.tick(input);
                assert!(
                    (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12,
                    "rate={rate} frame={frame}"
                );
            }
            dsp.clear();
            assert_eq!(dsp.tick((0., 0.)), (0., 0.));
        }
    }
}

#[test]
fn clap_process_has_tail_in_place_events_and_no_audio_allocations() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    p.activate();
    p.start();
    let mut left = [0f32; 256];
    let mut right = [0f32; 256];
    let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
    let input = audio_buffer(&mut channels);
    let mut output = audio_buffer(&mut channels);
    let mut dry = event(3, 0.0);
    dry.param_id = 1;
    let events = Events(vec![dry]);
    let list = events.list();
    // The event list must outlive all callbacks.
    let mut call = block(&input, &mut output, &list);
    call.frames_count = 256;
    left.fill(0.25);
    right.fill(-0.125);
    COUNT_ALLOC.with(|c| c.set(true));
    unsafe {
        assert_eq!(process(p.0, &call), CLAP_PROCESS_CONTINUE);
    }
    COUNT_ALLOC.with(|c| c.set(false));
    assert_eq!(&left[..4], &[0.25, 0.25, 0.25, 0.]);
    assert_eq!(&right[..4], &[-0.125, -0.125, -0.125, 0.]);
    let empty_events = Events(vec![]);
    let empty = empty_events.list();
    call.in_events = &empty;
    unsafe {
        reset(p.0);
    }
    let mut tail_energy = 0.0;
    ALLOCS.with(|c| c.set(0));
    COUNT_ALLOC.with(|c| c.set(true));
    for b in 0..400 {
        left.fill(0.);
        right.fill(0.);
        if b == 0 {
            left[0] = 1.;
            right[0] = 1.;
        }
        unsafe {
            assert_eq!(process(p.0, &call), CLAP_PROCESS_CONTINUE);
        }
        for v in left.iter().chain(&right) {
            assert!(v.is_finite());
        }
        if b > 10 {
            tail_energy += left.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        }
    }
    unsafe {
        reset(p.0);
    }
    left.fill(0.);
    right.fill(0.);
    unsafe {
        assert_eq!(process(p.0, &call), CLAP_PROCESS_CONTINUE);
    }
    COUNT_ALLOC.with(|c| c.set(false));
    assert!(tail_energy > 0.00001, "missing reverb tail: {tail_energy}");
    assert_eq!(left, [0.; 256]);
    assert_eq!(right, [0.; 256]);
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(p.value(), 0.3);
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
fn parameters_state_and_capacity_are_bounded() {
    let _lock = SERIAL.lock().unwrap();
    let p = Instance::new();
    for id in 0..PARAM_COUNT {
        let mut info = unsafe { core::mem::zeroed() };
        assert!(unsafe { param_info(p.0, id as u32, &mut info) });
        assert_eq!(info.max_value, 1.0);
        assert_eq!(info.default_value, DEFAULTS[id]);
        let mut e = event(0, 0.125 * id as f64);
        e.param_id = id as u32;
        unsafe {
            flush(p.0, &Events(vec![e]).list(), ptr::null());
        }
    }
    let mut data = Bytes {
        data: vec![],
        offset: 0,
        chunk: 3,
    };
    let stream = clap_ostream {
        ctx: (&mut data as *mut Bytes).cast(),
        write: Some(write_bytes),
    };
    assert!(unsafe { save(p.0, &stream) });
    assert_eq!(data.data.len(), 48);
    let q = Instance::new();
    assert!(restore(&q, data.data.clone(), 2));
    for id in 0..PARAM_COUNT {
        assert_eq!(unsafe { slot(q.0).unwrap().value(id) }, 0.125 * id as f64);
    }
    let mut bad = data.data.clone();
    bad[40..48].copy_from_slice(&f64::NAN.to_le_bytes());
    assert!(!restore(&q, bad, 8));
    assert!(!restore(&q, data.data[..47].to_vec(), 8));
    for id in 0..PARAM_COUNT {
        assert_eq!(unsafe { slot(q.0).unwrap().value(id) }, 0.125 * id as f64);
    }
    assert!(!unsafe { activate(p.0, 192000., 1, 256) });
    // Slot exhaustion is a clean factory failure; destroyed slots can be reused.
    let others: Vec<_> = (2..CAPACITY).map(|_| Instance::new()).collect();
    assert!(unsafe { create(&FACTORY, &HOST, PLUGIN_ID.as_ptr()).is_null() });
    drop(others);
    let _again = Instance::new();
}
