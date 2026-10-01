//! Desktop adapter. Scarlet can replace this crate without changing the render engine.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use resonara_core::{Controls, Engine, Project, Result};
use std::sync::{Arc, atomic::Ordering};
pub struct Audio {
    _stream: cpal::Stream,
    pub controls: Arc<Controls>,
    pub device: String,
}
impl Audio {
    pub fn start(project: &Project, start: u64) -> Result<Self> {
        project.validate()?;
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device. Select/configure ALSA or CoreAudio output.")?;
        let name = device.name()?;
        let supported = device.default_output_config()?;
        let config: cpal::StreamConfig = supported.clone().into();
        let controls = Arc::new(Controls::new(project));
        let error = controls.clone();
        let engine = Engine::new(project, controls.clone(), config.sample_rate.0, start);
        let channels = config.channels as usize;
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, engine, error, channels)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, engine, error, channels)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, engine, error, channels)?,
            _ => return Err("Unsupported output sample format".into()),
        };
        stream.play()?;
        Ok(Self {
            _stream: stream,
            controls,
            device: name,
        })
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
