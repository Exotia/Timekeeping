//! Statistics screen drawing.

use chrono::{Datelike, Duration, NaiveDate};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::Paragraph;

use super::{bar, block, minutes_span};
use crate::core::{
    Day, DayKind, HolidayCalendar, HoursFormat, Minutes, Rules, TodayCtx, day_stats, is_working_day,
};
use crate::tui::model::{Model, month_name};
use crate::tui::msg::{RangeKind, StatsData};
use crate::tui::theme::Theme;
use crate::tui::worker::month_range;

pub struct StatsView {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// (project name, net minutes worked, color index), sorted by minutes desc.
    pub project_totals: Vec<(String, Minutes, u8)>,
    /// The net time worked in the range, i.e. the sum of `project_totals`.
    pub total: Minutes,
    pub net: Minutes,
    pub target: Minutes,
    /// Vacation working days inside the selected range.
    pub vacation_in_range: u32,
    /// Vacation working days in the whole calendar year — what the allowance is against.
    pub vacation_used_year: u32,
    pub vacation_allowance: u32,
    pub sick: u32,
    pub flex: u32,
    pub holidays: u32,
    pub absences: u32,
    pub missing: u32,
    /// The bars of the overtime chart, oldest first.
    pub buckets: Vec<Bucket>,
    pub granularity: Granularity,
    /// The sum of the bucket balances — the chart's own total, which starts at
    /// `rules.start_date` and so can differ from `net - target` over the range.
    pub balance_total: Minutes,
    /// Indices into `buckets` of the whole range's best and worst period. The
    /// chart's footer names the best and worst of the periods it can show, which
    /// is the same thing whenever the chart is not cut short.
    pub best: Option<usize>,
    pub worst: Option<usize>,
    /// Today, so a chart that cannot show every bucket can end at the one the
    /// user is living in rather than in an empty future.
    pub today: NaiveDate,
}

/// The (from, to) date range covered by a given `RangeKind`, relative to `today`.
pub fn range_for(kind: RangeKind, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match kind {
        RangeKind::Week => {
            let monday =
                today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
            (monday, monday + chrono::Duration::days(6))
        }
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

/// How one bar of the chart is aggregated: one per day, per ISO week or per month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Day,
    Week,
    Month,
}

/// One bar of the overtime chart: the balance of the days it spans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub label: String,
    pub balance: Minutes,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// The bar width a range is drawn at: a week shows its days, a month its weeks,
/// a quarter and a year their months.
pub fn granularity_for(kind: RangeKind) -> Granularity {
    match kind {
        RangeKind::Week => Granularity::Day,
        RangeKind::ThisMonth | RangeKind::LastMonth => Granularity::Week,
        RangeKind::Quarter | RangeKind::Year => Granularity::Month,
    }
}

/// The end of the bucket `start` opens, before clamping to the end of the range.
fn bucket_end(start: NaiveDate, g: Granularity) -> NaiveDate {
    match g {
        Granularity::Day => start,
        // The ISO week `start` falls in, which the first bucket of a month usually
        // only enters part way through.
        Granularity::Week => {
            start + Duration::days(6 - start.weekday().num_days_from_monday() as i64)
        }
        Granularity::Month => month_range(start.year(), start.month()).1,
    }
}

fn bucket_label(start: NaiveDate, g: Granularity) -> String {
    match g {
        Granularity::Day => format!("{} {:02}", start.format("%a"), start.day()),
        Granularity::Week => format!("KW {}", start.iso_week().week()),
        Granularity::Month => month_name(start.month())[..3].to_string(),
    }
}

/// Split `from`..=`to` into buckets of `g` and sum the balance of the days that
/// fall into each. Buckets with no days at all stay in, flat at zero, so the
/// chart keeps the shape of the range.
#[allow(clippy::too_many_arguments)]
pub fn build_buckets(
    days: &[Day],
    rules: &Rules,
    cal: &HolidayCalendar,
    today: NaiveDate,
    clocked_in: bool,
    from: NaiveDate,
    to: NaiveDate,
    g: Granularity,
) -> Vec<Bucket> {
    let ctx = TodayCtx { today, clocked_in };
    let mut out: Vec<Bucket> = Vec::new();
    let mut start = from;
    while start <= to {
        let end = bucket_end(start, g).min(to);
        out.push(Bucket {
            label: bucket_label(start, g),
            balance: Minutes::ZERO,
            from: start,
            to: end,
        });
        let Some(next) = end.succ_opt() else { break };
        start = next;
    }
    for day in days {
        // The balance only runs from the configured start date; before it there
        // is no target to miss.
        if day.date < rules.start_date || day.date < from || day.date > to {
            continue;
        }
        if let Some(b) = out
            .iter_mut()
            .find(|b| day.date >= b.from && day.date <= b.to)
        {
            b.balance += day_stats(day, rules, cal, &ctx).balance;
        }
    }
    out
}

/// The indices of the largest and the smallest balance, or `(None, None)` when
/// every bucket is flat — there is no "best week" in a month nothing happened in.
pub fn best_worst(buckets: &[Bucket]) -> (Option<usize>, Option<usize>) {
    if buckets.iter().all(|b| b.balance == Minutes::ZERO) {
        return (None, None);
    }
    // Ties go to the earlier bucket, in both directions.
    let best = buckets
        .iter()
        .enumerate()
        .max_by_key(|(i, b)| (b.balance, std::cmp::Reverse(*i)))
        .map(|(i, _)| i);
    let worst = buckets
        .iter()
        .enumerate()
        .min_by_key(|(_, b)| b.balance)
        .map(|(i, _)| i);
    (best, worst)
}

/// Aggregate raw store data into the view model the screen draws from.
pub fn build_stats(
    data: &StatsData,
    rules: &Rules,
    cal: &HolidayCalendar,
    today: NaiveDate,
    allowance: u32,
    kind: RangeKind,
) -> StatsView {
    let ctx = TodayCtx {
        today,
        clocked_in: data.session_active,
    };
    let mut totals: std::collections::BTreeMap<String, Minutes> = Default::default();
    let mut v = StatsView {
        from: data.from,
        to: data.to,
        project_totals: vec![],
        total: Minutes::ZERO,
        net: Minutes::ZERO,
        target: Minutes::ZERO,
        vacation_in_range: 0,
        vacation_used_year: data.vacation_used_year,
        vacation_allowance: allowance,
        sick: 0,
        flex: 0,
        holidays: 0,
        absences: 0,
        missing: 0,
        buckets: vec![],
        granularity: granularity_for(kind),
        balance_total: Minutes::ZERO,
        best: None,
        worst: None,
        today,
    };
    for day in &data.days {
        let s = day_stats(day, rules, cal, &ctx);
        // The balance runs from the configured start date, exactly as the chart's
        // buckets do, so the two totals on the screen cannot contradict each other.
        // What was worked and which kinds the days had are facts about the range
        // itself and stay whole.
        if day.date >= rules.start_date {
            v.net += s.net;
            v.target += s.target;
            if s.missing {
                v.missing += 1;
            }
        }
        // Net per entry: each one carries its share of its session's break
        // deduction, so the project hours add up to the net that was worked.
        for (idx, e) in day.entries.iter().enumerate() {
            *totals.entry(e.project.clone()).or_default() += s.entry_nets[idx];
            v.total += s.entry_nets[idx];
        }
        if is_working_day(day.date) {
            match day.kind {
                DayKind::Vacation => v.vacation_in_range += 1,
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
    v.buckets = build_buckets(
        &data.days,
        rules,
        cal,
        today,
        data.session_active,
        data.from,
        data.to,
        v.granularity,
    );
    v.balance_total = v.buckets.iter().map(|b| b.balance).sum();
    (v.best, v.worst) = best_worst(&v.buckets);
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
    let v = build_stats(
        data,
        &m.rules,
        &m.cal,
        m.today,
        m.vacation_allowance,
        m.stats_range,
    );
    draw_stats(f, area, &m.theme, &v, m.stats_range, m.hours);
}

/// Width of the chart's right-aligned value column — `+100.00h` plus a space.
const VALUE_W: usize = 9;

/// The bar area of a chart row: label (7) + a space + the bars + a right-aligned
/// value (9) + a trailing space.
const CHART_GUTTER: usize = 7 + 1 + VALUE_W + 1;

/// Where the zero line sits inside a bar area of `width` columns, given the
/// largest negative and positive balance on show. It is a column of its own, so
/// negative bars end just left of it and positive ones start just right of it.
fn zero_column(width: usize, max_neg: i32, max_pos: i32) -> usize {
    let last = width.saturating_sub(1);
    if max_neg == 0 {
        return 0;
    }
    if max_pos == 0 {
        return last;
    }
    // Too narrow to split: the one free cell goes to the bigger side.
    if last < 2 {
        return if max_neg >= max_pos { last } else { 0 };
    }
    let share = last as f64 * max_neg as f64 / (max_neg + max_pos) as f64;
    (share.round() as usize).clamp(1, last.saturating_sub(1))
}

/// A bar length in cells: proportional to `span`, but never rounded away to
/// nothing — a day that is 4 minutes short still has to be visible.
fn bar_len(span: usize, value: i32, max: i32) -> usize {
    if value == 0 || max == 0 || span == 0 {
        return 0;
    }
    ((span as f64 * value as f64 / max as f64).round() as usize).clamp(1, span)
}

/// The slice of buckets a panel `rows` tall can show: the most recent ones,
/// ending at the bucket today falls in so that a year seen in September does not
/// scroll away into three empty winter months.
fn visible_buckets(v: &StatsView, rows: usize) -> &[Bucket] {
    if rows >= v.buckets.len() {
        return &v.buckets;
    }
    let end = v
        .buckets
        .iter()
        .rposition(|b| b.from <= v.today)
        .map_or(v.buckets.len(), |i| i + 1)
        .max(rows);
    &v.buckets[end - rows..end]
}

fn value_spans(m: Minutes, fmt: HoursFormat, t: &Theme) -> Vec<Span<'static>> {
    let s = minutes_span(m, fmt, t);
    let pad = VALUE_W.saturating_sub(s.content.chars().count());
    vec![Span::raw(" ".repeat(pad)), s]
}

/// The overtime chart: one bar per bucket, growing left or right of a zero line.
pub fn draw_chart(f: &mut Frame, area: Rect, t: &Theme, v: &StatsView, fmt: HoursFormat) {
    let inner_w = area.width.saturating_sub(2) as usize;
    // Borders, the zero axis and the two footer lines.
    let rows = area.height.saturating_sub(5) as usize;
    let shown = visible_buckets(v, rows);
    let bar_w = inner_w.saturating_sub(CHART_GUTTER).max(1);
    let max_neg = shown.iter().map(|b| -b.balance.0).max().unwrap_or(0).max(0);
    let max_pos = shown.iter().map(|b| b.balance.0).max().unwrap_or(0).max(0);
    let zero = zero_column(bar_w, max_neg, max_pos);
    let pos_span = bar_w - zero - 1;

    let mut lines: Vec<Line> = Vec::new();
    for b in shown {
        let (neg, pos) = if b.balance.0 < 0 {
            (bar_len(zero, -b.balance.0, max_neg), 0)
        } else {
            (0, bar_len(pos_span, b.balance.0, max_pos))
        };
        let mut l = vec![
            Span::styled(format!("{:<7}", b.label), Style::default().fg(t.text)),
            Span::raw(" ".repeat(1 + zero - neg)),
            Span::styled("█".repeat(neg), Style::default().fg(t.negative)),
            Span::styled("│", Style::default().fg(t.muted)),
            Span::styled("█".repeat(pos), Style::default().fg(t.positive)),
            Span::raw(" ".repeat(pos_span - pos)),
        ];
        l.extend(value_spans(b.balance, fmt, t));
        lines.push(Line::from(l));
    }
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(8 + zero)),
        Span::styled("0", Style::default().fg(t.muted)),
    ]));

    let mut footer = vec![
        Span::styled("total ", Style::default().fg(t.muted)),
        minutes_span(v.balance_total, fmt, t),
    ];
    // Over what is on show, so a chart that had to cut itself never names a
    // period the reader cannot see.
    let (best, worst) = best_worst(shown);
    for (label, idx) in [("best", best), ("worst", worst)] {
        let Some(b) = idx.and_then(|i| shown.get(i)) else {
            continue;
        };
        footer.push(Span::styled(
            format!(" · {label} {} ", b.label),
            Style::default().fg(t.muted),
        ));
        footer.push(minutes_span(b.balance, fmt, t));
    }
    lines.push(Line::from(footer));
    // Net, target and balance belong together: the target and the balance are
    // about the day, not about any project, so they are read here next to the
    // net they move and never beside the hours a project was worked.
    lines.push(Line::from(vec![
        Span::styled("net ", Style::default().fg(t.muted)),
        Span::raw(v.net.fmt_signed(fmt)),
        Span::styled(" · target ", Style::default().fg(t.muted)),
        Span::raw(format!("-{}", v.target.fmt_unsigned(fmt))),
        Span::styled(" · balance ", Style::default().fg(t.muted)),
        minutes_span(v.net - v.target, fmt, t),
    ]));

    let title = match v.granularity {
        Granularity::Day => "Balance per day",
        Granularity::Week => "Balance per week",
        Granularity::Month => "Balance per month",
    };
    let title = if shown.len() < v.buckets.len() {
        format!("… {title}")
    } else {
        title.to_string()
    };
    f.render_widget(Paragraph::new(lines).block(block(t, Some(&title))), area);
}

/// The height the chart panel asks for: its rows plus borders, axis and footer,
/// but never more than what is left once the projects and the day-type panels
/// have their minimum.
fn chart_height(v: &StatsView, area: Rect) -> u16 {
    let available = area.height.saturating_sub(3 + 5 + 6);
    // Two borders, the zero axis, two footer lines and at least one bar.
    if v.buckets.is_empty() || available < 6 {
        return 0;
    }
    ((v.buckets.len() + 5) as u16).min(available)
}

pub fn draw_stats(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    v: &StatsView,
    active: RangeKind,
    fmt: HoursFormat,
) {
    let chart_h = chart_height(v, area);
    let [sel, chart, proj, kinds] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(chart_h),
        Constraint::Min(5),
        Constraint::Length(6),
    ])
    .areas(area);

    let entries = [
        (RangeKind::Week, "1", "week"),
        (RangeKind::ThisMonth, "2", "month"),
        (RangeKind::LastMonth, "3", "last month"),
        (RangeKind::Quarter, "4", "quarter"),
        (RangeKind::Year, "5", "year"),
    ];
    let dates = format!("{} → {}", v.from, v.to);
    // Five ranges and the dates are wider than 80 columns with the airy gaps the
    // line has room for elsewhere, and the dates are the part worth keeping.
    let roomy: usize = entries
        .iter()
        .map(|(_, k, l)| k.chars().count() + l.chars().count() + 3 + 3)
        .sum::<usize>()
        + dates.chars().count();
    let gap = if roomy <= sel.width.saturating_sub(2) as usize {
        "   "
    } else {
        " "
    };
    let mut spans = Vec::new();
    for (k, key, label) in entries {
        let style = if k == active {
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.muted)
        };
        spans.push(Span::styled(format!("[{key}] {label}{gap}"), style));
    }
    spans.push(Span::styled(dates, Style::default().fg(t.text)));
    f.render_widget(
        Paragraph::new(Line::from(spans)).block(block(t, Some("Range"))),
        sel,
    );

    if chart_h > 0 {
        draw_chart(f, chart, t, v, fmt);
    }

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
                m.fmt_unsigned(fmt),
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
        Span::styled("net ", Style::default().fg(t.muted)),
        Span::raw(v.total.fmt_unsigned(fmt)),
    ]));
    f.render_widget(
        Paragraph::new(lines).block(block(t, Some("Projects"))),
        proj,
    );

    let kl = vec![
        Line::from(vec![
            Span::styled("vacation  ", Style::default().fg(t.chip_vacation)),
            Span::raw(format!(
                "{} in range · {} / {} used this year, {} left",
                v.vacation_in_range,
                v.vacation_used_year,
                v.vacation_allowance,
                v.vacation_allowance.saturating_sub(v.vacation_used_year)
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
    use crate::core::{
        Day, DayKind, Entry, HolidayCalendar, HoursFormat, Minutes, Rules, default_tiers,
    };
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
        // Tuesday 15 September 2026 sits in the ISO week Mon 14 … Sun 20.
        assert_eq!(
            range_for(RangeKind::Week, today),
            (d(2026, 9, 14), d(2026, 9, 20))
        );
        assert_eq!(
            range_for(RangeKind::Week, d(2026, 9, 20)),
            (d(2026, 9, 14), d(2026, 9, 20))
        );
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

    /// A `Rules` with the 7:48 daily target, tracking from the first of January.
    fn rules() -> Rules {
        Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            start_date: d(2026, 1, 1),
            initial_balance: Minutes::ZERO,
        }
    }

    /// A nine-hour work day: 540 gross − 48 break = 492 net, i.e. +24 on the target.
    fn plus24(date: NaiveDate) -> Day {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        Day {
            date,
            kind: DayKind::Work,
            entries: vec![Entry {
                id: 1,
                date,
                start: t(8),
                end: t(17),
                project: "Alpha".into(),
                comment: "".into(),
            }],
        }
    }

    /// A past work day with no entries: the full target is missing.
    fn missing(date: NaiveDate) -> Day {
        Day {
            date,
            kind: DayKind::Work,
            entries: vec![],
        }
    }

    fn buckets_of(days: Vec<Day>, from: NaiveDate, to: NaiveDate, g: Granularity) -> Vec<Bucket> {
        build_buckets(
            &days,
            &rules(),
            &HolidayCalendar::default(),
            d(2026, 9, 15),
            false,
            from,
            to,
            g,
        )
    }

    #[test]
    fn granularities() {
        assert_eq!(granularity_for(RangeKind::Week), Granularity::Day);
        assert_eq!(granularity_for(RangeKind::ThisMonth), Granularity::Week);
        assert_eq!(granularity_for(RangeKind::LastMonth), Granularity::Week);
        assert_eq!(granularity_for(RangeKind::Quarter), Granularity::Month);
        assert_eq!(granularity_for(RangeKind::Year), Granularity::Month);
    }

    #[test]
    fn week_range_buckets_every_day() {
        let (from, to) = range_for(RangeKind::Week, d(2026, 9, 15));
        let b = buckets_of(vec![plus24(d(2026, 9, 14))], from, to, Granularity::Day);
        assert_eq!(b.len(), 7);
        assert_eq!(
            b.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            [
                "Mon 14", "Tue 15", "Wed 16", "Thu 17", "Fri 18", "Sat 19", "Sun 20"
            ]
        );
        assert_eq!((b[0].from, b[0].to), (d(2026, 9, 14), d(2026, 9, 14)));
        assert_eq!(b[0].balance, Minutes(24));
        // Today is still open and the future has no target, so the rest is flat.
        assert!(b[1..].iter().all(|b| b.balance == Minutes::ZERO));
    }

    #[test]
    fn month_range_buckets_by_iso_week() {
        let (from, to) = range_for(RangeKind::ThisMonth, d(2026, 9, 15));
        let b = buckets_of(vec![], from, to, Granularity::Week);
        // 1 September 2026 is a Tuesday, so the first (partial) week is KW 36.
        assert_eq!(
            b.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["KW 36", "KW 37", "KW 38", "KW 39", "KW 40"]
        );
        assert_eq!((b[0].from, b[0].to), (d(2026, 9, 1), d(2026, 9, 6)));
        assert_eq!((b[1].from, b[1].to), (d(2026, 9, 7), d(2026, 9, 13)));
        assert_eq!((b[4].from, b[4].to), (d(2026, 9, 28), d(2026, 9, 30)));
    }

    #[test]
    fn year_range_buckets_by_month() {
        let (from, to) = range_for(RangeKind::Year, d(2026, 9, 15));
        let b = buckets_of(vec![], from, to, Granularity::Month);
        assert_eq!(b.len(), 12);
        assert_eq!(b[0].label, "Jan");
        assert_eq!(b[8].label, "Sep");
        assert_eq!(b[11].label, "Dec");
        assert_eq!((b[8].from, b[8].to), (d(2026, 9, 1), d(2026, 9, 30)));
        assert_eq!((b[11].from, b[11].to), (d(2026, 12, 1), d(2026, 12, 31)));
    }

    #[test]
    fn bucket_balances_sum_their_days() {
        let (from, to) = range_for(RangeKind::ThisMonth, d(2026, 9, 15));
        let days = vec![
            plus24(d(2026, 9, 1)),
            missing(d(2026, 9, 8)),
            plus24(d(2026, 9, 9)),
        ];
        let b = buckets_of(days, from, to, Granularity::Week);
        assert_eq!(b[0].balance, Minutes(24));
        assert_eq!(b[1].balance, Minutes(24 - 468));
        assert_eq!(b[2].balance, Minutes::ZERO);
    }

    #[test]
    fn days_before_the_start_date_do_not_count() {
        let mut r = rules();
        r.start_date = d(2026, 9, 9);
        let b = build_buckets(
            &[plus24(d(2026, 9, 1)), plus24(d(2026, 9, 9))],
            &r,
            &HolidayCalendar::default(),
            d(2026, 9, 15),
            false,
            d(2026, 9, 1),
            d(2026, 9, 30),
            Granularity::Week,
        );
        assert_eq!(b[0].balance, Minutes::ZERO);
        assert_eq!(b[1].balance, Minutes(24));
    }

    #[test]
    fn best_and_worst_buckets() {
        let (from, to) = range_for(RangeKind::ThisMonth, d(2026, 9, 15));
        let days = vec![plus24(d(2026, 9, 1)), missing(d(2026, 9, 8))];
        let flat = buckets_of(vec![], from, to, Granularity::Week);
        assert_eq!((best_worst(&flat)), (None, None));
        let b = buckets_of(days, from, to, Granularity::Week);
        assert_eq!(best_worst(&b), (Some(0), Some(1)));
        // Two equally good weeks: the earlier one is the one named.
        let tied = buckets_of(
            vec![plus24(d(2026, 9, 1)), plus24(d(2026, 9, 9))],
            from,
            to,
            Granularity::Week,
        );
        assert_eq!(best_worst(&tied).0, Some(0));
    }

    fn stats_data(from: NaiveDate, to: NaiveDate, days: Vec<Day>) -> StatsData {
        StatsData {
            from,
            to,
            days,
            projects: vec![],
            vacation_used_year: 0,
            session_active: false,
        }
    }

    fn view(kind: RangeKind, days: Vec<Day>) -> StatsView {
        let today = d(2026, 9, 15);
        let (from, to) = range_for(kind, today);
        build_stats(
            &stats_data(from, to, days),
            &rules(),
            &HolidayCalendar::default(),
            today,
            30,
            kind,
        )
    }

    /// The columns a row's `█` cells sit in, and the column of the zero line —
    /// the one `│` that is neither the left nor the right border of the panel.
    fn bars_and_zero(row: &str) -> (Vec<usize>, usize) {
        let chars: Vec<char> = row.chars().collect();
        let bars = chars
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == '█')
            .map(|(i, _)| i)
            .collect();
        let pipes: Vec<usize> = chars
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == '│')
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            pipes.len(),
            3,
            "expected two borders and a zero line: {row}"
        );
        (bars, pipes[1])
    }

    #[test]
    fn the_projects_balance_matches_the_chart_total() {
        // Tracking only starts on 9 September: the days before it are history the
        // balance never counted, and the two panels must agree on that.
        let mut r = rules();
        r.start_date = d(2026, 9, 9);
        let (from, to) = range_for(RangeKind::ThisMonth, d(2026, 9, 15));
        let days = vec![
            plus24(d(2026, 9, 1)),
            missing(d(2026, 9, 2)),
            plus24(d(2026, 9, 9)),
            missing(d(2026, 9, 10)),
        ];
        let v = build_stats(
            &stats_data(from, to, days),
            &r,
            &HolidayCalendar::default(),
            d(2026, 9, 15),
            30,
            RangeKind::ThisMonth,
        );
        assert_eq!(v.balance_total, Minutes(24 - 468));
        assert_eq!(v.net - v.target, v.balance_total);
        // A work day before the start date is not a day anybody is missing.
        assert_eq!(v.missing, 1);
        // What was worked in the range is still what was worked in the range —
        // net, so each nine-hour day counts the 492 minutes it earned.
        assert_eq!(v.total, Minutes(2 * 492));
        assert_eq!(
            v.project_totals.iter().map(|p| p.1).sum::<Minutes>(),
            v.total
        );
        let rows = render(100, 30, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Hm,
            )
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "total -07:24"), "{joined}");
        assert!(contains(&rows, "balance -07:24"), "{joined}");
        // The projects panel names the net time on projects and nothing else:
        // the target and the balance belong to the chart. Its footer is the
        // lower of the two `net` lines on the screen.
        let projects_footer = rows
            .iter()
            .rev()
            .find(|r| r.contains("net "))
            .unwrap_or_else(|| panic!("{joined}"));
        assert!(projects_footer.contains("16:24"), "{joined}");
        for stray in ["target", "balance", "worked", "gross"] {
            assert!(
                !projects_footer.contains(stray),
                "{stray} beside the projects: {joined}"
            );
        }
        // Net, target and balance all sit under the chart's total line.
        let footer = rows
            .iter()
            .find(|r| r.contains("net "))
            .unwrap_or_else(|| panic!("{joined}"));
        assert!(footer.contains("target "), "{joined}");
        assert!(footer.contains("balance "), "{joined}");
        let total_row = rows.iter().position(|r| r.contains("total ")).unwrap();
        let net_row = rows.iter().position(|r| r.contains("net ")).unwrap();
        assert_eq!(net_row, total_row + 1, "{joined}");
    }

    /// The chart's bar values, its footer and the projects panel all follow the
    /// display setting.
    #[test]
    fn renders_in_decimal_hours() {
        let v = view(
            RangeKind::ThisMonth,
            vec![plus24(d(2026, 9, 1)), missing(d(2026, 9, 8))],
        );
        let rows = render(100, 30, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Decimal,
            )
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "+0.40h"), "the +24 bar:\n{joined}");
        assert!(
            contains(&rows, "-7.80h"),
            "the missing day's bar:\n{joined}"
        );
        assert!(contains(&rows, "total -7.40h"), "the footer:\n{joined}");
        assert!(contains(&rows, "net 8.20h"), "the projects:\n{joined}");
        assert!(!contains(&rows, "00:24"), "an h:mm leak:\n{joined}");
    }

    #[test]
    fn a_bar_area_of_one_column_still_draws() {
        // Both signs on show and no room to split: the zero line takes the column.
        assert_eq!(zero_column(0, 5, 5), 0);
        assert_eq!(zero_column(1, 5, 5), 0);
        assert_eq!(zero_column(2, 5, 5), 1);
        assert_eq!(zero_column(10, 0, 5), 0);
        assert_eq!(zero_column(10, 5, 0), 9);
        let v = view(
            RangeKind::ThisMonth,
            vec![plus24(d(2026, 9, 1)), missing(d(2026, 9, 8))],
        );
        // Far below the minimum terminal, but a panic here would take the app down.
        let rows = render(20, 21, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Hm,
            )
        });
        assert!(rows.iter().all(|r| r.chars().count() <= 20));
    }

    #[test]
    fn the_range_line_keeps_its_dates_at_eighty_columns() {
        let v = view(RangeKind::Year, vec![]);
        for w in [80u16, 100] {
            let rows = render(w, 21, |f| {
                draw_stats(
                    f,
                    f.area(),
                    &Theme::dark(),
                    &v,
                    RangeKind::Year,
                    HoursFormat::Hm,
                )
            });
            let joined = rows.join("\n");
            assert!(
                contains(&rows, "2026-01-01 → 2026-12-31"),
                "the range is cut at {w} columns:\n{joined}"
            );
            assert!(contains(&rows, "[1] week"), "{joined}");
            assert!(contains(&rows, "[5] year"), "{joined}");
        }
    }

    #[test]
    fn month_chart_draws_a_bar_per_week() {
        let v = view(
            RangeKind::ThisMonth,
            vec![plus24(d(2026, 9, 1)), missing(d(2026, 9, 8))],
        );
        let rows = render(100, 30, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Hm,
            )
        });
        let joined = rows.join("\n");
        assert!(contains(&rows, "Balance per week"), "{joined}");
        assert!(contains(&rows, "KW 36"), "{joined}");
        assert!(contains(&rows, "KW 37"), "{joined}");
        assert!(contains(&rows, "+00:24"), "{joined}");
        assert!(contains(&rows, "-07:48"), "{joined}");
        // The axis row carries the zero label, the footer the totals.
        assert!(contains(&rows, "total "), "{joined}");
        assert!(contains(&rows, "best KW 36"), "{joined}");
        assert!(contains(&rows, "worst KW 37"), "{joined}");
        let plus = rows.iter().find(|r| r.contains("KW 36")).unwrap();
        let (bars, zero) = bars_and_zero(plus);
        assert!(bars.iter().all(|b| *b > zero), "{plus}");
        // The axis row carries nothing but the zero, right under the zero line.
        // Searched from the first bar down, so that a date elsewhere on the screen
        // that happens to have a digit in this column cannot stand in for it.
        let first_bar = rows.iter().position(|r| r.contains("KW 36")).unwrap();
        let axis = rows[first_bar..]
            .iter()
            .find(|r| r.chars().nth(zero) == Some('0'))
            .unwrap_or_else(|| panic!("no axis row with a 0 at column {zero}:\n{joined}"));
        assert_eq!(
            axis.chars()
                .filter(|c| *c != ' ' && *c != '│')
                .collect::<String>(),
            "0",
            "{axis}"
        );
    }

    #[test]
    fn a_negative_only_chart_grows_to_the_left() {
        let v = view(RangeKind::ThisMonth, vec![missing(d(2026, 9, 8))]);
        let rows = render(100, 30, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Hm,
            )
        });
        let row = rows.iter().find(|r| r.contains("KW 37")).unwrap();
        let (bars, zero) = bars_and_zero(row);
        assert!(!bars.is_empty(), "no bar drawn: {row}");
        assert!(
            bars.iter().all(|b| *b < zero),
            "bars must sit left of the zero line: {row}"
        );
        // With nothing positive the zero line hugs the right end of the bar area.
        assert!(zero > 80, "zero line too far left: {row}");
    }

    #[test]
    fn a_year_fits_into_eighty_columns() {
        let v = view(
            RangeKind::Year,
            vec![plus24(d(2026, 1, 5)), missing(d(2026, 2, 3))],
        );
        assert_eq!(v.buckets.len(), 12);
        // The body of an 80×24 terminal: title bar, status bar and hint row taken off.
        let rows = render(80, 21, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::Year,
                HoursFormat::Hm,
            )
        });
        let joined = rows.join("\n");
        assert!(
            rows.iter().all(|r| r.chars().count() <= 80),
            "a row overflows 80 columns:\n{joined}"
        );
        // Not all twelve months fit, so the title says the chart is cut.
        assert!(contains(&rows, "… Balance per month"), "{joined}");
        // The window ends at the month today is in.
        assert!(contains(&rows, "Sep"), "{joined}");
        // January and February are off screen, so the footer must not name them;
        // the three months on show are all flat, and there is no best among them.
        assert!(!contains(&rows, "best Jan"), "{joined}");
        assert!(!contains(&rows, "worst Feb"), "{joined}");
        assert!(contains(&rows, "total "), "{joined}");
        // Every panel still has its frame.
        for title in ["Range", "Balance per month", "Projects", "Days"] {
            assert!(contains(&rows, title), "{title} missing:\n{joined}");
        }
        assert_eq!(
            rows.iter().filter(|r| r.starts_with('╭')).count(),
            4,
            "{joined}"
        );
    }

    #[test]
    fn an_empty_range_draws_no_chart() {
        let mut v = view(RangeKind::ThisMonth, vec![]);
        v.buckets.clear();
        let rows = render(100, 30, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Hm,
            )
        });
        assert!(!contains(&rows, "Balance per"), "{}", rows.join("\n"));
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
            // Four vacation days taken this year, one of them inside the range.
            vacation_used_year: 4,
            session_active: false,
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
            RangeKind::ThisMonth,
        );
        // Net per project: Alpha's eight-hour day loses the 48-minute tier and
        // Beta's four-hour one loses 18.
        assert_eq!(v.project_totals[0], ("Alpha".to_string(), Minutes(432), 0));
        assert_eq!(v.project_totals[1], ("Beta".to_string(), Minutes(222), 0));
        assert_eq!(v.total, Minutes(654));
        assert_eq!(
            (v.vacation_in_range, v.sick, v.flex, v.missing),
            (1, 1, 1, 1)
        );
        // What is left of the allowance follows the year, not the selected range.
        assert_eq!(v.vacation_used_year, 4);
        // A month range is charted by ISO week: 1–6 September and 7–8 September.
        assert_eq!(v.granularity, Granularity::Week);
        assert_eq!(
            v.buckets
                .iter()
                .map(|b| b.label.as_str())
                .collect::<Vec<_>>(),
            ["KW 36", "KW 37"]
        );
        assert_eq!(
            v.balance_total,
            v.buckets.iter().map(|b| b.balance).sum::<Minutes>()
        );
        assert_eq!((v.best, v.worst), (Some(0), Some(1)));
        let rows = render(100, 24, |f| {
            draw_stats(
                f,
                f.area(),
                &Theme::dark(),
                &v,
                RangeKind::ThisMonth,
                HoursFormat::Hm,
            )
        });
        assert!(contains(&rows, "2026-09-01 → 2026-09-08"));
        assert!(contains(&rows, "Alpha"));
        // Alpha's 432 net minutes of the 654 worked in the range.
        assert!(contains(&rows, "66.1%"));
        assert!(contains(&rows, "vacation"));
        assert!(contains(
            &rows,
            "1 in range · 4 / 30 used this year, 26 left"
        ));
        assert!(contains(&rows, "[1] week"));
        assert!(contains(&rows, "[2] month"));
        assert!(contains(&rows, "[3] last month"));
        assert!(contains(&rows, "[4] quarter"));
        assert!(contains(&rows, "[5] year"));
    }
}
