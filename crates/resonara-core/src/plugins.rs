//! Control-thread-only CLAP preparation and persisted plug-in descriptions.
//!
//! Project files are data, never dynamic-library search paths. Only application
//! installation/search configuration authorizes a library location.
use crate::Result;
use resonara_clap::{HostPlugin, PluginOwner, RealtimePlugin};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
mod catalog;
pub use catalog::{ClapCatalog, ClapChoice, scan_installed};

pub const BUNDLED_GAIN_LIBRARY: &str = "resonara-gain.clap";
pub const BUNDLED_GAIN_ID: &str = "org.resonara.gain";
pub const MAX_STATE_BYTES: usize = 1024 * 1024;
pub const MAX_PROJECT_STATE_BYTES: usize = 16 * MAX_STATE_BYTES;
pub const MAX_PARAMETERS: usize = 1024;
const MAX_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClapParameter {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    #[serde(default)]
    pub stepped: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClapInsert {
    /// A display/identity basename only; never passed to the loader as a path.
    pub library: String,
    pub plugin_id: String,
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_state")]
    pub state: Vec<u8>,
    #[serde(default, deserialize_with = "deserialize_parameters")]
    pub parameters: Vec<ClapParameter>,
}

impl ClapInsert {
    /// Validate storage independently of local plug-in availability.
    pub fn validate(&self) -> Result<()> {
        if self.library.is_empty()
            || self.plugin_id.is_empty()
            || [&self.library, &self.plugin_id, &self.name]
                .iter()
                .any(|text| text.len() > MAX_TEXT_BYTES || text.contains('\0'))
        {
            return Err("Invalid CLAP plug-in metadata".into());
        }
        if self.state.len() > MAX_STATE_BYTES {
            return Err("CLAP state storage budget exceeded".into());
        }
        if self.parameters.len() > MAX_PARAMETERS {
            return Err("CLAP parameter count budget exceeded".into());
        }
        let mut ids = BTreeSet::new();
        for parameter in &self.parameters {
            if !ids.insert(parameter.id)
                || parameter.name.len() > MAX_TEXT_BYTES
                || parameter.name.contains('\0')
                || !parameter.min.is_finite()
                || !parameter.max.is_finite()
                || !parameter.value.is_finite()
                || parameter.min > parameter.max
                || !(parameter.min..=parameter.max).contains(&parameter.value)
            {
                return Err("Invalid CLAP parameter metadata".into());
            }
        }
        Ok(())
    }
    pub fn is_bundled_gain(&self) -> bool {
        self.library == BUNDLED_GAIN_LIBRARY && self.plugin_id == BUNDLED_GAIN_ID
    }
}

fn deserialize_limited<'de, D, T>(
    deserializer: D,
    limit: usize,
) -> std::result::Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Limited<T>(usize, std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Limited<T> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(formatter, "an array with at most {} entries", self.0)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(self.0));
            while let Some(value) = seq.next_element()? {
                if values.len() == self.0 {
                    return Err(serde::de::Error::custom("CLAP storage budget exceeded"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Limited(limit, std::marker::PhantomData))
}
fn deserialize_state<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<u8>, D::Error> {
    deserialize_limited(deserializer, MAX_STATE_BYTES)
}
fn deserialize_parameters<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<ClapParameter>, D::Error> {
    deserialize_limited(deserializer, MAX_PARAMETERS)
}

/// Resolve only application configuration and fixed installation locations.
/// A configured path is authoritative: failure does not unexpectedly load a
/// different installation. Relative configuration paths are rejected.
fn bundled_gain_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RESONARA_CLAP_LIBRARY") {
        let path = PathBuf::from(path);
        if !path.is_absolute() || !path.is_file() {
            return Err("RESONARA_CLAP_LIBRARY must name an existing absolute library file".into());
        }
        return Ok(path);
    }
    // Some native targets do not implement current_exe; their fixed system
    // installation must still be reachable when that discovery is unavailable.
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        let path = directory.join("plugins").join(BUNDLED_GAIN_LIBRARY);
        if path.is_file() {
            return Ok(path);
        }
    }
    let native = PathBuf::from("/system/plugins").join(BUNDLED_GAIN_LIBRARY);
    if native.is_file() {
        return Ok(native);
    }
    Err("Bundled Resonara Gain CLAP library is unavailable".into())
}

/// Cheap UI availability hint only. An existing file can still fail ABI/state
/// validation during preparation; `Controls::unavailable_plugins` reports that.
pub fn is_available(insert: &ClapInsert) -> bool {
    library_path(insert).is_ok()
}

fn library_path(insert: &ClapInsert) -> Result<PathBuf> {
    // Preserve the bundled override and reject an unrelated ID masquerading as it.
    if insert.library == BUNDLED_GAIN_LIBRARY {
        return if insert.is_bundled_gain() {
            bundled_gain_path()
        } else {
            Err("Unknown bundled CLAP identity".into())
        };
    }
    catalog::resolve(&insert.library)
}

fn load_inactive(insert: &ClapInsert) -> Result<HostPlugin> {
    insert.validate()?;
    let mut host = HostPlugin::load(&library_path(insert)?, Some(&insert.plugin_id))?;
    // Even an empty blob must be accepted by the effect. Never silently turn
    // a missing/corrupt saved state into a default-sounding successful export.
    host.load_state(&insert.state)?;
    Ok(host)
}

fn snapshot(host: &mut HostPlugin, library: &str) -> Result<ClapInsert> {
    let plugin_id = host.descriptor().id.clone();
    let name = host.descriptor().name.clone();
    let parameters = host
        .parameters()
        .iter()
        .map(|parameter| {
            Ok(ClapParameter {
                id: parameter.id,
                name: parameter.name.clone(),
                min: parameter.min_value,
                max: parameter.max_value,
                value: host.parameter_value(parameter.id)?,
                stepped: parameter.stepped,
                read_only: parameter.read_only,
                hidden: parameter.hidden,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let insert = ClapInsert {
        library: library.into(),
        plugin_id,
        name,
        state: host.save_state()?,
        parameters,
    };
    insert.validate()?;
    Ok(insert)
}

/// Inspect the bundled effect on the calling control thread, including its
/// default opaque state and generic parameter metadata. No activated DSP leaks.
pub fn load_bundled_gain() -> Result<ClapInsert> {
    let mut host = HostPlugin::load(&bundled_gain_path()?, Some(BUNDLED_GAIN_ID))?;
    snapshot(&mut host, BUNDLED_GAIN_LIBRARY)
}

/// Load a catalog identity on the owner/control thread, with default opaque state.
pub fn load_installed(choice: &ClapChoice) -> Result<ClapInsert> {
    let mut host = HostPlugin::load(&catalog::resolve(&choice.library)?, Some(&choice.plugin_id))?;
    snapshot(&mut host, &choice.library)
}

/// Restore, edit and resnapshot an inactive instance on the control thread.
/// Cached UI metadata never replaces the plug-in's authoritative state.
pub fn set_parameter(insert: &ClapInsert, id: u32, value: f64) -> Result<ClapInsert> {
    set_parameters(insert, &[(id, value)])
}

/// Apply one editor submission to one inactive instance, then save once.
pub fn set_parameters(insert: &ClapInsert, values: &[(u32, f64)]) -> Result<ClapInsert> {
    let mut host = load_inactive(insert)?;
    for &(id, value) in values {
        host.set_parameter(id, value)?;
    }
    snapshot(&mut host, &insert.library)
}

pub(crate) fn activate(
    insert: &ClapInsert,
    sample_rate: u32,
    quantum: usize,
) -> Result<(PluginOwner, RealtimePlugin)> {
    Ok(load_inactive(insert)?.activate(sample_rate as f64, quantum)?)
}

/// Main-thread editor for a stopped/bypassed insert. Playback editors instead
/// use the live PluginOwner, never a second instance of the playing processor.
pub struct ClapEditor {
    host: HostPlugin,
    library: String,
}
impl ClapEditor {
    pub fn open(insert: &ClapInsert) -> Result<Option<Self>> {
        let host = load_inactive(insert)?;
        if !host.open_editor()? {
            return Ok(None);
        }
        Ok(Some(Self {
            host,
            library: insert.library.clone(),
        }))
    }
    pub fn poll(&mut self) -> Result<bool> {
        self.host.service_main_thread()?;
        Ok(self.host.editor_is_open()?)
    }
    pub fn snapshot(&mut self, force: bool) -> Result<Option<ClapInsert>> {
        if !force && !self.host.take_editor_dirty()? {
            return Ok(None);
        }
        snapshot(&mut self.host, &self.library).map(Some)
    }
    pub fn close(&self) -> Result<()> {
        self.host.close_editor()?;
        Ok(())
    }
}
pub(crate) fn snapshot_live(
    owner: &PluginOwner,
    previous: &ClapInsert,
    force: bool,
) -> Result<Option<ClapInsert>> {
    let Some((values, state)) = owner.editor_snapshot(force)? else {
        return Ok(None);
    };
    let mut insert = previous.clone();
    insert.state = state;
    for parameter in &mut insert.parameters {
        if let Some((_, value)) = values.iter().find(|(id, _)| *id == parameter.id) {
            parameter.value = *value;
        }
    }
    insert.validate()?;
    Ok(Some(insert))
}
