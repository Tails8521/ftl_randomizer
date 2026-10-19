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