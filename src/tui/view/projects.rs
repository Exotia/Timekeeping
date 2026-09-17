//! Projects screen drawing.

use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Cell, Paragraph, Row as TRow, Table};

use super::block;
use crate::core::HoursFormat;
use crate::tui::model::Model;
use crate::tui::msg::ProjectsData;
use crate::tui::theme::Theme;

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let t = &m.theme;
    match &m.projects {
        Some(data) => draw_table(f, area, t, data, m.projects_cursor, m.hours),
        None => {
            let p = Paragraph::new(Span::styled("loading…", Style::default().fg(t.muted)))
                .block(block(t, Some("Projects")));
            f.render_widget(p, area);
        }
    }
}

/// The table of projects, and the count line under it.
pub fn draw_table(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    data: &ProjectsData,
    cursor: usize,
    fmt: HoursFormat,
) {
    let [table_area, count_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);

    let header = TRow::new(["PROJECT", "NET", "STATUS"].iter().map(|h| {
        Cell::from(Span::styled(
            *h,
            Style::default().fg(t.muted).add_modifier(Modifier::BOLD),
        ))
    }));

    let rows: Vec<TRow> = data
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let sel = i == cursor;
            // Archived projects are still listed, but they are out of the way:
            // muted, where an active one wears its own color.
            let name_style = if p.archived {
                Style::default().fg(t.muted)
            } else {
                Style::default()
                    .fg(t.project_color(p.color_index))
                    .add_modifier(Modifier::BOLD)
            };
            let net = data
                .net_by_project
                .get(&p.name)
                .copied()
                .unwrap_or_default();
            let mut row = TRow::new(vec![
                Cell::from(Line::from(vec![
                    Span::styled(if sel { "▶" } else { " " }, Style::default().fg(t.accent)),
                    Span::styled(p.name.clone(), name_style),
                ])),
                Cell::from(Span::styled(
                    net.fmt_unsigned(fmt),
                    Style::default().fg(t.text),
                )),
                Cell::from(Span::styled(
                    if p.archived { "archived" } else { "" },
                    Style::default().fg(t.muted),
                )),
            ]);
            if sel {
                row = row.style(Style::default().bg(t.bg_selected));
            }
            row
        })
        .collect();

    let widths = [
        Constraint::Min(10),
        Constraint::Length(9),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .block(block(t, Some("Projects")));
    f.render_widget(table, table_area);

    let archived = data.projects.iter().filter(|p| p.archived).count();
    let total = data.projects.len();
    f.render_widget(
        Paragraph::new(Span::styled(
            format!(
                " {total} project{} · {archived} archived",
                if total == 1 { "" } else { "s" }
            ),
            Style::default().fg(t.muted),
        )),
        count_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Minutes, Project};
    use crate::tui::view::testing::{contains, render, style_of};
    use std::collections::BTreeMap;
    use tuirealm::ratatui::style::Modifier;

    fn data() -> ProjectsData {
        ProjectsData {
            projects: vec![
                Project {
                    id: 1,
                    name: "Alpha".into(),
                    color_index: 0,
                    archived: false,
                },
                Project {
                    id: 2,
                    name: "Old".into(),
                    color_index: 1,
                    archived: true,
                },
            ],
            net_by_project: BTreeMap::from([("Alpha".to_string(), Minutes(222))]),
        }
    }

    #[test]
    fn renders_projects_with_hours_cursor_and_archived_marker() {
        let t = Theme::dark();
        let data = data();
        let rows = render(80, 20, |f| {
            draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm)
        });
        assert!(contains(&rows, "PROJECT"), "{rows:?}");
        assert!(contains(&rows, "03:42"), "{rows:?}");
        assert!(
            rows.iter()
                .any(|r| r.contains('▶') && r.contains("Old") && r.contains("archived")),
            "{rows:?}"
        );
        assert!(contains(&rows, "2 projects · 1 archived"), "{rows:?}");
        let bg = style_of(
            80,
            20,
            |f| draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm),
            "Old",
        )
        .bg;
        assert_eq!(bg, Some(t.bg_selected), "the cursor row is highlighted");
        let alpha = style_of(
            80,
            20,
            |f| draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm),
            "Alpha",
        );
        assert_eq!(alpha.fg, Some(t.project_color(0)));
        assert!(alpha.add_modifier.contains(Modifier::BOLD));
        let old = style_of(
            80,
            20,
            |f| draw_table(f, f.area(), &t, &data, 1, HoursFormat::Hm),
            "Old",
        );
        assert_eq!(old.fg, Some(t.muted));
    }

    #[test]
    fn a_single_project_count_line_is_singular() {
        let t = Theme::dark();
        let mut data = data();
        data.projects.truncate(1);
        let rows = render(80, 20, |f| {
            draw_table(f, f.area(), &t, &data, 0, HoursFormat::Hm)
        });
        assert!(contains(&rows, "1 project · 0 archived"), "{rows:?}");
    }
}
