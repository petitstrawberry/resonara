use super::*;
use resonara_core::{ClipEdit, Track};
use scarlet_ui::event::KeyModifiers;

fn fixture() -> Daw {
    let clip = Clip {
        source_channels: 2,
        start: 400,
        source_offset: 8,
        frames: 800,
        samples: Arc::new(
            (0..1000)
                .map(|i| [(i as f32 / 1000.) * 0.5, -0.25])
                .collect(),
        ),
        edit: ClipEdit::default(),
    };
    let s = Daw::new(Project {
        sample_rate: 8000,
        tracks: vec![Track {
            name: "Editor source".into(),
            clips: vec![clip],
            gain: 1.,
            pan: 0.,
            mute: false,
            solo: false,
            routing: Default::default(),
        }],
        ..Project::default()
    });
    s.choose(0, Some(0));
    s.open_editor_panel();
    s.region_editor.size.set(Size::new(800., 360.));
    s.refresh_region_editor();
    s
}

#[test]
fn editor_ranges_trim_reverse_and_history_preserve_source_and_transport() {
    let s = fixture();
    let source = s.model.borrow().project.tracks[0].clips[0].samples.clone();
    s.play();
    let controls = s.model.borrow().audio.as_ref().unwrap().controls.clone();
    s.region_editor.selection.set(Selection {
        anchor: 600,
        head: 200,
    });
    s.editor_trim();
    {
        let m = s.model.borrow();
        let clip = &m.project.tracks[0].clips[0];
        assert_eq!(
            (clip.start, clip.source_offset, clip.frames),
            (600, 208, 400)
        );
        assert!(Arc::ptr_eq(&source, &clip.samples));
        assert!(Arc::ptr_eq(&controls, &m.audio.as_ref().unwrap().controls));
        assert!(controls.playing.load(Ordering::Relaxed));
        assert_eq!(m.undo.len(), 1);
    }
    assert_eq!(s.region_editor.selection.get(), Selection::default());
    s.editor_reverse();
    let reversed = s.model.borrow().project.tracks[0].clips[0].sample_at(0.);
    assert_eq!(reversed, source[607]);
    s.undo(false);
    assert!(!s.model.borrow().project.tracks[0].clips[0].edit.reversed);
    s.undo(false);
    assert_eq!(s.model.borrow().project.tracks[0].clips[0].frames, 800);
    s.undo(true);
    assert_eq!(s.model.borrow().project.tracks[0].clips[0].frames, 400);
    assert!(controls.playing.load(Ordering::Relaxed));
}

#[test]
fn editor_split_cuts_both_bounds_in_one_undo_step_and_keeps_middle_selected() {
    let s = fixture();
    s.region_editor.selection.set(Selection {
        anchor: 200,
        head: 600,
    });
    s.editor_split();
    {
        let m = s.model.borrow();
        let clips = &m.project.tracks[0].clips;
        assert_eq!(clips.len(), 3);
        assert_eq!(
            clips
                .iter()
                .map(|c| (c.start, c.frames, c.source_offset))
                .collect::<Vec<_>>(),
            vec![(400, 200, 8), (600, 400, 208), (1000, 200, 608)]
        );
        assert_eq!(m.clip, Some(1));
        assert_eq!(m.undo.len(), 1);
    }
    s.undo(false);
    assert_eq!(s.model.borrow().project.tracks[0].clips.len(), 1);
    s.region_editor.selection.set(Selection {
        anchor: 400,
        head: 400,
    });
    s.editor_split();
    assert_eq!(s.model.borrow().project.tracks[0].clips.len(), 2);
}

#[test]
fn editor_gain_fades_and_normalize_are_undoable_and_reject_invalid_values() {
    let s = fixture();
    s.region_editor.gain.set("-6.0".into());
    s.editor_gain();
    s.region_editor.fade_in.set("10.0".into());
    s.region_editor.fade_out.set("20.0".into());
    s.editor_fades();
    {
        let m = s.model.borrow();
        let c = &m.project.tracks[0].clips[0];
        assert_eq!(c.edit.gain_db, -6.);
        assert_eq!((c.edit.fade_in, c.edit.fade_out), (80, 160));
        assert_eq!(c.sample_at(0.), [0.; 2]);
    }
    let before = s.model.borrow().undo.len();
    s.region_editor.gain.set("NaN".into());
    s.editor_gain();
    s.region_editor.fade_in.set("999999".into());
    s.editor_fades();
    assert_eq!(s.model.borrow().undo.len(), before);
    s.region_editor.normalize.set("-1.0".into());
    s.editor_normalize();
    assert!(s.region_processing_active());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while s.region_processing_active() && std::time::Instant::now() < deadline {
        s.poll_region_processing();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        !s.region_processing_active(),
        "normalize worker did not finish"
    );
    let clip = s.model.borrow().project.tracks[0].clips[0].clone();
    let peak = (0..clip.frames)
        .flat_map(|i| clip.sample_at(i as f64))
        .map(f32::abs)
        .fold(0., f32::max);
    assert!((20. * peak.log10() + 1.).abs() < 0.001);
    s.undo(false);
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].edit.gain_db,
        -6.
    );
}

#[test]
fn editor_zoom_is_independent_and_sample_cursor_supports_shift_arrows() {
    let s = fixture();
    let arrangement = (s.view_start.get(), s.view_span.get());
    s.region_editor.selection.set(Selection {
        anchor: 300,
        head: 304,
    });
    s.editor_zoom_selection();
    assert_eq!(
        (s.region_editor.start.get(), s.region_editor.span.get()),
        (300., 4.)
    );
    assert_eq!(
        s.region_editor
            .frame_at(s.region_editor.waveform_size().width / 2., 800),
        302
    );
    assert!(s.editor_key(KeyEvent::Pressed {
        keycode: KeyCode::Left,
        modifiers: KeyModifiers {
            shift: true,
            ..Default::default()
        }
    }));
    assert_eq!(
        s.region_editor.selection.get(),
        Selection {
            anchor: 300,
            head: 303
        }
    );
    s.editor_zoom(0.5);
    assert_eq!(s.region_editor.span.get(), 2.);
    s.editor_fit();
    assert_eq!(
        (s.region_editor.start.get(), s.region_editor.span.get()),
        (0., 800.)
    );
    assert_eq!((s.view_start.get(), s.view_span.get()), arrangement);
}

#[test]
fn waveform_mesh_is_retained_for_cursor_selection_transport_and_unrelated_edits() {
    let s = fixture();
    let before = s.region_editor.frame.get();
    s.animate_playhead(0.06);
    s.region_editor.selection.set(Selection {
        anchor: 200,
        head: 500,
    });
    s.refresh_region_editor();
    assert!(Arc::ptr_eq(&before, &s.region_editor.frame.get()));
    s.edit("Rename track", |m| {
        m.project.tracks[0].name = "Renamed".into();
        Ok(())
    });
    assert!(Arc::ptr_eq(&before, &s.region_editor.frame.get()));
    s.editor_reverse();
    assert!(!Arc::ptr_eq(&before, &s.region_editor.frame.get()));
}

#[test]
fn changing_or_removing_selected_region_clears_stale_ranges() {
    let s = fixture();
    s.region_editor.selection.set(Selection {
        anchor: 200,
        head: 600,
    });
    s.choose(0, None);
    assert_eq!(s.region_editor.selection.get(), Selection::default());
    assert!(s.region_editor.identity.get().is_none());
    s.choose(0, Some(0));
    s.region_editor.selection.set(Selection {
        anchor: 200,
        head: 600,
    });
    s.edit("Delete region", |m| {
        m.project.tracks[0].clips.clear();
        m.clip = None;
        Ok(())
    });
    assert_eq!(s.region_editor.selection.get(), Selection::default());
    s.undo(false);
    assert_eq!(s.region_editor.selection.get(), Selection::default());
    assert!(s.region_editor.identity.get().is_some());
}

fn canvas_bounds(element: &dyn Element, parent: Point) -> Option<(Point, Size)> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|r| r.as_any().is::<scarlet_ui::SgfxCanvasRenderObject>())
    {
        return Some((origin, element.bounds().size));
    }
    element
        .children()
        .iter()
        .find_map(|child| canvas_bounds(child.as_ref(), origin))
}

#[test]
fn mounted_editor_drags_backwards_and_pointer_cancel_restores_previous_selection() {
    let s = fixture();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.region_editor_view().create_element());
    tree.layout(LayoutConstraints::tight(800., 360.));
    let (origin, size) = canvas_bounds(tree.root().unwrap(), Point::ZERO).expect("editor canvas");
    assert!(origin.x >= SIDEBAR);
    assert!(origin.y >= TOOLBAR + RULER);
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    let at = |fraction: f32| (origin.x + size.width * fraction).round() as i32;
    let y = (origin.y + size.height * 0.5) as i32;
    for e in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: at(0.75),
            y,
            click_count: 1,
        },
        MouseEvent::Moved { x: at(0.25), y },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: at(0.25),
            y,
            click_count: 1,
        },
    ] {
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(e)));
    }
    let previous = s.region_editor.selection.get();
    assert!(
        previous.anchor > previous.head,
        "backwards range: {previous:?}"
    );
    assert!(previous.range().len().abs_diff(400) <= 3);
    for e in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: at(0.4),
            y,
            click_count: 1,
        },
        MouseEvent::Moved { x: at(0.95), y },
        MouseEvent::ButtonCancelled {
            button: MouseButton::Left,
            x: at(0.95),
            y,
        },
    ] {
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(e)));
    }
    assert_eq!(s.region_editor.selection.get(), previous);
    assert!(s.model.borrow().undo.is_empty());
    s.editor_trim();
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].frames,
        previous.range().len()
    );
    s.undo(false);
    assert_eq!(s.model.borrow().project.tracks[0].clips[0].frames, 800);
}

#[test]
fn edited_peak_cache_preserves_channel_polarity_and_follows_reverse() {
    let s = fixture();
    let mut clip = s.model.borrow().project.tracks[0].clips[0].clone();
    clip.set_reversed(true);
    clip.set_gain_db(-6.).unwrap();
    clip.set_fades(40, 80).unwrap();
    let mut peaks = wave::Peaks::default();
    let env = peaks.clip_envelope(&clip, 50, 52);
    for channel in 0..2 {
        let values = [clip.sample_at(50.)[channel], clip.sample_at(51.)[channel]];
        assert_eq!(env.min[channel], values[0].min(values[1]));
        assert_eq!(env.max[channel], values[0].max(values[1]));
    }
    assert!(env.max[0] > 0.);
    assert!(env.max[1] < 0.);
}

#[test]
fn editor_refresh_preserves_drafts_and_untouched_sub_display_precision() {
    let s = fixture();
    s.edit("Precise source settings", |m| {
        m.project.sample_rate = 48000;
        m.project.tracks[0].clips[0].set_gain_db(0.24)?;
        m.project.tracks[0].clips[0].set_fades(0, 1)
    });
    assert_eq!(s.region_editor.gain.get(), "0.2");
    assert_eq!(s.region_editor.fade_out.get(), "0.0");
    let before = s.model.borrow().undo.len();
    s.editor_gain();
    s.editor_fades();
    assert_eq!(s.model.borrow().undo.len(), before);
    s.region_editor.gain.set("-3.".into());
    s.region_editor.fade_in.set("1.0".into());
    s.refresh(true);
    s.follow_playhead.set(true);
    s.animate_playhead(100.);
    assert_eq!(s.region_editor.gain.get(), "-3.");
    assert_eq!(s.region_editor.fade_in.get(), "1.0");
    s.editor_fades();
    let c = s.model.borrow().project.tracks[0].clips[0].clone();
    assert_eq!(c.edit.fade_in, 48);
    assert_eq!(c.edit.fade_out, 1);
    assert_eq!(c.edit.gain_db, 0.24);
}

#[test]
fn editor_inherited_fades_require_explicit_reset_and_keep_undo() {
    let s = fixture();
    s.edit("Fade source", |m| {
        m.project.tracks[0].clips[0].set_fades(100, 200)
    });
    s.region_editor.selection.set(Selection {
        anchor: 200,
        head: 500,
    });
    s.editor_trim();
    let before = s.model.borrow().project.tracks[0].clips[0].clone();
    assert!(before.edit.has_inherited_fades());
    let history = s.model.borrow().undo.len();
    s.region_editor.fade_in.set("5.0".into());
    s.editor_fades();
    assert_eq!(s.model.borrow().undo.len(), history);
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].edit,
        before.edit
    );
    s.editor_reset_fades();
    assert!(
        !s.model.borrow().project.tracks[0].clips[0]
            .edit
            .has_inherited_fades()
    );
    s.undo(false);
    assert_eq!(
        s.model.borrow().project.tracks[0].clips[0].edit,
        before.edit
    );
}

#[test]
fn hidden_editor_does_not_build_mesh_until_opened() {
    let s = fixture();
    s.toggle_panel(lower_panel::PanelTab::Editor);
    let revision = s.region_editor.mesh_revision.get();
    s.editor_reverse();
    assert_eq!(s.region_editor.mesh_revision.get(), revision);
    s.open_editor_panel();
    assert!(s.region_editor.mesh_revision.get() > revision);
}

#[test]
fn mounted_editor_minimum_pane_keeps_waveform_inside_bounds_and_commits_release_position() {
    let s = fixture();
    s.region_editor.size.set(Size::new(560., 230.));
    s.refresh_region_editor();
    let mut tree = scarlet_ui::ElementTree::new();
    tree.set_root(s.region_editor_view().create_element());
    tree.layout(LayoutConstraints::tight(560., 230.));
    let (origin, size) = canvas_bounds(tree.root().unwrap(), Point::ZERO).unwrap();
    assert_eq!(origin.x, 167.);
    assert_eq!(size.width, 393.);
    assert!(origin.y + size.height + FOOTER <= 230.);
    let x = (origin.x + 50.) as i32;
    let y = (origin.y + 30.) as i32;
    let mut dispatcher = scarlet_ui::EventDispatcher::new();
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: x + 100,
            y,
            click_count: 1,
        },
    ] {
        assert!(dispatcher.dispatch(&mut tree, &Event::Mouse(event)));
    }
    assert!(s.region_editor.selection.get().range().len() > 100);
}

fn render_bounds<T: 'static>(element: &dyn Element, parent: Point, out: &mut Vec<(Point, Size)>) {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|r| r.as_any().is::<T>())
    {
        out.push((origin, element.render_object().unwrap().size()));
    }
    for child in element.children() {
        render_bounds::<T>(child.as_ref(), origin, out);
    }
}

fn assert_editor_geometry(s: &Daw, pipeline: &scarlet_ui::RenderingPipeline) {
    let root = pipeline.element_tree().root().unwrap();
    let mut canvases = vec![];
    let mut rulers = vec![];
    let mut selections = vec![];
    render_bounds::<scarlet_ui::SgfxCanvasRenderObject>(root, Point::ZERO, &mut canvases);
    render_bounds::<EditorRulerRender>(root, Point::ZERO, &mut rulers);
    render_bounds::<SelectionRender>(root, Point::ZERO, &mut selections);
    let (canvas_origin, canvas_size) = *canvases.last().expect("editor canvas mounted");
    let (ruler_origin, ruler_size) = rulers[0];
    let (selection_origin, selection_size) = selections[0];
    let expected = s.region_editor.waveform_size();
    assert!(
        (canvas_size.width - expected.width).abs() < 0.01,
        "canvas {canvas_size:?}, measured editor {:?}",
        s.region_editor.size.get()
    );
    assert!((canvas_size.height - expected.height).abs() < 0.01);
    assert!(
        (ruler_origin.x - canvas_origin.x).abs() < 0.01,
        "ruler {ruler_origin:?}, canvas {canvas_origin:?}"
    );
    assert!(
        (ruler_size.width - canvas_size.width).abs() < 0.01,
        "ruler {ruler_size:?}, canvas {canvas_size:?}"
    );
    assert!((ruler_origin.y + RULER - canvas_origin.y).abs() < 0.01);
    assert_eq!(selection_origin, canvas_origin);
    assert_eq!(selection_size, canvas_size);
}

#[test]
fn mounted_editor_initial_measurement_and_resizing_update_retained_children() {
    let s = fixture();
    // Reproduce opening a never-mounted editor in a larger real window. The
    // first view carries the 800px fallback, then layout measures 1092px.
    s.region_editor.size.set(Size::new(800., 300.));
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_paint_enabled(false);
    pipeline.set_root(s.region_editor_view().create_element());
    pipeline.layout_initial();
    for size in [
        Size::new(1092., 360.),
        Size::new(560., 230.),
        Size::new(1297., 460.),
    ] {
        pipeline.resize(size);
        for _ in 0..8 {
            let _ = pipeline.render();
        }
        assert_eq!(s.region_editor.size.get(), size);
        assert_editor_geometry(&s, &pipeline);
    }
    pipeline.teardown();
}

#[test]
fn mounted_full_window_editor_shortcut_and_region_selection_fill_lower_pane() {
    let s = fixture();
    s.show_panel(lower_panel::PanelTab::Mixer);
    s.choose(0, None);
    s.region_editor.size.set(Size::new(800., 300.));
    let mut pipeline = scarlet_ui::RenderingPipeline::new();
    pipeline.set_paint_enabled(false);
    pipeline.set_root(
        Window::new("Editor resize test", s.clone())
            .size(Size::new(1464., 932.))
            .create_element(),
    );
    pipeline.layout_initial();
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    assert!(s.handle_key(KeyEvent::Pressed {
        keycode: KeyCode::Char('e'),
        modifiers: KeyModifiers::default()
    }));
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    s.choose(0, Some(0));
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    assert!((s.region_editor.size.get().width - s.arrangement_size.get().width).abs() < 1.);
    assert!(s.region_editor.size.get().width > 1000.);
    assert_editor_geometry(&s, &pipeline);
    pipeline.resize(Size::new(1200., 860.));
    for _ in 0..8 {
        let _ = pipeline.render();
    }
    assert_editor_geometry(&s, &pipeline);
    pipeline.teardown();
}
