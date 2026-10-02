//! Manual CLAP GUI fixture: open two independent plugin instances through the
//! same HostPlugin API used by the DAW. No editor/UI dependency or plugin-ID dispatch.
use resonara_core::plugins;
use scarlet_ui::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
#[derive(Clone)]
struct Smoke {
    editors: Rc<RefCell<Vec<plugins::ClapEditor>>>,
    started: Rc<Cell<bool>>,
}
impl View for Smoke {
    fn create_element(&self) -> Box<dyn scarlet_ui::Element> {
        Text::new("Two CLAP instances · close this window to finish").create_element()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl Application for Smoke {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "clap-smoke",
            Window::new("CLAP GUI smoke", self.clone()).size(Size::new(620., 120.)),
        )
    }
    fn on_idle(&mut self) {
        if !self.started.replace(true) {
            for _ in 0..2 {
                let plugin = plugins::load_bundled_freeverb().expect("bundled Freeverb fixture");
                let editor = plugins::ClapEditor::open(&plugin)
                    .expect("CLAP GUI open")
                    .expect("plugin must provide native GUI");
                self.editors.borrow_mut().push(editor);
            }
            eprintln!("CLAP_GUI_SMOKE: two independent editors opened");
        }
        for editor in self.editors.borrow_mut().iter_mut() {
            editor.poll().expect("CLAP GUI timer");
            if let Some(p) = editor.snapshot(false).expect("CLAP state snapshot") {
                eprintln!(
                    "CLAP_GUI_SMOKE: {}",
                    p.parameters
                        .iter()
                        .map(|p| format!("{}={:.2}", p.name, p.value))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
    }
    fn on_shutdown(&mut self) {
        for e in self.editors.borrow().iter() {
            let _ = e.close();
        }
        self.editors.borrow_mut().clear();
    }
}
fn main() {
    let mut app = Smoke {
        editors: Rc::new(RefCell::new(vec![])),
        started: Rc::new(Cell::new(false)),
    };
    app.run().expect("UI");
}
