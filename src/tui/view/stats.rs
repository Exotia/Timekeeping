//! Statistics screen drawing.

use chrono::{Datelike, NaiveDate};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::Paragraph;

use super::{bar, block, minutes_span};
use crate::core::{DayKind, HolidayCalendar, Minutes, Rules, TodayCtx, day_stats, is_working_day};
use crate::tui::model::Model;
use crate::tui::msg::{RangeKind, StatsData};
use crate::tui::theme::Theme;
use crate::tui::worker::month_range;

pub struct StatsView {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub project_totals: Vec<(String, Minutes, u8)>,
    pub total: Minutes,
    pub net: Minutes,
    pub target: Minutes,
    pub vacation_used: u32,
    pub vacation_allowance: u32,
    pub sick: u32,
    pub flex: u32,
    pub holidays: u32,
    pub absences: u32,
    pub missing: u32,
}

/// The (from, to) date range covered by a given `RangeKind`, relative to `today`.
pub fn range_for(kind: RangeKind, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match kind {
        RangeKind::ThisMonth => month_range(today.year(), today.month()),
        RangeKind::LastMonth => {
            let (y, m) = if today.month() == 1 {
                (today.year() - 1, 12)
            } else {
                (today.year(), today.month() - 1)
            };
            month_range(y, m)
        }
        RangeKind::Quarter => {
            let q0 = (today.month() - 1) / 3 * 3 + 1;
            (
                month_range(today.year(), q0).0,
                month_range(today.year(), q0 + 2).1,
            )
        }
        RangeKind::Year => (
            NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(today.year(), 12, 31).unwrap(),
        ),
    }
}

/// Aggregate raw store data into the view model the screen draws from.
pub fn build_stats(
    data: &StatsData,
    rules: &Rules,
    cal: &HolidayCalendar,
    today: NaiveDate,
    allowance: u32,
) -> StatsView {
    let ctx = TodayCtx {
        today,
        clocked_in: false,
    };
    let mut totals: std::collections::BTreeMap<String, Minutes> = Default::default();
    let mut v = StatsView {
        from: data.from,
        to: data.to,
        project_totals: vec![],
        total: Minutes::ZERO,
        net: Minutes::ZERO,
        target: Minutes::ZERO,
        vacation_used: 0,
        vacation_allowance: allowance,
        sick: 0,
        flex: 0,
        holidays: 0,
        absences: 0,
        missing: 0,
    };
    for day in &data.days {
        let s = day_stats(day, rules, cal, &ctx);
        v.net += s.net;
        v.target += s.target;
        if s.missing {
            v.missing += 1;
        }
        for e in &day.entries {
            *totals.entry(e.project.clone()).or_default() += e.duration();
            v.total += e.duration();
        }
        if is_working_day(day.date) {
            match day.kind {
                DayKind::Vacation => v.vacation_used += 1,
                DayKind::Sick => v.sick += 1,
                DayKind::Flex => v.flex += 1,
                DayKind::Holiday => v.holidays += 1,
                DayKind::Absence { .. } => v.absences += 1,
                DayKind::Work => {}
            }
        }
    }
    v.project_totals = totals
        .into_iter()
        .map(|(n, m)| {
            let idx = data
                .projects
                .iter()
                .find(|p| p.name == n)
                .map(|p| p.color_index)
                .unwrap_or(0);
            (n, m, idx)
        })
        .collect();
    v.project_totals
        .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let Some(data) = &m.stats else {
        f.render_widget(
            Paragraph::new("loading…").style(Style::default().fg(m.theme.muted)),
            area,
        );
        return;
    };
    let v = build_stats(data, &m.rules, &m.cal, m.today, m.vacation_allowance);
    draw_stats(f, area, &m.theme, &v, m.stats_range);
}

pub fn draw_stats(f: &mut Frame, area: Rect, t: &Theme, v: &StatsView, active: RangeKind) {
    let [sel, proj, kinds] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(6),
    ])
    .areas(area);

    let mut spans = Vec::new();
    for (k, key, label) in [
        (RangeKind::ThisMonth, "1", "this month"),
        (RangeKind::LastMonth, "2", "last month"),
        (RangeKind::Quarter, "3", "quarter"),
        (RangeKind::Year, "4", "year"),
    ] {
        let style = if k == active {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.muted)
        };
        spans.push(Span::styled(format!("[{key}] {label}   "), style));
    }
    spans.push(Span::styled(
        format!("{} → {}", v.from, v.to),
        Style::default().fg(t.text),
    ));
    f.render_widget(
        Paragraph::new(Line::from(spans)).block(block(t, Some("Range"))),
        sel,
    );

    let total = v.total.0.max(1) as f64;
    let bar_w = proj.width.saturating_sub(2 + 20 + 2 + 9 + 8 + 2).max(10);
    let mut lines: Vec<Line> = v
        .project_totals
        .iter()
        .map(|(n, m, idx)| {
            let frac = m.0 as f64 / total;
            let mut l = vec![Span::styled(
                format!("{n:<20}"),
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
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "no entries in range",
            Style::default().fg(t.muted),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("worked ", Style::default().fg(t.muted)),
        Span::raw(v.total.hhmm()),
        Span::styled("   net ", Style::default().fg(t.muted)),
        Span::raw(v.net.hhmm()),
        Span::styled("   target ", Style::default().fg(t.muted)),
        Span::raw(v.target.hhmm()),
        Span::styled("   balance ", Style::default().fg(t.muted)),
        minutes_span(v.net - v.target, t),
    ]));
    f.render_widget(
        Paragraph::new(lines).block(block(t, Some("Projects"))),
        proj,
    );

    let kl = vec![
        Line::from(vec![
            Span::styled("vacation  ", Style::default().fg(t.chip_vacation)),
            Span::raw(format!(
                "{} / {} used, {} left",
                v.vacation_used,
                v.vacation_allowance,
                v.vacation_allowance.saturating_sub(v.vacation_used)
            )),
        ]),
        Line::from(vec![
            Span::styled("sick      ", Style::default().fg(t.chip_sick)),
            Span::raw(v.sick.to_string()),
            Span::styled("    flex  ", Style::default().fg(t.chip_flex)),
            Span::raw(v.flex.to_string()),
            Span::styled("    holidays  ", Style::default().fg(t.chip_holiday)),
            Span::raw(v.holidays.to_string()),
            Span::styled("    absences  ", Style::default().fg(t.chip_absence)),
            Span::raw(v.absences.to_string()),
        ]),
        Line::from(vec![
            Span::styled("missing   ", Style::default().fg(t.negative)),
            Span::raw(v.missing.to_string()),
        ]),
    ];
    f.render_widget(Paragraph::new(kl).block(block(t, Some("Days"))), kinds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Day, DayKind, Entry, HolidayCalendar, Minutes, Rules, default_tiers};
    use crate::tui::msg::{RangeKind, StatsData};
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }

    #[test]
    fn ranges() {
        let today = d(2026, 9, 15);
        assert_eq!(
            range_for(RangeKind::ThisMonth, today),
            (d(2026, 9, 1), d(2026, 9, 30))
        );
        assert_eq!(
            range_for(RangeKind::LastMonth, today),
            (d(2026, 8, 1), d(2026, 8, 31))
        );
        assert_eq!(
            range_for(RangeKind::Quarter, today),
            (d(2026, 7, 1), d(2026, 9, 30))
        );
        assert_eq!(
            range_for(RangeKind::Year, today),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
        assert_eq!(
            range_for(RangeKind::LastMonth, d(2026, 1, 10)),
            (d(2025, 12, 1), d(2025, 12, 31))
        );
    }

    #[test]
    fn builds_and_renders() {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let mk = |dd, kind, entries| Day {
            date: d(2026, 9, dd),
            kind,
            entries,
        };
        let e = |id, dd, s, en, p: &str| Entry {
            id,
            date: d(2026, 9, dd),
            start: t(s),
            end: t(en),
            project: p.into(),
            comment: "".into(),
        };
        let days = vec![
            mk(1, DayKind::Work, vec![e(1, 1, 8, 16, "Alpha")]),
            mk(2, DayKind::Vacation, vec![]),
            mk(3, DayKind::Sick, vec![]),
            mk(4, DayKind::Work, vec![e(2, 4, 8, 12, "Beta")]),
            mk(7, DayKind::Flex, vec![]),
            mk(8, DayKind::Work, vec![]), // missing
        ];
        let data = StatsData {
            from: d(2026, 9, 1),
            to: d(2026, 9, 8),
            days,
            projects: vec![],
        };
        let rules = Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            start_date: d(2026, 1, 1),
            initial_balance: Minutes::ZERO,
        };
        let v = build_stats(
            &data,
            &rules,
            &HolidayCalendar::default(),
            d(2026, 9, 15),
            30,
        );
        assert_eq!(v.project_totals[0], ("Alpha".to_string(), Minutes(480), 0));
        assert_eq!(v.total, Minutes(720));
        assert_eq!((v.vacation_used, v.sick, v.flex, v.missing), (1, 1, 1, 1));
        let rows = render(100, 24, |f| {
            draw_stats(f, f.area(), &Theme::dark(), &v, RangeKind::ThisMonth)
        });
        assert!(contains(&rows, "2026-09-01 → 2026-09-08"));
        assert!(contains(&rows, "Alpha"));
        assert!(contains(&rows, "66.7%"));
        assert!(contains(&rows, "vacation"));
        assert!(contains(&rows, "1 / 30"));
        assert!(contains(&rows, "[1] this month"));
    }
}
