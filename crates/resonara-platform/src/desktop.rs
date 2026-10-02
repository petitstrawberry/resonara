//! CPAL adapter for Linux and macOS.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use resonara_core::{Controls, Engine, PluginOwner, Project, Result};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
};
pub struct Audio {
    stream: Option<cpal::Stream>,
    plugin_owners: Vec<PluginOwner>,
    // CLAP activation/deactivation belongs to this adapter's creating thread.
    _owner_thread: PhantomData<Rc<()>>,
    pub controls: Arc<Controls>,
    pub device: String,
}
impl Audio {
    pub fn is_finished(&self) -> bool {
        !self.controls.playing.load(Ordering::Relaxed)
    }

    pub fn start(project: &Project, start: u64) -> Result<Self> {
        Self::start_with_metronome(project, start, false)
    }
    pub fn start_with_metronome(project: &Project, start: u64, metronome: bool) -> Result<Self> {
        project.validate()?;
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device. Select/configure ALSA or CoreAudio output.")?;
        let name = device.name()?;
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        let controls = Arc::new(Controls::new(project));
        controls.metronome.store(metronome, Ordering::Relaxed);
        let error = controls.clone();
        let mut engine = Engine::try_new(project, controls.clone(), config.sample_rate.0, start)?;
        let plugin_owners = engine.take_plugin_owners();
        let channels = config.channels as usize;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, engine, error, channels)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, engine, error, channels)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, engine, error, channels)?,
            _ => {
                drop(engine);
                return Err("Unsupported output sample format".into());
            }
        };
        stream.play()?;
        Ok(Self {
            stream: Some(stream),
            plugin_owners,
            _owner_thread: PhantomData,
            controls,
            device: name,
        })
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        // ALSA joins its callback thread in Stream::drop. The default CoreAudio
        // stream releases its AudioUnit/callback before this returns. No plugin
        // owner may be released while its realtime proxy is still in a callback.
        drop(self.stream.take());
        self.plugin_owners.clear();
    }
}
fn build<T: cpal::SizedSample + resonara_core::OutputSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: Engine,
    error: Arc<Controls>,
    channels: usize,
) -> Result<cpal::Stream> {
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| engine.render(data, channels),
        move |_| {
            error.error.store(true, Ordering::Relaxed);
        },
        None,
    )?)
}
