//! Full-screen snapshot tests: render the whole month screen through a
//! `TestBackend` at several terminal sizes and assert on the text that lands in
//! the buffer. They pin the responsive rules (comment column, minimum size)
//! and the shared chrome (title bar, key hints).

use chrono::NaiveDate;
use tk::core::{Day, DayKind, Entry};
use tk::tui::model::testing::{model, month_data};
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;

/// Render `draw` into a `w`×`h` buffer and return it as trimmed text rows.
fn rows(w: u16, h: u16, draw: impl FnOnce(&mut tuirealm::ratatui::Frame)) -> Vec<String> {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(draw).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// September 2026 with a single entry on the 14th carrying the comment we look for.
fn september_2026() -> (NaiveDate, Vec<Day>) {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let t = |h| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
    let days: Vec<Day> = (1..=30)
        .map(|dd| {
            let date = NaiveDate::from_ymd_opt(2026, 9, dd).unwrap();
            let entries = if dd == 14 {
                vec![Entry {
                    id: 1,
                    date,
                    start: t(9),
                    end: t(17),
                    project: "Alpha".into(),
                    comment: "snapshot".into(),
                }]
            } else {
                vec![]
            };
            Day {
                date,
                kind: DayKind::Work,
                entries,
            }
        })
        .collect();
    (today, days)
}

#[test]
fn month_screen_100x30_and_80x24() {
    let (today, days) = september_2026();
    let (mut m, _rx) = model(today);
    m.selected = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
    m.month = Some(month_data(today, days, None));

    let big = rows(100, 30, |f| m.draw(f));
    let joined = big.join("\n");
    assert!(
        big.iter().any(|r| r.contains("SEPTEMBER 2026")),
        "title bar missing:\n{joined}"
    );
    assert!(
        big.iter().any(|r| r.contains("snapshot")),
        "comment column missing at 100 cols:\n{joined}"
    );
    assert!(
        big.iter().any(|r| r.contains("q quit")),
        "quit hint missing:\n{joined}"
    );

    let small = rows(80, 24, |f| m.draw(f));
    let joined = small.join("\n");
    assert!(
        small.iter().any(|r| r.contains("Mon 14")),
        "selected day row missing at 80 cols:\n{joined}"
    );
    assert!(
        !small.iter().any(|r| r.contains("snapshot")),
        "comment column must be hidden below 90 cols:\n{joined}"
    );

    let tiny = rows(60, 20, |f| m.draw(f));
    let joined = tiny.join("\n");
    assert!(
        joined.contains("too small"),
        "minimum-size notice missing:\n{joined}"
    );
}

/// Decimal hours are the widest a duration ever gets, so the 80×24 minimum is
/// where the month table would break first if a column were too narrow.
#[test]
fn month_screen_80x24_in_decimal_hours() {
    let (today, days) = september_2026();
    let (mut m, _rx) = model(today);
    m.selected = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
    m.month = Some(month_data(today, days, None));
    m.hours = tk::core::HoursFormat::Decimal;

    let out = rows(80, 24, |f| m.draw(f));
    let joined = out.join("\n");
    assert!(
        out.iter().all(|r| r.chars().count() <= 80),
        "a row overflows 80 columns:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("Mon 14")),
        "selected day row missing:\n{joined}"
    );
    // 9:00–17:00 is 8h gross; 48 minutes of it go to the break tier.
    assert!(
        out.iter().any(|r| r.contains("+8.00h")),
        "the entry's gross:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("+7.20h")),
        "the day's net:\n{joined}"
    );
    assert!(
        !out.iter().any(|r| r.contains("08:00")),
        "an h:mm duration leaked into decimal mode:\n{joined}"
    );
    // Every panel keeps its frame: the table and the month summary.
    assert_eq!(
        out.iter().filter(|r| r.starts_with('╭')).count(),
        2,
        "a panel lost its top border:\n{joined}"
    );
}

/// The statistics screen at the same size, in decimal, on the widest range.
#[test]
fn stats_screen_80x24_year_in_decimal_hours() {
    let (mut m, _rx) = stats_year_model();
    m.hours = tk::core::HoursFormat::Decimal;
    let out = rows(80, 24, |f| m.draw(f));
    let joined = out.join("\n");
    assert!(
        out.iter().all(|r| r.chars().count() <= 80),
        "a row overflows 80 columns:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("Balance per month")),
        "chart panel missing:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("h")),
        "no decimal hours anywhere:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("-7.80h")),
        "the month that is a whole target short:\n{joined}"
    );
    assert_eq!(
        out.iter().filter(|r| r.starts_with('╭')).count(),
        4,
        "a panel lost its top border:\n{joined}"
    );
}

#[test]
fn key_hints_fit_the_minimum_terminal() {
    let (today, days) = september_2026();
    let (mut m, _rx) = model(today);
    m.month = Some(month_data(today, days, None));
    // The hint row is a single line; at the documented minimum width the month
    // screen's full set does not fit, so the essential keys must still be there.
    for w in [80u16, 100] {
        let out = rows(w, 24, |f| m.draw(f));
        let hints = out.last().unwrap().clone();
        assert!(
            hints.chars().count() <= w as usize,
            "hints overflow at {w}: {hints}"
        );
        assert!(
            hints.contains("q quit"),
            "quit hint missing at {w}: {hints}"
        );
        assert!(hints.contains("c settings"), "at {w}: {hints}");
        assert!(hints.contains("? help"), "at {w}: {hints}");
    }
}

/// A model parked on the statistics screen over the whole of 2026.
fn stats_year_model() -> (
    tk::tui::model::Model,
    std::sync::mpsc::Receiver<tk::tui::msg::StoreCmd>,
) {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, rx) = model(today);
    m.rules.start_date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    m.screen = tk::tui::model::Screen::Stats;
    m.stats_range = tk::tui::msg::RangeKind::Year;
    let t = |h| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
    // Nine-hour days are +00:24 each; 4 August is a Tuesday nobody booked, so
    // August is a whole target short and its bar grows the other way.
    let worked = |mo, dd, hours: u32| {
        let date = NaiveDate::from_ymd_opt(2026, mo, dd).unwrap();
        Day {
            date,
            kind: DayKind::Work,
            entries: vec![Entry {
                id: 1,
                date,
                start: t(8),
                end: t(8 + hours),
                project: "Alpha".into(),
                comment: "".into(),
            }],
        }
    };
    let days: Vec<Day> = vec![
        worked(1, 5, 9),
        worked(7, 6, 9),
        worked(7, 7, 11),
        Day {
            date: NaiveDate::from_ymd_opt(2026, 8, 4).unwrap(),
            kind: DayKind::Work,
            entries: vec![],
        },
        worked(9, 1, 9),
    ];
    m.stats = Some(tk::tui::msg::StatsData {
        from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        to: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        days,
        projects: vec![],
        vacation_used_year: 3,
        session_active: false,
    });
    (m, rx)
}

/// The statistics screen at the documented minimum size, on the year range: the
/// chart cannot show all twelve months there, so it must cut itself down instead
/// of spilling out of its frame.
#[test]
fn stats_screen_80x24_year() {
    let (m, _rx) = stats_year_model();
    let out = rows(80, 24, |f| m.draw(f));
    let joined = out.join("\n");
    assert!(
        out.iter().all(|r| r.chars().count() <= 80),
        "a row overflows 80 columns:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("[5] year")),
        "range selector missing:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("Balance per month")),
        "chart panel missing:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("Sep")),
        "the month of today must be on the chart:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("total ")),
        "chart footer missing:\n{joined}"
    );
    // The balance figures sit under the chart, and the projects panel is about
    // worked time alone — a break deduction beside project hours reads as if the
    // projects had lost the time.
    let footer = out
        .iter()
        .find(|r| r.contains("net "))
        .unwrap_or_else(|| panic!("{joined}"));
    assert!(
        footer.contains("target ") && footer.contains("balance "),
        "{joined}"
    );
    let projects_footer = out
        .iter()
        .find(|r| r.contains("worked "))
        .unwrap_or_else(|| panic!("{joined}"));
    for stray in ["net", "target", "balance"] {
        assert!(
            !projects_footer.contains(stray),
            "{stray} beside the projects:\n{joined}"
        );
    }
    // Every panel keeps its frame: range, chart, projects, day types.
    assert_eq!(
        out.iter().filter(|r| r.starts_with('╭')).count(),
        4,
        "a panel lost its top border:\n{joined}"
    );
    assert!(
        out.iter().any(|r| r.contains("5 year")),
        "stats key hints missing:\n{joined}"
    );
}
