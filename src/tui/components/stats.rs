//! Statistics screen key map.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::tui::msg::{Msg, RangeKind, UserEvent};

#[derive(Default, Component)]
pub struct StatsScreen {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for StatsScreen {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') {
            return Some(Msg::Quit);
        }
        Some(match code {
            Key::Char('1') => Msg::StatsRange(RangeKind::Week),
            Key::Char('2') => Msg::StatsRange(RangeKind::ThisMonth),
            Key::Char('3') => Msg::StatsRange(RangeKind::LastMonth),
            Key::Char('4') => Msg::StatsRange(RangeKind::Quarter),
            Key::Char('5') => Msg::StatsRange(RangeKind::Year),
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
    fn digits_pick_the_range() {
        let mut c = StatsScreen::default();
        let press = |c: &mut StatsScreen, ch| {
            c.on(&Event::Keyboard(KeyEvent::new(
                Key::Char(ch),
                KeyModifiers::NONE,
            )))
        };
        assert_eq!(press(&mut c, '1'), Some(Msg::StatsRange(RangeKind::Week)));
        assert_eq!(
            press(&mut c, '2'),
            Some(Msg::StatsRange(RangeKind::ThisMonth))
        );
        assert_eq!(
            press(&mut c, '3'),
            Some(Msg::StatsRange(RangeKind::LastMonth))
        );
        assert_eq!(
            press(&mut c, '4'),
            Some(Msg::StatsRange(RangeKind::Quarter))
        );
        assert_eq!(press(&mut c, '5'), Some(Msg::StatsRange(RangeKind::Year)));
        assert_eq!(press(&mut c, '6'), None);
    }
}
