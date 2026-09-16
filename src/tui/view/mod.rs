//! Pure drawing helpers shared by the TUI screens.

pub mod chrome;
pub mod day;
pub mod month;
pub mod stats;

use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders};

use super::theme::Theme;
use crate::core::{HoursFormat, Minutes};
use tuirealm::ratatui::style::Color;

/// A signed duration, colored by its sign and spelled the way `f` asks for.
pub fn minutes_span(m: Minutes, f: HoursFormat, theme: &Theme) -> Span<'static> {
    Span::styled(m.fmt_signed(f), theme.minutes_style(m))
}

pub fn chip(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

pub fn bar(frac: f64, width: u16, color: Color, theme: &Theme) -> Line<'static> {
    let filled = ((frac.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let empty = (width as usize).saturating_sub(filled);
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(theme.muted)),
    ])
}

pub fn block(theme: &Theme, title: Option<&str>) -> Block<'static> {
    let mut b = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border)
        .border_style(Style::default().fg(theme.muted));
    if let Some(t) = title {
        b = b.title(Span::styled(
            format!(" {t} "),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ));
    }
    b
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod testing {
    use tuirealm::ratatui::backend::TestBackend;
    use tuirealm::ratatui::{Frame, Terminal};

    /// Render with `f` into a WxH buffer and return the text rows (trailing spaces trimmed).
    pub fn render(w: u16, h: u16, f: impl FnOnce(&mut Frame)) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(f).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                let s: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
                s.trim_end().to_string()
            })
            .collect()
    }

    pub fn contains(rows: &[String], needle: &str) -> bool {
        rows.iter().any(|r| r.contains(needle))
    }
}
