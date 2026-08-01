use gtk4::glib;
use gtk4::prelude::*;
use gtk4::RootExt;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub struct GamepadHandler {
    source_id: Option<glib::SourceId>,
    _active: Rc<RefCell<bool>>,
}

impl GamepadHandler {
    pub fn start(window: &gtk4::ApplicationWindow) -> Self {
        let active = Rc::new(RefCell::new(true));
        let active_clone = active.clone();
        let window_clone = window.clone();

        let gilrs_instance = match gilrs::Gilrs::new() {
            Ok(g) => Some(Rc::new(RefCell::new(g))),
            Err(_) => None,
        };

        let source_id = if let Some(gilrs_ref) = gilrs_instance {
            let gilrs = gilrs_ref.clone();

            Some(glib::timeout_add_local(Duration::from_millis(16), move || {
                if !*active_clone.borrow() {
                    return glib::ControlFlow::Break;
                }
                if let Ok(mut g) = gilrs.try_borrow_mut() {
                    while let Some(event) = g.next_event() {
                        match event.event {
                            gilrs::EventType::ButtonPressed(btn, _) => {
                                handle_button(&window_clone, btn);
                            }
                            gilrs::EventType::AxisChanged(gilrs::Axis::DPadX, val, _) => {
                                if val.abs() > 0.5 {
                                    focus_move(&window_clone, val > 0.0);
                                }
                            }
                            gilrs::EventType::AxisChanged(gilrs::Axis::DPadY, val, _) => {
                                if val.abs() > 0.5 {
                                    focus_move_vertical(&window_clone, val < 0.0);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                glib::ControlFlow::Continue
            }))
        } else {
            None
        };

        GamepadHandler {
            source_id,
            _active: active,
        }
    }
}

impl Drop for GamepadHandler {
    fn drop(&mut self) {
        *self._active.borrow_mut() = false;
        if let Some(id) = self.source_id.take() {
            id.remove();
        }
    }
}

fn get_focus(widget: &gtk4::ApplicationWindow) -> Option<gtk4::Widget> {
    widget.root().and_then(|root| root.focus())
}

fn handle_button(window: &gtk4::ApplicationWindow, button: gilrs::Button) {
    match button {
        gilrs::Button::South => {
            if let Some(focused) = get_focus(window) {
                focused.activate();
            }
        }
        _ => {}
    }
}

fn focus_move(window: &gtk4::ApplicationWindow, forward: bool) {
    let dir = if forward {
        gtk4::DirectionType::TabForward
    } else {
        gtk4::DirectionType::TabBackward
    };
    if let Some(focused) = get_focus(window) {
        let _ = focused.child_focus(dir);
    }
}

fn focus_move_vertical(window: &gtk4::ApplicationWindow, up: bool) {
    let dir = if up {
        gtk4::DirectionType::Up
    } else {
        gtk4::DirectionType::Down
    };
    if let Some(focused) = get_focus(window) {
        let _ = focused.child_focus(dir);
    }
}
