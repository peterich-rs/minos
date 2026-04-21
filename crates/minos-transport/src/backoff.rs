//! Exponential backoff: 1s → 2s → 4s → 8s → 16s → 30s (capped).

use std::time::Duration;

const BASE: Duration = Duration::from_secs(1);
const CAP: Duration = Duration::from_secs(30);

#[must_use]
pub fn delay_for_attempt(attempt: u32) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let exp = u32::min(attempt - 1, 16); // avoid shift overflow
    let scaled = BASE.saturating_mul(1_u32 << exp);
    if scaled > CAP {
        CAP
    } else {
        scaled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(0, 0)]
    #[case(1, 1)]
    #[case(2, 2)]
    #[case(3, 4)]
    #[case(4, 8)]
    #[case(5, 16)]
    #[case(6, 30)]
    #[case(7, 30)]
    #[case(100, 30)]
    fn backoff_sequence(#[case] attempt: u32, #[case] expected_secs: u64) {
        assert_eq!(
            delay_for_attempt(attempt),
            Duration::from_secs(expected_secs)
        );
    }
}
