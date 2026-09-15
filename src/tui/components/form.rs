//! The entry form overlay: four text inputs plus an inline project picker.
//!
//! This is the only component that draws anything; everything else is painted by
//! `Model::draw`. It owns the field text (that is what `tui_realm_stdlib::Input` is
//! for) and reports every change back to the model as [`Msg::FormChanged`], so the
//! model can re-validate and push the footer line back in through
//! [`Attribute::Text`] / `Attribute::Custom(ERROR_FLAG)`.

use tui_realm_stdlib::components::Input;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};
use tuirealm::props::{AttrValue, Attribute, Borders, Props, QueryResult, Style};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::Modifier;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Clear, Paragraph};
use tuirealm::state::{State, StateValue};

use crate::tui::msg::{FormData, Msg, UserEvent};
use crate::tui::theme::Theme;
use crate::tui::view::block;
use crate::tui::view::chrome::centered;

/// Custom attribute the model sets when the footer text is an error message.
pub const ERROR_FLAG: &str = "error";

/// Index of the project field, the only one with a picker attached.
const PROJECT: usize = 2;
/// Index of the last field; Enter there submits.
const LAST: usize = 3;

const TITLES: [&str; 4] = ["Start", "End", "Project", "Comment"];
const PLACEHOLDERS: [&str; 4] = [
    "0800",
    "1730",
    "type to filter, up/down to pick",
    "optional",
];

/// The four inputs, painted in `t`.
fn build_fields(t: &Theme, initial: &FormData) -> [Input; 4] {
    let values = [
        &initial.start,
        &initial.end,
        &initial.project,
        &initial.comment,
    ];
    std::array::from_fn(|i| {
        Input::default()
            .title(TITLES[i])
            .value(values[i].as_str())
            .placeholder(PLACEHOLDERS[i])
            .borders(Borders::default().modifiers(t.border).color(t.muted))
            .foreground(t.text)
            // Dim the fields that do not have the caret, so the focus ring is obvious.
            .inactive(Style::default().fg(t.muted))
            .invalid_style(Style::default().fg(t.negative))
    })
}

pub struct EntryForm {
    props: Props,
    fields: [Input; 4],
    focus: usize,
    id: Option<i64>,
    projects: Vec<String>,
    picker_idx: usize,
    theme: Theme,
}

impl EntryForm {
    pub fn new(initial: FormData, projects: Vec<String>) -> Self {
        // `new`'s signature is pinned, so the palette arrives afterwards through
        // `with_theme`; until then the built-in dark theme stands in (component unit
        // tests never set one).
        let theme = Theme::dark();
        let mut f = Self {
            props: Props::default(),
            fields: build_fields(&theme, &initial),
            focus: 0,
            id: initial.id,
            projects,
            picker_idx: 0,
            theme,
        };
        f.set_focus(0);
        f
    }

    /// Repaint the overlay in the application's theme. Chained straight onto
    /// [`EntryForm::new`], so the inputs still hold exactly their initial text and
    /// rebuilding them loses nothing.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        let data = self.data();
        self.theme = t.clone();
        self.fields = build_fields(&self.theme, &data);
        self.set_focus(self.focus);
        self
    }

    fn set_focus(&mut self, i: usize) {
        for (n, fld) in self.fields.iter_mut().enumerate() {
            fld.attr(Attribute::Focus, AttrValue::Flag(n == i));
        }
        self.focus = i;
        if i == PROJECT {
            self.picker_idx = 0;
        }
    }

    fn value(&self, i: usize) -> String {
        match self.fields[i].state() {
            State::Single(StateValue::String(s)) => s,
            _ => String::new(),
        }
    }

    pub fn data(&self) -> FormData {
        FormData {
            id: self.id,
            start: self.value(0),
            end: self.value(1),
            project: self.value(PROJECT),
            comment: self.value(LAST),
        }
    }

    /// Known projects whose name contains the typed text (case-insensitive).
    fn matches(&self) -> Vec<String> {
        let q = self.value(PROJECT).trim().to_lowercase();
        self.projects
            .iter()
            .filter(|p| p.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    /// Copy the highlighted project into the field. Without a match the typed text
    /// stays as it is and becomes a new project on submit.
    fn accept_pick(&mut self) {
        let m = self.matches();
        if let Some(p) = m.get(self.picker_idx).cloned() {
            self.fields[PROJECT].attr(Attribute::Value, AttrValue::String(p));
        }
    }

    fn footer(&self) -> (String, bool) {
        let text = self
            .props
            .get(Attribute::Text)
            .and_then(AttrValue::as_string)
            .cloned()
            .unwrap_or_default();
        let is_err = self
            .props
            .get(Attribute::Custom(ERROR_FLAG))
            .and_then(AttrValue::as_flag)
            .unwrap_or(false);
        (text, is_err)
    }
}

impl Component for EntryForm {
    fn view(&mut self, f: &mut Frame, area: Rect) {
        let (muted, text, negative, bg_selected) = (
            self.theme.muted,
            self.theme.text,
            self.theme.negative,
            self.theme.bg_selected,
        );
        let r = centered(area, 64, 16);
        f.render_widget(Clear, r);
        let title = if self.id.is_some() {
            "Edit entry"
        } else {
            "New entry"
        };
        // The house panel: `theme.border` type, `muted` border, bold `text` title.
        let outer = block(&self.theme, Some(title)).title_bottom(Span::styled(
            " Tab next · Ctrl+S save · Esc cancel ",
            Style::default().fg(muted),
        ));
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
        self.fields[0].view(f, s);
        self.fields[1].view(f, e);
        self.fields[PROJECT].view(f, proj);

        let m = self.matches();
        let spans: Vec<Span> = if m.is_empty() {
            vec![Span::styled(
                " no match — will be created as a new project ",
                Style::default().fg(muted),
            )]
        } else {
            m.iter()
                .enumerate()
                .take(6)
                .flat_map(|(i, p)| {
                    // `bg_selected` is the role the day and month tables already use
                    // for "the row under the cursor", so the picker highlight reads as
                    // the same kind of selection; bold lifts it off a subtle background.
                    let style = if i == self.picker_idx && self.focus == PROJECT {
                        Style::default()
                            .fg(text)
                            .bg(bg_selected)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(muted)
                    };
                    [Span::styled(format!(" {p} "), style), Span::raw(" ")]
                })
                .collect()
        };
        f.render_widget(Paragraph::new(Line::from(spans)), picker);

        self.fields[LAST].view(f, comment);

        let (line, is_err) = self.footer();
        let style = Style::default().fg(if is_err { negative } else { muted });
        f.render_widget(Paragraph::new(Span::styled(line, style)), footer);
    }

    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> {
        self.props.get_for_query(attr)
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
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
        let KeyEvent { code, modifiers } = *ev.as_keyboard()?;
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            // Ctrl+S saves; every other control chord is swallowed so that e.g.
            // Ctrl+C does not end up typed into a field.
            return match code {
                Key::Char('s') => Some(Msg::FormSubmit(self.data())),
                _ => None,
            };
        }
        match code {
            Key::Esc => return Some(Msg::FormCancel),
            Key::Tab | Key::Down if self.focus != PROJECT => self.set_focus((self.focus + 1) % 4),
            Key::BackTab | Key::Up if self.focus != PROJECT => self.set_focus((self.focus + 3) % 4),
            // In the project field the arrows drive the picker instead.
            Key::Down => {
                let last = self.matches().len().saturating_sub(1);
                self.picker_idx = (self.picker_idx + 1).min(last);
            }
            Key::Up => self.picker_idx = self.picker_idx.saturating_sub(1),
            // Tab out of the project field takes the highlighted match with it.
            Key::Tab => {
                self.accept_pick();
                self.set_focus(LAST);
            }
            Key::BackTab => self.set_focus(PROJECT - 1),
            Key::Enter if self.focus == PROJECT => {
                self.accept_pick();
                self.set_focus(LAST);
            }
            Key::Enter if self.focus == LAST => return Some(Msg::FormSubmit(self.data())),
            Key::Enter => self.set_focus(self.focus + 1),
            _ => {
                let cmd = match code {
                    Key::Char(c) => Cmd::Type(c),
                    Key::Backspace => Cmd::Delete,
                    Key::Delete => Cmd::Cancel,
                    Key::Left => Cmd::Move(Direction::Left),
                    Key::Right => Cmd::Move(Direction::Right),
                    Key::Home => Cmd::GoTo(Position::Begin),
                    Key::End => Cmd::GoTo(Position::End),
                    // Not a key of ours: nothing moved, so nothing to repaint.
                    _ => return None,
                };
                // `Changed` is an edit, `Visual` a cursor move; anything else (`NoChange`
                // at either end of the text, `Invalid`) left the overlay as it was.
                if !matches!(
                    self.fields[self.focus].perform(cmd),
                    CmdResult::Changed(_) | CmdResult::Submit(_) | CmdResult::Visual
                ) {
                    return None;
                }
                if self.focus == PROJECT {
                    // The filter moved; start again at the first match.
                    self.picker_idx = 0;
                }
            }
        }
        // Every handled key changes how the overlay looks, and the main loop only repaints
        // after an `update`, so report back even for pure navigation.
        Some(Msg::FormChanged(self.data()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::view::testing::{contains, render};

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
        assert_eq!(f.focus, 1);
        typed(&mut f, "1230");
        assert_eq!(f.data().end, "1230");
        // Enter on a middle field just advances.
        assert_eq!(f.on(&key(Key::Enter)), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.focus, PROJECT);
        // Typing filters the picker case-insensitively.
        typed(&mut f, "al");
        assert_eq!(f.matches(), vec!["Alpha".to_string(), "alphabet".into()]);
        // Down moves the highlight, Enter accepts it and moves on to the comment.
        f.on(&key(Key::Down));
        assert_eq!(f.on(&key(Key::Enter)), Some(Msg::FormChanged(f.data())));
        assert_eq!(f.data().project, "alphabet");
        assert_eq!(f.focus, LAST);
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
