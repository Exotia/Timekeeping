//! Help overlay: any key closes it.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct HelpOverlay {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for HelpOverlay {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        // Ctrl+C quits from anywhere; every other key just closes the overlay.
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') {
            return Some(Msg::Quit);
        }
        Some(Msg::ToggleHelp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_key_closes_but_ctrl_c_quits() {
        let mut h = HelpOverlay::default();
        assert_eq!(
            h.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('x'),
                KeyModifiers::NONE
            ))),
            Some(Msg::ToggleHelp)
        );
        assert_eq!(
            h.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::Quit)
        );
    }
}
