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

/// The seamless working sessions of a day, as the indices of the intervals each
/// one is made of.
///
/// Intervals are taken in start order and merged while the next one begins at or
/// before the end reached so far: a project switch at noon, an overlap and an
/// interval nested in a longer one all stay inside the same session. Any gap of
/// a minute or more starts the next session. This is the one place the grouping
/// is decided; [`sessions`] and [`entry_nets`] both read it, so the span a
/// session is charged on and the entries that carry the charge can never drift
/// apart.
pub fn session_members(intervals: &[(i32, i32)]) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..intervals.len()).collect();
    order.sort_by_key(|&i| intervals[i].0);
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut end = 0;
    for i in order {
        let (s, e) = intervals[i];
        match out.last_mut() {
            Some(last) if s <= end => {
                last.push(i);
                end = end.max(e);
            }
            _ => {
                out.push(vec![i]);
                end = e;
            }
        }
    }
    out
}

/// The seamless working sessions of a day as `(start, end)` spans.
///
/// The grouping is [`session_members`]; a session's span runs from the earliest
/// start to the furthest end of the intervals in it.
pub fn sessions(intervals: &[(i32, i32)]) -> Vec<(i32, i32)> {
    session_members(intervals)
        .iter()
        .map(|g| {
            let s = g.iter().map(|&i| intervals[i].0).min().unwrap_or(0);
            let e = g.iter().map(|&i| intervals[i].1).max().unwrap_or(0);
            (s, e)
        })
        .collect()
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

/// The net minutes of every entry of a day, in the order the entries come in.
///
/// A session's deduction (see [`session_deduction`]) is not a property of any one
/// entry of it, so it is shared out over the entries in proportion to their gross
/// length, rounded to whole minutes by the largest-remainder method: the shares
/// add up to the session's deduction exactly, and the entries' nets to the day's
/// net. An entry's net is its gross minus its share. A session can never take
/// more off than was worked in it, so no share is negative or larger than the
/// entry it belongs to.
pub fn entry_nets(entries: &[Entry], tiers: &[BreakTier]) -> Vec<Minutes> {
    let intervals: Vec<(i32, i32)> = entries.iter().map(Entry::interval).collect();
    let mut nets: Vec<Minutes> = intervals.iter().map(|(s, e)| Minutes(e - s)).collect();
    for group in session_members(&intervals) {
        let span = {
            let s = group.iter().map(|&i| intervals[i].0).min().unwrap_or(0);
            let e = group.iter().map(|&i| intervals[i].1).max().unwrap_or(0);
            e - s
        };
        // Overlapping entries can sum to more than the session spans, so the cap
        // is what the entries hold, not the span the tier was read from.
        let gross: i64 = group.iter().map(|&i| nets[i].0 as i64).sum();
        let ded = (deduction(Minutes(span), tiers).0 as i64).min(gross).max(0);
        if gross == 0 || ded == 0 {
            continue;
        }
        // Whole-minute shares first, then the minutes rounding left over go to
        // the largest remainders — ties to the earlier entry.
        let mut shares: Vec<i64> = group
            .iter()
            .map(|&i| ded * nets[i].0 as i64 / gross)
            .collect();
        let mut rank: Vec<usize> = (0..group.len()).collect();
        rank.sort_by_key(|&k| {
            let rem = ded * nets[group[k]].0 as i64 % gross;
            (std::cmp::Reverse(rem), k)
        });
        let left = ded - shares.iter().sum::<i64>();
        for &k in rank.iter().take(left.max(0) as usize) {
            shares[k] += 1;
        }
        for (k, &i) in group.iter().enumerate() {
            nets[i] = Minutes(nets[i].0 - shares[k] as i32);
        }
    }
    nets
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
    fn session_members_group_the_indices_the_sessions_merge() {
        // The grouping is the one `sessions` merges by, kept as indices into the
        // caller's slice so a share can be handed back to the entry it came from.
        assert_eq!(
            session_members(&[(480, 720), (720, 1020)]),
            vec![vec![0, 1]]
        );
        // Sorted by start, so the indices come out in clock order, not slice order.
        assert_eq!(
            session_members(&[(660, 1020), (480, 720)]),
            vec![vec![1, 0]]
        );
        assert_eq!(
            session_members(&[(480, 720), (721, 1020)]),
            vec![vec![0], vec![1]]
        );
        assert_eq!(
            session_members(&[(480, 1020), (540, 600), (1080, 1200)]),
            vec![vec![0, 1], vec![2]]
        );
        assert_eq!(session_members(&[]), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn a_session_deduction_is_split_over_its_entries_by_gross() {
        let t = default_tiers();
        // 08–12 and 12–17 are one nine-hour session losing 48 minutes. The first
        // entry's share is 48·240/540 = 21.33 → 21, the second's 26.67 → 27: the
        // largest remainder takes the odd minute. Nets 3:39 and 4:33 add up to
        // the day's 8:12.
        let day = [e(1, (8, 0), (12, 0)), e(2, (12, 0), (17, 0))];
        assert_eq!(entry_nets(&day, &t), vec![Minutes(219), Minutes(273)]);
        assert_eq!(
            entry_nets(&day, &t).iter().copied().sum::<Minutes>(),
            Minutes(492)
        );
        // Stored out of order, the shares still line up with the entries.
        let day = [e(2, (12, 0), (17, 0)), e(1, (8, 0), (12, 0))];
        assert_eq!(entry_nets(&day, &t), vec![Minutes(273), Minutes(219)]);
    }

    #[test]
    fn every_session_is_split_on_its_own() {
        let t = default_tiers();
        // One entry is one session, and it carries the whole deduction.
        assert_eq!(entry_nets(&[e(1, (8, 0), (17, 0))], &t), vec![Minutes(492)]);
        // A 45-minute lunch makes two sessions of 4:00 and 4:15; each loses 18
        // minutes, and with one entry apiece there is nothing to share out.
        let day = [e(1, (8, 0), (12, 0)), e(2, (12, 45), (17, 0))];
        assert_eq!(entry_nets(&day, &t), vec![Minutes(222), Minutes(237)]);
        // Three entries, a switch inside the afternoon session: only that
        // session's 18 minutes are split, 9 and 9 on two equal halves.
        let day = [
            e(1, (8, 0), (11, 0)),
            e(2, (12, 0), (14, 0)),
            e(3, (14, 0), (16, 0)),
        ];
        assert_eq!(
            entry_nets(&day, &t),
            vec![Minutes(180), Minutes(111), Minutes(111)]
        );
    }

    #[test]
    fn no_deduction_leaves_the_gross_untouched() {
        let day = [e(1, (8, 0), (11, 0)), e(2, (12, 0), (15, 0))];
        // Two sessions of exactly three hours: neither is over the first tier.
        assert_eq!(
            entry_nets(&day, &default_tiers()),
            vec![Minutes(180), Minutes(180)]
        );
        assert_eq!(entry_nets(&day, &[]), vec![Minutes(180), Minutes(180)]);
        assert_eq!(entry_nets(&[], &default_tiers()), Vec::<Minutes>::new());
    }

    #[test]
    fn a_share_is_never_negative_nor_larger_than_the_entry() {
        // A hand-written tier table may deduct more than a session is long; the
        // session's own gross caps what its entries can lose, so no net goes
        // below zero.
        let t = vec![BreakTier {
            after: Minutes(60),
            deduct: Minutes(1000),
        }];
        let day = [e(1, (8, 0), (10, 0)), e(2, (10, 0), (11, 0))];
        let nets = entry_nets(&day, &t);
        assert_eq!(nets, vec![Minutes::ZERO, Minutes::ZERO]);
        for (net, entry) in nets.iter().zip(day.iter()) {
            assert!(net.0 >= 0, "{net:?}");
            assert!(*net <= entry.duration(), "{net:?}");
        }
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
