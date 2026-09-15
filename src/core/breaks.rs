use super::Minutes;

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
}
