use super::*;
use scarlet_ui::{ElementTree, EventDispatcher, LayoutConstraints, event::KeyModifiers};

fn aux_session() -> (Daw, BusId) {
    let s = Daw::new(Project::default());
    s.size.set(Size::new(1000., 790.));
    s.add_track(None);
    s.add_track(None);
    s.add_bus();
    let id = s.model.borrow().selected_bus.unwrap();
    s.set_output(RoutingTarget::Track(0), Destination::Bus(id));
    s.add_send(RoutingTarget::Track(1), id);
    s.choose_bus(id);
    (s, id)
}

fn rebuild(tree: &mut ElementTree) {
    tree.root_mut().unwrap().rebuild();
    tree.layout(LayoutConstraints::tight(1000., 790.));
}

fn click(dispatcher: &mut EventDispatcher, tree: &mut ElementTree, point: Point) {
    for event in [
        MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x: point.x as i32,
            y: point.y as i32,
            click_count: 1,
        },
        MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x: point.x as i32,
            y: point.y as i32,
            click_count: 1,
        },
    ] {
        dispatcher.dispatch(tree, &Event::Mouse(event));
    }
}

fn options_bounds(element: &dyn scarlet_ui::Element, parent: Point) -> Option<Rect> {
    let origin = Point::new(
        parent.x + element.position().x,
        parent.y + element.position().y,
    );
    if element
        .render_object()
        .is_some_and(|render| render.as_any().is::<AuxMenuAnchorRender>())
    {
        return Some(Rect::from_xywh(
            origin.x,
            origin.y,
            element.bounds().size.width,
            element.bounds().size.height,
        ));
    }
    element
        .children()
        .iter()
        .find_map(|child| options_bounds(child.as_ref(), origin))
}

fn has_text(element: &dyn scarlet_ui::Element, expected: &str) -> bool {
    if let Some(render) = element.render_object() {
        let mut context = scarlet_ui::renderer::PaintContext::new();
        render.paint(&mut context, Point::ZERO);
        if context.commands().iter().any(|command| {
            matches!(command, scarlet_ui::renderer::PaintCommand::DrawText { text, .. } if text == expected)
        }) {
            return true;
        }
    }
    element
        .children()
        .iter()
        .any(|child| has_text(child.as_ref(), expected))
}

#[test]
fn aux_options_are_compact_and_delete_repairs_routes_with_undo() {
    let (s, id) = aux_session();
    let before = s.model.borrow().project.clone();
    let undo_before = s.model.borrow().undo.len();
    let mut tree = ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(LayoutConstraints::tight(1000., 790.));
    // Apply the measured arrangement and inspector sizes before dispatching.
    // The mounted app's pipeline performs this follow-up layout automatically.
    rebuild(&mut tree);
    assert!(!has_text(tree.root().unwrap(), "Delete aux"));
    let options = options_bounds(tree.root().unwrap(), Point::ZERO).unwrap();
    assert_eq!(options.size, Size::new(24., 24.));
    let mut dispatcher = EventDispatcher::new();
    click(
        &mut dispatcher,
        &mut tree,
        Point::new(options.origin.x + 12., options.origin.y + 12.),
    );
    let menu = s.track_menu.get().expect("Aux options opens the menu");
    assert_eq!(menu.bus, Some(id));
    assert_eq!(menu.anchor.x, options.origin.x);
    assert_eq!(menu.anchor.y, options.origin.y + 27.);
    assert_eq!(s.model.borrow().undo.len(), undo_before);
    rebuild(&mut tree);
    assert!(has_text(tree.root().unwrap(), "Delete aux"));
    click(
        &mut dispatcher,
        &mut tree,
        Point::new(menu.anchor.x + 80., menu.anchor.y + 22.),
    );
    assert!(s.track_menu.get().is_none());
    {
        let m = s.model.borrow();
        assert!(m.project.bus(id).is_none());
        assert_eq!(m.project.tracks[0].routing.output, Destination::Master);
        assert!(m.project.tracks[1].routing.sends.is_empty());
        assert_eq!(m.undo.len(), undo_before + 1);
        m.project.validate().unwrap();
    }
    s.undo(false);
    let m = s.model.borrow();
    assert_eq!(m.project.buses, before.buses);
    assert_eq!(m.project.tracks[0].routing, before.tracks[0].routing);
    assert_eq!(m.project.tracks[1].routing, before.tracks[1].routing);
    assert_eq!(m.selected_bus, Some(id));
}

#[test]
fn aux_menu_keyboard_and_dismiss_never_apply_track_actions() {
    let (s, id) = aux_session();
    let undo_before = s.model.borrow().undo.len();
    let mut tree = ElementTree::new();
    tree.set_root(s.create_element());
    tree.layout(LayoutConstraints::tight(1000., 790.));
    let mut dispatcher = EventDispatcher::new();
    s.open_aux_menu(id, Point::new(950., 780.));
    let menu = s.track_menu.get().unwrap();
    assert_eq!(menu.anchor, Point::new(780., 746.));
    rebuild(&mut tree);
    dispatcher.dispatch(
        &mut tree,
        &Event::Keyboard(KeyEvent::Pressed {
            keycode: KeyCode::Escape,
            modifiers: KeyModifiers::default(),
        }),
    );
    assert!(s.track_menu.get().is_none());
    assert_eq!(s.model.borrow().undo.len(), undo_before);
    s.open_aux_menu(id, Point::new(200., 200.));
    rebuild(&mut tree);
    click(&mut dispatcher, &mut tree, Point::new(800., 50.));
    assert!(s.track_menu.get().is_none());
    assert_eq!(s.model.borrow().undo.len(), undo_before);
    s.open_aux_menu(id, Point::new(200., 200.));
    rebuild(&mut tree);
    for keycode in [KeyCode::Down, KeyCode::Up, KeyCode::Enter] {
        dispatcher.dispatch(
            &mut tree,
            &Event::Keyboard(KeyEvent::Pressed {
                keycode,
                modifiers: KeyModifiers::default(),
            }),
        );
    }
    assert!(s.model.borrow().project.bus(id).is_none());
    assert_eq!(s.model.borrow().project.tracks.len(), 2);
    assert_eq!(s.model.borrow().undo.len(), undo_before + 1);
}
