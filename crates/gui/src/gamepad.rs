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
                    let window = window_clone.clone();
                    match event.event {
                        gilrs::EventType::ButtonPressed(btn, _) => {
                            glib::idle_add_once(move || {
                                handle_button(&window, btn);
                            });
                        }
                        gilrs::EventType::AxisChanged(gilrs::Axis::DPadX, val, _) => {
                            if val.abs() > 0.5 {
                                glib::idle_add_once(move || {
                                    focus_move(&window, val > 0.0);
                                });
                            }
                        }
                        gilrs::EventType::AxisChanged(gilrs::Axis::DPadY, val, _) => {
                            if val.abs() > 0.5 {
                                glib::idle_add_once(move || {
                                    focus_move_vertical(&window, val < 0.0);
                                });
                            }
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(16));
            }
        });

        GamepadHandler { _active: active }
    }
}

impl Drop for GamepadHandler {
    fn drop(&mut self) {
        *self._active.borrow_mut() = false;
    }
}

fn handle_button(window: &gtk4::ApplicationWindow, button: gilrs::Button) {
    match button {
        gilrs::Button::South => activate_focused(window),
        gilrs::Button::East => {
            // B button = Escape / back
            if let Some(focused) = window.focus_widget() {
                focused.grab_focus();
            }
        }
        _ => {}
    }
}

fn activate_focused(window: &gtk4::ApplicationWindow) {
    if let Some(focused) = window.focus_widget() {
        focused.activate();
    }
}

fn focus_move(window: &gtk4::ApplicationWindow, forward: bool) {
    let direction = if forward {
        gtk4::DirectionType::TabForward
    } else {
        gtk4::DirectionType::TabBackward
    };
    // Signal keynav to move focus
    if let Some(focused) = window.focus_widget() {
        let _ = focused.child_focus(direction);
    }
}

fn focus_move_vertical(_window: &gtk4::ApplicationWindow, up: bool) {
    let dir = if up {
        gtk4::DirectionType::Up
    } else {
        gtk4::DirectionType::Down
    };
    if let Some(focused) = _window.focus_widget() {
        let _ = focused.child_focus(dir);
    }
}
