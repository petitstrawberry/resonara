//! Control-thread inspection of an installed native CLAP, without creating a GUI.
use resonara_clap::{HostPlugin, discover};
use std::{error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Usage: inspect <native.clap>")?,
    );
    for descriptor in discover(&path)? {
        println!(
            "{}: {} {} ({:?})",
            descriptor.id, descriptor.name, descriptor.version, descriptor.features
        );
        let mut plugin = HostPlugin::load(&path, Some(&descriptor.id))?;
        println!("parameters: {}", plugin.parameters().len());
        for parameter in plugin.parameters() {
            println!(
                "{}: {} [{}, {}] = {}",
                parameter.id,
                parameter.name,
                parameter.min_value,
                parameter.max_value,
                plugin.parameter_value(parameter.id)?
            );
        }
        if std::env::args().any(|a| a == "--boost") {
            for (name, value) in [
                ("Band 1 Used", 1.),
                ("Band 1 Enabled", 1.),
                ("Band 1 Frequency", 1000f64.log2()),
                ("Band 1 Gain", 6.),
            ] {
                let id = plugin
                    .parameters()
                    .iter()
                    .find(|p| p.name == name)
                    .unwrap()
                    .id;
                plugin.set_parameter(id, value)?;
                println!(
                    "EDIT {} = {} ({})",
                    name,
                    plugin.parameter_value(id)?,
                    plugin.parameter_text(id, value)?
                );
            }
        }
        println!("Cocoa GUI: {:?}", plugin.gui_support(c"cocoa")?);
        let (owner, mut runtime) = plugin.activate(48000., 128)?;
        let mut peak = 0f32;
        let mut energy = 0.;
        for block in 0..375 {
            let mut audio = [[0.; 2]; 128];
            for (i, frame) in audio.iter_mut().enumerate() {
                let value =
                    ((block * 128 + i) as f32 * 1000. * std::f32::consts::TAU / 48000.).sin() * 0.1;
                *frame = [value, -value];
            }
            runtime.process(&mut audio)?;
            for frame in audio {
                for value in frame {
                    if !value.is_finite() {
                        return Err("Nonfinite DSP output".into());
                    }
                    peak = peak.max(value.abs());
                    if block >= 100 {
                        energy += (value as f64).powi(2);
                    }
                }
            }
        }
        println!(
            "48,000 frames rendered; peak={peak}, RMS={}",
            (energy / (275. * 256.)).sqrt()
        );
        drop(runtime);
        drop(owner);
    }
    Ok(())
}
