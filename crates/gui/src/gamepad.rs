use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub struct GamepadHandler {
    _active: Rc<RefCell<bool>>,
}

impl GamepadHandler {
    pub fn start(window: &gtk4::ApplicationWindow) -> Self {
        let active = Rc::new(RefCell::new(true));
        let active_clone = active.clone();
        let window_clone = window.clone();

        std::thread::spawn(move || {
            let mut gilrs = match gilrs::Gilrs::new() {
                Ok(g) => g,
                Err(_) => return,
            };

            while *active_clone.borrow() {
                while let Some(event) = gilrs.next_event() {
                    let w = window_clone.clone();
                    match event.event {
                        gilrs::EventType::ButtonPressed(btn, _) => {
                            gtk4::glib::idle_add_once(move || {
                                handle_button(&w, btn);
                            });
                        }
                        gilrs::EventType::AxisChanged(gilrs::Axis::DPadX, val, _) => {
                            if val.abs() > 0.5 {
                                gtk4::glib::idle_add_once(move || {
                                    focus_move(&w, val > 0.0);
                                });
                            }
                        }
                        gilrs::EventType::AxisChanged(gilrs::Axis::DPadY, val, _) => {
                            if val.abs() > 0.5 {
                                gtk4::glib::idle_add_once(move || {
                                    focus_move_vertical(&w, val < 0.0);
                                });
                            }
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(16));
            }
        });

        GamepadHandler {
            _active: active,
        }
    }
}

impl Drop for GamepadHandler {
    fn drop(&mut self) {
        *self._active.borrow_mut() = false;
    }
}

fn handle_button(window: &gtk4::ApplicationWindow, button: gilrs::Button) {
    match button {
        gilrs::Button::South => {
            if let Some(focused) = window.focus_widget() {
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
    if let Some(focused) = window.focus_widget() {
        let _ = focused.child_focus(dir);
    }
}

fn focus_move_vertical(window: &gtk4::ApplicationWindow, up: bool) {
    let dir = if up {
        gtk4::DirectionType::Up
    } else {
        gtk4::DirectionType::Down
    };
    if let Some(focused) = window.focus_widget() {
        let _ = focused.child_focus(dir);
    }
}
