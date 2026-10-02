//! Scarlet-native CLAP Freeverb, ported from the MIT-licensed trevyn/freeverb DSP.
#![cfg_attr(not(test), no_std)]
#![deny(unsafe_op_in_unsafe_fn)]

use clap_sys::{
    entry::*,
    events::*,
    ext::{audio_ports::*, params::*, state::*, thread_check::*},
    factory::plugin_factory::*,
    host::*,
    plugin::*,
    process::*,
    stream::*,
    version::*,
};
use core::{
    cell::UnsafeCell,
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

mod dsp;
const PLUGIN_ID: &CStr = c"org.scarlet.freeverb";
const CAPACITY: usize = 8;
const PARAM_COUNT: usize = 5;
const DEFAULTS: [f64; PARAM_COUNT] = [0.3, 1.0, 0.5, 0.5, 1.0];
const PARAM_NAMES: [&[u8]; PARAM_COUNT] = [b"Wet", b"Dry", b"Room size", b"Damping", b"Width"];
const CREATED: u8 = 0;
const INITIALIZED: u8 = 1;
const ACTIVE: u8 = 2;
const PROCESSING: u8 = 3;

struct FeatureList([*const c_char; 4]);
// Immutable pointers to immutable, static NUL-terminated strings.
unsafe impl Sync for FeatureList {}
static FEATURES: FeatureList = FeatureList([
    c"audio-effect".as_ptr(),
    c"reverb".as_ptr(),
    c"stereo".as_ptr(),
    ptr::null(),
]);
static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID.as_ptr(),
    name: c"Scarlet Freeverb".as_ptr(),
    vendor: c"Scarlet / Ian Hobson".as_ptr(),
    url: c"https://github.com/petitstrawberry/resonara".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.1.0".as_ptr(),
    description: c"MIT Freeverb stereo reverb; freestanding Scarlet CLAP port".as_ptr(),
    features: FEATURES.0.as_ptr(),
};
static ENTRY_USERS: AtomicU32 = AtomicU32::new(0);
static SLOTS: [Slot; CAPACITY] = [const { Slot::new() }; CAPACITY];

struct Slot {
    taken: AtomicBool,
    plugin: UnsafeCell<clap_plugin>,
    host: AtomicPtr<clap_host>,
    thread_check: AtomicPtr<clap_host_thread_check>,
    host_params: AtomicPtr<clap_host_params>,
    values: [AtomicU64; PARAM_COUNT],
    dsp: UnsafeCell<dsp::Freeverb>,
    lifecycle: AtomicU8,
    max_frames: AtomicU32,
}
// Atomic admission gives factory.create_plugin exclusive access to a free slot.
// The plugin vtable/data pointer is immutable from publication until destroy;
// parameters are atomic; DSP storage is exclusively used by the audio thread
// (or activate on the owner thread while inactive). CLAP requires no calls during/after
// destroy and no process/flush overlap. The host owns buffer and event validity.
unsafe impl Sync for Slot {}
impl Slot {
    const fn new() -> Self {
        Self {
            taken: AtomicBool::new(false),
            plugin: UnsafeCell::new(clap_plugin {
                desc: &DESCRIPTOR,
                plugin_data: ptr::null_mut(),
                init: Some(plugin_init),
                destroy: Some(destroy),
                activate: Some(activate),
                deactivate: Some(deactivate),
                start_processing: Some(start),
                stop_processing: Some(stop),
                reset: Some(reset),
                process: Some(process),
                get_extension: Some(extension),
                on_main_thread: Some(on_main),
            }),
            host: AtomicPtr::new(ptr::null_mut()),
            thread_check: AtomicPtr::new(ptr::null_mut()),
            host_params: AtomicPtr::new(ptr::null_mut()),
            values: [
                AtomicU64::new(DEFAULTS[0].to_bits()),
                AtomicU64::new(DEFAULTS[1].to_bits()),
                AtomicU64::new(DEFAULTS[2].to_bits()),
                AtomicU64::new(DEFAULTS[3].to_bits()),
                AtomicU64::new(DEFAULTS[4].to_bits()),
            ],
            dsp: UnsafeCell::new(dsp::Freeverb::empty()),
            lifecycle: AtomicU8::new(CREATED),
            max_frames: AtomicU32::new(0),
        }
    }
    fn value(&self, id: usize) -> f64 {
        f64::from_bits(self.values[id].load(Ordering::Relaxed))
    }
    fn set(&self, id: usize, value: f64) {
        self.values[id].store(value.to_bits(), Ordering::Relaxed);
    }
    // Only the audio thread accesses DSP while active; CLAP forbids overlapping
    // process/flush/reset, and activate configures it before processing starts.
    unsafe fn apply_values(&self) {
        let dsp = unsafe { &mut *self.dsp.get() };
        dsp.set_wet(self.value(0));
        dsp.set_dry(self.value(1));
        dsp.set_room_size(self.value(2));
        dsp.set_dampening(self.value(3));
        dsp.set_width(self.value(4));
    }
    fn transition(&self, from: u8, to: u8) -> bool {
        self.lifecycle
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    fn thread_ok(&self, audio: bool) -> bool {
        let checker = self.thread_check.load(Ordering::Acquire);
        if checker.is_null() {
            return true;
        }
        // Host extensions and host object live until this instance is destroyed.
        unsafe {
            let call = if audio {
                (*checker).is_audio_thread
            } else {
                (*checker).is_main_thread
            };
            call.is_none_or(|f| f(self.host.load(Ordering::Acquire)))
        }
    }
}

// Every callback has a valid, still-live plugin argument under the CLAP ABI.
// Null is rejected defensively; arbitrary dangling foreign pointers cannot be validated.
unsafe fn slot<'a>(plugin: *const clap_plugin) -> Option<&'a Slot> {
    unsafe { plugin.as_ref()?.plugin_data.cast::<Slot>().as_ref() }
}
unsafe fn is_id(value: *const c_char, expected: &CStr) -> bool {
    if value.is_null() {
        return false;
    }
    for (index, byte) in expected.to_bytes_with_nul().iter().enumerate() {
        if unsafe { *value.add(index) as u8 } != *byte {
            return false;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(get_factory),
};
unsafe extern "C" fn entry_init(_path: *const c_char) -> bool {
    ENTRY_USERS
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        .is_ok()
}
unsafe extern "C" fn entry_deinit() {
    let _ = ENTRY_USERS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
}
unsafe extern "C" fn get_factory(id: *const c_char) -> *const c_void {
    if ENTRY_USERS.load(Ordering::Acquire) != 0 && unsafe { is_id(id, CLAP_PLUGIN_FACTORY_ID) } {
        (&FACTORY as *const clap_plugin_factory).cast()
    } else {
        ptr::null()
    }
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(plugin_count),
    get_plugin_descriptor: Some(descriptor),
    create_plugin: Some(create),
};
unsafe extern "C" fn plugin_count(_factory: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn descriptor(
    _factory: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    if index == 0 { &DESCRIPTOR } else { ptr::null() }
}
unsafe extern "C" fn create(
    _factory: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    if ENTRY_USERS.load(Ordering::Acquire) == 0
        || host.is_null()
        || !unsafe { is_id(id, PLUGIN_ID) }
        || !clap_version_is_compatible(unsafe { (*host).clap_version })
    {
        return ptr::null();
    }
    for instance in &SLOTS {
        if instance
            .taken
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            instance.host.store(host.cast_mut(), Ordering::Relaxed);
            instance
                .thread_check
                .store(ptr::null_mut(), Ordering::Relaxed);
            instance
                .host_params
                .store(ptr::null_mut(), Ordering::Relaxed);
            for (id, value) in DEFAULTS.into_iter().enumerate() {
                instance.set(id, value);
            }
            instance.lifecycle.store(CREATED, Ordering::Relaxed);
            instance.max_frames.store(0, Ordering::Relaxed);
            unsafe {
                (*instance.plugin.get()).plugin_data = (instance as *const Slot).cast_mut().cast();
            }
            return instance.plugin.get();
        }
    }
    ptr::null()
}
unsafe extern "C" fn plugin_init(plugin: *const clap_plugin) -> bool {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return false;
    };
    if s.lifecycle.load(Ordering::Acquire) != CREATED {
        return false;
    }
    let host = s.host.load(Ordering::Acquire);
    if let Some(get) = unsafe { (*host).get_extension } {
        s.thread_check.store(
            unsafe { get(host, CLAP_EXT_THREAD_CHECK.as_ptr()) }
                .cast_mut()
                .cast(),
            Ordering::Release,
        );
        s.host_params.store(
            unsafe { get(host, CLAP_EXT_PARAMS.as_ptr()) }
                .cast_mut()
                .cast(),
            Ordering::Release,
        );
    }
    s.thread_ok(false) && s.transition(CREATED, INITIALIZED)
}
unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    if let Some(s) = unsafe { slot(plugin) }
        && s.lifecycle.load(Ordering::Acquire) <= INITIALIZED
        && s.thread_ok(false)
    {
        s.taken.store(false, Ordering::Release);
    }
}
unsafe extern "C" fn activate(plugin: *const clap_plugin, rate: f64, min: u32, max: u32) -> bool {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return false;
    };
    if !s.thread_ok(false)
        || !rate.is_finite()
        || !(8000.0..=96000.0).contains(&rate)
        || min == 0
        || min > max
        || max > i32::MAX as u32
    {
        return false;
    }
    if !s.transition(INITIALIZED, ACTIVE) {
        return false;
    }
    unsafe {
        (&mut *s.dsp.get()).configure(rate as usize);
        s.apply_values();
    }
    s.max_frames.store(max, Ordering::Release);
    true
}
unsafe extern "C" fn deactivate(plugin: *const clap_plugin) {
    if let Some(s) = unsafe { slot(plugin) }
        && s.thread_ok(false)
    {
        s.transition(ACTIVE, INITIALIZED);
    }
}
unsafe extern "C" fn start(plugin: *const clap_plugin) -> bool {
    unsafe { slot(plugin) }.is_some_and(|s| s.thread_ok(true) && s.transition(ACTIVE, PROCESSING))
}
unsafe extern "C" fn stop(plugin: *const clap_plugin) {
    if let Some(s) = unsafe { slot(plugin) }
        && s.thread_ok(true)
    {
        s.transition(PROCESSING, ACTIVE);
    }
}
unsafe extern "C" fn reset(plugin: *const clap_plugin) {
    if let Some(s) = unsafe { slot(plugin) }
        && s.lifecycle.load(Ordering::Acquire) >= ACTIVE
        && s.thread_ok(true)
    {
        unsafe {
            (&mut *s.dsp.get()).clear();
        }
    }
}
unsafe extern "C" fn on_main(_plugin: *const clap_plugin) {}

unsafe fn event_value(header: *const clap_event_header) -> Option<(usize, f64)> {
    let header = unsafe { header.as_ref()? };
    if header.space_id != CLAP_CORE_EVENT_SPACE_ID
        || header.type_ != CLAP_EVENT_PARAM_VALUE
        || header.size < core::mem::size_of::<clap_event_param_value>() as u32
    {
        return None;
    }
    let event = unsafe { &*(header as *const clap_event_header).cast::<clap_event_param_value>() };
    if event.param_id >= PARAM_COUNT as u32
        || event.note_id != -1
        || event.port_index != -1
        || event.channel != -1
        || event.key != -1
        || !event.value.is_finite()
    {
        return None;
    }
    Some((event.param_id as usize, event.value.clamp(0.0, 1.0)))
}
unsafe fn render(
    s: &Slot,
    input: *const clap_sys::audio_buffer::clap_audio_buffer,
    output: *mut clap_sys::audio_buffer::clap_audio_buffer,
    from: u32,
    to: u32,
) {
    let dsp = unsafe { &mut *s.dsp.get() };
    let left_in = unsafe { *(*input).data32 };
    let right_in = unsafe { *(*input).data32.add(1) };
    let left_out = unsafe { *(*output).data32 };
    let right_out = unsafe { *(*output).data32.add(1) };
    for frame in from as usize..to as usize {
        // Read both channels before writing: in-place processing is supported.
        let input = unsafe { (*left_in.add(frame) as f64, *right_in.add(frame) as f64) };
        let (left, right) = dsp.tick(input);
        unsafe {
            *left_out.add(frame) = left as f32;
            *right_out.add(frame) = right as f32;
        }
    }
}
unsafe extern "C" fn process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return CLAP_PROCESS_ERROR;
    };
    let Some(p) = (unsafe { process.as_ref() }) else {
        return CLAP_PROCESS_ERROR;
    };
    if s.lifecycle.load(Ordering::Acquire) != PROCESSING
        || !s.thread_ok(true)
        || p.frames_count > s.max_frames.load(Ordering::Acquire)
        || p.audio_inputs_count != 1
        || p.audio_outputs_count != 1
        || p.audio_inputs.is_null()
        || p.audio_outputs.is_null()
    {
        return CLAP_PROCESS_ERROR;
    }
    let input = unsafe { &*p.audio_inputs };
    // No &mut audio samples: in-place input/output aliasing is explicitly supported.
    let output = unsafe { &*p.audio_outputs };
    if input.channel_count != 2
        || output.channel_count != 2
        || input.data32.is_null()
        || output.data32.is_null()
    {
        return CLAP_PROCESS_ERROR;
    }
    for channel in 0..2usize {
        if unsafe {
            (*input.data32.add(channel)).is_null() || (*output.data32.add(channel)).is_null()
        } {
            return CLAP_PROCESS_ERROR;
        }
    }
    unsafe {
        (*p.audio_outputs).constant_mask = 0;
    }
    let mut cursor = 0;
    unsafe {
        s.apply_values();
    }
    if let Some(events) = unsafe { p.in_events.as_ref() }
        && let (Some(size), Some(get)) = (events.size, events.get)
    {
        let count = unsafe { size(events) };
        for index in 0..count {
            let event = unsafe { get(events, index) };
            let Some((id, value)) = (unsafe { event_value(event) }) else {
                continue;
            };
            let time = unsafe { (*event).time };
            // A bad event must not change state or index outside this block.
            if time >= p.frames_count || time < cursor {
                continue;
            }
            unsafe {
                render(s, p.audio_inputs, p.audio_outputs, cursor, time);
            }
            cursor = time;
            s.set(id, value);
            unsafe {
                s.apply_values();
            }
        }
    }
    unsafe {
        render(s, p.audio_inputs, p.audio_outputs, cursor, p.frames_count);
    }
    CLAP_PROCESS_CONTINUE
}

static PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports {
    count: Some(port_count),
    get: Some(port_info),
};
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(param_count),
    get_info: Some(param_info),
    get_value: Some(param_value),
    value_to_text: Some(value_to_text),
    text_to_value: Some(text_to_value),
    flush: Some(flush),
};
static STATE: clap_plugin_state = clap_plugin_state {
    save: Some(save),
    load: Some(load),
};
unsafe extern "C" fn extension(plugin: *const clap_plugin, id: *const c_char) -> *const c_void {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return ptr::null();
    };
    if s.lifecycle.load(Ordering::Acquire) < INITIALIZED {
        return ptr::null();
    }
    if unsafe { is_id(id, CLAP_EXT_AUDIO_PORTS) } {
        (&PORTS as *const clap_plugin_audio_ports).cast()
    } else if unsafe { is_id(id, CLAP_EXT_PARAMS) } {
        (&PARAMS as *const clap_plugin_params).cast()
    } else if unsafe { is_id(id, CLAP_EXT_STATE) } {
        (&STATE as *const clap_plugin_state).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn port_count(_plugin: *const clap_plugin, _input: bool) -> u32 {
    1
}
fn name<const N: usize>(text: &[u8]) -> [c_char; N] {
    let mut out = [0; N];
    for (to, from) in out.iter_mut().take(N.saturating_sub(1)).zip(text) {
        *to = *from as c_char;
    }
    out
}
unsafe extern "C" fn port_info(
    _plugin: *const clap_plugin,
    index: u32,
    input: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 || info.is_null() {
        return false;
    }
    unsafe {
        info.write(clap_audio_port_info {
            id: 0,
            name: name(if input { b"Stereo In" } else { b"Stereo Out" }),
            flags: CLAP_AUDIO_PORT_IS_MAIN | CLAP_AUDIO_PORT_REQUIRES_COMMON_SAMPLE_SIZE,
            channel_count: 2,
            port_type: CLAP_PORT_STEREO.as_ptr(),
            in_place_pair: 0,
        });
    }
    true
}
unsafe extern "C" fn param_count(_plugin: *const clap_plugin) -> u32 {
    PARAM_COUNT as u32
}
unsafe extern "C" fn param_info(
    _plugin: *const clap_plugin,
    index: u32,
    info: *mut clap_param_info,
) -> bool {
    if index >= PARAM_COUNT as u32 || info.is_null() {
        return false;
    }
    unsafe {
        info.write(clap_param_info {
            id: index,
            flags: CLAP_PARAM_IS_AUTOMATABLE,
            cookie: ptr::null_mut(),
            name: name(PARAM_NAMES[index as usize]),
            module: name(b""),
            min_value: 0.0,
            max_value: 1.0,
            default_value: DEFAULTS[index as usize],
        });
    }
    true
}
unsafe extern "C" fn param_value(plugin: *const clap_plugin, id: u32, out: *mut f64) -> bool {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return false;
    };
    if id >= PARAM_COUNT as u32 || out.is_null() {
        return false;
    }
    unsafe {
        out.write(s.value(id as usize));
    }
    true
}
unsafe extern "C" fn value_to_text(
    _plugin: *const clap_plugin,
    id: u32,
    value: f64,
    out: *mut c_char,
    capacity: u32,
) -> bool {
    if id >= PARAM_COUNT as u32
        || !value.is_finite()
        || !(0.0..=1.0).contains(&value)
        || out.is_null()
        || capacity < 6
    {
        return false;
    }
    let n = (value * 1000.0 + 0.5) as u32;
    let bytes = [
        b'0' + (n / 1000) as u8,
        b'.',
        b'0' + (n / 100 % 10) as u8,
        b'0' + (n / 10 % 10) as u8,
        b'0' + (n % 10) as u8,
        0,
    ];
    for (index, byte) in bytes.iter().enumerate() {
        unsafe {
            out.add(index).write(*byte as c_char);
        }
    }
    true
}
unsafe extern "C" fn text_to_value(
    _plugin: *const clap_plugin,
    id: u32,
    text: *const c_char,
    out: *mut f64,
) -> bool {
    if id >= PARAM_COUNT as u32 || text.is_null() || out.is_null() {
        return false;
    }
    let mut value = 0.0;
    let mut factor = 1.0;
    let mut decimal = false;
    let mut digits = 0;
    for i in 0..32 {
        let byte = unsafe { *text.add(i) as u8 };
        if byte == 0 {
            if digits == 0 || !(0.0..=1.0).contains(&value) {
                return false;
            }
            unsafe {
                out.write(value);
            }
            return true;
        }
        match byte {
            b'.' if !decimal => decimal = true,
            b'0'..=b'9' => {
                digits += 1;
                if decimal {
                    factor *= 0.1;
                    value += (byte - b'0') as f64 * factor;
                } else {
                    value = value * 10.0 + (byte - b'0') as f64;
                }
            }
            _ => return false,
        }
    }
    false
}
unsafe extern "C" fn flush(
    plugin: *const clap_plugin,
    input: *const clap_input_events,
    _output: *const clap_output_events,
) {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return;
    };
    let active = s.lifecycle.load(Ordering::Acquire) >= ACTIVE;
    if !s.thread_ok(active) {
        return;
    }
    let Some(events) = (unsafe { input.as_ref() }) else {
        return;
    };
    let (Some(size), Some(get)) = (events.size, events.get) else {
        return;
    };
    for index in 0..unsafe { size(events) } {
        if let Some((id, value)) = unsafe { event_value(get(events, index)) } {
            s.set(id, value);
        }
    }
}
unsafe extern "C" fn save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return false;
    };
    if !s.thread_ok(false) {
        return false;
    }
    let Some(stream) = (unsafe { stream.as_ref() }) else {
        return false;
    };
    let Some(write) = stream.write else {
        return false;
    };
    let mut bytes = [0u8; 8 + PARAM_COUNT * 8];
    bytes[..8].copy_from_slice(b"SFVB\x01\x00\x00\x00");
    for id in 0..PARAM_COUNT {
        bytes[8 + id * 8..16 + id * 8].copy_from_slice(&s.value(id).to_le_bytes());
    }
    let mut offset = 0usize;
    while offset < bytes.len() {
        let size = bytes.len() - offset;
        let written = unsafe { write(stream, bytes.as_ptr().add(offset).cast(), size as u64) };
        if written <= 0 || written as u64 > size as u64 {
            return false;
        }
        offset += written as usize;
    }
    true
}
unsafe extern "C" fn load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    let Some(s) = (unsafe { slot(plugin) }) else {
        return false;
    };
    if !s.thread_ok(false) {
        return false;
    }
    let Some(stream) = (unsafe { stream.as_ref() }) else {
        return false;
    };
    let Some(read) = stream.read else {
        return false;
    };
    let mut bytes = [0u8; 8 + PARAM_COUNT * 8];
    let mut offset = 0usize;
    while offset < bytes.len() {
        let size = bytes.len() - offset;
        let count = unsafe { read(stream, bytes.as_mut_ptr().add(offset).cast(), size as u64) };
        if count <= 0 || count as u64 > size as u64 {
            return false;
        }
        offset += count as usize;
    }
    if &bytes[..8] != b"SFVB\x01\x00\x00\x00" {
        return false;
    }
    let mut values = [0.0; PARAM_COUNT];
    for (id, value) in values.iter_mut().enumerate() {
        let mut data = [0u8; 8];
        data.copy_from_slice(&bytes[8 + id * 8..16 + id * 8]);
        *value = f64::from_le_bytes(data);
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return false;
        }
    }
    // Reject the entire corrupt state without changing any parameter.
    for (id, value) in values.into_iter().enumerate() {
        s.set(id, value);
    }
    let params = s.host_params.load(Ordering::Acquire);
    if let Some(params) = unsafe { params.as_ref() }
        && let Some(rescan) = params.rescan
    {
        unsafe {
            rescan(s.host.load(Ordering::Acquire), CLAP_PARAM_RESCAN_VALUES);
        }
    }
    true
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    // No intended path panics. A foreign ABI contract violation must never unwind.
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(not(test))]
mod memory;

#[cfg(test)]
mod tests;
