use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

use super::{
    BreakTier, CoreError, Day, DayKind, Entry, HolidayCalendar, Minutes, minutes_of, recorded_gaps,
    session_deduction,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub daily_target: Minutes,
    pub tiers: Vec<BreakTier>,
    pub start_date: NaiveDate,
    pub initial_balance: Minutes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TodayCtx {
    pub today: NaiveDate,
    pub clocked_in: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayStats {
    pub date: NaiveDate,
    pub kind: DayKind,
    pub gross: Minutes,
    /// The day's recorded pauses, for information: what the user actually took
    /// off between entries. The deduction is charged per session, not from this.
    pub gaps: Minutes,
    pub deduction: Minutes,
    pub net: Minutes,
    pub target: Minutes,
    pub balance: Minutes,
    pub missing: bool,
    pub holiday_name: Option<String>,
    pub is_weekend: bool,
}

pub fn is_working_day(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

pub fn effective_kind(stored: Option<&DayKind>, date: NaiveDate, cal: &HolidayCalendar) -> DayKind {
    if let Some(k) = stored {
        return k.clone();
    }
    if cal.is_holiday(date) {
        DayKind::Holiday
    } else {
        DayKind::Work
    }
}

pub fn day_stats(day: &Day, rules: &Rules, cal: &HolidayCalendar, ctx: &TodayCtx) -> DayStats {
    let gross: Minutes = day.entries.iter().map(Entry::duration).sum();
    let gaps = recorded_gaps(&day.entries);
    let ded = session_deduction(&intervals_of(&day.entries), &rules.tiers);
    let net = Minutes((gross - ded).0.max(0));
    let is_weekend = !is_working_day(day.date);
    let is_past = day.date < ctx.today;
    let is_today = day.date == ctx.today;

    let target_applies = !is_weekend
        && day.kind.has_target()
        && (is_past || (is_today && !ctx.clocked_in && !day.entries.is_empty()));
    let target = if target_applies {
        rules.daily_target
    } else {
        Minutes::ZERO
    };

    let missing = is_past && !is_weekend && day.kind == DayKind::Work && day.entries.is_empty();

    DayStats {
        date: day.date,
        kind: day.kind.clone(),
        gross,
        gaps,
        deduction: ded,
        net,
        target,
        balance: net - target,
        missing,
        holiday_name: cal.holiday_name(day.date),
        is_weekend,
    }
}

pub fn running_balance<'a>(
    stats: impl IntoIterator<Item = &'a DayStats>,
    rules: &Rules,
) -> Minutes {
    rules.initial_balance
        + stats
            .into_iter()
            .filter(|s| s.date >= rules.start_date)
            .map(|s| s.balance)
            .sum::<Minutes>()
}

fn intervals_of(entries: &[Entry]) -> Vec<(i32, i32)> {
    entries.iter().map(Entry::interval).collect()
}

fn normalized(start: NaiveTime, end: NaiveTime) -> (i32, i32) {
    let s = minutes_of(start);
    let mut e = minutes_of(end);
    if e <= s {
        e += 1440;
    }
    (s, e)
}

pub fn check_overlap(
    existing: &[Entry],
    start: NaiveTime,
    end: NaiveTime,
    ignore_id: Option<i64>,
) -> Result<(), CoreError> {
    let (ns, ne) = normalized(start, end);
    for e in existing.iter().filter(|e| Some(e.id) != ignore_id) {
        let (os, oe) = e.interval();
        if ns < oe && ne > os {
            return Err(CoreError::Overlap);
        }
    }
    Ok(())
}

/// Minutes elapsed in a running session, as whole date-times.
///
/// Taking both ends as `NaiveDateTime` is what makes a session that started yesterday
/// come out right: a bare `NaiveTime` difference wraps at midnight and can express at
/// most 24 hours.
pub fn running_minutes(session_start: NaiveDateTime, now: NaiveDateTime) -> Minutes {
    Minutes((now - session_start).num_minutes().max(0) as i32)
}

/// Net for today if a session of `running` minutes, opened at `running_since`,
/// were closed right now.
///
/// The open session counts as one more interval, so it is grouped into sessions
/// like any entry: come back from lunch and the afternoon is already a session
/// of its own while the clock is still running, instead of the figures jumping
/// the moment the entry is written.
// NOTE: date-blind — a session opened yesterday reads as a gap against today's
// entries.
pub fn provisional_net_for(
    entries: &[Entry],
    running_since: NaiveTime,
    running: Minutes,
    rules: &Rules,
) -> Minutes {
    let gross = entries.iter().map(Entry::duration).sum::<Minutes>() + running;
    let mut ivs = intervals_of(entries);
    let s = minutes_of(running_since);
    ivs.push((s, s + running.0));
    let ded = session_deduction(&ivs, &rules.tiers);
    Minutes((gross - ded).0.max(0))
}

/// Net for today if the running clock-in were closed right now.
///
/// Same-day form kept for callers that only hold clock times; prefer
/// [`provisional_net_for`] together with [`running_minutes`], which also handles a
/// session that started on an earlier date.
pub fn provisional_net(
    entries: &[Entry],
    running_since: NaiveTime,
    now: NaiveTime,
    rules: &Rules,
) -> Minutes {
    let (s, e) = normalized(running_since, now);
    let running = if now == running_since {
        Minutes::ZERO
    } else {
        Minutes(e - s)
    };
    provisional_net_for(entries, running_since, running, rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::default_tiers;
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    fn rules() -> Rules {
        Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            start_date: d(2026, 9, 1),
            initial_balance: Minutes(60),
        }
    }
    fn cal() -> HolidayCalendar {
        HolidayCalendar::new(vec![])
    }
    fn entry(id: i64, date: NaiveDate, s: NaiveTime, e: NaiveTime) -> Entry {
        Entry {
            id,
            date,
            start: s,
            end: e,
            project: "Alpha".into(),
            comment: String::new(),
        }
    }
    fn work(date: NaiveDate, entries: Vec<Entry>) -> Day {
        Day {
            date,
            kind: DayKind::Work,
            entries,
        }
    }
    fn ctx(today: NaiveDate, clocked_in: bool) -> TodayCtx {
        TodayCtx { today, clocked_in }
    }

    #[test]
    fn working_days_are_mon_to_fri() {
        assert!(is_working_day(d(2026, 9, 14))); // Mon
        assert!(is_working_day(d(2026, 9, 18))); // Fri
        assert!(!is_working_day(d(2026, 9, 19))); // Sat
        assert!(!is_working_day(d(2026, 9, 20))); // Sun
    }

    #[test]
    fn effective_kind_prefers_stored_then_holiday_then_work() {
        let c = cal();
        let xmas = d(2026, 12, 25);
        assert_eq!(effective_kind(None, xmas, &c), DayKind::Holiday);
        assert_eq!(
            effective_kind(Some(&DayKind::Work), xmas, &c),
            DayKind::Work
        );
        assert_eq!(effective_kind(None, d(2026, 9, 15), &c), DayKind::Work);
    }

    #[test]
    fn full_past_work_day() {
        let day = work(
            d(2026, 9, 14),
            vec![entry(1, d(2026, 9, 14), t(8, 0), t(17, 0))],
        );
        let s = day_stats(&day, &rules(), &cal(), &ctx(d(2026, 9, 15), false));
        assert_eq!(s.gross, Minutes(540));
        assert_eq!(s.deduction, Minutes(48));
        assert_eq!(s.net, Minutes(492));
        assert_eq!(s.target, Minutes(468));
        assert_eq!(s.balance, Minutes(24));
        assert!(!s.missing);
    }

    #[test]
    fn deduction_is_per_session_not_per_entry() {
        let day = work(
            d(2026, 9, 14),
            vec![
                entry(1, d(2026, 9, 14), t(8, 0), t(11, 0)),
                entry(2, d(2026, 9, 14), t(11, 0), t(14, 0)),
            ],
        );
        let s = day_stats(&day, &rules(), &cal(), &ctx(d(2026, 9, 15), false));
        assert_eq!(s.gross, Minutes(360));
        assert_eq!(s.deduction, Minutes(18));
    }

    #[test]
    fn past_weekday_without_entries_is_missing() {
        let s = day_stats(
            &work(d(2026, 9, 14), vec![]),
            &rules(),
            &cal(),
            &ctx(d(2026, 9, 15), false),
        );
        assert!(s.missing);
        assert_eq!(s.balance, Minutes(-468));
    }

    #[test]
    fn weekend_has_no_target_and_is_not_missing() {
        let s = day_stats(
            &work(d(2026, 9, 12), vec![]),
            &rules(),
            &cal(),
            &ctx(d(2026, 9, 15), false),
        );
        assert!(s.is_weekend);
        assert!(!s.missing);
        assert_eq!(s.balance, Minutes(0));
        let s2 = day_stats(
            &work(
                d(2026, 9, 12),
                vec![entry(1, d(2026, 9, 12), t(10, 0), t(12, 0))],
            ),
            &rules(),
            &cal(),
            &ctx(d(2026, 9, 15), false),
        );
        assert_eq!(s2.balance, Minutes(120));
    }

    #[test]
    fn kinds() {
        let r = rules();
        let c = cal();
        let cx = ctx(d(2026, 9, 15), false);
        let mk = |kind| Day {
            date: d(2026, 9, 14),
            kind,
            entries: vec![],
        };
        assert_eq!(
            day_stats(&mk(DayKind::Vacation), &r, &c, &cx).balance,
            Minutes(0)
        );
        assert_eq!(
            day_stats(&mk(DayKind::Sick), &r, &c, &cx).balance,
            Minutes(0)
        );
        assert_eq!(
            day_stats(
                &mk(DayKind::Absence {
                    label: "trip".into()
                }),
                &r,
                &c,
                &cx
            )
            .balance,
            Minutes(0)
        );
        assert_eq!(
            day_stats(&mk(DayKind::Flex), &r, &c, &cx).balance,
            Minutes(-468)
        );
        let hol = Day {
            date: d(2026, 12, 25),
            kind: DayKind::Holiday,
            entries: vec![],
        };
        let hs = day_stats(&hol, &r, &c, &cx);
        assert_eq!(hs.balance, Minutes(0));
        assert_eq!(hs.holiday_name.as_deref(), Some("1. Weihnachtstag"));
        assert!(!day_stats(&mk(DayKind::Vacation), &r, &c, &cx).missing);
    }

    #[test]
    fn today_rules() {
        let today = d(2026, 9, 15);
        let r = rules();
        let c = cal();
        // no entries, not clocked in: no target, not missing
        let s = day_stats(&work(today, vec![]), &r, &c, &ctx(today, false));
        assert_eq!(s.target, Minutes(0));
        assert!(!s.missing);
        // one entry, not clocked in: target applies
        let s = day_stats(
            &work(today, vec![entry(1, today, t(8, 0), t(12, 0))]),
            &r,
            &c,
            &ctx(today, false),
        );
        assert_eq!(s.target, Minutes(468));
        assert_eq!(s.balance, Minutes(240 - 18 - 468));
        // one entry, clocked in: target withheld
        let s = day_stats(
            &work(today, vec![entry(1, today, t(8, 0), t(12, 0))]),
            &r,
            &c,
            &ctx(today, true),
        );
        assert_eq!(s.target, Minutes(0));
        // future day: nothing
        let s = day_stats(&work(d(2026, 9, 16), vec![]), &r, &c, &ctx(today, false));
        assert_eq!(s.target, Minutes(0));
        assert!(!s.missing);
    }

    #[test]
    fn running_balance_sums_from_initial() {
        let r = rules();
        let c = cal();
        let cx = ctx(d(2026, 9, 16), false);
        let days = [
            work(
                d(2026, 9, 14),
                vec![entry(1, d(2026, 9, 14), t(8, 0), t(17, 0))],
            ), // +24
            work(d(2026, 9, 15), vec![]), // -468
        ];
        let stats: Vec<DayStats> = days.iter().map(|dd| day_stats(dd, &r, &c, &cx)).collect();
        assert_eq!(running_balance(&stats, &r), Minutes(60 + 24 - 468));
    }

    #[test]
    fn overlap_detection() {
        let ex = vec![
            entry(1, d(2026, 9, 14), t(9, 0), t(12, 0)),
            entry(2, d(2026, 9, 14), t(22, 0), t(2, 0)), // crosses midnight
        ];
        assert!(check_overlap(&ex, t(12, 0), t(13, 0), None).is_ok()); // touching is fine
        assert_eq!(
            check_overlap(&ex, t(11, 0), t(13, 0), None),
            Err(CoreError::Overlap)
        );
        assert_eq!(
            check_overlap(&ex, t(23, 0), t(23, 30), None),
            Err(CoreError::Overlap)
        );
        assert!(check_overlap(&ex, t(11, 0), t(13, 0), Some(1)).is_ok()); // editing entry 1
        assert_eq!(
            check_overlap(&ex, t(12, 0), t(12, 0), None),
            Err(CoreError::Overlap)
        ); // 24h span
    }

    #[test]
    fn provisional_net_includes_running_entry() {
        let today = d(2026, 9, 15);
        let ex = vec![entry(1, today, t(8, 0), t(12, 0))]; // 240
        // Running since 13:00, now 15:30 → +150 → gross 390. The hour of lunch
        // splits the day: the 4:00 morning loses 18, the 2:30 so far loses
        // nothing.
        assert_eq!(
            provisional_net(&ex, t(13, 0), t(15, 30), &rules()),
            Minutes(390 - 18)
        );
    }

    #[test]
    fn running_minutes_spans_midnight() {
        let start = NaiveDateTime::new(d(2026, 9, 14), t(23, 0));
        let now = NaiveDateTime::new(d(2026, 9, 15), t(1, 0));
        assert_eq!(running_minutes(start, now), Minutes(120));
        // A clock still on its first minute, and a clock skew backwards, both clamp at 0.
        assert_eq!(running_minutes(start, start), Minutes::ZERO);
        assert_eq!(
            running_minutes(start, NaiveDateTime::new(d(2026, 9, 14), t(22, 0))),
            Minutes::ZERO
        );
        // More than a day is expressible, unlike a bare `NaiveTime` difference.
        assert_eq!(
            running_minutes(start, NaiveDateTime::new(d(2026, 9, 16), t(0, 0))),
            Minutes(1500)
        );
        // …and it feeds the provisional net unchanged.
        let ex = vec![entry(1, d(2026, 9, 14), t(8, 0), t(12, 0))]; // 240
        assert_eq!(
            provisional_net_for(&ex, t(23, 0), running_minutes(start, now), &rules()),
            // Two sessions: the 4:00 morning loses 18, the two hours since 23:00
            // lose nothing.
            Minutes(240 + 120 - 18)
        );
    }

    #[test]
    fn the_deduction_is_charged_per_seamless_session() {
        let day = d(2026, 9, 14);
        let mk = |entries| {
            day_stats(
                &work(day, entries),
                &rules(),
                &cal(),
                &ctx(d(2026, 9, 15), false),
            )
        };
        // Seamless: a project switch at noon is not a pause, so the whole nine
        // hours are one session and lose the top tier.
        let s = mk(vec![
            entry(1, day, t(8, 0), t(12, 0)),
            entry(2, day, t(12, 0), t(17, 0)),
        ]);
        assert_eq!(s.gaps, Minutes::ZERO);
        assert_eq!(s.deduction, Minutes(48));
        assert_eq!(s.net, Minutes(492)); // 8:12
        // A 45-minute lunch splits the day into 4:00 and 4:15; each is over three
        // hours, so each loses 18 minutes.
        let s = mk(vec![
            entry(1, day, t(8, 0), t(12, 0)),
            entry(2, day, t(12, 45), t(17, 0)),
        ]);
        assert_eq!(s.gaps, Minutes(45));
        assert_eq!(s.deduction, Minutes(36));
        assert_eq!(s.net, Minutes(459)); // 7:39
        // A single minute off is already a new session: 6:30 loses 48, 2:29 loses
        // nothing.
        let s = mk(vec![
            entry(1, day, t(8, 0), t(14, 30)),
            entry(2, day, t(14, 31), t(17, 0)),
        ]);
        assert_eq!(s.deduction, Minutes(48));
        assert_eq!(s.net, Minutes(539 - 48));
        // Three short sessions: only the four-hour one is over a tier.
        let s = mk(vec![
            entry(1, day, t(8, 0), t(10, 0)),
            entry(2, day, t(10, 30), t(12, 0)),
            entry(3, day, t(13, 0), t(17, 0)),
        ]);
        assert_eq!(s.deduction, Minutes(18));
        assert_eq!(s.net, Minutes(450 - 18));
        // One entry is one session, and it keeps its deduction.
        let s = mk(vec![entry(1, day, t(8, 0), t(17, 0))]);
        assert_eq!(s.gaps, Minutes::ZERO);
        assert_eq!(s.deduction, Minutes(48));
        // Overlapping entries never stopped the clock: still one session.
        let s = mk(vec![
            entry(1, day, t(8, 0), t(12, 0)),
            entry(2, day, t(11, 0), t(17, 0)),
        ]);
        assert_eq!(s.gaps, Minutes::ZERO);
        assert_eq!(s.deduction, Minutes(48));
        // A day without entries is not charged.
        assert_eq!(mk(vec![]).deduction, Minutes::ZERO);
        // An entry over midnight is one session of its normalised length: 22:00
        // to 05:00 is seven hours.
        let s = mk(vec![entry(1, day, t(22, 0), t(5, 0))]);
        assert_eq!(s.gross, Minutes(420));
        assert_eq!(s.deduction, Minutes(48));
    }

    #[test]
    fn the_running_session_is_a_session_of_its_own() {
        let today = d(2026, 9, 15);
        let ex = vec![entry(1, today, t(8, 0), t(12, 0))]; // 240
        // Back at 12:45 after a 45-minute lunch, now 17:00: 240 + 255 = 495 gross
        // in two sessions, each over three hours, so 18 + 18 comes off.
        assert_eq!(
            provisional_net_for(&ex, t(12, 45), Minutes(255), &rules()),
            Minutes(495 - 36)
        );
        // Straight back at 12:00: one seamless nine-hour session, top tier.
        assert_eq!(
            provisional_net_for(&ex, t(12, 0), Minutes(300), &rules()),
            Minutes(540 - 48)
        );
        // A session still on its first minute is no interval at all: it touches
        // the morning and leaves it a four-hour session.
        assert_eq!(
            provisional_net_for(&ex, t(12, 0), Minutes::ZERO, &rules()),
            Minutes(240 - 18)
        );
    }
}
