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
/// A session's deduction (see [`session_deduction`]) is not a property of any
/// one entry of it, so it has to be handed to one. Two rules decide that, in
/// this order:
///
/// 1. **Explicit shares.** An entry whose `break_share` is `Some` pays that
///    many minutes, capped by its own gross. If the explicit shares of a
///    session come to more than the session's deduction, they are scaled down
///    in proportion to each other, to whole minutes by the largest-remainder
///    method (ties to the earlier entry), so they add up to the deduction
///    exactly and nothing is left for the unassigned entries.
/// 2. **The default: the project worked last.** What the deduction still needs
///    falls on the *last unassigned* entry of the session as far as its gross
///    allows, then on the one before it, and so on. This is where a break
///    actually lands: you stop working, and the project you were on when you
///    stopped pays for it. If every entry is assigned and the explicit shares
///    fall short, the remainder walks the same way over the session's entries
///    from the last one backwards — an explicit share is a floor in that case,
///    not a ceiling, because the day's net may not disagree with the day's
///    deduction.
///
/// A session can never take more off than was worked in it, so the shares of a
/// session always add up to its deduction, no share is larger than the entry it
/// belongs to, and no net is negative.
pub fn entry_nets(entries: &[Entry], tiers: &[BreakTier]) -> Vec<Minutes> {
    let intervals: Vec<(i32, i32)> = entries.iter().map(Entry::interval).collect();
    let gross: Vec<i32> = intervals.iter().map(|(s, e)| e - s).collect();
    let mut nets: Vec<Minutes> = gross.iter().map(|g| Minutes(*g)).collect();
    for group in session_members(&intervals) {
        let span = {
            let s = group.iter().map(|&i| intervals[i].0).min().unwrap_or(0);
            let e = group.iter().map(|&i| intervals[i].1).max().unwrap_or(0);
            e - s
        };
        // Overlapping entries can sum to more than the session spans, so the cap
        // is what the entries hold, not the span the tier was read from.
        let capacity: i32 = group.iter().map(|&i| gross[i]).sum();
        let ded = deduction(Minutes(span), tiers).0.clamp(0, capacity);
        if ded == 0 {
            continue;
        }
        let members: Vec<(i32, Option<i32>)> = group
            .iter()
            .map(|&i| (gross[i], entries[i].break_share.map(|m| m.0)))
            .collect();
        for (k, share) in session_shares(&members, ded).into_iter().enumerate() {
            nets[group[k]] = Minutes(gross[group[k]] - share);
        }
    }
    nets
}

/// The share of `ded` every member of one session pays, aligned with `group`.
///
/// `group` is in clock order (see [`session_members`]); the rules are the ones
/// [`entry_nets`] documents.
fn session_shares(group: &[(i32, Option<i32>)], ded: i32) -> Vec<i32> {
    let mut shares = vec![0; group.len()];
    let explicit: Vec<Option<i32>> = group
        .iter()
        .map(|(g, s)| s.map(|s| s.clamp(0, *g)))
        .collect();
    let assigned: i32 = explicit.iter().flatten().sum();
    if assigned > ded {
        // More assigned than there is to give: scale the explicit shares down to
        // the deduction, whole minutes by the largest remainder, ties earliest.
        for (k, e) in explicit.iter().enumerate() {
            if let Some(e) = e {
                shares[k] = (ded as i64 * *e as i64 / assigned as i64) as i32;
            }
        }
        let mut rank: Vec<usize> = (0..group.len())
            .filter(|&k| explicit[k].is_some())
            .collect();
        rank.sort_by_key(|&k| {
            let rem = ded as i64 * explicit[k].unwrap_or(0) as i64 % assigned as i64;
            (std::cmp::Reverse(rem), k)
        });
        let left = ded - shares.iter().sum::<i32>();
        for &k in rank.iter().take(left.max(0) as usize) {
            shares[k] += 1;
        }
        return shares;
    }
    for (k, e) in explicit.iter().enumerate() {
        shares[k] = e.unwrap_or(0);
    }
    let mut left = ded - assigned;
    // The default rule: backwards from the last unassigned entry…
    for k in (0..group.len()).rev() {
        if left == 0 {
            break;
        }
        if explicit[k].is_none() {
            let take = left.min(group[k].0);
            shares[k] += take;
            left -= take;
        }
    }
    // …and, once those are full, backwards over the assigned ones too, because
    // the session's shares have to come to its deduction.
    for k in (0..group.len()).rev() {
        if left == 0 {
            break;
        }
        let take = left.min(group[k].0 - shares[k]);
        shares[k] += take;
        left -= take;
    }
    shares
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
            break_share: None,
        }
    }

    /// The same entry with an explicit break share of `share` minutes.
    fn share(id: i64, s: (u32, u32), en: (u32, u32), share: i32) -> Entry {
        Entry {
            break_share: Some(Minutes(share)),
            ..e(id, s, en)
        }
    }

    /// Every entry's share, i.e. what `entry_nets` took off it.
    fn shares(entries: &[Entry], tiers: &[BreakTier]) -> Vec<Minutes> {
        entry_nets(entries, tiers)
            .iter()
            .zip(entries)
            .map(|(net, e)| e.duration() - *net)
            .collect()
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
    fn the_deduction_falls_on_the_last_entry_of_the_session() {
        let t = default_tiers();
        // 08–12 and 12–17 are one nine-hour session losing 48 minutes. Nobody
        // assigned a share, so the project worked last carries the whole break:
        // the morning keeps its 4:00 and the afternoon nets 4:12 — 8:12 together.
        let day = [e(1, (8, 0), (12, 0)), e(2, (12, 0), (17, 0))];
        assert_eq!(entry_nets(&day, &t), vec![Minutes(240), Minutes(252)]);
        assert_eq!(
            entry_nets(&day, &t).iter().copied().sum::<Minutes>(),
            Minutes(492)
        );
        // Stored out of order, the shares still line up with the entries: "last"
        // is last by the clock, not by position in the slice.
        let day = [e(2, (12, 0), (17, 0)), e(1, (8, 0), (12, 0))];
        assert_eq!(entry_nets(&day, &t), vec![Minutes(252), Minutes(240)]);
    }

    #[test]
    fn the_default_share_spills_backwards_when_the_last_entry_is_too_short() {
        let t = default_tiers();
        // 08–14 straight on to a ten-minute tail: one session of 6:10, losing 48.
        // The tail can only give the ten minutes it has; the other 38 come off
        // the entry before it.
        let day = [e(1, (8, 0), (14, 0)), e(2, (14, 0), (14, 10))];
        assert_eq!(shares(&day, &t), vec![Minutes(38), Minutes(10)]);
        assert_eq!(entry_nets(&day, &t), vec![Minutes(322), Minutes::ZERO]);
    }

    #[test]
    fn an_explicit_share_is_taken_before_the_default() {
        let t = default_tiers();
        // Half an hour put on the morning: the remaining 18 minutes of the 48
        // still fall on the last entry.
        let day = [share(1, (8, 0), (12, 0), 30), e(2, (12, 0), (17, 0))];
        assert_eq!(shares(&day, &t), vec![Minutes(30), Minutes(18)]);
        assert_eq!(entry_nets(&day, &t), vec![Minutes(210), Minutes(282)]);
        // A share larger than the entry it sits on is capped by that entry's
        // gross, and the rest goes back to the default rule.
        let day = [share(1, (8, 0), (8, 20), 60), e(2, (8, 20), (17, 0))];
        assert_eq!(shares(&day, &t), vec![Minutes(20), Minutes(28)]);
        // An explicit zero is a share, not an absence of one: it keeps the whole
        // deduction off that entry.
        let day = [share(1, (8, 0), (12, 0), 0), e(2, (12, 0), (17, 0))];
        assert_eq!(shares(&day, &t), vec![Minutes::ZERO, Minutes(48)]);
    }

    #[test]
    fn explicit_shares_short_of_the_deduction_are_a_floor() {
        let t = default_tiers();
        // 30 + 10 = 40 of the 48: the eight minutes left over go on the last
        // entry, on top of the ten it was given. An explicit share is a floor
        // once every entry of the session has one, not a ceiling.
        let day = [
            share(1, (8, 0), (12, 0), 30),
            share(2, (12, 0), (17, 0), 10),
        ];
        assert_eq!(shares(&day, &t), vec![Minutes(30), Minutes(18)]);
        assert_eq!(entry_nets(&day, &t), vec![Minutes(210), Minutes(282)]);
        // And the shortfall walks backwards when the last entry is full: the
        // tail is given its whole ten minutes, so the remaining 38 land on the
        // morning even though that was assigned too.
        let day = [
            share(1, (8, 0), (14, 0), 0),
            share(2, (14, 0), (14, 10), 10),
        ];
        assert_eq!(shares(&day, &t), vec![Minutes(38), Minutes(10)]);
    }

    #[test]
    fn over_assigned_shares_are_scaled_down_proportionally() {
        let t = default_tiers();
        // An hour put on each half of a session that only loses 48 minutes: the
        // shares are scaled to what there is to give, 24 and 24.
        let day = [
            share(1, (8, 0), (12, 0), 60),
            share(2, (12, 0), (17, 0), 60),
        ];
        assert_eq!(shares(&day, &t), vec![Minutes(24), Minutes(24)]);
        assert_eq!(entry_nets(&day, &t), vec![Minutes(216), Minutes(276)]);
        // 25 + 26 = 51 over the 48: scaled to 23.52 and 24.47, and the odd
        // minute goes to the largest remainder — the first entry here.
        let day = [
            share(1, (8, 0), (12, 0), 25),
            share(2, (12, 0), (17, 0), 26),
        ];
        assert_eq!(shares(&day, &t), vec![Minutes(24), Minutes(24)]);
        // Nothing is left for an unassigned entry of an over-assigned session.
        let day = [
            share(1, (8, 0), (12, 0), 60),
            share(2, (12, 0), (16, 0), 60),
            e(3, (16, 0), (17, 0)),
        ];
        assert_eq!(
            shares(&day, &t),
            vec![Minutes(24), Minutes(24), Minutes::ZERO]
        );
    }

    #[test]
    fn only_the_middle_entry_of_a_session_can_be_assigned() {
        let t = default_tiers();
        // 08–11, 11–13, 13–17: one eight-hour session losing 48. The middle
        // entry is given 20, and the 28 left over fall on the last one.
        let day = [
            e(1, (8, 0), (11, 0)),
            share(2, (11, 0), (13, 0), 20),
            e(3, (13, 0), (17, 0)),
        ];
        assert_eq!(
            shares(&day, &t),
            vec![Minutes::ZERO, Minutes(20), Minutes(28)]
        );
        assert_eq!(
            entry_nets(&day, &t),
            vec![Minutes(180), Minutes(100), Minutes(212)]
        );
    }

    #[test]
    fn a_share_only_ever_belongs_to_its_own_session() {
        let t = default_tiers();
        // A 45-minute lunch: two sessions of 4:00 and 4:15, each losing 18. A
        // share put on the morning cannot pay for the afternoon's break.
        let day = [
            share(1, (8, 0), (12, 0), 18),
            e(2, (12, 45), (15, 0)),
            e(3, (15, 0), (17, 0)),
        ];
        assert_eq!(
            shares(&day, &t),
            vec![Minutes(18), Minutes::ZERO, Minutes(18)]
        );
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
        // session is charged, and its 18 minutes fall on the entry it ended on.
        let day = [
            e(1, (8, 0), (11, 0)),
            e(2, (12, 0), (14, 0)),
            e(3, (14, 0), (16, 0)),
        ];
        assert_eq!(
            entry_nets(&day, &t),
            vec![Minutes(180), Minutes(120), Minutes(102)]
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
