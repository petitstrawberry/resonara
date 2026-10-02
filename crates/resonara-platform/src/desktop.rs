//! CPAL adapter for Linux and macOS.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use resonara_core::{
    Controls, Project, Result,
    live::{Playback, PlaybackRenderer},
};
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, atomic::Ordering},
};
pub struct Audio {
    stream: Option<cpal::Stream>,
    playback: Playback,
    // CLAP activation/deactivation belongs to this adapter's creating thread.
    _owner_thread: PhantomData<Rc<()>>,
    pub controls: Arc<Controls>,
    pub device: String,
}
impl Audio {
    pub fn update(&mut self, project: &Project) -> Result<()> {
        self.playback.update(project)?;
        self.controls = self.playback.controls.clone();
        Ok(())
    }
    pub fn collect_retired(&mut self) {
        self.playback.collect_retired();
    }
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
        let (playback, engine) = Playback::new(project, config.sample_rate.0, start, metronome)?;
        let controls = playback.controls.clone();
        let error = controls.clone();
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
            playback,
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
    }
}
fn build<T: cpal::SizedSample + resonara_core::OutputSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: PlaybackRenderer,
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
