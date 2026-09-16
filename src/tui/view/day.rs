//! Day editor drawing.

use chrono::NaiveDate;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Cell, Paragraph, Row, Table};

use super::{block, chip, minutes_span};
use crate::core::{DayKind, DayStats, HoursFormat, TodayCtx, day_stats};
use crate::tui::model::Model;
use crate::tui::msg::DayData;
use crate::tui::theme::Theme;

pub const KIND_CYCLE: [&str; 6] = ["work", "vacation", "flex", "holiday", "sick", "absence"];

pub fn kind_index(k: &DayKind) -> usize {
    KIND_CYCLE
        .iter()
        .position(|s| *s == k.as_str())
        .unwrap_or(0)
}

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let Some(data) = &m.day else {
        f.render_widget(
            Paragraph::new("loading…").style(Style::default().fg(m.theme.muted)),
            area,
        );
        return;
    };
    let clocked_in = m.month.as_ref().is_some_and(|x| x.session.is_some());
    let stats = day_stats(
        &data.day,
        &m.rules,
        &m.cal,
        &TodayCtx {
            today: m.today,
            clocked_in,
        },
    );
    draw_day(
        f,
        area,
        &m.theme,
        data,
        &stats,
        m.day_cursor,
        &KIND_CYCLE,
        kind_index(&data.day.kind),
        m.hours,
    );
}

fn long_date(d: NaiveDate) -> String {
    d.format("%A, %-d %B %Y").to_string()
}

#[allow(clippy::too_many_arguments)]
pub fn draw_day(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    data: &DayData,
    stats: &DayStats,
    cursor: usize,
    kinds: &[&str],
    kind_idx: usize,
    fmt: HoursFormat,
) {
    let [head, kind_a, table_a, foot] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .areas(area);

    let mut title = vec![Span::styled(
        long_date(data.day.date),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )];
    if let Some(h) = &stats.holiday_name {
        title.push(Span::styled(
            format!("   {h}"),
            Style::default().fg(t.chip_holiday),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(title)), head);

    let mut ks: Vec<Span> = vec![Span::styled("← → ", Style::default().fg(t.muted))];
    for (i, k) in kinds.iter().enumerate() {
        let k: &str = k;
        let sel = i == kind_idx;
        let text = if sel {
            format!("[{k}]")
        } else {
            format!(" {k} ")
        };
        let style = if sel {
            let kind = DayKind::parse(k, None).unwrap_or(DayKind::Work);
            Style::default()
                .fg(t.kind_color(&kind))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.muted)
        };
        ks.push(Span::styled(text, style));
        ks.push(Span::raw(" "));
    }
    f.render_widget(
        Paragraph::new(Line::from(ks)).block(block(t, Some("Day type"))),
        kind_a,
    );

    let header = Row::new(
        ["", "START", "END", "GROSS", "PROJECT", "COMMENT"].map(|h| {
            Cell::from(Span::styled(
                h,
                Style::default().fg(t.muted).add_modifier(Modifier::BOLD),
            ))
        }),
    );
    let rows: Vec<Row> = data
        .day
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let color = data
                .projects
                .iter()
                .find(|p| p.name == e.project)
                .map(|p| t.project_color(p.color_index))
                .unwrap_or(t.text);
            let mut r = Row::new(vec![
                Cell::from(if i == cursor { "▶" } else { " " }),
                Cell::from(e.start.format("%H:%M").to_string()),
                Cell::from(e.end.format("%H:%M").to_string()),
                Cell::from(e.duration().fmt_signed(fmt)),
                Cell::from(Span::styled(e.project.clone(), Style::default().fg(color))),
                Cell::from(Span::styled(
                    e.comment.clone(),
                    Style::default().fg(t.muted),
                )),
            ]);
            if i == cursor {
                r = r.style(Style::default().bg(t.bg_selected));
            }
            r
        })
        .collect();
    let empty_hint = if data.day.entries.is_empty() {
        Some("no entries — press a to add")
    } else {
        None
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(18),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(block(t, Some("Entries")));
    f.render_widget(table, table_a);
    if let Some(h) = empty_hint {
        let inner = Rect {
            x: table_a.x + 2,
            y: table_a.y + 2,
            width: table_a.width.saturating_sub(4),
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Span::styled(h, Style::default().fg(t.muted))),
            inner,
        );
    }

    let mut foot_line = vec![
        Span::styled("gross ", Style::default().fg(t.muted)),
        Span::raw(stats.gross.fmt_signed(fmt)),
        Span::styled("   break ", Style::default().fg(t.muted)),
        Span::raw(format!("-{}", stats.deduction.fmt_unsigned(fmt))),
        Span::styled("   net ", Style::default().fg(t.muted)),
        minutes_span(stats.net, fmt, t),
        Span::styled("   target ", Style::default().fg(t.muted)),
        Span::raw(format!("-{}", stats.target.fmt_unsigned(fmt))),
        Span::styled("   day ", Style::default().fg(t.muted)),
        minutes_span(stats.balance, fmt, t),
    ];
    if data.day.kind != DayKind::Work {
        foot_line.insert(
            0,
            chip(
                &data.day.kind.display_name().to_uppercase(),
                t.kind_color(&data.day.kind),
            ),
        );
        foot_line.insert(1, Span::raw("  "));
    }
    f.render_widget(
        Paragraph::new(Line::from(foot_line)).block(block(t, None)),
        foot,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        Day, DayKind, Entry, HolidayCalendar, HoursFormat, Minutes, Rules, TodayCtx, day_stats,
        default_tiers,
    };
    use crate::tui::msg::DayData;
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use chrono::{NaiveDate, NaiveTime};

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn rules(date: NaiveDate) -> Rules {
        Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            break_gap: Minutes(30),
            start_date: date,
            initial_balance: Minutes::ZERO,
        }
    }

    /// Render a work day of `spans` and return its rows.
    fn day_rows(date: NaiveDate, spans: &[(NaiveTime, NaiveTime)]) -> Vec<String> {
        let day = Day {
            date,
            kind: DayKind::Work,
            entries: spans
                .iter()
                .enumerate()
                .map(|(i, (s, e))| Entry {
                    id: i as i64 + 1,
                    date,
                    start: *s,
                    end: *e,
                    project: "Alpha".into(),
                    comment: String::new(),
                })
                .collect(),
        };
        let stats = day_stats(
            &day,
            &rules(date),
            &HolidayCalendar::default(),
            &TodayCtx {
                today: date.succ_opt().unwrap(),
                clocked_in: false,
            },
        );
        let data = DayData {
            day,
            projects: vec![],
        };
        render(100, 24, |f| {
            draw_day(
                f,
                f.area(),
                &Theme::dark(),
                &data,
                &stats,
                0,
                &KIND_CYCLE,
                0,
                HoursFormat::Hm,
            )
        })
    }

    #[test]
    fn the_footer_break_follows_the_sessions_of_the_day() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
        // 12:00 → 12:45 splits the day in two: 4:00 and 4:15, each over three
        // hours, so each loses 18 minutes.
        let rows = day_rows(date, &[(t(8, 0), t(12, 0)), (t(12, 45), t(17, 0))]);
        assert!(contains(&rows, "gross +08:15"), "{}", rows.join("\n"));
        assert!(contains(&rows, "break -00:36"), "{}", rows.join("\n"));
        assert!(contains(&rows, "net +07:39"), "{}", rows.join("\n"));
        // Straight on at noon — a project switch, not a break — and the tier applies.
        let rows = day_rows(date, &[(t(8, 0), t(12, 0)), (t(12, 0), t(17, 0))]);
        assert!(contains(&rows, "gross +09:00"), "{}", rows.join("\n"));
        assert!(contains(&rows, "break -00:48"), "{}", rows.join("\n"));
        assert!(contains(&rows, "net +08:12"), "{}", rows.join("\n"));
    }

    /// The entry's gross cell and every figure in the footer follow the setting.
    #[test]
    fn renders_the_day_in_decimal_hours() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
        let day = Day {
            date,
            kind: DayKind::Work,
            entries: vec![Entry {
                id: 1,
                date,
                start: t(8, 0),
                end: t(17, 0),
                project: "Alpha".into(),
                comment: String::new(),
            }],
        };
        let stats = day_stats(
            &day,
            &rules(date),
            &HolidayCalendar::default(),
            &TodayCtx {
                today: date.succ_opt().unwrap(),
                clocked_in: false,
            },
        );
        let data = DayData {
            day,
            projects: vec![],
        };
        let rows = render(100, 24, |f| {
            draw_day(
                f,
                f.area(),
                &Theme::dark(),
                &data,
                &stats,
                0,
                &KIND_CYCLE,
                0,
                HoursFormat::Decimal,
            )
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "+9.00h"), "the entry's gross:\n{joined}");
        assert!(contains(&rows, "gross +9.00h"), "{joined}");
        assert!(contains(&rows, "break -0.80h"), "{joined}");
        assert!(contains(&rows, "net +8.20h"), "{joined}");
        assert!(contains(&rows, "target -7.80h"), "{joined}");
        assert!(contains(&rows, "day +0.40h"), "{joined}");
        assert!(!contains(&rows, "09:00"), "an h:mm leak:\n{joined}");
    }

    #[test]
    fn renders_entries_kind_selector_and_footer() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
        let day = Day {
            date,
            kind: DayKind::Work,
            entries: vec![
                Entry {
                    id: 1,
                    date,
                    start: t(9, 0),
                    end: t(12, 0),
                    project: "Alpha".into(),
                    comment: "morning".into(),
                },
                Entry {
                    id: 2,
                    date,
                    start: t(13, 0),
                    end: t(16, 0),
                    project: "Beta".into(),
                    comment: "".into(),
                },
            ],
        };
        let stats = day_stats(
            &day,
            &rules(date),
            &HolidayCalendar::default(),
            &TodayCtx {
                today: date.succ_opt().unwrap(),
                clocked_in: false,
            },
        );
        let data = DayData {
            day,
            projects: vec![],
        };
        let rows = render(100, 24, |f| {
            draw_day(
                f,
                f.area(),
                &Theme::dark(),
                &data,
                &stats,
                1,
                &KIND_CYCLE,
                0,
                HoursFormat::Hm,
            )
        });
        assert!(contains(&rows, "Monday, 14 September 2026"));
        assert!(contains(&rows, "[work]"));
        assert!(contains(&rows, "vacation"));
        assert!(contains(&rows, "Alpha"));
        assert!(rows.iter().any(|r| r.contains('▶') && r.contains("Beta")));
        assert!(contains(&rows, "gross +06:00"));
        // 12:00–13:00 splits the day into two three-hour sessions, and three
        // hours exactly is under the first tier: nothing is deducted.
        assert!(contains(&rows, "break -00:00"));
        assert!(contains(&rows, "net +06:00"));
    }
}
