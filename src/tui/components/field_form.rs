//! `FieldForm`: the labelled-input machinery shared by the entry form and the
//! settings overlay.
//!
//! It owns N [`Input`] fields, the focus ring, the editing and navigation keys and
//! the footer line the model pushes back in through [`Attribute::Text`] and
//! `Attribute::Custom(ERROR_FLAG)`. It deliberately knows nothing about messages:
//! [`FieldForm::handle_key`] reports a [`FieldFormEvent`] and the component around
//! it decides which `Msg` that is.
//!
//! Two ways to draw it: [`FieldForm::view`] lays the fields out one per row, which
//! is what the settings overlay wants; a component with a layout of its own (the
//! entry form puts start and end side by side and threads a project picker between
//! the rows) builds its own `Layout` and calls [`FieldForm::panel`],
//! [`FieldForm::field_view`] and [`FieldForm::footer_widget`] itself.

use std::borrow::Cow;

use tui_realm_stdlib::components::Input;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::Component;
use tuirealm::event::{Key, KeyEvent, KeyModifiers};
use tuirealm::props::{AttrValue, Attribute, Borders, Props, QueryResult, Style};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::Modifier;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Clear, Paragraph};
use tuirealm::state::{State, StateValue};

use crate::tui::theme::Theme;
use crate::tui::view::block;
use crate::tui::view::chrome::centered;

/// Custom attribute the model sets when the footer text is an error message.
pub const ERROR_FLAG: &str = "error";

/// Width of every form overlay, and the hint line under its bottom border.
const WIDTH: u16 = 64;
const HINTS: &str = " Tab next · Ctrl+S save · Esc cancel ";

/// One labelled input: what it is called and what it shows while empty.
///
/// Both are `Cow`, so a form with fixed fields still names them with string
/// literals while one built at runtime (the break split, a field per entry of a
/// session) can own its labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    pub label: Cow<'static, str>,
    pub placeholder: Cow<'static, str>,
}

impl FieldSpec {
    pub fn new(
        label: impl Into<Cow<'static, str>>,
        placeholder: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            label: label.into(),
            placeholder: placeholder.into(),
        }
    }
}

/// What a key did to the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldFormEvent {
    /// The text or the focus moved: re-validate and repaint.
    Changed,
    Submit,
    Cancel,
    /// Ctrl+C: quit the whole application, overlay or not.
    Quit,
    /// Not a key of ours, or a key that changed nothing.
    Ignored,
}

/// The known project names containing `query`, case-insensitively.
pub fn filter_projects(projects: &[String], query: &str) -> Vec<String> {
    let q = query.trim().to_lowercase();
    projects
        .iter()
        .filter(|p| p.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// The row of project chips under a project field, `idx` highlighted while `active`.
///
/// Shared by the entry form and the clock-in picker, so both offer, filter and
/// highlight projects in exactly the same way.
pub fn picker_line(t: &Theme, matches: &[String], idx: usize, active: bool) -> Line<'static> {
    if matches.is_empty() {
        return Line::from(Span::styled(
            " no match — will be created as a new project ",
            Style::default().fg(t.muted),
        ));
    }
    Line::from(
        matches
            .iter()
            .enumerate()
            .take(6)
            .flat_map(|(i, p)| {
                // `bg_selected` is the role the day and month tables already use for
                // "the row under the cursor", so the picker highlight reads as the same
                // kind of selection; bold lifts it off a subtle background.
                let style = if i == idx && active {
                    Style::default()
                        .fg(t.text)
                        .bg(t.bg_selected)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.muted)
                };
                [Span::styled(format!(" {p} "), style), Span::raw(" ")]
            })
            .collect::<Vec<Span>>(),
    )
}

pub struct FieldForm {
    props: Props,
    specs: Vec<FieldSpec>,
    fields: Vec<Input>,
    focus: usize,
    theme: Theme,
}

/// The inputs, painted in `t`.
fn build_fields(t: &Theme, specs: &[FieldSpec], values: &[String]) -> Vec<Input> {
    specs
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            Input::default()
                .title(spec.label.to_string())
                .value(values.get(i).map(String::as_str).unwrap_or(""))
                .placeholder(spec.placeholder.to_string())
                .borders(Borders::default().modifiers(t.border).color(t.muted))
                .foreground(t.text)
                // Dim the fields that do not have the caret, so the focus ring is obvious.
                .inactive(Style::default().fg(t.muted))
                .invalid_style(Style::default().fg(t.negative))
        })
        .collect()
}

impl FieldForm {
    pub fn new(specs: &[FieldSpec], values: &[String]) -> Self {
        // The palette arrives afterwards through `with_theme`, because the components
        // built on this one have pinned constructors; until then the built-in dark
        // theme stands in (component unit tests never set one).
        let theme = Theme::dark();
        let mut f = Self {
            props: Props::default(),
            specs: specs.to_vec(),
            fields: build_fields(&theme, specs, values),
            focus: 0,
            theme,
        };
        f.set_focus(0);
        f
    }

    /// Repaint in the application's theme, keeping the text that is already typed.
    pub fn with_theme(mut self, t: &Theme) -> Self {
        let values = self.values();
        self.theme = t.clone();
        self.fields = build_fields(&self.theme, &self.specs, &values);
        self.set_focus(self.focus);
        self
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn set_focus(&mut self, i: usize) {
        for (n, fld) in self.fields.iter_mut().enumerate() {
            fld.attr(Attribute::Focus, AttrValue::Flag(n == i));
        }
        self.focus = i;
    }

    pub fn value(&self, i: usize) -> String {
        match self.fields[i].state() {
            State::Single(StateValue::String(s)) => s,
            _ => String::new(),
        }
    }

    pub fn values(&self) -> Vec<String> {
        (0..self.fields.len()).map(|i| self.value(i)).collect()
    }

    pub fn set_value(&mut self, i: usize, s: &str) {
        self.fields[i].attr(Attribute::Value, AttrValue::String(s.to_string()));
    }

    /// Props access, so the component around this one can forward `attr`/`query`
    /// straight through: the footer text lives here.
    pub fn attr(&mut self, attr: Attribute, value: AttrValue) {
        self.props.set(attr, value);
    }

    pub fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> {
        self.props.get_for_query(attr)
    }

    /// The footer line the model last pushed in, and whether it is an error.
    pub fn footer(&self) -> (String, bool) {
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

    /// The house panel: `theme.border` type, `muted` border, bold `text` title, key
    /// hints along the bottom edge.
    pub fn panel(&self, title: &str) -> tuirealm::ratatui::widgets::Block<'static> {
        block(&self.theme, Some(title))
            .title_bottom(Span::styled(HINTS, Style::default().fg(self.theme.muted)))
    }

    /// Draw field `i` into `area`.
    pub fn field_view(&mut self, i: usize, f: &mut Frame, area: Rect) {
        self.fields[i].view(f, area);
    }

    /// The footer as a widget, muted or in the negative color.
    pub fn footer_widget(&self) -> Paragraph<'static> {
        let (line, is_err) = self.footer();
        let color = if is_err {
            self.theme.negative
        } else {
            self.theme.muted
        };
        Paragraph::new(Span::styled(line, Style::default().fg(color)))
    }

    /// Height the default layout needs for `n` fields, borders included.
    fn height(&self, extra: bool) -> u16 {
        3 * self.fields.len() as u16 + u16::from(extra) + 3
    }

    /// The default layout: one field per row, then `extra_rows`, then the footer.
    pub fn view(&mut self, f: &mut Frame, area: Rect, title: &str, extra_rows: Option<Line>) {
        let r = centered(area, WIDTH, self.height(extra_rows.is_some()));
        f.render_widget(Clear, r);
        let outer = self.panel(title);
        let inner = outer.inner(r);
        f.render_widget(outer, r);

        let mut constraints: Vec<Constraint> =
            self.fields.iter().map(|_| Constraint::Length(3)).collect();
        if extra_rows.is_some() {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(0));
        constraints.push(Constraint::Length(1));
        let rows = Layout::vertical(constraints).split(inner);

        for i in 0..self.fields.len() {
            self.field_view(i, f, rows[i]);
        }
        if let Some(line) = extra_rows {
            f.render_widget(Paragraph::new(line), rows[self.fields.len()]);
        }
        f.render_widget(self.footer_widget(), rows[rows.len() - 1]);
    }

    /// Navigation and editing. The component around this one gets first refusal on
    /// any key it treats specially (the entry form's project picker, say).
    pub fn handle_key(&mut self, k: &KeyEvent) -> FieldFormEvent {
        let KeyEvent { code, modifiers } = *k;
        let last = self.fields.len().saturating_sub(1);
        if modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl+C quits from anywhere, Ctrl+S saves; every other control chord is
            // swallowed so that it does not end up typed into a field.
            return match code {
                Key::Char('c') => FieldFormEvent::Quit,
                Key::Char('s') => FieldFormEvent::Submit,
                _ => FieldFormEvent::Ignored,
            };
        }
        match code {
            Key::Esc => return FieldFormEvent::Cancel,
            Key::Tab | Key::Down => self.set_focus((self.focus + 1) % self.fields.len()),
            Key::BackTab | Key::Up => {
                self.set_focus((self.focus + last) % self.fields.len());
            }
            Key::Enter if self.focus == last => return FieldFormEvent::Submit,
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
                    _ => return FieldFormEvent::Ignored,
                };
                // `Changed` is an edit, `Visual` a cursor move; anything else (`NoChange`
                // at either end of the text, `Invalid`) left the overlay as it was.
                if !matches!(
                    self.fields[self.focus].perform(cmd),
                    CmdResult::Changed(_) | CmdResult::Submit(_) | CmdResult::Visual
                ) {
                    return FieldFormEvent::Ignored;
                }
            }
        }
        // Every handled key changes how the overlay looks, and the main loop only
        // repaints after an `update`, so report back even for pure navigation.
        FieldFormEvent::Changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use tuirealm::event::{Key, KeyEvent, KeyModifiers};
    use tuirealm::props::{AttrValue, Attribute};

    fn specs() -> Vec<FieldSpec> {
        vec![
            FieldSpec::new("One", "first"),
            FieldSpec::new("Two", "second"),
            FieldSpec::new("Three", "third"),
        ]
    }

    fn form() -> FieldForm {
        FieldForm::new(&specs(), &["".to_string(), "".into(), "".into()])
    }

    fn key(k: Key) -> KeyEvent {
        KeyEvent::new(k, KeyModifiers::NONE)
    }

    fn typed(f: &mut FieldForm, s: &str) -> FieldFormEvent {
        let mut last = FieldFormEvent::Ignored;
        for c in s.chars() {
            last = f.handle_key(&key(Key::Char(c)));
        }
        last
    }

    #[test]
    fn navigates_edits_and_reports_events() {
        let mut f = form();
        assert_eq!(typed(&mut f, "abc"), FieldFormEvent::Changed);
        assert_eq!(f.values(), vec!["abc", "", ""]);
        // Tab and Enter walk forward, Shift+Tab walks back, both wrapping.
        assert_eq!(f.handle_key(&key(Key::Tab)), FieldFormEvent::Changed);
        assert_eq!(f.focus(), 1);
        assert_eq!(f.handle_key(&key(Key::Enter)), FieldFormEvent::Changed);
        assert_eq!(f.focus(), 2);
        // Enter on the last field submits instead of wrapping.
        assert_eq!(f.handle_key(&key(Key::Enter)), FieldFormEvent::Submit);
        assert_eq!(f.handle_key(&key(Key::BackTab)), FieldFormEvent::Changed);
        assert_eq!(f.focus(), 1);
        assert_eq!(f.handle_key(&key(Key::Tab)), FieldFormEvent::Changed);
        assert_eq!(f.handle_key(&key(Key::Tab)), FieldFormEvent::Changed);
        assert_eq!(f.focus(), 0, "Tab wraps round");
        // Editing keys.
        assert_eq!(f.handle_key(&key(Key::Backspace)), FieldFormEvent::Changed);
        assert_eq!(f.values()[0], "ab");
        f.set_focus(1);
        // Nothing to delete in an empty field: nothing changed, so nothing to repaint.
        assert_eq!(f.handle_key(&key(Key::Backspace)), FieldFormEvent::Ignored);
        // A key the form has no use for is ignored rather than typed.
        assert_eq!(
            f.handle_key(&key(Key::Function(1))),
            FieldFormEvent::Ignored
        );
        // Esc cancels; Ctrl+S saves; Ctrl+C quits; every other chord is swallowed.
        assert_eq!(f.handle_key(&key(Key::Esc)), FieldFormEvent::Cancel);
        let ctrl = |c| KeyEvent::new(Key::Char(c), KeyModifiers::CONTROL);
        assert_eq!(f.handle_key(&ctrl('s')), FieldFormEvent::Submit);
        assert_eq!(f.handle_key(&ctrl('c')), FieldFormEvent::Quit);
        assert_eq!(f.handle_key(&ctrl('x')), FieldFormEvent::Ignored);
        assert_eq!(f.values(), vec!["ab", "", ""], "no chord reached a field");
    }

    #[test]
    fn set_value_and_theme_keep_the_text() {
        let mut f = form();
        f.set_value(2, "third value");
        assert_eq!(f.values()[2], "third value");
        let f = f.with_theme(&Theme::light());
        assert_eq!(f.values()[2], "third value");
    }

    #[test]
    fn draws_a_titled_box_with_labels_footer_and_extra_row() {
        let mut f = form();
        f.set_value(0, "hello");
        f.attr(
            Attribute::Text,
            AttrValue::String("all good here".to_string()),
        );
        let rows = render(80, 24, |fr| {
            let area = fr.area();
            f.view(fr, area, "Settings", Some(Line::from("an extra row")));
        });
        assert!(contains(&rows, "Settings"), "{rows:?}");
        for label in ["One", "Two", "Three"] {
            assert!(contains(&rows, label), "{label} missing: {rows:?}");
        }
        assert!(contains(&rows, "hello"));
        assert!(contains(&rows, "an extra row"));
        assert!(contains(&rows, "all good here"));
        assert!(contains(&rows, "Esc cancel"), "the key hints: {rows:?}");
    }
}
