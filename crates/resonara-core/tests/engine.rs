use resonara_core::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{Arc, atomic::Ordering},
};
thread_local! {static GUARD:Cell<bool>=const {Cell::new(false)};static ALLOCS:Cell<usize>=const {Cell::new(0)};static FREES:Cell<usize>=const {Cell::new(0)};}
struct Audit;
unsafe impl GlobalAlloc for Audit {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        GUARD.with(|g| {
            if g.get() {
                ALLOCS.with(|c| c.set(c.get() + 1))
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        GUARD.with(|g| {
            if g.get() {
                FREES.with(|c| c.set(c.get() + 1))
            }
        });
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        GUARD.with(|g| {
            if g.get() {
                ALLOCS.with(|c| c.set(c.get() + 1))
            }
        });
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static ALLOCATOR: Audit = Audit;
fn constant(value: f32, start: u64, frames: usize) -> Track {
    Track {
        name: "test".into(),
        clips: vec![Clip {
            start,
            source_offset: 0,
            frames,
            samples: Arc::new(vec![[value, value]; frames]),
        }],
        gain: 1.,
        pan: 0.,
        mute: false,
        solo: false,
    }
}
fn project() -> Project {
    Project {
        tracks: vec![constant(0.25, 0, 4), constant(0.5, 2, 2)],
        master: 1.,
        ..Project::default()
    }
}
fn render(p: &Project, rate: u32, frames: usize) -> Vec<f32> {
    let controls = Arc::new(Controls::new(p));
    let mut e = Engine::new(p, controls, rate, 0);
    let mut out = vec![0.; frames * 2];
    e.render(&mut out, 2);
    out
}
#[test]
fn timeline_mix_and_eof() {
    let p = project();
    assert_eq!(
        render(&p, 48000, 6),
        vec![
            0.25, 0.25, 0.25, 0.25, 0.75, 0.75, 0.75, 0.75, 0., 0., 0., 0.
        ]
    );
}
#[test]
fn mute_solo_pan_master_and_clipping() {
    let mut p = project();
    p.tracks[1].solo = true;
    p.tracks[1].pan = 1.;
    p.master = 0.5;
    assert_eq!(
        render(&p, 48000, 4),
        vec![0., 0., 0., 0., 0., 0.25, 0., 0.25]
    );
    p.tracks[1].mute = true;
    assert_eq!(render(&p, 48000, 4), vec![0.; 8]);
    p.tracks[0].solo = true;
    p.tracks[0].gain = 2.;
    p.tracks[1].mute = false;
    p.tracks[1].gain = 2.;
    p.master = 2.;
    assert_eq!(&render(&p, 48000, 4)[4..], [1., 1., 1., 1.]);
}
#[test]
fn split_is_nondestructive_and_trim_uses_source_offset() {
    let mut p = project();
    let before = render(&p, 48000, 4);
    p.split(0, 2).unwrap();
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!(Arc::ptr_eq(
        &p.tracks[0].clips[0].samples,
        &p.tracks[0].clips[1].samples
    ));
    assert_eq!(render(&p, 48000, 4), before);
    p.trim(0, 1, 3).unwrap();
    assert_eq!(p.tracks[0].clips[0].source_offset, 1);
    assert_eq!(p.tracks[0].clips[1].frames, 1);
    assert!(p.split(0, 0).is_err());
    assert!(p.trim(0, 2, 1).is_err());
}
#[test]
fn output_rate_and_mono_surround() {
    let p = project();
    assert_eq!(render(&p, 96000, 8).len(), 16);
    let c = Arc::new(Controls::new(&p));
    let mut e = Engine::new(&p, c.clone(), 24000, 0);
    let mut out = [0.; 3];
    e.render(&mut out, 1);
    assert_eq!(out, [0.25, 0.75, 0.]);
    let mut e = Engine::new(&p, Arc::new(Controls::new(&p)), 48000, 0);
    let mut out = [1.; 6];
    e.render(&mut out, 6);
    assert_eq!(out, [0.25, 0.25, 0., 0., 0., 0.]);
}
#[test]
fn live_atomics_and_pause() {
    let p = project();
    let c = Arc::new(Controls::new(&p));
    let mut e = Engine::new(&p, c.clone(), 48000, 0);
    let mut out = [0.; 2];
    e.render(&mut out, 2);
    c.tracks[0].gain.store(0.5f32.to_bits(), Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.125; 2]);
    c.playing.store(false, Ordering::Relaxed);
    e.render(&mut out, 2);
    assert_eq!(out, [0.; 2]);
    assert_eq!(c.position.load(Ordering::Relaxed), 2);
}
#[test]
fn callback_never_allocates_or_frees() {
    let p = Project::demo();
    let c = Arc::new(Controls::new(&p));
    let mut e = Engine::new(&p, c, 44100, 0);
    let mut out = [0.; 512];
    ALLOCS.with(|v| v.set(0));
    FREES.with(|v| v.set(0));
    GUARD.with(|v| v.set(true));
    for _ in 0..1000 {
        e.render(&mut out, 2);
    }
    GUARD.with(|v| v.set(false));
    assert_eq!(ALLOCS.with(Cell::get), 0);
    assert_eq!(FREES.with(Cell::get), 0);
}
#[test]
fn validation_rejects_invalid_state() {
    let mut p = project();
    p.tracks[0].clips[0].source_offset = 100;
    assert!(p.validate().is_err());
    let mut p = project();
    p.master = f32::NAN;
    assert!(p.validate().is_err());
    let mut p = project();
    p.version = 9;
    assert!(p.validate().is_err());
}
#[test]
fn integer_conversions() {
    assert_eq!(i16::from_f32(-1.), -32767);
    assert_eq!(u16::from_f32(-1.), 0);
    assert_eq!(u16::from_f32(1.), 65535);
}
