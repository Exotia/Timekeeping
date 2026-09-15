//! Invisible component turning ticks and store replies into `Msg`s.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::Event;

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct Bridge {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for Bridge {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Tick => Some(Msg::Tick),
            Event::User(UserEvent::Store(r)) => Some(Msg::Store(r.clone())),
            _ => None,
        }
    }
}
