//! The break-split overlay: one field per entry of a session, each holding the
//! minutes that entry pays towards the session's break deduction.
//!
//! The fields, the keys and the footer are [`FieldForm`], like the settings
//! overlay; what is this component's own is that the fields are built at
//! runtime — a session has as many entries as it has — and that the entry ids
//! travel with them, so a submit names the entries it is about rather than
//! their positions on screen. The model validates on every keystroke and pushes
//! the footer (`assigned X / D`, or why the box cannot be saved) back in through
//! [`Attribute::Text`] / `Attribute::Custom(ERROR_FLAG)`.

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::Event;
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::state::State;

use super::field_form::{FieldForm, FieldFormEvent, FieldSpec};
use crate::core::Minutes;
use crate::tui::msg::{Msg, UserEvent};
use crate::tui::theme::Theme;

/// One row of the box: which entry it is about, what to call it, and the share
/// it would pay if it were left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitField {
    pub entry_id: i64,
    /// `08:00–12:00  Alpha  (+04:00)`
    pub label: String,
    /// The current share, as minutes, or empty for "unassigned".
    pub value: String,
    /// What the default rule would give this entry, shown while the field is
    /// empty: the box says what it is about to do, not only what was typed.
    pub placeholder: String,
}

/// One field's typed text as a share: empty is "unassigned", anything else has
/// to be a whole number of minutes that is not negative.
///
/// The one place the text of a share is read, so the component (which decides
/// whether a submit can be parsed at all) and the model (which turns the same
/// text into a footer line or an error) can never disagree about what a field
/// means.
pub fn parse_share(text: &str) -> Result<Option<Minutes>, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<i32>() {
        Ok(m) if m >= 0 => Ok(Some(Minutes(m))),
        _ => Err(format!("'{t}' is not a number of minutes")),
    }
}

pub struct BreakSplitForm {
    form: FieldForm,
    title: String,
    ids: Vec<i64>,
}

impl BreakSplitForm {
    pub fn new(title: String, fields: &[SplitField]) -> Self {
        let specs: Vec<FieldSpec> = fields
            .iter()
            .map(|f| FieldSpec::new(f.label.clone(), f.placeholder.clone()))
            .collect();
        let values: Vec<String> = fields.iter().map(|f| f.value.clone()).collect();
        Self {
            form: FieldForm::new(&specs, &values),
            title,
            ids: fields.iter().map(|f| f.entry_id).collect(),
        }
    }

    /// Repaint the overlay in the application's theme, keeping the field text.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        self.form = self.form.with_theme(t);
        self
    }

    /// The raw field text, in the order the entries were given.
    pub fn values(&self) -> Vec<String> {
        self.form.values()
    }

    /// The shares as they would be saved, or `None` when a field does not read
    /// as one: an unparseable box is reported as a change, so the footer says
    /// what is wrong and the overlay stays open.
    fn shares(&self) -> Option<Vec<(i64, Option<Minutes>)>> {
        self.values()
            .iter()
            .map(|v| parse_share(v).ok())
            .collect::<Option<Vec<_>>>()
            .map(|shares| self.ids.iter().copied().zip(shares).collect())
    }

    #[cfg(test)]
    fn set_value(&mut self, i: usize, s: &str) {
        self.form.set_value(i, s);
    }
}

impl Component for BreakSplitForm {
    fn view(&mut self, f: &mut Frame, area: Rect) {
        let title = self.title.clone();
        self.form.view(f, area, &title, None);
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

impl AppComponent<Msg, UserEvent> for BreakSplitForm {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let k = *ev.as_keyboard()?;
        match self.form.handle_key(&k) {
            FieldFormEvent::Quit => Some(Msg::Quit),
            FieldFormEvent::Submit => Some(match self.shares() {
                Some(shares) => Msg::BreakSplitSubmit(shares),
                None => Msg::BreakSplitChanged(self.values()),
            }),
            FieldFormEvent::Cancel => Some(Msg::BreakSplitCancel),
            FieldFormEvent::Changed => Some(Msg::BreakSplitChanged(self.values())),
            FieldFormEvent::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::testing::{contains, render};
    use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

    fn fields() -> Vec<SplitField> {
        vec![
            SplitField {
                entry_id: 7,
                label: "08:00–12:00  Alpha  (+04:00)".into(),
                value: "30".into(),
                placeholder: "0".into(),
            },
            SplitField {
                entry_id: 9,
                label: "12:00–17:00  Beta  (+05:00)".into(),
                value: String::new(),
                placeholder: "18".into(),
            },
        ]
    }

    fn form() -> BreakSplitForm {
        BreakSplitForm::new("Break split — 2026-09-15".into(), &fields())
    }

    #[test]
    fn a_share_reads_as_minutes_or_as_unassigned() {
        assert_eq!(parse_share(""), Ok(None));
        assert_eq!(parse_share("  "), Ok(None));
        assert_eq!(parse_share("48"), Ok(Some(Minutes(48))));
        assert_eq!(parse_share(" 0 "), Ok(Some(Minutes::ZERO)));
        assert!(parse_share("-1").is_err());
        assert!(parse_share("half an hour").is_err());
        assert!(parse_share("0:30").is_err());
    }

    #[test]
    fn draws_a_field_per_entry_with_its_share_and_the_footer() {
        let mut f = form();
        f.attr(
            Attribute::Text,
            AttrValue::String("assigned 00:30 / 00:48".to_string()),
        );
        let rows = render(100, 30, |fr| {
            let area = fr.area();
            f.view(fr, area);
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "Break split — 2026-09-15"), "{joined}");
        assert!(contains(&rows, "08:00–12:00  Alpha  (+04:00)"), "{joined}");
        assert!(contains(&rows, "12:00–17:00  Beta  (+05:00)"), "{joined}");
        assert!(contains(&rows, "30"), "the share already set:\n{joined}");
        assert!(
            contains(&rows, "18"),
            "the default share as a placeholder:\n{joined}"
        );
        assert!(contains(&rows, "assigned 00:30 / 00:48"), "{joined}");
        assert!(contains(&rows, "Esc cancel"), "the key hints:\n{joined}");
    }

    #[test]
    fn keys_map_to_break_split_messages() {
        let mut f = form();
        let key = |k| Event::Keyboard(KeyEvent::new(k, KeyModifiers::NONE));
        // A submit names the entries, not the rows: the ids travel with the
        // fields, so the model never has to guess which entry a share is for.
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('s'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::BreakSplitSubmit(vec![
                (7, Some(Minutes(30))),
                (9, None)
            ]))
        );
        assert_eq!(f.on(&key(Key::Esc)), Some(Msg::BreakSplitCancel));
        assert_eq!(
            f.on(&key(Key::Tab)),
            Some(Msg::BreakSplitChanged(vec!["30".into(), String::new()]))
        );
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::Quit)
        );
        // A field that does not read as minutes is a change, not a submit: the
        // model puts the reason in the footer and the box stays open.
        f.set_value(1, "abc");
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('s'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::BreakSplitChanged(vec!["30".into(), "abc".into()]))
        );
    }
}
