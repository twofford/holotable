//! New York 2069 rules.
//!
//! M0 only holds check classification, as a first example of a pure, fully tested
//! rules function. The Tension pool and the GM's choices arrive in M6.

/// How hard a check is. The GM sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    VeryHard,
    NighImpossible,
}

impl Difficulty {
    /// The highest total that still fails. Partial success covers the next 10 points.
    pub const fn fail_max(self) -> i32 {
        match self {
            Difficulty::Easy => 5,
            Difficulty::Medium => 10,
            Difficulty::Hard => 15,
            Difficulty::VeryHard => 20,
            Difficulty::NighImpossible => 25,
        }
    }
}

/// The result of a check, before the GM picks what happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Failure,
    Partial,
    Full,
}

/// Classifies a check. `natural` is the d20 face; `total` is d20 + stat + skill + modifiers.
/// A natural 1 always fails.
pub fn classify(natural: u8, total: i32, difficulty: Difficulty) -> Outcome {
    let fail_max = difficulty.fail_max();
    if natural == 1 || total <= fail_max {
        Outcome::Failure
    } else if total <= fail_max + 10 {
        Outcome::Partial
    } else {
        Outcome::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Difficulty::*;
    use Outcome::*;

    #[test]
    fn boundaries_match_the_table() {
        // (difficulty, last failure, last partial)
        let table = [
            (Easy, 5, 15),
            (Medium, 10, 20),
            (Hard, 15, 25),
            (VeryHard, 20, 30),
            (NighImpossible, 25, 35),
        ];
        for (d, fail, partial) in table {
            assert_eq!(classify(10, fail, d), Failure, "{d:?} {fail}");
            assert_eq!(classify(10, fail + 1, d), Partial, "{d:?} {}", fail + 1);
            assert_eq!(classify(10, partial, d), Partial, "{d:?} {partial}");
            assert_eq!(classify(10, partial + 1, d), Full, "{d:?} {}", partial + 1);
        }
    }

    #[test]
    fn natural_one_always_fails() {
        assert_eq!(classify(1, 99, Easy), Failure);
    }
}
