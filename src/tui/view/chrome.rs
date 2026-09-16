//! Chrome shared by all TUI screens: title bar, status bar, key hints, overlays.

use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Alignment, Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Clear, Paragraph, Wrap};

use super::{block, minutes_span};
use crate::core::Minutes;
use crate::tui::theme::Theme;

pub struct TitleInfo {
    pub title: String,
    pub balance: Minutes,
    /// (project, clock-in time "HH:MM", running minutes)
    pub clock: Option<(String, String, Minutes)>,
    /// (used, allowance)
    pub vacation: Option<(u32, u32)>,
}

pub fn draw_title_bar(f: &mut Frame, area: Rect, t: &Theme, info: &TitleInfo) {
    let mut right: Vec<Span> = vec![
        Span::styled("balance ", Style::default().fg(t.muted)),
        minutes_span(info.balance, t),
    ];
    if let Some((project, since, running)) = &info.clock {
        // A session opened before `tk` recorded the project simply has none to name.
        let project = if project.is_empty() {
            String::new()
        } else {
            format!("{project} ")
        };
        right.push(Span::raw("   "));
        right.push(Span::styled(
            format!("⏱ {project}in since {since} ({})", running.hhmm()),
            Style::default().fg(t.warning),
        ));
    }
    if let Some((used, allow)) = info.vacation {
        right.push(Span::raw("   "));
        right.push(Span::styled(
            format!("vacation {used}/{allow}"),
            Style::default().fg(t.muted),
        ));
    }
    let [l, r] = Layout::horizontal([Constraint::Min(20), Constraint::Length(70)]).areas(area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", info.title),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))),
        l,
    );
    f.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        r,
    );
}

pub fn draw_status_bar(f: &mut Frame, area: Rect, t: &Theme, status: Option<(&str, bool)>) {
    let line = match status {
        Some((msg, true)) => Line::from(Span::styled(
            format!(" ✖ {msg}"),
            Style::default().fg(t.negative),
        )),
        Some((msg, false)) => Line::from(Span::styled(
            format!(" ✔ {msg}"),
            Style::default().fg(t.positive),
        )),
        None => Line::from(""),
    };
    f.render_widget(Paragraph::new(line), area);
}

/// Rendered width of a hint row: the leading space, every `key label` pair and
/// the two spaces between them.
pub fn hints_width(hints: &[(&str, &str)]) -> u16 {
    let pairs: usize = hints
        .iter()
        .map(|(k, d)| k.chars().count() + 1 + d.chars().count())
        .sum();
    (1 + pairs + 2 * hints.len().saturating_sub(1)) as u16
}

pub fn draw_key_hints(f: &mut Frame, area: Rect, t: &Theme, hints: &[(&str, &str)]) {
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, d)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(
            *k,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {d}"), Style::default().fg(t.muted)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn draw_too_small(f: &mut Frame, area: Rect, t: &Theme, size: (u16, u16)) {
    let text = format!(
        "Terminal too small: {}×{}. Need at least 80×24.",
        size.0, size.1
    );
    f.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(t.warning)))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

pub fn draw_confirm(f: &mut Frame, area: Rect, t: &Theme, question: &str) {
    let r = centered(area, (question.chars().count() as u16 + 6).max(30), 5);
    f.render_widget(Clear, r);
    let p = Paragraph::new(vec![
        Line::from(question),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "y",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" yes   ", Style::default().fg(t.muted)),
            Span::styled(
                "n",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" no", Style::default().fg(t.muted)),
        ]),
    ])
    .alignment(Alignment::Center)
    .block(block(t, Some("Confirm")));
    f.render_widget(p, r);
}

pub fn draw_help(f: &mut Frame, area: Rect, t: &Theme, title: &str, keys: &[(&str, &str)]) {
    let h = (keys.len() as u16 + 2).min(area.height);
    let r = centered(area, 50, h);
    f.render_widget(Clear, r);
    let lines: Vec<Line> = keys
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(
                    format!("{k:>10}  "),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw(*d),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block(t, Some(title))), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Minutes;
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};

    #[test]
    fn title_bar_shows_month_balance_and_clock() {
        let t = Theme::dark();
        let rows = render(100, 1, |f| {
            draw_title_bar(
                f,
                f.area(),
                &t,
                &TitleInfo {
                    title: "SEPTEMBER 2026".into(),
                    balance: Minutes(750),
                    clock: Some(("Alpha".into(), "08:12".into(), Minutes(221))),
                    vacation: Some((21, 30)),
                },
            );
        });
        assert!(contains(&rows, "SEPTEMBER 2026"));
        assert!(contains(&rows, "+12:30"));
        assert!(contains(&rows, "Alpha in since 08:12"));
        assert!(contains(&rows, "03:41"));
        assert!(contains(&rows, "21/30"));
    }

    #[test]
    fn key_hints_and_status() {
        let t = Theme::dark();
        let rows = render(100, 2, |f| {
            let [a, b] = tuirealm::ratatui::layout::Layout::vertical([
                tuirealm::ratatui::layout::Constraint::Length(1),
                tuirealm::ratatui::layout::Constraint::Length(1),
            ])
            .areas(f.area());
            draw_status_bar(f, a, &t, Some(("saved", false)));
            draw_key_hints(f, b, &t, &[("↑↓", "day"), ("q", "quit")]);
        });
        assert!(contains(&rows, "saved"));
        assert!(contains(&rows, "↑↓ day"));
        assert!(contains(&rows, "q quit"));
    }

    #[test]
    fn too_small_notice() {
        let t = Theme::dark();
        let rows = render(40, 10, |f| draw_too_small(f, f.area(), &t, (40, 10)));
        assert!(contains(&rows, "80"));
        assert!(contains(&rows, "24"));
    }
}
