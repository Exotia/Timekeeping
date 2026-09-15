//! Help overlay: any key closes it.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::Event;

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct HelpOverlay {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for HelpOverlay {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        ev.as_keyboard().map(|_| Msg::ToggleHelp)
    }
}
