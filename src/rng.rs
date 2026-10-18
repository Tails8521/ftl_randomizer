//! The random generator of the whole randomizer, so one seed replays a run.
//!
//! xorshift64* is a dozen lines and good enough for this library's purpose.
//! Reproducibility needs two things: draws in the same order, and walks over
//! ordered collections only (`Vec`, `BTreeMap`, slices). A `HashMap` walk hands
//! out its entries in an order that changes between runs, which is enough to
//! make the same seed produce something else every time.

/// The random state of one run.
pub struct Rng(u64);

impl Rng {
    /// A state from a seed. Zero is not a state xorshift can use, so seed 0
    /// gets a fixed one instead.
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 { 0x2545_F491_4F6C_DD1D } else { seed })
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0 .. limit`.
    pub fn below(&mut self, limit: u64) -> u64 {
        self.next_u64() % limit
    }

    /// An index in `0 .. len`.
    pub fn index(&mut self, len: usize) -> usize {
        self.below(len as u64) as usize
    }

    /// The index of an entry drawn by weight, each weight being its share of the
    /// total. Weights may be in thousandths, percent or any other common unit.
    /// At least one weight has to be above zero.
    pub fn pick(&mut self, weights: &[u32]) -> usize {
        let total: u64 = weights.iter().map(|&weight| weight as u64).sum();
        let mut roll = self.below(total);
        let mut index = 0;
        while roll >= weights[index] as u64 {
            roll -= weights[index] as u64;
            index += 1;
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn draw(seed: u64) -> Vec<u64> {
        let mut rng = Rng::new(seed);
        (0..1000).map(|_| rng.next_u64()).collect()
    }

    #[test]
    fn a_seed_repeats_itself() {
        let one = draw(12345);
        assert_eq!(one, draw(12345));
        assert_ne!(one, draw(54321));
        assert_eq!(one.iter().collect::<BTreeSet<_>>().len(), one.len(), "no repeats");
    }

    /// Seed 0 is not a usable xorshift state, so it gets a fixed one.
    #[test]
    fn seed_zero_still_randoms() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u64(), 0);
        assert_ne!(rng.next_u64(), 0);
    }

    #[test]
    fn a_seed_below_the_limit() {
        let mut rng = Rng::new(7);
        for limit in [1, 2, 3, 4, 8, 12, 1000] {
            for _ in 0..200 {
                assert!(rng.below(limit) < limit, "{limit}");
            }
        }
        // every value of a small limit does show up
        let mut seen = [false; 4];
        for _ in 0..500 {
            seen[rng.index(4)] = true;
        }
        assert!(seen.iter().all(|&hit| hit));
    }

    #[test]
    fn pick_follows_the_weights() {
        let mut rng = Rng::new(3);
        for _ in 0..500 {
            assert_eq!(rng.pick(&[0, 500]), 1, "a zero weight is never picked");
            assert_eq!(rng.pick(&[500, 0]), 0);
        }

        let mut counts = [0usize; 3];
        for _ in 0..30_000 {
            counts[rng.pick(&[100, 300, 600])] += 1;
        }
        for (index, count) in counts.iter().enumerate() {
            let share = 100.0 * *count as f64 / 30_000.0;
            let expected = [10.0, 30.0, 60.0][index];
            assert!(
                (share - expected).abs() < 2.0,
                "entry {index} came up {share:.1}%, expected {expected}%"
            );
        }
    }
}