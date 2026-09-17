//! Projects screen key map.

use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, Component)]
pub struct ProjectsScreen {
    component: KeyOnly,
}

impl AppComponent<Msg, UserEvent> for ProjectsScreen {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') {
            return Some(Msg::Quit);
        }
        Some(match code {
            Key::Up | Key::Char('k') => Msg::ProjectsSelect(-1),
            Key::Down | Key::Char('j') => Msg::ProjectsSelect(1),
            Key::Char('a') => Msg::ProjectsToggleArchive,
            Key::Char('r') => Msg::ProjectsRename,
            Key::Char('n') => Msg::ProjectsAdd,
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
    fn letters_map_to_project_actions() {
        let mut c = ProjectsScreen::default();
        let press = |c: &mut ProjectsScreen, ch| {
            c.on(&Event::Keyboard(KeyEvent::new(
                Key::Char(ch),
                KeyModifiers::NONE,
            )))
        };
        assert_eq!(press(&mut c, 'a'), Some(Msg::ProjectsToggleArchive));
        assert_eq!(press(&mut c, 'r'), Some(Msg::ProjectsRename));
        assert_eq!(press(&mut c, 'n'), Some(Msg::ProjectsAdd));
        assert_eq!(press(&mut c, 'j'), Some(Msg::ProjectsSelect(1)));
        assert_eq!(press(&mut c, 'k'), Some(Msg::ProjectsSelect(-1)));
        assert_eq!(press(&mut c, 'u'), Some(Msg::ToggleHours));
        assert_eq!(press(&mut c, 'q'), Some(Msg::Back));
        assert_eq!(press(&mut c, 'z'), None);
    }
}
