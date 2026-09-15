//! Month screen drawing: table rows (entries, kind chips, weekend collapse,
//! missing markers, week footers) plus a project/summary panel below.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Weekday};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Cell, Paragraph, Row as TRow, Table};

use super::{bar, block, chip, minutes_span};
use crate::core::{DayKind, DayStats, HolidayCalendar, Minutes, Rules, TodayCtx, day_stats};
use crate::tui::model::Model;
use crate::tui::msg::MonthData;
use crate::tui::theme::Theme;

/// What a single table row of the month view represents.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    /// One time entry of a work day; `first` marks the entry that carries the
    /// day label / net-of-day cell (subsequent entries of the same day only
    /// show their own start/end/gross).
    Entry { entry_idx: usize, first: bool },
    /// A past weekday with no entries.
    Missing,
    /// A future (or otherwise inapplicable) weekday with nothing to show yet.
    Empty,
    /// A non-work day kind (vacation / flex / holiday / sick / absence).
    Kind,
    /// A Saturday+Sunday pair collapsed into a single "weekend" row.
    Weekend,
    /// Trailing per-ISO-week summary row.
    WeekFooter {
        week: u32,
        week_balance: Minutes,
        running: Minutes,
    },
}

#[derive(Debug, Clone)]
pub struct Row {
    pub date: NaiveDate,
    pub kind: RowKind,
    pub stats: Option<DayStats>,
}

#[derive(Debug, Clone)]
pub struct MonthView {
    pub rows: Vec<Row>,
    pub target: Minutes,
    pub net: Minutes,
    pub balance: Minutes,
    /// (project name, gross minutes worked, color index), sorted by minutes desc.
    pub project_totals: Vec<(String, Minutes, u8)>,
    /// (label, count) e.g. ("vacation", 1) or ("missing", 2).
    pub kind_counts: Vec<(String, u32)>,
}

pub fn build_month_view(
    data: &MonthData,
    rules: &Rules,
    cal: &HolidayCalendar,
    today: NaiveDate,
) -> MonthView {
    let ctx = TodayCtx {
        today,
        clocked_in: data.session.is_some(),
    };
    let mut rows = Vec::new();
    let (mut target, mut net) = (Minutes::ZERO, Minutes::ZERO);
    let mut running = data.balance_before;
    let mut week_balance = Minutes::ZERO;
    let mut totals: BTreeMap<String, Minutes> = BTreeMap::new();
    let mut counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let last = data.days.last().map(|d| d.date);

    let mut i = 0;
    while i < data.days.len() {
        let day = &data.days[i];
        let s = day_stats(day, rules, cal, &ctx);
        target += s.target;
        net += s.net;
        if day.date >= rules.start_date {
            running += s.balance;
            week_balance += s.balance;
        }
        for e in &day.entries {
            *totals.entry(e.project.clone()).or_insert(Minutes::ZERO) += e.duration();
        }
        if day.kind != DayKind::Work {
            *counts.entry(day.kind.as_str()).or_insert(0) += 1;
        }
        if s.missing {
            *counts.entry("missing").or_insert(0) += 1;
        }

        let is_sat = day.date.weekday() == Weekday::Sat;
        let next_is_sun_empty = is_sat
            && data
                .days
                .get(i + 1)
                .is_some_and(|n| n.date.weekday() == Weekday::Sun && n.entries.is_empty());
        if is_sat && day.entries.is_empty() && day.kind == DayKind::Work && next_is_sun_empty {
            let sun = &data.days[i + 1];
            let ss = day_stats(sun, rules, cal, &ctx);
            rows.push(Row {
                date: day.date,
                kind: RowKind::Weekend,
                stats: Some(s),
            });
            if sun.date >= rules.start_date {
                running += ss.balance;
                week_balance += ss.balance;
            }
            push_footer(&mut rows, sun.date, &mut week_balance, running);
            i += 2;
            continue;
        }

        if !day.entries.is_empty() {
            for (idx, _) in day.entries.iter().enumerate() {
                rows.push(Row {
                    date: day.date,
                    kind: RowKind::Entry {
                        entry_idx: idx,
                        first: idx == 0,
                    },
                    stats: Some(s.clone()),
                });
            }
        } else if day.kind != DayKind::Work {
            rows.push(Row {
                date: day.date,
                kind: RowKind::Kind,
                stats: Some(s.clone()),
            });
        } else if s.is_weekend {
            rows.push(Row {
                date: day.date,
                kind: RowKind::Weekend,
                stats: Some(s.clone()),
            });
        } else if s.missing {
            rows.push(Row {
                date: day.date,
                kind: RowKind::Missing,
                stats: Some(s.clone()),
            });
        } else {
            rows.push(Row {
                date: day.date,
                kind: RowKind::Empty,
                stats: Some(s.clone()),
            });
        }

        if day.date.weekday() == Weekday::Sun || Some(day.date) == last {
            push_footer(&mut rows, day.date, &mut week_balance, running);
        }
        i += 1;
    }

    // Project totals use gross entry durations ("hours per project").
    let mut project_totals: Vec<(String, Minutes, u8)> = totals
        .into_iter()
        .map(|(name, m)| {
            let idx = data
                .projects
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.color_index)
                .unwrap_or(0);
            (name, m, idx)
        })
        .collect();
    project_totals.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    MonthView {
        rows,
        target,
        net,
        balance: net - target,
        project_totals,
        kind_counts: counts
            .into_iter()
            .map(|(k, n)| (k.to_string(), n))
            .collect(),
    }
}

fn push_footer(rows: &mut Vec<Row>, date: NaiveDate, week_balance: &mut Minutes, running: Minutes) {
    let week = date.iso_week().week();
    rows.push(Row {
        date,
        kind: RowKind::WeekFooter {
            week,
            week_balance: *week_balance,
            running,
        },
        stats: None,
    });
    *week_balance = Minutes::ZERO;
}

pub fn row_index_of(v: &MonthView, date: NaiveDate) -> Option<usize> {
    v.rows
        .iter()
        .position(|r| r.date == date && !matches!(r.kind, RowKind::WeekFooter { .. }))
        .or_else(|| {
            v.rows.iter().position(|r| {
                matches!(r.kind, RowKind::Weekend)
                    && (r.date == date || r.date.succ_opt() == Some(date))
            })
        })
}

/// A row is selected iff it belongs to the selected date and isn't a
/// week-footer; a collapsed weekend row is selected for either its Saturday
/// or the following Sunday.
fn row_is_selected(r: &Row, selected: NaiveDate) -> bool {
    match r.kind {
        RowKind::WeekFooter { .. } => false,
        RowKind::Weekend => r.date == selected || r.date.succ_opt() == Some(selected),
        _ => r.date == selected,
    }
}

const DAY_W: u16 = 9;

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let Some(data) = &m.month else {
        f.render_widget(
            Paragraph::new("loading…").style(Style::default().fg(m.theme.muted)),
            area,
        );
        return;
    };
    let v = build_month_view(data, &m.rules, &m.cal, m.today);
    let stacked = area.width < 100;
    let summary_h = (v.project_totals.len() as u16 + 4).max(5);
    let [table_a, summary_a] = if stacked {
        Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(summary_h.min((area.height / 3).max(1))),
        ])
        .areas(area)
    } else {
        Layout::vertical([Constraint::Min(8), Constraint::Length(summary_h)]).areas(area)
    };
    draw_table(
        f,
        table_a,
        &m.theme,
        &v,
        data,
        m.selected,
        m.today,
        area.width >= 90,
    );
    draw_summary(f, summary_a, &m.theme, &v);
}

fn day_label(date: NaiveDate) -> String {
    format!("{} {:02}", date.format("%a"), date.day())
}

#[allow(clippy::too_many_arguments)]
pub fn draw_table(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    v: &MonthView,
    data: &MonthData,
    selected: NaiveDate,
    today: NaiveDate,
    show_comment: bool,
) {
    let inner_w = area.width.saturating_sub(2);
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(DAY_W),
        Constraint::Length(16),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(8),
    ];
    if show_comment {
        widths.push(Constraint::Min(10));
    }
    let fixed: u16 = DAY_W + 16 + 7 + 7 + 8 + 8 + 6;
    let comment_w = inner_w.saturating_sub(fixed) as usize;
    let span = (widths.len() as u16).saturating_sub(1).max(1);

    let header = TRow::new(
        ["DAY", "PROJECT", "START", "END", "GROSS", "NET", "COMMENT"]
            .iter()
            .take(widths.len())
            .map(|h| {
                Cell::from(Span::styled(
                    *h,
                    Style::default().fg(t.muted).add_modifier(Modifier::BOLD),
                ))
            }),
    );

    let rows: Vec<TRow> = v
        .rows
        .iter()
        .map(|r| {
            let sel = row_is_selected(r, selected);
            let is_today = r.date == today;
            let mark = if sel { "▶" } else { " " };
            let day_style = if is_today {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else if matches!(r.kind, RowKind::Missing) {
                Style::default().fg(t.negative)
            } else {
                Style::default().fg(t.text)
            };
            let day_cell = |show: bool| {
                Cell::from(Line::from(vec![
                    Span::styled(mark, Style::default().fg(t.accent)),
                    Span::styled(
                        if show {
                            day_label(r.date)
                        } else {
                            String::new()
                        },
                        day_style,
                    ),
                ]))
            };
            let empty = || Cell::from("");
            let banner = |text: Line<'static>| -> Vec<Cell> {
                vec![day_cell(true), Cell::from(text).column_span(span)]
            };
            let cells: Vec<Cell> = match &r.kind {
                RowKind::Entry { entry_idx, first } => {
                    let e = &data
                        .days
                        .iter()
                        .find(|d| d.date == r.date)
                        .expect("row date always matches a loaded day")
                        .entries[*entry_idx];
                    let color = data
                        .projects
                        .iter()
                        .find(|p| p.name == e.project)
                        .map(|p| t.project_color(p.color_index))
                        .unwrap_or(t.text);
                    let s = r.stats.as_ref().expect("entry rows carry stats");
                    let mut c = vec![
                        day_cell(*first),
                        Cell::from(Span::styled(
                            truncate(&e.project, 15),
                            Style::default().fg(color),
                        )),
                        Cell::from(e.start.format("%H:%M").to_string()),
                        Cell::from(e.end.format("%H:%M").to_string()),
                        Cell::from(Span::styled(
                            e.duration().to_string(),
                            Style::default().fg(t.text),
                        )),
                        Cell::from(if *first {
                            minutes_span(s.net, t)
                        } else {
                            Span::raw("")
                        }),
                    ];
                    if show_comment {
                        c.push(Cell::from(Span::styled(
                            truncate(&e.comment, comment_w),
                            Style::default().fg(t.muted),
                        )));
                    }
                    c
                }
                RowKind::Missing => banner(Line::from(chip("missing", t.negative))),
                RowKind::Empty => {
                    banner(Line::from(Span::styled("·", Style::default().fg(t.muted))))
                }
                RowKind::Weekend => banner(Line::from(Span::styled(
                    "weekend",
                    Style::default().fg(t.muted),
                ))),
                RowKind::Kind => {
                    let s = r.stats.as_ref().expect("kind rows carry stats");
                    let label = s.kind.display_name().to_uppercase();
                    let extra = match &s.kind {
                        DayKind::Holiday => s.holiday_name.clone().unwrap_or_default(),
                        DayKind::Flex => format!("{}", -s.target),
                        _ => String::new(),
                    };
                    banner(Line::from(vec![
                        chip(&label, t.kind_color(&s.kind)),
                        Span::raw("  "),
                        Span::styled(extra, Style::default().fg(t.muted)),
                    ]))
                }
                RowKind::WeekFooter {
                    week,
                    week_balance,
                    running,
                } => {
                    let text = Line::from(vec![
                        Span::styled(format!("KW {week:02}  "), Style::default().fg(t.muted)),
                        minutes_span(*week_balance, t),
                        Span::styled("  →  ", Style::default().fg(t.muted)),
                        minutes_span(*running, t),
                    ]);
                    vec![empty(), Cell::from(text).column_span(span)]
                }
            };
            let mut row = TRow::new(cells);
            if sel {
                row = row.style(Style::default().bg(t.bg_selected));
            }
            row
        })
        .collect();

    // Keep the selected row visible: offset so that it stays within the viewport.
    let sel_idx = row_index_of(v, selected);
    let viewport = area.height.saturating_sub(3) as usize;
    let offset = sel_idx
        .map(|s| s.saturating_sub(viewport.saturating_sub(1) / 2))
        .unwrap_or(0)
        .min(v.rows.len().saturating_sub(viewport));
    let visible: Vec<TRow> = rows.into_iter().skip(offset).collect();

    let table = Table::new(visible, widths)
        .header(header)
        .column_spacing(1)
        .block(block(t, None));
    f.render_widget(table, area);
}

pub fn draw_summary(f: &mut Frame, area: Rect, t: &Theme, v: &MonthView) {
    let total: i32 = v.project_totals.iter().map(|p| p.1.0).sum::<i32>().max(1);
    let bar_w = area
        .width
        .saturating_sub(2 + 18 + 2 + 8 + 2 + 8 + 2)
        .max(10);
    let mut lines: Vec<Line> = v
        .project_totals
        .iter()
        .map(|(name, m, idx)| {
            let frac = m.0 as f64 / total as f64;
            let mut l = vec![Span::styled(
                format!("{:<18}", truncate(name, 18)),
                Style::default().fg(t.project_color(*idx)),
            )];
            l.extend(bar(frac, bar_w, t.project_color(*idx), t).spans);
            l.push(Span::raw(format!(
                "  {:>8}  {:>5.1}%",
                m.hhmm(),
                frac * 100.0
            )));
            Line::from(l)
        })
        .collect();
    lines.push(Line::from(vec![
        Span::styled("target ", Style::default().fg(t.muted)),
        Span::raw(format!("-{}", v.target.hhmm())),
        Span::styled("   net ", Style::default().fg(t.muted)),
        Span::raw(format!("+{}", v.net.hhmm())),
        Span::styled("   month ", Style::default().fg(t.muted)),
        minutes_span(v.balance, t),
    ]));
    if !v.kind_counts.is_empty() {
        let counts: Vec<String> = v
            .kind_counts
            .iter()
            .map(|(label, n)| format!("{label} {n}"))
            .collect();
        lines.push(Line::from(Span::styled(
            counts.join("   "),
            Style::default().fg(t.muted),
        )));
    }
    f.render_widget(Paragraph::new(lines).block(block(t, Some("Month"))), area);
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(w.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Day, DayKind, Entry, HolidayCalendar, Project, Rules, default_tiers};
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn fixture() -> (MonthData, Rules, HolidayCalendar) {
        let rules = Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            start_date: d(2026, 9, 1),
            initial_balance: Minutes::ZERO,
        };
        let cal = HolidayCalendar::new(vec![d(2026, 9, 17)]);
        let mut days = Vec::new();
        let mut cur = d(2026, 9, 1);
        while cur <= d(2026, 9, 30) {
            let kind = if cur == d(2026, 9, 17) {
                DayKind::Holiday
            } else if cur == d(2026, 9, 2) {
                DayKind::Vacation
            } else if cur == d(2026, 9, 3) {
                DayKind::Flex
            } else {
                DayKind::Work
            };
            let entries = match cur.day() {
                1 => vec![Entry {
                    id: 1,
                    date: cur,
                    start: t(8, 0),
                    end: t(12, 0),
                    project: "Beta".into(),
                    comment: "half".into(),
                }],
                14 => vec![
                    Entry {
                        id: 2,
                        date: cur,
                        start: t(9, 0),
                        end: t(12, 0),
                        project: "Alpha".into(),
                        comment: "morning".into(),
                    },
                    Entry {
                        id: 3,
                        date: cur,
                        start: t(13, 0),
                        end: t(16, 0),
                        project: "Alpha".into(),
                        comment: "afternoon".into(),
                    },
                ],
                _ => vec![],
            };
            days.push(Day {
                date: cur,
                kind,
                entries,
            });
            cur = cur.succ_opt().unwrap();
        }
        let data = MonthData {
            year: 2026,
            month: 9,
            days,
            balance_before: Minutes::ZERO,
            balance_total: Minutes(-100),
            session: None,
            projects: vec![
                Project {
                    id: 1,
                    name: "Alpha".into(),
                    color_index: 0,
                    archived: false,
                },
                Project {
                    id: 2,
                    name: "Beta".into(),
                    color_index: 1,
                    archived: false,
                },
            ],
            vacation_used_this_year: 1,
        };
        (data, rules, cal)
    }

    #[test]
    fn builds_rows_in_spec_order() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        // Tue 1: one entry row; Wed 2: vacation kind row; Thu 3: flex; Fri 4:
        // missing; Sat/Sun 5-6: one weekend row; then week footer; Mon 7: missing.
        let seq = &v.rows[0..7];
        assert!(matches!(seq[0].kind, RowKind::Entry { .. }));
        assert!(matches!(seq[1].kind, RowKind::Kind));
        assert!(matches!(seq[2].kind, RowKind::Kind));
        assert!(matches!(seq[3].kind, RowKind::Missing));
        assert!(matches!(seq[4].kind, RowKind::Weekend));
        assert!(matches!(seq[5].kind, RowKind::WeekFooter { .. }));
        assert!(matches!(seq[6].kind, RowKind::Missing));
        // Mon 14 has two entry rows
        let idx = row_index_of(&v, d(2026, 9, 14)).unwrap();
        assert!(matches!(
            v.rows[idx].kind,
            RowKind::Entry { first: true, .. }
        ));
        assert!(matches!(
            v.rows[idx + 1].kind,
            RowKind::Entry { first: false, .. }
        ));
        // Thu 17 extra holiday
        let h = row_index_of(&v, d(2026, 9, 17)).unwrap();
        assert!(matches!(v.rows[h].kind, RowKind::Kind));
        // future day 16 is Empty
        let e = row_index_of(&v, d(2026, 9, 16)).unwrap();
        assert!(matches!(v.rows[e].kind, RowKind::Empty));
        // totals: net = 4h-18 + (6h-18) = 222 + 342 = 564; target = weekdays 1..15
        // with target: 1,3,4,7,8,9,10,11,14 (15=today, no entries -> no target);
        // day2 vacation and day17 holiday carry no target.
        assert_eq!(v.net, Minutes(564));
        assert_eq!(v.target, Minutes(468 * 9));
        // Project totals use gross entry durations: Alpha 360, Beta 240.
        assert_eq!(v.project_totals[0].0, "Alpha");
        assert_eq!(v.project_totals[0].1, Minutes(360));
        assert!(
            v.kind_counts
                .iter()
                .any(|(k, n)| k == "vacation" && *n == 1)
        );
    }

    #[test]
    fn week_footer_carries_running_balance() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let footers: Vec<&Row> = v
            .rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::WeekFooter { .. }))
            .collect();
        assert_eq!(footers.len(), 5);
        if let RowKind::WeekFooter {
            week,
            week_balance,
            running,
        } = footers[0].kind
        {
            assert_eq!(week, 36);
            // Tue1 +222-468, Wed2 0, Thu3 -468, Fri4 -468 => -1182
            assert_eq!(week_balance, Minutes(222 - 468 - 468 - 468));
            assert_eq!(running, Minutes(222 - 468 - 468 - 468));
        } else {
            panic!("expected WeekFooter");
        }
    }

    #[test]
    fn renders_table_100x30() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let rows = render(100, 40, |f| {
            draw_table(
                f,
                f.area(),
                &t,
                &v,
                &data,
                d(2026, 9, 14),
                d(2026, 9, 15),
                true,
            )
        });
        assert!(contains(&rows, "Tue 01"));
        assert!(contains(&rows, "Beta"));
        assert!(contains(&rows, "08:00"));
        assert!(contains(&rows, "+03:42")); // net of 4h
        assert!(contains(&rows, "VACATION"));
        assert!(contains(&rows, "FLEX"));
        assert!(contains(&rows, "missing"));
        assert!(contains(&rows, "weekend"));
        assert!(contains(&rows, "KW 36"));
        assert!(contains(&rows, "morning"));
        assert!(contains(&rows, "Extra holiday"));
        assert!(rows.iter().any(|r| r.contains('▶') && r.contains("Mon 14")));
    }

    #[test]
    fn hides_comment_below_90_columns() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let rows = render(85, 40, |f| {
            draw_table(
                f,
                f.area(),
                &t,
                &v,
                &data,
                d(2026, 9, 14),
                d(2026, 9, 15),
                false,
            )
        });
        assert!(!contains(&rows, "morning"));
        assert!(contains(&rows, "Mon 14"));
    }

    #[test]
    fn summary_has_bars_and_totals() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let rows = render(100, 8, |f| draw_summary(f, f.area(), &t, &v));
        assert!(contains(&rows, "Alpha"));
        assert!(contains(&rows, "█"));
        assert!(contains(&rows, "60.0%"));
        assert!(contains(&rows, "target"));
        assert!(contains(&rows, "+09:24"));
        assert!(contains(&rows, "vacation 1"));
    }
}
