//! The clock-in / switch overlay: one project field with the entry form's picker.
//!
//! `i` on the month view opens it, titled for what it is about to do: clocking in
//! on a project, or switching away from the one that is running. The field, the
//! keys and the footer are [`FieldForm`]; the filtered list of known projects and
//! its highlight are the same [`picker_line`] the entry form draws, so both
//! overlays offer projects in exactly the same way.

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyModifiers};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::widgets::{Clear, Paragraph};
use tuirealm::state::State;

use super::field_form::{FieldForm, FieldFormEvent, FieldSpec, filter_projects, picker_line};
use crate::tui::msg::{Msg, UserEvent};
use crate::tui::theme::Theme;
use crate::tui::view::chrome::centered;

fn fields() -> Vec<FieldSpec> {
    vec![FieldSpec::new("Project", "type to filter, up/down to pick")]
}

/// Width and height of the overlay: one field, the offered projects, the footer.
const WIDTH: u16 = 64;
const HEIGHT: u16 = 7;

pub struct ProjectPicker {
    form: FieldForm,
    title: String,
    projects: Vec<String>,
    idx: usize,
}

impl ProjectPicker {
    /// `projects` is offered in the order given, so the caller decides what the
    /// overlay preselects (the last used project) and what it leaves out (the
    /// project already running).
    pub fn new(title: String, projects: Vec<String>) -> Self {
        Self {
            form: FieldForm::new(&fields(), &[String::new()]),
            title,
            projects,
            idx: 0,
        }
    }

    /// Repaint the overlay in the application's theme.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        self.form = self.form.with_theme(t);
        self
    }

    fn matches(&self) -> Vec<String> {
        filter_projects(&self.projects, &self.form.value(0))
    }

    /// What a submit takes: the highlighted project, or — with nothing matching the
    /// typed text — that text, which the store turns into a new project.
    fn picked(&self) -> String {
        self.matches()
            .get(self.idx)
            .cloned()
            .unwrap_or_else(|| self.form.value(0).trim().to_string())
    }
}

impl Component for ProjectPicker {
    fn view(&mut self, f: &mut Frame, area: Rect) {
        let r = centered(area, WIDTH, HEIGHT);
        f.render_widget(Clear, r);
        let outer = self.form.panel(&self.title);
        let inner = outer.inner(r);
        f.render_widget(outer, r);
        let [field, picker, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
        self.form.field_view(0, f, field);
        let line = picker_line(self.form.theme(), &self.matches(), self.idx, true);
        f.render_widget(Paragraph::new(line), picker);
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

impl AppComponent<Msg, UserEvent> for ProjectPicker {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let k = *ev.as_keyboard()?;
        // The arrows move the highlight (there is only one field to focus), and Tab
        // accepts it — the same keys the entry form's project field answers to.
        if !k.modifiers.contains(KeyModifiers::CONTROL) {
            match k.code {
                Key::Down => {
                    let last = self.matches().len().saturating_sub(1);
                    self.idx = (self.idx + 1).min(last);
                    return Some(Msg::ClockPickerChanged);
                }
                Key::Up => {
                    self.idx = self.idx.saturating_sub(1);
                    return Some(Msg::ClockPickerChanged);
                }
                Key::Tab => return Some(Msg::ClockPickerSubmit(self.picked())),
                _ => {}
            }
        }
        match self.form.handle_key(&k) {
            FieldFormEvent::Quit => Some(Msg::Quit),
            FieldFormEvent::Submit => Some(Msg::ClockPickerSubmit(self.picked())),
            FieldFormEvent::Cancel => Some(Msg::ClockPickerCancel),
            FieldFormEvent::Changed => {
                // The filter moved: start again at the first match.
                self.idx = 0;
                Some(Msg::ClockPickerChanged)
            }
            FieldFormEvent::Ignored => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::msg::{Msg, UserEvent};
    use crate::tui::view::testing::{contains, render};
    use tuirealm::component::AppComponent;
    use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

    fn key(k: Key) -> Event<UserEvent> {
        Event::Keyboard(KeyEvent::new(k, KeyModifiers::NONE))
    }

    fn picker() -> ProjectPicker {
        ProjectPicker::new(
            "Switch project — currently Alpha".into(),
            vec!["Beta".into(), "Gamma".into(), "beta-two".into()],
        )
    }

    #[test]
    fn draws_its_title_the_field_and_the_projects() {
        let mut p = picker();
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            p.view(fr, area);
        });
        assert!(contains(&rows, "Switch project"), "{rows:?}");
        assert!(contains(&rows, "currently Alpha"), "{rows:?}");
        assert!(contains(&rows, "Project"), "{rows:?}");
        assert!(contains(&rows, "Beta"), "{rows:?}");
        assert!(contains(&rows, "Gamma"), "{rows:?}");
        assert!(contains(&rows, "Esc cancel"), "the key hints: {rows:?}");
    }

    #[test]
    fn typing_filters_and_enter_submits_the_highlighted_project() {
        let mut p = picker();
        // Nothing typed: the first offer is what Enter takes.
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::ClockPickerSubmit("Beta".into()))
        );
        let mut p = picker();
        for c in "bet".chars() {
            p.on(&key(Key::Char(c)));
        }
        // The filter is case-insensitive, and ↓ moves the highlight inside it.
        p.on(&key(Key::Down));
        assert_eq!(
            p.on(&key(Key::Tab)),
            Some(Msg::ClockPickerSubmit("beta-two".into()))
        );
        // A name that matches nothing is submitted as typed, and becomes a new project.
        let mut p = picker();
        for c in "Zeta".chars() {
            p.on(&key(Key::Char(c)));
        }
        assert_eq!(
            p.on(&key(Key::Enter)),
            Some(Msg::ClockPickerSubmit("Zeta".into()))
        );
    }

    #[test]
    fn esc_cancels_and_ctrl_c_quits() {
        let mut p = picker();
        assert_eq!(p.on(&key(Key::Esc)), Some(Msg::ClockPickerCancel));
        assert_eq!(
            p.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Msg::Quit)
        );
    }
}
