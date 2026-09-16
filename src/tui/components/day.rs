//! Day editor screen key map.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct DayScreen {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for DayScreen {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') {
            return Some(Msg::Quit);
        }
        Some(match code {
            Key::Up | Key::Char('k') => Msg::DaySelect(-1),
            Key::Down | Key::Char('j') => Msg::DaySelect(1),
            Key::Char('a') => Msg::DayAdd,
            Key::Char('e') | Key::Enter => Msg::DayEdit,
            Key::Char('d') | Key::Delete => Msg::DayDelete,
            Key::Char('b') => Msg::DayBreakSplit,
            Key::Left | Key::Char('h') => Msg::DayKindPrev,
            Key::Right | Key::Char('l') => Msg::DayKindNext,
            Key::Char('u') => Msg::ToggleHours,
            Key::Char('?') => Msg::ToggleHelp,
            Key::Esc | Key::Char('q') => Msg::Back,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u_toggles_the_hours_format() {
        let mut c = DayScreen::default();
        let ev = Event::Keyboard(KeyEvent::new(Key::Char('u'), KeyModifiers::NONE));
        assert_eq!(c.on(&ev), Some(Msg::ToggleHours));
    }
}
