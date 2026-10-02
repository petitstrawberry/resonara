//! A device-independent native host smoke executable, also suitable for Scarlet.
use resonara_clap::{HostPlugin, discover, quarantined_instance_count};
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/system/plugins/resonara-gain.clap"));
    let descriptors = discover(&path)?;
    if descriptors.len() != 1 || descriptors[0].id != "org.resonara.gain" {
        return Err("Unexpected gain plugin descriptor".into());
    }
    let mut plugin = HostPlugin::load(&path, Some("org.resonara.gain"))?;
    plugin.set_parameter(0, 0.5)?;
    let state = plugin.save_state()?;
    plugin.set_parameter(0, 1.5)?;
    plugin.load_state(&state)?;
    let (owner, mut realtime) = plugin.activate(48_000., 128)?;
    // Native worker verifies thread-check plus backend-side proxy destruction.
    std::thread::spawn(move || -> Result<(), resonara_clap::ProcessError> {
        let mut block = [[0.5, -0.25]; 128];
        realtime.process(&mut block)?;
        assert!(block.iter().all(|frame| *frame == [0.25, -0.125]));
        drop(realtime);
        Ok(())
    })
    .join()
    .map_err(|_| "CLAP worker panicked")??;
    let mut plugin = owner.deactivate()?;
    if plugin.save_state()? != state || quarantined_instance_count() != 0 {
        return Err("CLAP state or teardown verification failed".into());
    }
    drop(plugin);
    println!("RESONARA_CLAP_NATIVE_OK");
    Ok(())
}
