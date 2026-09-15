//! Confirm-dialog key map: `y`/Enter confirms, `n`/Esc/`q` cancels.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct ConfirmDialog {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for ConfirmDialog {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        // Ctrl+C quits from anywhere, overlays included.
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') {
            return Some(Msg::Quit);
        }
        match code {
            Key::Char('y') | Key::Enter => Some(Msg::ConfirmYes),
            Key::Char('n') | Key::Esc | Key::Char('q') => Some(Msg::ConfirmNo),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_quits_from_the_dialog() {
        let mut c = ConfirmDialog::default();
        assert_eq!(
            c.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::Quit)
        );
        // A bare `c` is not one of the dialog's keys and changes nothing.
        assert_eq!(
            c.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::NONE
            ))),
            None
        );
        assert_eq!(
            c.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('y'),
                KeyModifiers::NONE
            ))),
            Some(Msg::ConfirmYes)
        );
    }
}
