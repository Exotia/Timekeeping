//! Confirm-dialog key map: `y`/Enter confirms, `n`/Esc/`q` cancels.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent};

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct ConfirmDialog {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for ConfirmDialog {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, .. } = ev.as_keyboard()?;
        match code {
            Key::Char('y') | Key::Enter => Some(Msg::ConfirmYes),
            Key::Char('n') | Key::Esc | Key::Char('q') => Some(Msg::ConfirmNo),
            _ => None,
        }
    }
}
