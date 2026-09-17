//! One-field name box: rename a project, or name a new one.
//!
//! The projects screen's `r` and `n` both ask for a single line of text, so the
//! box is the [`FieldForm`] machinery with one field and nothing else: `Enter`
//! submits what is typed, `Esc` cancels. What the name is *for* is the model's
//! business — the box only carries the text back.

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::Event;
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::state::State;

use super::field_form::{FieldForm, FieldFormEvent, FieldSpec};
use crate::tui::msg::{Msg, UserEvent};
use crate::tui::theme::Theme;

pub struct TextPrompt {
    title: String,
    form: FieldForm,
}

impl TextPrompt {
    pub fn new(title: String, label: &str, initial: &str) -> Self {
        Self {
            title,
            form: FieldForm::new(
                &[FieldSpec::new(label.to_string(), "")],
                &[initial.to_string()],
            ),
        }
    }

    /// Repaint the box in the application's theme.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        self.form = self.form.with_theme(t);
        self
    }
}

impl Component for TextPrompt {
    fn view(&mut self, f: &mut Frame, area: Rect) {
        self.form.view(f, area, &self.title, None);
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

impl AppComponent<Msg, UserEvent> for TextPrompt {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let k = *ev.as_keyboard()?;
        match self.form.handle_key(&k) {
            FieldFormEvent::Quit => Some(Msg::Quit),
            FieldFormEvent::Submit => Some(Msg::PromptSubmit(self.form.value(0))),
            FieldFormEvent::Cancel => Some(Msg::PromptCancel),
            FieldFormEvent::Changed => Some(Msg::PromptChanged),
            FieldFormEvent::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuirealm::event::{Key, KeyEvent, KeyModifiers};

    fn key(k: Key) -> Event<UserEvent> {
        Event::Keyboard(KeyEvent::new(k, KeyModifiers::NONE))
    }

    #[test]
    fn enter_submits_the_typed_name_and_esc_cancels() {
        let mut p = TextPrompt::new("New project".into(), "Name", "");
        for c in "Gamma".chars() {
            p.on(&key(Key::Char(c)));
        }
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::PromptSubmit("Gamma".into()))
        );
        // Prefilled: the name is there to be edited, not retyped.
        let mut p = TextPrompt::new("Rename project".into(), "Name", "Alpha");
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::PromptSubmit("Alpha".into()))
        );
        assert_eq!(p.on(&key(Key::Esc)), Some(Msg::PromptCancel));
    }
}
