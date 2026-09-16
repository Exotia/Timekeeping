//! The entry form overlay: four text inputs plus an inline project picker.
//!
//! The inputs, the focus ring, the editing keys and the footer are
//! [`FieldForm`]; what is left here is the picker — filtering the known projects
//! by what is typed, moving the highlight with the arrows, and taking the
//! highlighted name along when focus leaves the field. The overlay reports every
//! change back to the model as [`Msg::FormChanged`], so the model can re-validate
//! and push the footer line back in through [`Attribute::Text`] /
//! `Attribute::Custom(ERROR_FLAG)`.

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyModifiers};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::widgets::{Clear, Paragraph};
use tuirealm::state::State;

use super::field_form::{FieldForm, FieldFormEvent, FieldSpec, filter_projects, picker_line};
use crate::tui::msg::{FormData, Msg, UserEvent};
use crate::tui::theme::Theme;
use crate::tui::view::chrome::centered;

/// Custom attribute the model sets when the footer text is an error message.
pub use super::field_form::ERROR_FLAG;

/// Index of the project field, the only one with a picker attached.
const PROJECT: usize = 2;
/// Index of the last field; Enter there submits.
const LAST: usize = 3;

const FIELDS: [FieldSpec; 4] = [
    FieldSpec {
        label: "Start",
        placeholder: "0800",
    },
    FieldSpec {
        label: "End",
        placeholder: "1730",
    },
    FieldSpec {
        label: "Project",
        placeholder: "type to filter, up/down to pick",
    },
    FieldSpec {
        label: "Comment",
        placeholder: "optional",
    },
];

pub struct EntryForm {
    form: FieldForm,
    id: Option<i64>,
    projects: Vec<String>,
    picker_idx: usize,
}

impl EntryForm {
    pub fn new(initial: FormData, projects: Vec<String>) -> Self {
        let values = [
            initial.start.clone(),
            initial.end.clone(),
            initial.project.clone(),
            initial.comment.clone(),
        ];
        Self {
            form: FieldForm::new(&FIELDS, &values),
            id: initial.id,
            projects,
            picker_idx: 0,
        }
    }

    /// Repaint the overlay in the application's theme. Chained straight onto
    /// [`EntryForm::new`], so the inputs still hold exactly their initial text and
    /// rebuilding them loses nothing.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        self.form = self.form.with_theme(t);
        self
    }

    fn focus(&self) -> usize {
        self.form.focus()
    }

    fn set_focus(&mut self, i: usize) {
        self.form.set_focus(i);
        if i == PROJECT {
            self.picker_idx = 0;
        }
    }

    pub fn data(&self) -> FormData {
        let v = self.form.values();
        FormData {
            id: self.id,
            start: v[0].clone(),
            end: v[1].clone(),
            project: v[PROJECT].clone(),
            comment: v[LAST].clone(),
        }
    }

    /// Known projects whose name contains the typed text (case-insensitive).
    fn matches(&self) -> Vec<String> {
        filter_projects(&self.projects, &self.form.value(PROJECT))
    }

    /// Copy the highlighted project into the field. Without a match the typed text
    /// stays as it is and becomes a new project on submit.
    fn accept_pick(&mut self) {
        if let Some(p) = self.matches().get(self.picker_idx).cloned() {
            self.form.set_value(PROJECT, &p);
        }
    }

    /// The row of project chips under the project field.
    fn picker_line(&self) -> Line<'static> {
        picker_line(
            self.form.theme(),
            &self.matches(),
            self.picker_idx,
            self.focus() == PROJECT,
        )
    }
}

impl Component for EntryForm {
    /// A layout of its own: start and end share the first row, and the picker sits
    /// between the project and comment fields.
    fn view(&mut self, f: &mut Frame, area: Rect) {
        let r = centered(area, 64, 16);
        f.render_widget(Clear, r);
        let title = if self.id.is_some() {
            "Edit entry"
        } else {
            "New entry"
        };
        let outer = self.form.panel(title);
        let inner = outer.inner(r);
        f.render_widget(outer, r);
        let [row1, proj, picker, comment, _gap, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(inner);
        let [s, e] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(row1);
        self.form.field_view(0, f, s);
        self.form.field_view(1, f, e);
        self.form.field_view(PROJECT, f, proj);
        f.render_widget(Paragraph::new(self.picker_line()), picker);
        self.form.field_view(LAST, f, comment);
        f.render_widget(self.form.footer_widget(), footer);
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

impl AppComponent<Msg, UserEvent> for EntryForm {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let k = *ev.as_keyboard()?;
        // In the project field the arrows drive the picker instead of the focus ring,
        // and leaving the field takes the highlighted match along.
        if !k.modifiers.contains(KeyModifiers::CONTROL) && self.focus() == PROJECT {
            match k.code {
                Key::Down => {
                    let last = self.matches().len().saturating_sub(1);
                    self.picker_idx = (self.picker_idx + 1).min(last);
                    return Some(Msg::FormChanged(self.data()));
                }
                Key::Up => {
                    self.picker_idx = self.picker_idx.saturating_sub(1);
                    return Some(Msg::FormChanged(self.data()));
                }
                Key::Tab | Key::Enter => {
                    self.accept_pick();
                    self.set_focus(LAST);
                    return Some(Msg::FormChanged(self.data()));
                }
                Key::BackTab => {
                    self.set_focus(PROJECT - 1);
                    return Some(Msg::FormChanged(self.data()));
                }
                _ => {}
            }
        }
        match self.form.handle_key(&k) {
            FieldFormEvent::Quit => Some(Msg::Quit),
            FieldFormEvent::Submit => Some(Msg::FormSubmit(self.data())),
            FieldFormEvent::Cancel => Some(Msg::FormCancel),
            FieldFormEvent::Changed => {
                if self.focus() == PROJECT {
                    // The filter moved, or focus just arrived: start at the first match.
                    self.picker_idx = 0;
                }
                Some(Msg::FormChanged(self.data()))
            }
            FieldFormEvent::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::testing::{contains, render};
    use tuirealm::event::KeyEvent;

    fn key(k: Key) -> Event<UserEvent> {
        Event::Keyboard(KeyEvent::new(k, KeyModifiers::NONE))
    }

    fn typed(f: &mut EntryForm, s: &str) -> Option<Msg> {
        let mut last = None;
        for c in s.chars() {
            last = f.on(&key(Key::Char(c)));
        }
        last
    }

    #[test]
    fn typing_moves_focus_and_reports_changes() {
        let mut f = EntryForm::new(
            FormData::default(),
            vec!["Alpha".into(), "Beta".into(), "alphabet".into()],
        );
        assert_eq!(typed(&mut f, "900"), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.data().start, "900");
        // Tab leaves the start field and reports the (unchanged) data so we repaint.
        assert_eq!(f.on(&key(Key::Tab)), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.focus(), 1);
        typed(&mut f, "1230");
        assert_eq!(f.data().end, "1230");
        // Enter on a middle field just advances.
        assert_eq!(f.on(&key(Key::Enter)), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.focus(), PROJECT);
        // Typing filters the picker case-insensitively.
        typed(&mut f, "al");
        assert_eq!(f.matches(), vec!["Alpha".to_string(), "alphabet".into()]);
        // Down moves the highlight, Enter accepts it and moves on to the comment.
        f.on(&key(Key::Down));
        assert_eq!(f.on(&key(Key::Enter)), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.data().project, "alphabet");
        assert_eq!(f.focus(), LAST);
        typed(&mut f, "note");
        // Enter on the last field submits everything.
        assert_eq!(
            f.on(&key(Key::Enter)),
            Some(Msg::FormSubmit(FormData {
                id: None,
                start: "900".into(),
                end: "1230".into(),
                project: "alphabet".into(),
                comment: "note".into(),
            }))
        );
        assert_eq!(f.on(&key(Key::Esc)), Some(Msg::FormCancel));
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('s'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::FormSubmit(f.data()))
        );
        // Ctrl+C quits even with the overlay in front.
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::Quit)
        );
        // Any other chord is still swallowed rather than typed.
        assert_eq!(
            f.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('x'),
                KeyModifiers::CONTROL
            ))),
            None
        );
    }

    #[test]
    fn backspace_and_unmatched_project_are_kept() {
        let mut f = EntryForm::new(FormData::default(), vec!["Alpha".into()]);
        typed(&mut f, "80");
        assert_eq!(f.on(&key(Key::Backspace)), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.data().start, "8");
        f.on(&key(Key::Backspace));
        // Nothing left to delete: no change, so no message.
        assert_eq!(f.on(&key(Key::Backspace)), None);
        f.set_focus(PROJECT);
        typed(&mut f, "Zeta");
        assert!(f.matches().is_empty());
        // Accepting an empty picker keeps the typed name, which becomes a new project.
        f.on(&key(Key::Tab));
        assert_eq!(f.data().project, "Zeta");
    }

    #[test]
    fn draws_a_box_with_labels_and_footer() {
        let mut f = EntryForm::new(
            FormData {
                id: Some(3),
                start: "09:00".into(),
                end: "12:30".into(),
                project: "Alpha".into(),
                comment: "hi".into(),
            },
            vec!["Alpha".into()],
        );
        f.attr(
            Attribute::Text,
            AttrValue::String("gross +03:30 · break -00:18 · net +03:12".into()),
        );
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            f.view(fr, area);
        });
        assert!(contains(&rows, "Edit entry"));
        assert!(contains(&rows, "Start"));
        assert!(contains(&rows, "End"));
        assert!(contains(&rows, "Project"));
        assert!(contains(&rows, "Comment"));
        assert!(contains(&rows, "09:00"));
        assert!(contains(&rows, "12:30"));
        assert!(contains(&rows, "net +03:12"));
    }

    /// Every cell the overlay paints, so a test can assert which colors it used.
    fn cells(f: &mut EntryForm) -> Vec<tuirealm::ratatui::buffer::Cell> {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|fr| {
            let area = fr.area();
            f.view(fr, area)
        })
        .unwrap();
        term.backend().buffer().content().to_vec()
    }

    #[test]
    fn paints_only_with_theme_roles() {
        use std::collections::HashSet;
        use tuirealm::props::Color;

        // Unmistakable stand-ins, so a role that is not actually consulted cannot pass.
        let mut th = Theme::dark();
        th.muted = Color::Rgb(1, 1, 1);
        th.text = Color::Rgb(2, 2, 2);
        th.negative = Color::Rgb(3, 3, 3);
        th.bg_selected = Color::Rgb(4, 4, 4);

        let mut f = EntryForm::new(FormData::default(), vec!["Alpha".into()]).with_theme(&th);
        f.set_focus(PROJECT);
        f.attr(
            Attribute::Text,
            AttrValue::String("end: must differ from start".into()),
        );
        f.attr(Attribute::Custom(ERROR_FLAG), AttrValue::Flag(true));

        let painted = cells(&mut f);
        let fgs: HashSet<Color> = painted.iter().map(|c| c.fg).collect();
        let bgs: HashSet<Color> = painted.iter().map(|c| c.bg).collect();

        assert!(fgs.contains(&th.muted), "border, hints and blurred fields");
        assert!(fgs.contains(&th.text), "title and the focused field");
        assert!(fgs.contains(&th.negative), "the footer error");
        assert!(bgs.contains(&th.bg_selected), "the picker highlight");

        // Nothing is painted with a hardcoded color any more.
        for raw in [
            Color::Red,
            Color::Cyan,
            Color::Black,
            Color::DarkGray,
            Color::White,
        ] {
            assert!(
                !fgs.contains(&raw) && !bgs.contains(&raw),
                "raw {raw:?} left in the overlay"
            );
        }

        // A clean footer is muted, not red.
        f.attr(Attribute::Custom(ERROR_FLAG), AttrValue::Flag(false));
        let fgs: HashSet<Color> = cells(&mut f).iter().map(|c| c.fg).collect();
        assert!(!fgs.contains(&th.negative));
    }

    #[test]
    fn picker_row_shows_only_the_matching_projects() {
        let mut f = EntryForm::new(
            FormData::default(),
            vec!["Alpha".into(), "Beta".into(), "alphabet".into()],
        );
        f.set_focus(PROJECT);
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            f.view(fr, area)
        });
        // Nothing typed: every project is on offer.
        assert!(contains(&rows, "Alpha"));
        assert!(contains(&rows, "Beta"));
        typed(&mut f, "al");
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            f.view(fr, area)
        });
        assert!(contains(&rows, "alphabet"));
        assert!(!contains(&rows, "Beta"), "filtered out: {rows:?}");
        typed(&mut f, "zzz");
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            f.view(fr, area)
        });
        assert!(contains(&rows, "new project"), "{rows:?}");
    }
}
