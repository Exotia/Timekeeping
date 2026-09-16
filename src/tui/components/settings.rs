//! The settings overlay: the four core settings, in the same kind of box as the
//! entry form.
//!
//! Everything about the fields is [`FieldForm`]; this component only names them
//! and turns its events into the `Settings*` messages. The model validates on
//! every keystroke and pushes the footer back in through [`Attribute::Text`] /
//! `Attribute::Custom(ERROR_FLAG)`, exactly as it does for the entry form.

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::Event;
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::state::State;

use super::field_form::{FieldForm, FieldFormEvent, FieldSpec};
use crate::tui::msg::{Msg, SettingsData, UserEvent};
use crate::tui::theme::Theme;

fn fields() -> Vec<FieldSpec> {
    vec![
        FieldSpec::new("Start date", "2026-09-15"),
        FieldSpec::new("Initial balance", "+00:00"),
        FieldSpec::new("Daily target", "07:48"),
        FieldSpec::new("Vacation days", "30"),
    ]
}

pub struct SettingsForm {
    form: FieldForm,
}

impl SettingsForm {
    pub fn new(initial: SettingsData) -> Self {
        let values = [
            initial.start,
            initial.balance,
            initial.target,
            initial.vacation,
        ];
        Self {
            form: FieldForm::new(&fields(), &values),
        }
    }

    /// Repaint the overlay in the application's theme, keeping the field text.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        self.form = self.form.with_theme(t);
        self
    }

    pub fn data(&self) -> SettingsData {
        let v = self.form.values();
        SettingsData {
            start: v[0].clone(),
            balance: v[1].clone(),
            target: v[2].clone(),
            vacation: v[3].clone(),
        }
    }

    #[cfg(test)]
    fn set_focus(&mut self, i: usize) {
        self.form.set_focus(i);
    }
}

impl Component for SettingsForm {
    fn view(&mut self, f: &mut Frame, area: Rect) {
        self.form.view(f, area, "Settings", None);
    }

    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> {
        self.form.query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.form.attr(attr, value);
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        CmdResult::Invalid(cmd)
    }
}

impl AppComponent<Msg, UserEvent> for SettingsForm {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let k = *ev.as_keyboard()?;
        match self.form.handle_key(&k) {
            FieldFormEvent::Quit => Some(Msg::Quit),
            FieldFormEvent::Submit => Some(Msg::SettingsSubmit(self.data())),
            FieldFormEvent::Cancel => Some(Msg::SettingsCancel),
            FieldFormEvent::Changed => Some(Msg::SettingsChanged(self.data())),
            FieldFormEvent::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::testing::{contains, render};
    use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

    fn data() -> SettingsData {
        SettingsData {
            start: "2026-01-01".into(),
            balance: "+00:00".into(),
            target: "07:48".into(),
            vacation: "30".into(),
        }
    }

    #[test]
    fn draws_the_four_labels_and_the_values() {
        let mut f = SettingsForm::new(data());
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            f.view(fr, area);
        });
        assert!(contains(&rows, "Settings"), "{rows:?}");
        for label in [
            "Start date",
            "Initial balance",
            "Daily target",
            "Vacation days",
        ] {
            assert!(contains(&rows, label), "{label} missing: {rows:?}");
        }
        assert!(contains(&rows, "2026-01-01"));
        assert!(contains(&rows, "07:48"));
    }

    #[test]
    fn keys_map_to_settings_messages() {
        let mut f = SettingsForm::new(data());
        let key = |k| Event::Keyboard(KeyEvent::new(k, KeyModifiers::NONE));
        assert_eq!(f.on(&key(Key::Tab)), Some(Msg::SettingsChanged(f.data())));
        assert_eq!(f.on(&key(Key::Esc)), Some(Msg::SettingsCancel));
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('s'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::SettingsSubmit(f.data()))
        );
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::Quit)
        );
        // Typing lands in the focused field and reports the whole form back.
        f.set_focus(3);
        assert_eq!(
            f.on(&key(Key::Backspace)),
            Some(Msg::SettingsChanged(f.data()))
        );
        assert_eq!(f.data().vacation, "3");
    }
}
