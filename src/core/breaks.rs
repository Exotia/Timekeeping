use super::{Entry, Minutes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakTier {
    pub after: Minutes,
    pub deduct: Minutes,
}

pub fn default_tiers() -> Vec<BreakTier> {
    vec![
        BreakTier {
            after: Minutes(180),
            deduct: Minutes(18),
        },
        BreakTier {
            after: Minutes(360),
            deduct: Minutes(48),
        },
    ]
}

/// Deduction of the tier with the largest `after` that is strictly below `gross`.
pub fn deduction(gross: Minutes, tiers: &[BreakTier]) -> Minutes {
    tiers
        .iter()
        .filter(|t| t.after < gross)
        .max_by_key(|t| t.after)
        .map(|t| t.deduct)
        .unwrap_or(Minutes::ZERO)
}

/// Total of the pauses between consecutive entries, in the order the clock ran.
///
/// Entries are taken as normalised intervals (an entry that ends at or before it
/// starts runs past midnight), sorted by start; the pause before an entry that
/// starts while the previous one is still running is 0, never negative.
pub fn recorded_gaps(entries: &[Entry]) -> Minutes {
    gaps_between(&entries.iter().map(Entry::interval).collect::<Vec<_>>())
}

/// [`recorded_gaps`] for callers that hold intervals rather than entries — the
/// provisional net of a day whose last "entry" is the session still running.
///
/// Each pause is measured against the furthest end reached so far, not against
/// the previous interval by start: an interval nested inside a longer one leaves
/// no pause behind, and the pause after it starts where the longer one ended.
pub fn gaps_between(intervals: &[(i32, i32)]) -> Minutes {
    let mut iv = intervals.to_vec();
    iv.sort_by_key(|(s, _)| *s);
    let mut gaps = 0;
    let mut max_end = match iv.first() {
        Some((_, e)) => *e,
        None => return Minutes::ZERO,
    };
    for (s, e) in iv.iter().skip(1) {
        gaps += (s - max_end).max(0);
        max_end = max_end.max(*e);
    }
    Minutes(gaps)
}

/// The seamless working sessions of a day.
///
/// Intervals are sorted by start and merged while the next one begins at or
/// before the end reached so far: a project switch at noon, an overlap and an
/// interval nested in a longer one all stay inside the same session. Any gap of
/// a minute or more starts the next session.
pub fn sessions(intervals: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut iv = intervals.to_vec();
    iv.sort_by_key(|(s, _)| *s);
    let mut out: Vec<(i32, i32)> = Vec::with_capacity(iv.len());
    for (s, e) in iv {
        match out.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// The break deduction of a whole day: the tier deduction of every seamless
/// session on its own length, summed.
///
/// The statutory break belongs to the stretch actually worked without stopping,
/// so a real pause splits the day and each part is judged on its own: two four-
/// hour halves are charged twice for being over three hours, while the same
/// hours worked straight through are charged once for being over six. A day
/// without entries has no session and is not charged.
pub fn session_deduction(intervals: &[(i32, i32)], tiers: &[BreakTier]) -> Minutes {
    sessions(intervals)
        .iter()
        .map(|(s, e)| deduction(Minutes(e - s), tiers))
        .sum()
}

/// The break deduction of a whole day: none once the day's recorded pauses reach
/// `break_gap`, the tier deduction for `gross` otherwise.
///
/// A recorded pause is the user's own break, so the law is already satisfied and
/// `tk` takes nothing off. A `break_gap` of 0 therefore switches the automatic
/// deduction off altogether: a day with no pause at all still has `gaps >=
/// break_gap`.
pub fn day_deduction(
    gross: Minutes,
    gaps: Minutes,
    tiers: &[BreakTier],
    break_gap: Minutes,
) -> Minutes {
    if gaps >= break_gap {
        Minutes::ZERO
    } else {
        deduction(gross, tiers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_are_strict() {
        let t = default_tiers();
        assert_eq!(deduction(Minutes(0), &t), Minutes(0));
        assert_eq!(deduction(Minutes(180), &t), Minutes(0)); // exactly 3h: not over
        assert_eq!(deduction(Minutes(181), &t), Minutes(18));
        assert_eq!(deduction(Minutes(360), &t), Minutes(18)); // exactly 6h: still tier 1
        assert_eq!(deduction(Minutes(361), &t), Minutes(48));
        assert_eq!(deduction(Minutes(600), &t), Minutes(48));
    }

    #[test]
    fn no_tiers_no_deduction() {
        assert_eq!(deduction(Minutes(999), &[]), Minutes(0));
    }

    #[test]
    fn unsorted_tiers_still_pick_highest_matching() {
        let t = vec![
            BreakTier {
                after: Minutes(360),
                deduct: Minutes(48),
            },
            BreakTier {
                after: Minutes(180),
                deduct: Minutes(18),
            },
        ];
        assert_eq!(deduction(Minutes(400), &t), Minutes(48));
        assert_eq!(deduction(Minutes(200), &t), Minutes(18));
    }

    fn e(id: i64, s: (u32, u32), en: (u32, u32)) -> Entry {
        Entry {
            id,
            date: chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap(),
            start: chrono::NaiveTime::from_hms_opt(s.0, s.1, 0).unwrap(),
            end: chrono::NaiveTime::from_hms_opt(en.0, en.1, 0).unwrap(),
            project: "Alpha".into(),
            comment: String::new(),
        }
    }

    #[test]
    fn gaps_of_a_seamless_day_are_zero() {
        // A project switch at noon leaves no pause behind.
        let day = [e(1, (8, 0), (12, 0)), e(2, (12, 0), (17, 0))];
        assert_eq!(recorded_gaps(&day), Minutes::ZERO);
        assert_eq!(recorded_gaps(&[e(1, (8, 0), (17, 0))]), Minutes::ZERO);
        assert_eq!(recorded_gaps(&[]), Minutes::ZERO);
    }

    #[test]
    fn gaps_sum_the_pauses_between_consecutive_entries() {
        let day = [e(1, (8, 0), (12, 0)), e(2, (12, 45), (17, 0))];
        assert_eq!(recorded_gaps(&day), Minutes(45));
        // Three entries, two pauses of a quarter of an hour each.
        let day = [
            e(1, (8, 0), (10, 0)),
            e(2, (10, 15), (12, 0)),
            e(3, (12, 15), (17, 0)),
        ];
        assert_eq!(recorded_gaps(&day), Minutes(30));
    }

    #[test]
    fn gaps_sort_by_start_and_never_go_negative() {
        // Stored out of order: the pause is between 12:00 and 12:45 either way.
        let day = [e(2, (12, 45), (17, 0)), e(1, (8, 0), (12, 0))];
        assert_eq!(recorded_gaps(&day), Minutes(45));
        // Overlapping entries are not a pause of negative length.
        let day = [e(1, (8, 0), (12, 0)), e(2, (11, 0), (17, 0))];
        assert_eq!(recorded_gaps(&day), Minutes::ZERO);
        // An entry that crosses midnight is normalised past 24:00, so the evening
        // pause before it counts and the wrap does not.
        let day = [e(1, (8, 0), (12, 0)), e(2, (22, 0), (2, 0))];
        assert_eq!(recorded_gaps(&day), Minutes(600));
    }

    #[test]
    fn an_interval_nested_in_another_is_no_pause() {
        // 08:00–17:00 with 09:00–10:00 and 11:00–12:00 inside it: whatever those
        // overlapping intervals mean, the clock never stopped, so there is no
        // break to credit. Reachable through `provisional_net_for`, which adds
        // the running session to the day's entries without an overlap check.
        assert_eq!(
            gaps_between(&[(480, 1020), (540, 600), (660, 720)]),
            Minutes::ZERO
        );
        // And a real pause after the nested pair is measured from the furthest
        // end reached so far, not from the end of the last interval by start.
        assert_eq!(
            gaps_between(&[(480, 1020), (540, 600), (1080, 1200)]),
            Minutes(60)
        );
    }

    #[test]
    fn a_long_enough_pause_cancels_the_deduction() {
        let t = default_tiers();
        let gross = Minutes(540);
        // No pause, or one that is too short: the tier applies as before.
        assert_eq!(
            day_deduction(gross, Minutes::ZERO, &t, Minutes(30)),
            Minutes(48)
        );
        assert_eq!(
            day_deduction(gross, Minutes(29), &t, Minutes(30)),
            Minutes(48)
        );
        // Exactly the threshold already counts as a real break.
        assert_eq!(
            day_deduction(gross, Minutes(30), &t, Minutes(30)),
            Minutes::ZERO
        );
        assert_eq!(
            day_deduction(gross, Minutes(45), &t, Minutes(30)),
            Minutes::ZERO
        );
    }

    #[test]
    fn a_threshold_of_zero_cancels_every_day() {
        // `break_gap = 0` is "trust me, I take my breaks": even a day without a
        // single recorded pause satisfies `gaps >= break_gap`.
        let t = default_tiers();
        assert_eq!(
            day_deduction(Minutes(540), Minutes::ZERO, &t, Minutes::ZERO),
            Minutes::ZERO
        );
    }

    #[test]
    fn sessions_merge_what_the_clock_never_stopped_between() {
        // A project switch at noon leaves no gap: one seamless session.
        assert_eq!(sessions(&[(480, 720), (720, 1020)]), vec![(480, 1020)]);
        // A single minute off already starts the next session.
        assert_eq!(
            sessions(&[(480, 720), (721, 1020)]),
            vec![(480, 720), (721, 1020)]
        );
        // Overlapping intervals are one session, whatever order they arrive in…
        assert_eq!(sessions(&[(660, 1020), (480, 720)]), vec![(480, 1020)]);
        // …and one nested inside another neither splits it nor shortens it.
        assert_eq!(sessions(&[(480, 1020), (540, 600)]), vec![(480, 1020)]);
        assert_eq!(
            sessions(&[(480, 1020), (540, 600), (1080, 1200)]),
            vec![(480, 1020), (1080, 1200)]
        );
        assert_eq!(sessions(&[]), Vec::<(i32, i32)>::new());
    }

    #[test]
    fn session_deduction_charges_each_session_on_its_own_length() {
        let t = default_tiers();
        // 08–12 and 12–17: one nine-hour session, the top tier.
        assert_eq!(
            session_deduction(&[(480, 720), (720, 1020)], &t),
            Minutes(48)
        );
        // 08–12 and 12:45–17: 4h and 4:15, each over three hours.
        assert_eq!(
            session_deduction(&[(480, 720), (765, 1020)], &t),
            Minutes(36)
        );
        // 08–14:30 and 14:31–17: 6:30 is over six hours, 2:29 is under three.
        assert_eq!(
            session_deduction(&[(480, 870), (871, 1020)], &t),
            Minutes(48)
        );
        // Two short sessions and one of four hours.
        assert_eq!(
            session_deduction(&[(480, 600), (630, 720), (780, 1020)], &t),
            Minutes(18)
        );
        // A day without entries is not charged.
        assert_eq!(session_deduction(&[], &t), Minutes::ZERO);
    }
}
