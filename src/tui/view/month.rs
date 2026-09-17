//! Month screen drawing: table rows (entries, kind chips, weekend collapse,
//! missing markers, week footers) plus a project/summary panel below.

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Weekday};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Cell, Paragraph, Row as TRow, Table};

use super::{bar, block, chip, kind_label, minutes_span};
use crate::core::{
    DayKind, DayStats, HolidayCalendar, HoursFormat, Minutes, Rules, TodayCtx, day_stats,
};
use crate::tui::model::Model;
use crate::tui::msg::MonthData;
use crate::tui::theme::Theme;

/// What a single table row of the month view represents.
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    /// One time entry of a work day; `first` marks the entry that carries the
    /// day label (subsequent entries of the same day leave that cell empty).
    /// Every entry row shows its own gross and its own net.
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
    /// (project name, net minutes worked, color index), sorted by minutes desc.
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
        for (idx, e) in day.entries.iter().enumerate() {
            *totals.entry(e.project.clone()).or_insert(Minutes::ZERO) += s.entry_nets[idx];
        }
        if day.kind != DayKind::Work {
            *counts.entry(day.kind.as_str()).or_insert(0) += 1;
        }
        if s.missing {
            *counts.entry("missing").or_insert(0) += 1;
        }

        let is_sat = day.date.weekday() == Weekday::Sat;
        // Only collapse Sat+Sun into one banner row when BOTH are plain,
        // entry-free work days; a special kind (vacation/flex/holiday/sick/
        // absence) on either day always gets its own kind-chip row (spec
        // §9.1: every special-kind day gets one row with a colored chip).
        let next_is_sun_work_empty = is_sat
            && data.days.get(i + 1).is_some_and(|n| {
                n.date.weekday() == Weekday::Sun && n.kind == DayKind::Work && n.entries.is_empty()
            });
        if is_sat && day.entries.is_empty() && day.kind == DayKind::Work && next_is_sun_work_empty {
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

    // Project totals are net: every entry carries its share of its session's
    // break deduction, so the hours on the projects add up to the month's net.
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
/// week-footer; a *collapsed* weekend row (spanning both Saturday and
/// Sunday) is also selected for the following Sunday. A lone weekend row
/// (Saturday or Sunday processed on its own, e.g. because the other day of
/// the pair has a special kind or entries) only matches its own date: we
/// detect "collapsed" by checking that no other row in the view already
/// owns the following day.
fn row_is_selected(v: &MonthView, r: &Row, selected: NaiveDate) -> bool {
    match r.kind {
        RowKind::WeekFooter { .. } => false,
        RowKind::Weekend => {
            if r.date == selected {
                return true;
            }
            r.date.succ_opt() == Some(selected)
                && !v.rows.iter().any(|other| {
                    other.date == selected && !matches!(other.kind, RowKind::WeekFooter { .. })
                })
        }
        _ => r.date == selected,
    }
}

const DAY_W: u16 = 9;
/// Width of the GROSS and NET columns — wide enough for `+100.00h`.
const DUR_W: u16 = 9;

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
        m.anchor.map(|a| (a.min(m.selected), a.max(m.selected))),
        m.today,
        area.width >= 90,
        m.hours,
    );
    draw_summary(f, summary_a, &m.theme, &v, m.hours);
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
    // The day-type keys' range, anchor and cursor in date order: every day row
    // inside it is drawn as selected.
    range: Option<(NaiveDate, NaiveDate)>,
    today: NaiveDate,
    show_comment: bool,
    fmt: HoursFormat,
) {
    let inner_w = area.width.saturating_sub(2);
    // GROSS and NET are 9 wide: `+100.00h` is the longest a cell can get.
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(DAY_W),
        Constraint::Length(16),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(DUR_W),
        Constraint::Length(DUR_W),
    ];
    if show_comment {
        widths.push(Constraint::Min(10));
    }
    let fixed: u16 = DAY_W + 16 + 7 + 7 + DUR_W + DUR_W + 6;
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
            let sel = row_is_selected(v, r, selected);
            let in_range = range.is_some_and(|(from, to)| {
                !matches!(r.kind, RowKind::WeekFooter { .. }) && r.date >= from && r.date <= to
            });
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
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        )),
                        Cell::from(Span::styled(
                            e.start.format("%H:%M").to_string(),
                            Style::default().fg(t.text),
                        )),
                        Cell::from(Span::styled(
                            e.end.format("%H:%M").to_string(),
                            Style::default().fg(t.text),
                        )),
                        Cell::from(Span::styled(
                            e.duration().fmt_signed(fmt),
                            Style::default().fg(t.text),
                        )),
                        Cell::from(minutes_span(s.entry_nets[*entry_idx], fmt, t)),
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
                        DayKind::Flex => (-s.target).fmt_signed(fmt),
                        _ => String::new(),
                    };
                    banner(Line::from(vec![
                        kind_label(&label, t.kind_color(&s.kind)),
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
                        minutes_span(*week_balance, fmt, t),
                        Span::styled("  →  ", Style::default().fg(t.muted)),
                        minutes_span(*running, fmt, t),
                    ]);
                    vec![empty(), Cell::from(text).column_span(span)]
                }
            };
            let mut row = TRow::new(cells);
            if sel || in_range {
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

pub fn draw_summary(f: &mut Frame, area: Rect, t: &Theme, v: &MonthView, fmt: HoursFormat) {
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
                m.fmt_unsigned(fmt),
                frac * 100.0
            )));
            Line::from(l)
        })
        .collect();
    lines.push(Line::from(vec![
        Span::styled("target ", Style::default().fg(t.muted)),
        Span::raw(format!("-{}", v.target.fmt_unsigned(fmt))),
        Span::styled("   net ", Style::default().fg(t.muted)),
        Span::raw(format!("+{}", v.net.fmt_unsigned(fmt))),
        Span::styled("   month ", Style::default().fg(t.muted)),
        minutes_span(v.balance, fmt, t),
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
    use crate::tui::view::testing::{contains, render, style_of};
    use chrono::{NaiveDate, NaiveTime};
    use tuirealm::ratatui::style::Color;

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
                    break_share: None,
                }],
                14 => vec![
                    Entry {
                        id: 2,
                        date: cur,
                        start: t(9, 0),
                        end: t(12, 0),
                        project: "Alpha".into(),
                        comment: "morning".into(),
                        break_share: None,
                    },
                    Entry {
                        id: 3,
                        date: cur,
                        start: t(13, 0),
                        end: t(16, 0),
                        project: "Alpha".into(),
                        comment: "afternoon".into(),
                        break_share: None,
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
            last_used_project: Some("Beta".into()),
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
        // totals: net = 4h-18 + 6h = 222 + 360 = 582 — the 14th runs 9-12 and
        // 13-16, two sessions of exactly three hours, and three hours is under
        // the first tier, so neither is charged; target = weekdays 1..15
        // with target: 1,3,4,7,8,9,10,11,14 (15=today, no entries -> no target);
        // day2 vacation and day17 holiday carry no target.
        assert_eq!(v.net, Minutes(582));
        assert_eq!(v.target, Minutes(468 * 9));
        // Project totals are net: Alpha's two three-hour sessions lose nothing
        // and keep 360, Beta's single four-hour session loses 18 and keeps 222.
        assert_eq!(v.project_totals[0].0, "Alpha");
        assert_eq!(v.project_totals[0].1, Minutes(360));
        assert_eq!(v.project_totals[1], ("Beta".to_string(), Minutes(222), 1));
        // Which is the same time the summary's `net` is made of.
        assert_eq!(v.project_totals.iter().map(|p| p.1).sum::<Minutes>(), v.net);
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
    fn special_kind_on_sunday_gets_its_own_row() {
        // Sat 2026-09-05 stays a plain work day; Sun 2026-09-06 becomes a
        // Vacation day with no entries. They must NOT collapse into one
        // "weekend" banner: the Sunday keeps its own kind-chip row.
        let (mut data, rules, cal) = fixture();
        let sun = data
            .days
            .iter_mut()
            .find(|dd| dd.date == d(2026, 9, 6))
            .unwrap();
        sun.kind = DayKind::Vacation;
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));

        let sat_idx = row_index_of(&v, d(2026, 9, 5)).unwrap();
        let sun_idx = row_index_of(&v, d(2026, 9, 6)).unwrap();
        assert!(matches!(v.rows[sat_idx].kind, RowKind::Weekend));
        assert!(matches!(v.rows[sun_idx].kind, RowKind::Kind));
        assert_eq!(sun_idx, sat_idx + 1);
        // The week-36 footer still follows right after the Sunday row.
        assert!(matches!(
            v.rows[sun_idx + 1].kind,
            RowKind::WeekFooter { .. }
        ));
        if let RowKind::WeekFooter { week, .. } = v.rows[sun_idx + 1].kind {
            assert_eq!(week, 36);
        }

        // kind_counts counts both vacation days: Wed 2 and now Sun 6.
        assert!(
            v.kind_counts
                .iter()
                .any(|(k, n)| k == "vacation" && *n == 2)
        );

        // The rendered Sunday row shows its own VACATION chip.
        let t = Theme::dark();
        let rows = render(100, 40, |f| {
            draw_table(
                f,
                f.area(),
                &t,
                &v,
                &data,
                d(2026, 9, 6),
                None,
                d(2026, 9, 15),
                true,
                HoursFormat::Hm,
            )
        });
        assert!(contains(&rows, "Sun 06"));
        assert!(contains(&rows, "VACATION"));

        // The other three, still-plain weekends each collapse into exactly
        // one Weekend row (no regression from the fix); combined with the
        // now-lone Saturday Weekend row that's 4 Weekend rows total, same
        // as the unmodified fixture.
        let weekend_rows = v
            .rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Weekend))
            .count();
        assert_eq!(weekend_rows, 4);

        // The unmodified fixture (all weekends plain) still collapses every
        // Sat+Sun pair into exactly one Weekend row per weekend (4 weekends
        // in September 2026).
        let (plain_data, rules2, cal2) = fixture();
        let plain_v = build_month_view(&plain_data, &rules2, &cal2, d(2026, 9, 15));
        let plain_weekend_rows = plain_v
            .rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Weekend))
            .count();
        assert_eq!(plain_weekend_rows, 4);
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
                None,
                d(2026, 9, 15),
                true,
                HoursFormat::Hm,
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

    /// Every duration in the table and the summary follows the display setting.
    #[test]
    fn renders_table_and_summary_in_decimal_hours() {
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
                None,
                d(2026, 9, 15),
                true,
                HoursFormat::Decimal,
            )
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "+4.00h"), "the 4h entry's gross:\n{joined}");
        assert!(
            contains(&rows, "+3.70h"),
            "its net after the tier:\n{joined}"
        );
        // The flex row spells out the target it gives back, and the week footers
        // carry the week and running balances.
        assert!(contains(&rows, "-7.80h"), "the flex row:\n{joined}");
        assert!(contains(&rows, "-19.70h"), "the KW 36 footer:\n{joined}");
        assert!(!contains(&rows, "+03:42"), "an h:mm leak:\n{joined}");

        let rows = render(100, 8, |f| {
            draw_summary(f, f.area(), &t, &v, HoursFormat::Decimal)
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "-70.20h"), "the month target:\n{joined}");
        assert!(contains(&rows, "+9.70h"), "the month net:\n{joined}");
        assert!(contains(&rows, "-60.50h"), "the month balance:\n{joined}");
        // Project hours are unsigned, so they lose the sign but not the unit.
        assert!(contains(&rows, "6.00h"), "Alpha's hours:\n{joined}");
        assert!(!contains(&rows, "09:42"), "an h:mm leak:\n{joined}");
    }

    /// A day worked straight through on two projects: the session's deduction is
    /// shared out, so each entry row carries its own net and the projects carry
    /// net time.
    #[test]
    fn every_entry_row_shows_its_own_net() {
        let (mut data, rules, cal) = fixture();
        let day = data
            .days
            .iter_mut()
            .find(|dd| dd.date == d(2026, 9, 14))
            .unwrap();
        day.entries = vec![
            Entry {
                id: 2,
                date: day.date,
                start: t(8, 0),
                end: t(12, 0),
                project: "Alpha".into(),
                comment: "morning".into(),
                break_share: None,
            },
            Entry {
                id: 3,
                date: day.date,
                start: t(12, 0),
                end: t(17, 0),
                project: "Beta".into(),
                comment: "afternoon".into(),
                break_share: None,
            },
        ];
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        // One nine-hour session losing 48, all of it on the project worked
        // last: nothing off Alpha's morning, 48 off Beta's afternoon. Beta also
        // holds the 1st's 222, so it leads.
        assert_eq!(v.project_totals[0], ("Beta".to_string(), Minutes(474), 1));
        assert_eq!(v.project_totals[1], ("Alpha".to_string(), Minutes(240), 0));
        assert_eq!(v.project_totals.iter().map(|p| p.1).sum::<Minutes>(), v.net);

        let th = Theme::dark();
        let rows = render(100, 30, |f| {
            draw_table(
                f,
                f.area(),
                &th,
                &v,
                &data,
                d(2026, 9, 14),
                None,
                d(2026, 9, 15),
                true,
                HoursFormat::Hm,
            )
        });
        let joined = rows.join("\n");
        let morning = rows
            .iter()
            .find(|r| r.contains("morning"))
            .unwrap_or_else(|| panic!("{joined}"));
        assert!(
            morning.contains("+04:00"),
            "its gross and its net:\n{joined}"
        );
        assert!(!morning.contains("+03:"), "nothing came off it:\n{joined}");
        let afternoon = rows
            .iter()
            .find(|r| r.contains("afternoon"))
            .unwrap_or_else(|| panic!("{joined}"));
        assert!(afternoon.contains("+05:00"), "its gross:\n{joined}");
        assert!(afternoon.contains("+04:12"), "its net:\n{joined}");
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
                None,
                d(2026, 9, 15),
                false,
                HoursFormat::Hm,
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
        let rows = render(100, 8, |f| {
            draw_summary(f, f.area(), &t, &v, HoursFormat::Hm)
        });
        assert!(contains(&rows, "Alpha"));
        assert!(contains(&rows, "█"));
        // Alpha's 360 net minutes of the month's 582.
        assert!(contains(&rows, "61.9%"));
        assert!(contains(&rows, "target"));
        assert!(contains(&rows, "+09:42"));
        assert!(contains(&rows, "vacation 1"));
    }
    /// A day off is information, not an alarm: its label is plain text in the
    /// kind's color, no filled chip. "missing" keeps its chip, that one is a
    /// warning.
    #[test]
    fn non_work_kinds_lose_their_chip_but_missing_keeps_it() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let draw = |f: &mut Frame| {
            draw_table(
                f,
                f.area(),
                &t,
                &v,
                &data,
                d(2026, 9, 14),
                None,
                d(2026, 9, 15),
                true,
                HoursFormat::Hm,
            )
        };
        let flex = style_of(100, 40, draw, "FLEX");
        assert_eq!(flex.fg, Some(t.chip_flex));
        assert!(matches!(flex.bg, None | Some(Color::Reset)), "{flex:?}");
        assert!(!flex.add_modifier.contains(Modifier::BOLD));
        let vacation = style_of(100, 40, draw, "VACATION");
        assert_eq!(vacation.fg, Some(t.chip_vacation));
        assert!(matches!(vacation.bg, None | Some(Color::Reset)));
        let missing = style_of(100, 40, draw, "missing");
        assert_eq!(missing.bg, Some(t.negative));
    }

    /// The project name is what a work day is about, so it carries the weight.
    #[test]
    fn work_day_project_name_is_bold_in_its_project_color() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let beta = style_of(
            100,
            40,
            |f| {
                draw_table(
                    f,
                    f.area(),
                    &t,
                    &v,
                    &data,
                    d(2026, 9, 14),
                    None,
                    d(2026, 9, 15),
                    true,
                    HoursFormat::Hm,
                )
            },
            "Beta",
        );
        assert_eq!(beta.fg, Some(t.project_color(1)));
        assert!(beta.add_modifier.contains(Modifier::BOLD));
    }

    /// Start and end times are drawn in the theme's text color rather than
    /// whatever the terminal defaults to.
    #[test]
    fn work_day_times_use_the_text_color() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let start = style_of(
            100,
            40,
            |f| {
                draw_table(
                    f,
                    f.area(),
                    &t,
                    &v,
                    &data,
                    d(2026, 9, 14),
                    None,
                    d(2026, 9, 15),
                    true,
                    HoursFormat::Hm,
                )
            },
            "08:00",
        );
        assert_eq!(start.fg, Some(t.text));
    }

    // --- range marking ---

    #[test]
    fn rows_between_anchor_and_cursor_share_the_selection_background() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let draw = |f: &mut Frame| {
            draw_table(
                f,
                f.area(),
                &t,
                &v,
                &data,
                d(2026, 9, 10),
                Some((d(2026, 9, 8), d(2026, 9, 10))),
                d(2026, 9, 15),
                true,
                HoursFormat::Hm,
            )
        };
        assert_eq!(style_of(100, 40, draw, "Tue 08").bg, Some(t.bg_selected));
        assert_eq!(style_of(100, 40, draw, "Wed 09").bg, Some(t.bg_selected));
        assert_eq!(style_of(100, 40, draw, "Thu 10").bg, Some(t.bg_selected));
        assert_ne!(style_of(100, 40, draw, "Fri 11").bg, Some(t.bg_selected));
        assert_ne!(style_of(100, 40, draw, "Mon 07").bg, Some(t.bg_selected));
        let rows = render(100, 40, draw);
        assert!(
            rows.iter().any(|r| r.contains('▶') && r.contains("Thu 10")),
            "cursor keeps its mark"
        );
        assert!(!rows.iter().any(|r| r.contains('▶') && r.contains("Tue 08")));
    }

    /// A week footer sits inside the range's dates but is no day: it must stay
    /// plain, or the highlight would look like it ran on past the cursor.
    #[test]
    fn a_week_footer_inside_the_range_stays_plain() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let draw = |f: &mut Frame| {
            draw_table(
                f,
                f.area(),
                &t,
                &v,
                &data,
                d(2026, 9, 8),
                Some((d(2026, 9, 3), d(2026, 9, 8))),
                d(2026, 9, 15),
                true,
                HoursFormat::Hm,
            )
        };
        assert_eq!(style_of(100, 40, draw, "Thu 03").bg, Some(t.bg_selected));
        assert_ne!(style_of(100, 40, draw, "KW 36").bg, Some(t.bg_selected));
    }

    /// The whole way from the key to the screen: what `V` anchored is what the
    /// month screen lights up.
    #[test]
    fn the_anchor_the_model_holds_is_what_the_month_screen_lights_up() {
        let (data, ..) = fixture();
        let (mut m, _rx) = crate::tui::model::testing::model(d(2026, 9, 15));
        m.selected = d(2026, 9, 8);
        m.month = Some(data);
        m.update(crate::tui::msg::Msg::ToggleAnchor);
        m.update(crate::tui::msg::Msg::SelectDay(2));
        let t = m.theme.clone();
        let draw = |f: &mut Frame| draw(&m, f, f.area());
        assert_eq!(style_of(100, 40, draw, "Tue 08").bg, Some(t.bg_selected));
        assert_eq!(style_of(100, 40, draw, "Wed 09").bg, Some(t.bg_selected));
        assert_eq!(style_of(100, 40, draw, "Thu 10").bg, Some(t.bg_selected));
        assert_ne!(style_of(100, 40, draw, "Fri 11").bg, Some(t.bg_selected));
    }
}
