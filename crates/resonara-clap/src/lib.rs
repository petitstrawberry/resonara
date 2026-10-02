//! Small, deliberately bounded CLAP effect host for trusted native plug-ins.
//!
//! Supported: one stereo float32 input/output, generic inactive parameters, opaque
//! state, no latency, no note ports, no GUI. Loading a DSO executes native code;
//! validation is not a security boundary. Use native binaries built for the OS.
//!
//! Create, activate, deactivate, state and destroy on the same owner/main thread.
//! `RealtimePlugin` may move to an exclusive audio callback; keep its paired
//! `PluginOwner` on the creating thread until that callback/proxy is destroyed.
//! Each nonempty quantum is bracketed with start/process/stop on that quantum's
//! symbolic audio thread. There is no allocation, lock,
//! library loading or state work in the host's process path. Plug-ins must honor
//! their own realtime contract. Off-owner destruction intentionally leaks the
//! instance and DSO rather than running illegal callbacks or unloading live code.
//! Callers should stop/rebuild an insert on `MainThreadRequest`/`RestartRequested`.
//! Main callbacks are serviced while inactive, never silently called on audio.
mod loader;

use clap_sys::{
    audio_buffer::clap_audio_buffer,
    entry::clap_plugin_entry,
    events::*,
    ext::{audio_ports::*, latency::*, note_ports::*, params::*, state::*, thread_check::*},
    factory::plugin_factory::*,
    host::clap_host,
    id::CLAP_INVALID_ID,
    plugin::{clap_plugin, clap_plugin_descriptor},
    process::*,
    stream::*,
    version::*,
};
use std::{
    cell::{Cell, UnsafeCell},
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    fmt,
    marker::PhantomData,
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    },
};

const MAX_PLUGINS: u32 = 1024;
const MAX_PARAMETERS: u32 = 4096;
pub const MAX_STATE_BYTES: usize = 16 * 1024 * 1024;
static QUARANTINED_INSTANCES: AtomicUsize = AtomicUsize::new(0);
/// Diagnostic count of instances leaked to avoid an illegal lifecycle callback.
/// Successful backend teardown must leave this unchanged.
pub fn quarantined_instance_count() -> usize {
    QUARANTINED_INSTANCES.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);
impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    pub features: Vec<String>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub stepped: bool,
    pub read_only: bool,
    pub hidden: bool,
}

struct SharedLibrary {
    _library: loader::Library,
    entry: clap_plugin_entry,
    factory: *const clap_plugin_factory,
}
impl SharedLibrary {
    fn factory(&self) -> &clap_plugin_factory {
        unsafe { &*self.factory }
    }
}
unsafe impl Send for SharedLibrary {}
unsafe impl Sync for SharedLibrary {}
impl Drop for SharedLibrary {
    fn drop(&mut self) {
        unsafe {
            (self.entry.deinit.unwrap())();
        }
    }
}
struct RegistryEntry {
    library: Arc<SharedLibrary>,
    leases: usize,
}
static LIBRARIES: Mutex<BTreeMap<PathBuf, RegistryEntry>> = Mutex::new(BTreeMap::new());
struct LibraryLease {
    library: Option<Arc<SharedLibrary>>,
    path: PathBuf,
}
impl LibraryLease {
    fn load(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .map_err(|e| Error::new(format!("Plugin path: {e}")))?;
        let mut registry = LIBRARIES
            .lock()
            .map_err(|_| Error::new("CLAP registry poisoned"))?;
        if let Some(entry) = registry.get_mut(&path) {
            entry.leases += 1;
            return Ok(Self {
                library: Some(entry.library.clone()),
                path,
            });
        }
        let library = loader::Library::open(&path)?;
        let entry = unsafe { *library.symbol(c"clap_entry")?.cast::<clap_plugin_entry>() };
        if entry.clap_version.major != 1
            || entry.init.is_none()
            || entry.deinit.is_none()
            || entry.get_factory.is_none()
        {
            return Err(Error::new("Invalid or incompatible CLAP entry"));
        }
        let cpath = CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| Error::new("Invalid plugin path"))?;
        if !unsafe { entry.init.unwrap()(cpath.as_ptr()) } {
            return Err(Error::new("CLAP entry init failed"));
        }
        let factory = unsafe {
            entry.get_factory.unwrap()(CLAP_PLUGIN_FACTORY_ID.as_ptr())
                .cast::<clap_plugin_factory>()
        };
        if factory.is_null() {
            unsafe {
                entry.deinit.unwrap()();
            }
            return Err(Error::new("CLAP plugin factory is missing"));
        }
        let callbacks = unsafe { &*factory };
        if callbacks.get_plugin_count.is_none()
            || callbacks.get_plugin_descriptor.is_none()
            || callbacks.create_plugin.is_none()
        {
            unsafe {
                entry.deinit.unwrap()();
            }
            return Err(Error::new("Invalid CLAP factory"));
        }
        let library = Arc::new(SharedLibrary {
            _library: library,
            entry,
            factory,
        });
        registry.insert(
            path.clone(),
            RegistryEntry {
                library: library.clone(),
                leases: 1,
            },
        );
        Ok(Self {
            library: Some(library),
            path,
        })
    }
    fn shared(&self) -> &SharedLibrary {
        self.library.as_ref().unwrap()
    }
}
impl Drop for LibraryLease {
    fn drop(&mut self) {
        // Entry init/deinit must never overlap or occur while another instance
        // from this DSO is alive. The registry owns one reference and counts leases.
        if let Ok(mut registry) = LIBRARIES.lock() {
            if let Some(entry) = registry.get_mut(&self.path) {
                entry.leases -= 1;
                if entry.leases == 0 {
                    registry.remove(&self.path);
                }
            }
            drop(self.library.take()); // final entry deinit + dlclose under lock
        } else if let Some(library) = self.library.take() {
            std::mem::forget(library);
        }
    }
}

/// Read descriptors without creating instances. This executes the DSO entry.
pub fn discover(path: &Path) -> Result<Vec<PluginDescriptor>> {
    let library = LibraryLease::load(path)?;
    descriptors(library.shared())
}
fn descriptors(library: &SharedLibrary) -> Result<Vec<PluginDescriptor>> {
    let count = unsafe { library.factory().get_plugin_count.unwrap()(library.factory) };
    if count == 0 || count > MAX_PLUGINS {
        return Err(Error::new("Invalid CLAP descriptor count"));
    }
    let mut descriptors = Vec::with_capacity(count as usize);
    for index in 0..count {
        let descriptor =
            unsafe { library.factory().get_plugin_descriptor.unwrap()(library.factory, index) };
        if descriptor.is_null() {
            return Err(Error::new("Null CLAP descriptor"));
        }
        let descriptor = unsafe { read_descriptor(&*descriptor)? };
        if descriptors
            .iter()
            .any(|d: &PluginDescriptor| d.id == descriptor.id)
        {
            return Err(Error::new("Duplicate CLAP plugin ID"));
        }
        descriptors.push(descriptor);
    }
    Ok(descriptors)
}
unsafe fn text(ptr: *const c_char) -> Result<String> {
    if ptr.is_null() {
        return Ok(String::new());
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| Error::new("Invalid UTF-8 CLAP metadata"))
}
unsafe fn read_descriptor(d: &clap_plugin_descriptor) -> Result<PluginDescriptor> {
    if d.clap_version.major != 1 {
        return Err(Error::new("Incompatible CLAP descriptor"));
    }
    let id = unsafe { text(d.id)? };
    let name = unsafe { text(d.name)? };
    if id.is_empty() || name.is_empty() {
        return Err(Error::new("Missing CLAP identity"));
    }
    let mut features = Vec::new();
    if !d.features.is_null() {
        let mut terminated = false;
        for i in 0..128 {
            let p = unsafe { *d.features.add(i) };
            if p.is_null() {
                terminated = true;
                break;
            }
            features.push(unsafe { text(p)? });
        }
        if !terminated {
            return Err(Error::new("Too many CLAP feature tags"));
        }
    }
    Ok(PluginDescriptor {
        id,
        name,
        vendor: unsafe { text(d.vendor)? },
        version: unsafe { text(d.version)? },
        description: unsafe { text(d.description)? },
        features,
    })
}
fn array_text(chars: &[c_char]) -> Result<String> {
    let end = chars
        .iter()
        .position(|&c| c == 0)
        .ok_or_else(|| Error::new("Unterminated CLAP metadata"))?;
    String::from_utf8(chars[..end].iter().map(|&c| c as u8).collect())
        .map_err(|_| Error::new("Invalid UTF-8 parameter"))
}

struct HostContext {
    owner: usize,
    audio: AtomicUsize,
    restart: AtomicBool,
    callback: AtomicBool,
    flush: AtomicBool,
    rescan: AtomicU32,
    dirty: AtomicBool,
    latency_changed: AtomicBool,
}
impl HostContext {
    fn new() -> Result<Self> {
        let owner = loader::thread_token();
        if owner == 0 {
            return Err(Error::new("Host thread identity is unavailable"));
        }
        Ok(Self {
            owner,
            audio: AtomicUsize::new(0),
            restart: AtomicBool::new(false),
            callback: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            rescan: AtomicU32::new(0),
            dirty: AtomicBool::new(false),
            latency_changed: AtomicBool::new(false),
        })
    }
    fn check_owner(&self) -> Result<()> {
        if loader::thread_token() != self.owner {
            return Err(Error::new(
                "CLAP operation requires its original main thread",
            ));
        }
        Ok(())
    }
}
unsafe fn context<'a>(host: *const clap_host) -> &'a HostContext {
    unsafe { &*((*host).host_data.cast::<HostContext>()) }
}
unsafe extern "C" fn get_host_extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_THREAD_CHECK {
        (&THREAD_CHECK as *const clap_host_thread_check).cast()
    } else if id == CLAP_EXT_PARAMS {
        (&HOST_PARAMS as *const clap_host_params).cast()
    } else if id == CLAP_EXT_STATE {
        (&HOST_STATE as *const clap_host_state).cast()
    } else if id == CLAP_EXT_LATENCY {
        (&HOST_LATENCY as *const clap_host_latency).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn request_restart(h: *const clap_host) {
    unsafe { context(h) }.restart.store(true, Ordering::Relaxed);
}
unsafe extern "C" fn request_callback(h: *const clap_host) {
    unsafe { context(h) }
        .callback
        .store(true, Ordering::Relaxed);
}
unsafe extern "C" fn request_process(h: *const clap_host) {
    unsafe { context(h) }.flush.store(true, Ordering::Relaxed);
}
unsafe extern "C" fn is_main_thread(h: *const clap_host) -> bool {
    loader::thread_token() == unsafe { context(h) }.owner
}
unsafe extern "C" fn is_audio_thread(h: *const clap_host) -> bool {
    let token = loader::thread_token();
    token != 0 && token == unsafe { context(h) }.audio.load(Ordering::Relaxed)
}
static THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(is_main_thread),
    is_audio_thread: Some(is_audio_thread),
};
unsafe extern "C" fn rescan(h: *const clap_host, flags: u32) {
    unsafe { context(h) }
        .rescan
        .fetch_or(flags, Ordering::Relaxed);
}
unsafe extern "C" fn clear(_: *const clap_host, _: u32, _: u32) { /* no automation, modulation, or cached cookies */
}
static HOST_PARAMS: clap_host_params = clap_host_params {
    rescan: Some(rescan),
    clear: Some(clear),
    request_flush: Some(request_process),
};
unsafe extern "C" fn mark_dirty(h: *const clap_host) {
    unsafe { context(h) }.dirty.store(true, Ordering::Relaxed);
}
static HOST_STATE: clap_host_state = clap_host_state {
    mark_dirty: Some(mark_dirty),
};
unsafe extern "C" fn latency_changed(h: *const clap_host) {
    unsafe { context(h) }
        .latency_changed
        .store(true, Ordering::Relaxed);
}
static HOST_LATENCY: clap_host_latency = clap_host_latency {
    changed: Some(latency_changed),
};

struct Instance {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    context: Box<HostContext>,
    descriptor: PluginDescriptor,
    parameters: Vec<ParameterInfo>,
    params: Option<clap_plugin_params>,
    state: clap_plugin_state,
    latency: Option<clap_plugin_latency>,
    active: bool,
    _library: LibraryLease,
}
unsafe impl Send for Instance {}
impl Drop for Instance {
    fn drop(&mut self) {
        debug_assert_eq!(loader::thread_token(), self.context.owner);
        unsafe {
            if self.active {
                (*self.plugin).deactivate.unwrap()(self.plugin);
            }
            (*self.plugin).destroy.unwrap()(self.plugin);
        }
    }
}

/// Inactive, owner-thread-only controller. Native plug-ins are trusted code.
pub struct HostPlugin {
    inner: Option<Box<Instance>>,
    _not_send: PhantomData<*mut ()>,
}
impl HostPlugin {
    pub fn load(path: &Path, plugin_id: Option<&str>) -> Result<Self> {
        let library = LibraryLease::load(path)?;
        let descriptors = descriptors(library.shared())?;
        let descriptor = match plugin_id {
            Some(id) => descriptors
                .into_iter()
                .find(|d| d.id == id)
                .ok_or_else(|| Error::new("Requested CLAP plugin ID is absent"))?,
            None => {
                if descriptors.len() == 1 {
                    descriptors.into_iter().next().unwrap()
                } else {
                    return Err(Error::new(
                        "This CLAP library has multiple plug-ins; choose a plugin ID",
                    ));
                }
            }
        };
        if !descriptor.features.iter().any(|f| f == "audio-effect")
            || descriptor
                .features
                .iter()
                .any(|f| f == "instrument" || f == "note-effect")
        {
            return Err(Error::new("Only CLAP audio effects are supported"));
        }
        let mut context = Box::new(HostContext::new()?);
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *context as *mut HostContext).cast(),
            name: c"Resonara".as_ptr(),
            vendor: c"Resonara".as_ptr(),
            url: c"".as_ptr(),
            version: c"0.1.0".as_ptr(),
            get_extension: Some(get_host_extension),
            request_restart: Some(request_restart),
            request_process: Some(request_process),
            request_callback: Some(request_callback),
        });
        let id =
            CString::new(descriptor.id.as_str()).map_err(|_| Error::new("Invalid plugin ID"))?;
        let plugin = unsafe {
            library.shared().factory().create_plugin.unwrap()(
                library.shared().factory,
                &*host,
                id.as_ptr(),
            )
        };
        if plugin.is_null() {
            return Err(Error::new("CLAP create_plugin failed"));
        }
        let api = unsafe { &*plugin };
        // A malformed instance without destroy cannot be reclaimed safely. Keep
        // its host + DSO alive, rather than freeing pointers it may have retained.
        if api.destroy.is_none() {
            QUARANTINED_INSTANCES.fetch_add(1, Ordering::Relaxed);
            std::mem::forget((host, context, library));
            return Err(Error::new(
                "CLAP destroy callback missing; invalid instance quarantined",
            ));
        }
        if api.init.is_none()
            || api.activate.is_none()
            || api.deactivate.is_none()
            || api.start_processing.is_none()
            || api.stop_processing.is_none()
            || api.reset.is_none()
            || api.process.is_none()
            || api.get_extension.is_none()
            || api.on_main_thread.is_none()
        {
            unsafe {
                api.destroy.unwrap()(plugin);
            }
            return Err(Error::new("Required CLAP lifecycle callback missing"));
        }
        if !unsafe { api.init.unwrap()(plugin) } {
            unsafe {
                api.destroy.unwrap()(plugin);
            }
            return Err(Error::new("CLAP plugin init failed"));
        }
        // Establish RAII before extension validation so all failures destroy.
        let mut instance = Box::new(Instance {
            plugin,
            _host: host,
            context,
            descriptor,
            parameters: Vec::new(),
            params: None,
            state: clap_plugin_state {
                save: None,
                load: None,
            },
            latency: None,
            active: false,
            _library: library,
        });
        let created_descriptor = unsafe { (*instance.plugin).desc };
        if created_descriptor.is_null()
            || unsafe { read_descriptor(&*created_descriptor)? }.id != instance.descriptor.id
        {
            return Err(Error::new(
                "CLAP instance descriptor differs from selected plugin",
            ));
        }
        instance.validate_ports()?;
        instance.params = unsafe { instance.extension::<clap_plugin_params>(CLAP_EXT_PARAMS) };
        instance.state = unsafe { instance.extension::<clap_plugin_state>(CLAP_EXT_STATE) }
            .ok_or_else(|| Error::new("CLAP state extension is required"))?;
        if instance.state.save.is_none() || instance.state.load.is_none() {
            return Err(Error::new("Invalid CLAP state extension"));
        }
        instance.latency = unsafe { instance.extension::<clap_plugin_latency>(CLAP_EXT_LATENCY) };
        if instance.latency.is_some_and(|l| l.get.is_none()) {
            return Err(Error::new("Invalid CLAP latency extension"));
        }
        instance.read_parameters()?;
        let mut result = Self {
            inner: Some(instance),
            _not_send: PhantomData,
        };
        result.service_main_thread()?;
        Ok(result)
    }
    fn instance(&self) -> &Instance {
        self.inner.as_ref().unwrap()
    }
    fn instance_mut(&mut self) -> &mut Instance {
        self.inner.as_mut().unwrap()
    }
    pub fn descriptor(&self) -> &PluginDescriptor {
        &self.instance().descriptor
    }
    pub fn parameters(&self) -> &[ParameterInfo] {
        &self.instance().parameters
    }
    pub fn parameter_value(&self, id: u32) -> Result<f64> {
        let i = self.instance();
        i.context.check_owner()?;
        let p = i
            .parameters
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::new("Unknown CLAP parameter"))?;
        let mut value = 0.;
        if !unsafe { i.params.unwrap().get_value.unwrap()(i.plugin, id, &mut value) }
            || !value.is_finite()
            || value < p.min_value
            || value > p.max_value
        {
            return Err(Error::new("Invalid CLAP parameter value"));
        }
        Ok(value)
    }
    pub fn parameter_text(&self, id: u32, value: f64) -> Result<String> {
        let i = self.instance();
        i.context.check_owner()?;
        if !value.is_finite() || !i.parameters.iter().any(|p| p.id == id) {
            return Err(Error::new("Invalid CLAP parameter"));
        }
        let mut buffer = [0 as c_char; 256];
        if !unsafe {
            i.params.unwrap().value_to_text.unwrap()(
                i.plugin,
                id,
                value,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        } {
            return Err(Error::new("CLAP parameter formatting failed"));
        }
        array_text(&buffer)
    }
    pub fn set_parameter(&mut self, id: u32, value: f64) -> Result<()> {
        let i = self.instance();
        i.context.check_owner()?;
        let p = i
            .parameters
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::new("Unknown CLAP parameter"))?;
        if p.read_only
            || !value.is_finite()
            || value < p.min_value
            || value > p.max_value
            || (p.stepped && value.fract() != 0.)
        {
            return Err(Error::new("Invalid or read-only CLAP parameter value"));
        }
        let event = clap_event_param_value {
            header: clap_event_header {
                size: size_of::<clap_event_param_value>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: CLAP_EVENT_IS_LIVE,
            },
            param_id: id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        };
        let input = clap_input_events {
            ctx: (&event as *const clap_event_param_value).cast_mut().cast(),
            size: Some(one_event),
            get: Some(get_one_event),
        };
        let output = output_events(&i.context);
        unsafe {
            i.params.unwrap().flush.unwrap()(i.plugin, &input, &output);
        }
        i.context.dirty.store(true, Ordering::Relaxed);
        self.service_main_thread()
    }
    pub fn save_state(&mut self) -> Result<Vec<u8>> {
        self.service_main_thread()?;
        let i = self.instance();
        let mut output = StateOutput {
            bytes: Vec::new(),
            failed: false,
        };
        let stream = clap_ostream {
            ctx: (&mut output as *mut StateOutput).cast(),
            write: Some(write_state),
        };
        if !unsafe { i.state.save.unwrap()(i.plugin, &stream) } || output.failed {
            return Err(Error::new("CLAP state save failed or exceeded 16 MiB"));
        }
        i.context.dirty.store(false, Ordering::Relaxed);
        Ok(output.bytes)
    }
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<()> {
        let i = self.instance();
        i.context.check_owner()?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(Error::new("CLAP state exceeds 16 MiB"));
        }
        let mut input = StateInput { bytes, offset: 0 };
        let stream = clap_istream {
            ctx: (&mut input as *mut StateInput<'_>).cast(),
            read: Some(read_state),
        };
        if !unsafe { i.state.load.unwrap()(i.plugin, &stream) } {
            return Err(Error::new("CLAP state load failed"));
        }
        self.instance_mut().read_parameters()?;
        self.instance().validate_ports()?;
        self.service_main_thread()
    }
    /// Service bounded main-thread requests while inactive. A persistent restart
    /// request is unsupported, and reported rather than acknowledged dishonestly.
    pub fn service_main_thread(&mut self) -> Result<()> {
        let i = self.instance_mut();
        i.context.check_owner()?;
        for _ in 0..16 {
            if !i.context.callback.swap(false, Ordering::Relaxed) {
                break;
            }
            unsafe {
                (*i.plugin).on_main_thread.unwrap()(i.plugin);
            }
        }
        if i.context.callback.load(Ordering::Relaxed) {
            return Err(Error::new("CLAP main-thread callback did not settle"));
        }
        if i.context.flush.swap(false, Ordering::Relaxed)
            && let Some(params) = i.params
        {
            unsafe {
                params.flush.unwrap()(i.plugin, &empty_events(), &output_events(&i.context));
            }
        }
        if i.context.rescan.swap(0, Ordering::Relaxed) != 0 {
            i.read_parameters()?;
        }
        if i.context.restart.load(Ordering::Relaxed) {
            return Err(Error::new(
                "CLAP requested unsupported restart; reload this insert",
            ));
        }
        Ok(())
    }
    pub fn activate(
        mut self,
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<(PluginOwner, RealtimePlugin)> {
        self.service_main_thread()?;
        if !sample_rate.is_finite()
            || !(8000.0..=384000.0).contains(&sample_rate)
            || !(1..=65536).contains(&max_frames)
        {
            return Err(Error::new("Invalid CLAP activation format"));
        }
        let i = self.instance_mut();
        i.validate_ports()?;
        // Prepare all owned audio memory before calling activate.
        let input = [vec![0.; max_frames], vec![0.; max_frames]];
        let output = [vec![0.; max_frames], vec![0.; max_frames]];
        if !unsafe { (*i.plugin).activate.unwrap()(i.plugin, sample_rate, 1, max_frames as u32) } {
            return Err(Error::new("CLAP activate failed"));
        }
        i.active = true;
        if let Some(latency) = i.latency
            && unsafe { latency.get.unwrap()(i.plugin) } != 0
        {
            return Err(Error::new("CLAP latency compensation is unsupported"));
        }
        i.context.latency_changed.store(false, Ordering::Relaxed);
        if i.context.restart.load(Ordering::Relaxed)
            || i.context.callback.load(Ordering::Relaxed)
            || i.context.rescan.load(Ordering::Relaxed) != 0
        {
            return Err(Error::new(
                "CLAP requested unsupported changes during activation",
            ));
        }
        let shared = Arc::new(InstanceCell {
            inner: UnsafeCell::new(self.inner.take()),
        });
        Ok((
            PluginOwner {
                shared: Some(shared.clone()),
            },
            RealtimePlugin {
                shared,
                input,
                output,
                max_frames,
                steady_time: 0,
                failure: None,
                _not_sync: PhantomData,
            },
        ))
    }
}
impl Drop for HostPlugin {
    fn drop(&mut self) {
        drop_on_owner(self.inner.take());
    }
}
fn drop_on_owner(instance: Option<Box<Instance>>) {
    if let Some(instance) = instance {
        if loader::thread_token() == instance.context.owner {
            drop(instance);
        } else {
            QUARANTINED_INSTANCES.fetch_add(1, Ordering::Relaxed);
            std::mem::forget(instance);
        }
    }
}
impl Instance {
    unsafe fn extension<T: Copy>(&self, id: &CStr) -> Option<T> {
        let p =
            unsafe { (*self.plugin).get_extension.unwrap()(self.plugin, id.as_ptr()).cast::<T>() };
        if p.is_null() {
            None
        } else {
            Some(unsafe { *p })
        }
    }
    fn validate_ports(&self) -> Result<()> {
        let ports = unsafe { self.extension::<clap_plugin_audio_ports>(CLAP_EXT_AUDIO_PORTS) }
            .ok_or_else(|| Error::new("CLAP audio ports are required"))?;
        let count = ports
            .count
            .ok_or_else(|| Error::new("Invalid CLAP audio ports"))?;
        let get = ports
            .get
            .ok_or_else(|| Error::new("Invalid CLAP audio ports"))?;
        for input in [true, false] {
            if unsafe { count(self.plugin, input) } != 1 {
                return Err(Error::new(
                    "CLAP effect must have exactly one input and output",
                ));
            }
            let mut info: clap_audio_port_info = unsafe { std::mem::zeroed() };
            if !unsafe { get(self.plugin, 0, input, &mut info) }
                || info.id == CLAP_INVALID_ID
                || info.channel_count != 2
                || info.flags & CLAP_AUDIO_PORT_IS_MAIN == 0
                || info.port_type.is_null()
                || unsafe { CStr::from_ptr(info.port_type) } != CLAP_PORT_STEREO
            {
                return Err(Error::new("CLAP effect must use main stereo float32 ports"));
            }
        }
        if let Some(notes) =
            unsafe { self.extension::<clap_plugin_note_ports>(CLAP_EXT_NOTE_PORTS) }
        {
            let count = notes
                .count
                .ok_or_else(|| Error::new("Invalid CLAP note ports"))?;
            if unsafe { count(self.plugin, true) } != 0 || unsafe { count(self.plugin, false) } != 0
            {
                return Err(Error::new("CLAP note ports are unsupported"));
            }
        }
        Ok(())
    }
    fn read_parameters(&mut self) -> Result<()> {
        self.parameters.clear();
        let Some(params) = self.params else {
            return Ok(());
        };
        if params.count.is_none()
            || params.get_info.is_none()
            || params.get_value.is_none()
            || params.value_to_text.is_none()
            || params.text_to_value.is_none()
            || params.flush.is_none()
        {
            return Err(Error::new("Invalid CLAP parameters extension"));
        }
        let count = unsafe { params.count.unwrap()(self.plugin) };
        if count > MAX_PARAMETERS {
            return Err(Error::new("Too many CLAP parameters"));
        }
        for index in 0..count {
            let mut info: clap_param_info = unsafe { std::mem::zeroed() };
            if !unsafe { params.get_info.unwrap()(self.plugin, index, &mut info) }
                || info.id == CLAP_INVALID_ID
                || !info.min_value.is_finite()
                || !info.max_value.is_finite()
                || !info.default_value.is_finite()
                || info.min_value > info.max_value
                || !(info.min_value..=info.max_value).contains(&info.default_value)
                || self.parameters.iter().any(|p| p.id == info.id)
            {
                return Err(Error::new("Invalid CLAP parameter metadata"));
            }
            self.parameters.push(ParameterInfo {
                id: info.id,
                name: array_text(&info.name)?,
                module: array_text(&info.module)?,
                min_value: info.min_value,
                max_value: info.max_value,
                default_value: info.default_value,
                stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
                read_only: info.flags & CLAP_PARAM_IS_READONLY != 0,
                hidden: info.flags & CLAP_PARAM_IS_HIDDEN != 0,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    Continue,
    ContinueIfNotQuiet,
    Tail,
    Sleep,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    InvalidBlock,
    StartFailed,
    PluginError,
    InvalidStatus,
    NonFiniteOutput,
    MainThreadRequest,
    RestartRequested,
    UnsupportedChange,
}
impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CLAP processing failed: {self:?}")
    }
}
impl std::error::Error for ProcessError {}

/// Exclusive, preallocated audio-side proxy. Drop it before its paired owner.
/// Failures are sticky; dry audio remains unchanged.
pub struct RealtimePlugin {
    shared: Arc<InstanceCell>,
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
    max_frames: usize,
    steady_time: i64,
    failure: Option<ProcessError>,
    _not_sync: PhantomData<Cell<()>>,
}
unsafe impl Send for RealtimePlugin {}
impl fmt::Debug for RealtimePlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RealtimePlugin")
            .field("descriptor", &self.instance().descriptor)
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}
impl RealtimePlugin {
    fn instance(&self) -> &Instance {
        unsafe { (&*self.shared.inner.get()).as_ref().unwrap() }
    }
    pub fn descriptor(&self) -> &PluginDescriptor {
        &self.instance().descriptor
    }
    pub fn failure(&self) -> Option<ProcessError> {
        self.failure
    }
    pub fn process(
        &mut self,
        audio: &mut [[f32; 2]],
    ) -> std::result::Result<ProcessStatus, ProcessError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if audio.is_empty() {
            return Ok(ProcessStatus::Continue);
        }
        if audio.len() > self.max_frames
            || audio.iter().any(|v| !v[0].is_finite() || !v[1].is_finite())
        {
            return Err(ProcessError::InvalidBlock);
        }
        let result = self.process_inner(audio);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }
    fn process_inner(
        &mut self,
        audio: &mut [[f32; 2]],
    ) -> std::result::Result<ProcessStatus, ProcessError> {
        let shared = &self.shared;
        let i = unsafe { (&*shared.inner.get()).as_ref().unwrap() };
        check_requests(&i.context)?;
        for (index, frame) in audio.iter().enumerate() {
            self.input[0][index] = frame[0];
            self.input[1][index] = frame[1];
        }
        self.output[0][..audio.len()].fill(0.);
        self.output[1][..audio.len()].fill(0.);
        let mut input_ptrs = [self.input[0].as_mut_ptr(), self.input[1].as_mut_ptr()];
        let mut output_ptrs = [self.output[0].as_mut_ptr(), self.output[1].as_mut_ptr()];
        let input = clap_audio_buffer {
            data32: input_ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let mut output = clap_audio_buffer {
            data32: output_ptrs.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let events = empty_events();
        let output_events = output_events(&i.context);
        let process = clap_process {
            steady_time: self.steady_time,
            frames_count: audio.len() as u32,
            transport: ptr::null(),
            audio_inputs: &input,
            audio_outputs: &mut output,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &events,
            out_events: &output_events,
        };
        // &mut self guarantees one symbolic audio-thread at a time. Native thread
        // tokens let callbacks from unrelated plug-in workers report false.
        let token = loader::thread_token();
        if token == 0 {
            return Err(ProcessError::StartFailed);
        }
        i.context.audio.store(token, Ordering::Relaxed);
        if !unsafe { (*i.plugin).start_processing.unwrap()(i.plugin) } {
            i.context.audio.store(0, Ordering::Relaxed);
            return Err(ProcessError::StartFailed);
        }
        let status = unsafe { (*i.plugin).process.unwrap()(i.plugin, &process) };
        unsafe {
            (*i.plugin).stop_processing.unwrap()(i.plugin);
        }
        i.context.audio.store(0, Ordering::Relaxed);
        self.steady_time = self.steady_time.saturating_add(audio.len() as i64);
        i.context.flush.store(false, Ordering::Relaxed); // process serviced request_process/flush
        check_requests(&i.context)?;
        let status = match status {
            CLAP_PROCESS_ERROR => return Err(ProcessError::PluginError),
            CLAP_PROCESS_CONTINUE => ProcessStatus::Continue,
            CLAP_PROCESS_CONTINUE_IF_NOT_QUIET => ProcessStatus::ContinueIfNotQuiet,
            CLAP_PROCESS_TAIL => ProcessStatus::Tail,
            CLAP_PROCESS_SLEEP => ProcessStatus::Sleep,
            _ => return Err(ProcessError::InvalidStatus),
        };
        // CLAP permits a constant channel to write only its first sample.
        for channel in 0..2 {
            let count = if output.constant_mask & (1 << channel) != 0 {
                1
            } else {
                audio.len()
            };
            if self.output[channel][..count].iter().any(|v| !v.is_finite()) {
                return Err(ProcessError::NonFiniteOutput);
            }
        }
        for (index, frame) in audio.iter_mut().enumerate() {
            for (channel, sample) in frame.iter_mut().enumerate() {
                *sample = self.output[channel][if output.constant_mask & (1 << channel) != 0 {
                    0
                } else {
                    index
                }];
            }
        }
        Ok(status)
    }
}
// Dropping the audio proxy never calls deactivate/destroy/deinit/dlclose.
// The non-realtime owner guard must outlive this proxy.

struct InstanceCell {
    inner: UnsafeCell<Option<Box<Instance>>>,
}
// The sole runtime proxy mutates DSP, and the owner may take the instance only
// after Arc::get_mut proves that the runtime proxy is gone. Host callbacks touch
// atomics only. Plugin metadata is immutable while active.
unsafe impl Send for InstanceCell {}
unsafe impl Sync for InstanceCell {}
impl Drop for InstanceCell {
    fn drop(&mut self) {
        // A prematurely dropped/wrong-thread owner must not cause audio-side
        // destruction later. Quarantine the whole instance including its DSO.
        if let Some(instance) = self.inner.get_mut().take() {
            QUARANTINED_INSTANCES.fetch_add(1, Ordering::Relaxed);
            std::mem::forget(instance);
        }
    }
}

/// Retain on the creating thread until the backend has destroyed its realtime
/// proxy. Declare/drop stream or worker before this guard. Although movable so
/// it can reside temporarily in Engine, destruction is owner-thread-only.
pub struct PluginOwner {
    shared: Option<Arc<InstanceCell>>,
}
impl fmt::Debug for PluginOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginOwner")
            .field("realtime_alive", &self.realtime_alive())
            .finish()
    }
}
impl PluginOwner {
    pub fn realtime_alive(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|s| Arc::strong_count(s) > 1)
    }
    /// Retrieve inactive state after dropping the realtime proxy, on the owner.
    pub fn deactivate(mut self) -> Result<HostPlugin> {
        let shared = self.shared.as_mut().unwrap();
        let cell =
            Arc::get_mut(shared).ok_or_else(|| Error::new("CLAP audio proxy is still alive"))?;
        let i = cell.inner.get_mut().as_mut().unwrap();
        i.context.check_owner()?;
        unsafe {
            (*i.plugin).deactivate.unwrap()(i.plugin);
        }
        i.active = false;
        Ok(HostPlugin {
            inner: cell.inner.get_mut().take(),
            _not_send: PhantomData,
        })
    }
}
impl Drop for PluginOwner {
    fn drop(&mut self) {
        if let Some(mut shared) = self.shared.take()
            && let Some(cell) = Arc::get_mut(&mut shared)
        {
            drop_on_owner(cell.inner.get_mut().take());
        }
        // If another proxy exists, InstanceCell quarantines on final drop.
    }
}
fn check_requests(c: &HostContext) -> std::result::Result<(), ProcessError> {
    if c.restart.load(Ordering::Relaxed) {
        Err(ProcessError::RestartRequested)
    } else if c.callback.load(Ordering::Relaxed) {
        Err(ProcessError::MainThreadRequest)
    } else if c.latency_changed.load(Ordering::Relaxed)
        || c.rescan.load(Ordering::Relaxed) & !(CLAP_PARAM_RESCAN_VALUES | CLAP_PARAM_RESCAN_TEXT)
            != 0
    {
        Err(ProcessError::UnsupportedChange)
    } else {
        Ok(())
    }
}
unsafe extern "C" fn zero_events(_: *const clap_input_events) -> u32 {
    0
}
unsafe extern "C" fn null_event(_: *const clap_input_events, _: u32) -> *const clap_event_header {
    ptr::null()
}
fn empty_events() -> clap_input_events {
    clap_input_events {
        ctx: ptr::null_mut(),
        size: Some(zero_events),
        get: Some(null_event),
    }
}
unsafe extern "C" fn one_event(_: *const clap_input_events) -> u32 {
    1
}
unsafe extern "C" fn get_one_event(
    events: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    if index == 0 {
        unsafe { (*events).ctx.cast() }
    } else {
        ptr::null()
    }
}
fn output_events(context: &HostContext) -> clap_output_events {
    clap_output_events {
        ctx: (context as *const HostContext).cast_mut().cast(),
        try_push: Some(push_output_event),
    }
}
unsafe extern "C" fn push_output_event(
    events: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    if event.is_null() {
        return false;
    }
    let header = unsafe { &*event };
    if header.space_id != CLAP_CORE_EVENT_SPACE_ID {
        return false;
    }
    let context = unsafe { &*((*events).ctx.cast::<HostContext>()) };
    match header.type_ {
        CLAP_EVENT_PARAM_VALUE if header.size as usize >= size_of::<clap_event_param_value>() => {
            let value = unsafe { &*event.cast::<clap_event_param_value>() };
            if !value.value.is_finite()
                || value.note_id != -1
                || value.port_index != -1
                || value.channel != -1
                || value.key != -1
            {
                return false;
            }
            context.dirty.store(true, Ordering::Relaxed);
            context
                .rescan
                .fetch_or(CLAP_PARAM_RESCAN_VALUES, Ordering::Relaxed);
            true
        }
        CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END
            if header.size as usize >= size_of::<clap_event_param_gesture>() =>
        {
            true
        }
        _ => false,
    }
}
struct StateOutput {
    bytes: Vec<u8>,
    failed: bool,
}
unsafe extern "C" fn write_state(
    stream: *const clap_ostream,
    data: *const c_void,
    size: u64,
) -> i64 {
    let output = unsafe { &mut *((*stream).ctx.cast::<StateOutput>()) };
    let Ok(size) = usize::try_from(size) else {
        output.failed = true;
        return -1;
    };
    if size > MAX_STATE_BYTES.saturating_sub(output.bytes.len()) || (size != 0 && data.is_null()) {
        output.failed = true;
        return -1;
    }
    if size != 0 {
        output
            .bytes
            .extend_from_slice(unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) });
    }
    size as i64
}
struct StateInput<'a> {
    bytes: &'a [u8],
    offset: usize,
}
unsafe extern "C" fn read_state(stream: *const clap_istream, data: *mut c_void, size: u64) -> i64 {
    let input = unsafe { &mut *((*stream).ctx.cast::<StateInput<'_>>()) };
    let size = usize::try_from(size)
        .unwrap_or(usize::MAX)
        .min(input.bytes.len() - input.offset);
    if size != 0 && data.is_null() {
        return -1;
    }
    if size != 0 {
        unsafe {
            ptr::copy_nonoverlapping(
                input.bytes.as_ptr().add(input.offset),
                data.cast::<u8>(),
                size,
            );
        }
    }
    input.offset += size;
    size as i64
}
