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
