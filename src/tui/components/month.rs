//! Month screen key map.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::core::DayKind;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct MonthScreen {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for MonthScreen {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') {
            return Some(Msg::Quit);
        }
        Some(match code {
            Key::Char('q') => Msg::Quit,
            Key::Up | Key::Char('k') => Msg::SelectDay(-1),
            Key::Down | Key::Char('j') => Msg::SelectDay(1),
            Key::Char('[') | Key::PageUp => Msg::SelectMonth(-1),
            Key::Char(']') | Key::PageDown => Msg::SelectMonth(1),
            Key::Char('t') => Msg::GoToday,
            Key::Enter => Msg::OpenDay,
            Key::Char('s') => Msg::OpenStats,
            Key::Char('c') => Msg::OpenSettings,
            Key::Char('i') => Msg::OpenClockPicker,
            Key::Char('o') => Msg::ClockOut,
            Key::Char('b') => Msg::TakeBreak,
            Key::Char('v') => Msg::SetKind(DayKind::Vacation),
            Key::Char('f') => Msg::SetKind(DayKind::Flex),
            Key::Char('x') => Msg::SetKind(DayKind::Sick),
            Key::Char('p') => Msg::SetKind(DayKind::Holiday),
            Key::Char('w') => Msg::SetKind(DayKind::Work),
            Key::Char('V') => Msg::ToggleAnchor,
            Key::Char('u') => Msg::ToggleHours,
            Key::Char('?') => Msg::ToggleHelp,
            Key::Esc => Msg::Back,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u_toggles_the_hours_format() {
        let mut c = MonthScreen::default();
        let ev = Event::Keyboard(KeyEvent::new(Key::Char('u'), KeyModifiers::NONE));
        assert_eq!(c.on(&ev), Some(Msg::ToggleHours));
    }

    #[test]
    fn b_asks_for_a_break() {
        let mut c = MonthScreen::default();
        let ev = Event::Keyboard(KeyEvent::new(Key::Char('b'), KeyModifiers::NONE));
        assert_eq!(c.on(&ev), Some(Msg::TakeBreak));
    }

    #[test]
    fn shift_v_toggles_the_range_anchor() {
        let mut c = MonthScreen::default();
        let ev = Event::Keyboard(KeyEvent::new(Key::Char('V'), KeyModifiers::SHIFT));
        assert_eq!(c.on(&ev), Some(Msg::ToggleAnchor));
    }
}
