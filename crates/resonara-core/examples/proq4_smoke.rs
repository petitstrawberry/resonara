//! Optional integration against the user's installed licensed Pro-Q 4.
//! No vendor binary or preset is bundled. Runs on the OS main/owner thread.
use resonara_core::{Clip, Controls, Engine, Insert, InsertKind, Project, Result, Track, plugins};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

fn render(project: &Project) -> Result<(f64, f32)> {
    let controls = Arc::new(Controls::new(project));
    let mut engine = Engine::try_new(project, controls.clone(), 48000, 0)?;
    if engine.graph_info().unavailable_plugins != 0 {
        return Err("Pro-Q was not admitted to the graph".into());
    }
    let mut output = [0.; 256];
    let mut energy = 0.;
    let mut peak = 0f32;
    for block in 0..375 {
        engine.render(&mut output, 2);
        if controls.error.load(Ordering::Relaxed) {
            return Err("Pro-Q processing failed".into());
        }
        for value in output {
            if !value.is_finite() {
                return Err("Nonfinite output".into());
            }
            peak = peak.max(value.abs());
            if block >= 100 {
                energy += (value as f64).powi(2);
            }
        }
    }
    Ok(((energy / (275. * 256.)).sqrt(), peak))
}

fn main() -> Result<()> {
    let catalog = plugins::scan_installed();
    let choice = catalog
        .effects
        .iter()
        .find(|c| c.plugin_id == "com.FabFilter.Pro-Q.4")
        .ok_or_else(|| format!("Installed Pro-Q 4 not found: {:?}", catalog.warnings))?;
    let plugin = plugins::load_installed(choice)?;
    println!(
        "Loaded {}: {} parameters, {} opaque state bytes",
        plugin.name,
        plugin.parameters.len(),
        plugin.state.len()
    );
    let parameter = |name: &str| -> Result<u32> {
        Ok(plugin
            .parameters
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("Missing parameter {name}"))?
            .id)
    };
    let boosted = plugins::set_parameters(
        &plugin,
        &[
            (parameter("Band 1 Used")?, 1.),
            (parameter("Band 1 Enabled")?, 1.),
            (parameter("Band 1 Frequency")?, 1000f64.log2()),
            (parameter("Band 1 Gain")?, 6.),
        ],
    )?;
    let samples = Arc::new(
        (0..48000)
            .map(|i| {
                let v = ((i as f32) * std::f32::consts::TAU * 1000. / 48000.).sin() * 0.05;
                [v, -v]
            })
            .collect(),
    );
    let mut project = Project {
        sample_rate: 48000,
        tracks: vec![Track {
            name: "Pro-Q 4 verification".into(),
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            clips: vec![Clip {
                source_channels: 2,
                start: 0,
                source_offset: 0,
                frames: 48000,
                samples,
            }],
            routing: Default::default(),
        }],
        ..Default::default()
    };
    project.tracks[0].routing.inserts.push(Insert {
        kind: InsertKind::Clap { plugin },
        bypass: false,
    });
    let flat = render(&project)?;
    project.tracks[0].routing.inserts[0].kind = InsertKind::Clap { plugin: boosted };
    let eq = render(&project)?;
    let gain_db = 20. * (eq.0 / flat.0).log10();
    println!("Measured flat={flat:?}, EQ={eq:?}, gain={gain_db} dB");
    if (gain_db - 6.).abs() > 0.1 {
        return Err(format!("EQ gain mismatch: {gain_db} dB").into());
    }
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or_else(|| "artifacts/proq4".into()),
    );
    std::fs::create_dir_all(&directory)?;
    project.save(&directory.join("eq.resonara.json"))?;
    let loaded = Project::load(&directory.join("eq.resonara.json"))?;
    let reopened = render(&loaded)?;
    if (reopened.0 - eq.0).abs() > 1e-7 {
        return Err("State reopen changed DSP output".into());
    }
    loaded.export_wav(&directory.join("eq.wav"))?;
    project.tracks[0].routing.inserts[0].bypass = true;
    let bypass = render(&project)?;
    if (bypass.0 - flat.0).abs() > 1e-6 {
        return Err("Bypass output mismatch".into());
    }
    println!(
        "Pro-Q 4 smoke passed: EQ={gain_db:.4} dB, flat RMS={:.8}, EQ RMS={:.8}, peak={:.8}; save/reopen/export/bypass passed",
        flat.0, eq.0, eq.1
    );
    Ok(())
}
