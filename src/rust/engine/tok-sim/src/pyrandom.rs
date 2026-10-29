//! CPython's `random.Random`, reproduced bit for bit.
//!
//! The core is the Mersenne Twister MT19937 of `Modules/_randommodule.c`,
//! seeded from an integer the way `random_seed` does it: the absolute value is
//! split into little-endian 32-bit words and passed to `init_by_array`. The
//! distribution methods follow `Lib/random.py` with the same operation order,
//! so results are identical to CPython 3.8 to 3.13 (the formulas used here did
//! not change in that range; 3.11 and 3.12 only added default arguments).
//!
//! `log`, `exp`, `sin` and `cos` come from the platform C library both in
//! CPython's `math` module and in Rust's `f64` methods. Results that pass
//! through them are identical to CPython running on the same platform, but can
//! differ in the last bit between platforms (for example the Windows UCRT and
//! glibc), for CPython itself as much as for this port. `random()`,
//! `uniform()`, `triangular()` and the integer core use only exactly rounded
//! IEEE operations and agree everywhere.

use thiserror::Error;

const N: usize = 624;
const M: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

/// `random.NV_MAGICCONST`, `4 * exp(-0.5) / sqrt(2.0)`.
pub const NV_MAGICCONST: f64 = 1.715_527_769_921_413_5;

/// `random.TWOPI`, `2.0 * pi`.
pub const TWOPI: f64 = std::f64::consts::TAU;

/// A seed that Python's `int()` would reject.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SeedError {
    /// `int(float)` of a NaN or an infinity raises in Python.
    #[error("seed {0} is not a finite number")]
    NotFinite(f64),
}

/// The state of a `random.Random` instance: the Mersenne Twister and the
/// second normal deviate cached by `gauss()`.
#[derive(Debug, Clone)]
pub struct PyRandom {
    mt: [u32; N],
    index: usize,
    gauss_next: Option<f64>,
}

impl PyRandom {
    /// `random.Random(seed)` for an integer seed.
    pub fn new(seed: i64) -> Self {
        let m = seed.unsigned_abs();
        Self::from_magnitude(&[m as u32, (m >> 32) as u32])
    }

    /// `random.Random(n)` where `n` is an integer of any size given by its
    /// absolute value as little-endian 32-bit words. Python seeds `n` and `-n`
    /// identically, so the sign is not needed.
    pub fn from_magnitude(words: &[u32]) -> Self {
        let mut rng = PyRandom {
            mt: [0; N],
            index: N,
            gauss_next: None,
        };
        rng.seed_magnitude(words);
        rng
    }

    /// `random.Random(int(x))`: the seed of `sim.py`, which is
    /// `int(float(text))`. The fractional part is truncated toward zero.
    pub fn from_float_seed(x: f64) -> Result<Self, SeedError> {
        Ok(Self::from_magnitude(&float_seed_words(x)?))
    }

    /// `rng.seed(n)` with `n` given as in [`PyRandom::from_magnitude`]. Also
    /// clears the value cached by [`PyRandom::gauss`].
    pub fn seed_magnitude(&mut self, words: &[u32]) {
        let used = words.iter().rposition(|w| *w != 0).map_or(1, |i| i + 1);
        let key = if words.is_empty() {
            &[0][..]
        } else {
            &words[..used]
        };
        self.init_by_array(key);
        self.gauss_next = None;
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..N {
            let prev = self.mt[i - 1];
            self.mt[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(i as u32);
        }
        self.index = N;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19_650_218);
        let mut i = 1;
        let mut j = 0;
        for _ in 0..N.max(key.len()) {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..N - 1 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941))
                .wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
        }
        self.mt[0] = 0x8000_0000;
    }

    fn twist(&mut self) {
        let mag = |y: u32| if y & 1 == 0 { 0 } else { MATRIX_A };
        for kk in 0..N - M {
            let y = (self.mt[kk] & UPPER_MASK) | (self.mt[kk + 1] & LOWER_MASK);
            self.mt[kk] = self.mt[kk + M] ^ (y >> 1) ^ mag(y);
        }
        for kk in N - M..N - 1 {
            let y = (self.mt[kk] & UPPER_MASK) | (self.mt[kk + 1] & LOWER_MASK);
            self.mt[kk] = self.mt[kk + M - N] ^ (y >> 1) ^ mag(y);
        }
        let y = (self.mt[N - 1] & UPPER_MASK) | (self.mt[0] & LOWER_MASK);
        self.mt[N - 1] = self.mt[M - 1] ^ (y >> 1) ^ mag(y);
        self.index = 0;
    }

    /// The next 32-bit output of the generator (`genrand_uint32`), equal to
    /// `rng.getrandbits(32)`.
    pub fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            self.twist();
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// `rng.random()`: a float in [0, 1) with 53 random bits (`genrand_res53`).
    pub fn random(&mut self) -> f64 {
        let a = self.next_u32() >> 5;
        let b = self.next_u32() >> 6;
        (f64::from(a) * 67_108_864.0 + f64::from(b)) * (1.0 / 9_007_199_254_740_992.0)
    }

    /// `rng.uniform(a, b)`.
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.random()
    }

    /// `rng.triangular(low, high, mode)`; `None` is Python's `mode=None`
    /// (the midpoint). When `high - low` is zero and a mode is given, Python
    /// catches the division by zero and returns `low`; the draw is consumed
    /// either way.
    pub fn triangular(&mut self, low: f64, high: f64, mode: Option<f64>) -> f64 {
        let mut u = self.random();
        let mut c = match mode {
            None => 0.5,
            Some(_) if high - low == 0.0 => return low,
            Some(m) => (m - low) / (high - low),
        };
        let (mut low, mut high) = (low, high);
        if u > c {
            u = 1.0 - u;
            c = 1.0 - c;
            std::mem::swap(&mut low, &mut high);
        }
        low + (high - low) * (u * c).sqrt()
    }

    /// `rng.normalvariate(mu, sigma)` (Kinderman and Monahan ratio of uniforms).
    pub fn normalvariate(&mut self, mu: f64, sigma: f64) -> f64 {
        loop {
            let u1 = self.random();
            let u2 = 1.0 - self.random();
            let z = NV_MAGICCONST * (u1 - 0.5) / u2;
            let zz = z * z / 4.0;
            if zz <= -u2.ln() {
                return mu + z * sigma;
            }
        }
    }

    /// `rng.gauss(mu, sigma)`. Each pair of draws yields two deviates; the
    /// second one is kept for the next call, as `gauss_next` in Python.
    pub fn gauss(&mut self, mu: f64, sigma: f64) -> f64 {
        let z = match self.gauss_next.take() {
            Some(z) => z,
            None => {
                let x2pi = self.random() * TWOPI;
                let g2rad = (-2.0 * (1.0 - self.random()).ln()).sqrt();
                self.gauss_next = Some(x2pi.sin() * g2rad);
                x2pi.cos() * g2rad
            }
        };
        mu + z * sigma
    }

    /// `rng.lognormvariate(mu, sigma)`. Where Python's `math.exp` raises
    /// `OverflowError`, this returns positive infinity.
    pub fn lognormvariate(&mut self, mu: f64, sigma: f64) -> f64 {
        self.normalvariate(mu, sigma).exp()
    }

    /// `rng.expovariate(lambd)`. `lambd` must be non-zero: Python raises
    /// `ZeroDivisionError` there (after consuming the draw), while this follows
    /// IEEE division.
    pub fn expovariate(&mut self, lambd: f64) -> f64 {
        -(1.0 - self.random()).ln() / lambd
    }
}

/// Little-endian 32-bit words of `abs(int(x))`.
fn float_seed_words(x: f64) -> Result<Vec<u32>, SeedError> {
    if !x.is_finite() {
        return Err(SeedError::NotFinite(x));
    }
    let v = x.trunc().abs();
    if v == 0.0 {
        return Ok(vec![0]);
    }
    // v >= 1 is a normal number: v = mantissa * 2^shift with a 53-bit mantissa.
    let bits = v.to_bits();
    let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let shift = ((bits >> 52) & 0x7ff) as i32 - 1075;
    if shift <= 0 {
        // The low -shift bits are zero because v is an integer.
        let m = mantissa >> shift.unsigned_abs();
        return Ok(vec![m as u32, (m >> 32) as u32]);
    }
    let shift = shift as usize;
    let wide = u128::from(mantissa) << (shift % 32);
    let mut words = vec![0u32; shift / 32];
    words.extend((0..4).map(|k| (wide >> (32 * k)) as u32));
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_their_definition() {
        assert_eq!(NV_MAGICCONST, 4.0 * (-0.5f64).exp() / 2.0f64.sqrt());
        assert_eq!(TWOPI, 2.0 * std::f64::consts::PI);
    }

    #[test]
    fn reference_outputs_of_mt19937() {
        // init_by_array({0x123, 0x234, 0x345, 0x456}) from the reference
        // mt19937ar.c test output.
        let mut rng = PyRandom::from_magnitude(&[0x123, 0x234, 0x345, 0x456]);
        let first: Vec<u32> = (0..5).map(|_| rng.next_u32()).collect();
        assert_eq!(
            first,
            [
                1_067_595_299,
                955_945_823,
                477_289_528,
                4_107_218_783,
                4_228_976_476
            ]
        );
    }

    #[test]
    fn seed_sign_and_width() {
        let a = PyRandom::new(-42).random();
        let b = PyRandom::new(42).random();
        assert_eq!(a.to_bits(), b.to_bits());
        let c = PyRandom::from_magnitude(&[42, 0, 0]).random();
        assert_eq!(b.to_bits(), c.to_bits());
        assert_eq!(PyRandom::new(0).random(), 0.844_421_851_525_048_1);
        assert_eq!(
            PyRandom::from_magnitude(&[]).random(),
            0.844_421_851_525_048_1
        );
    }

    #[test]
    fn float_seeds() {
        assert_eq!(float_seed_words(-7.9), Ok(vec![7, 0]));
        assert_eq!(float_seed_words(0.5), Ok(vec![0]));
        assert_eq!(float_seed_words(2f64.powi(70)), Ok(vec![0, 0, 64, 0]));
        assert!(PyRandom::from_float_seed(f64::NAN).is_err());
        assert!(PyRandom::from_float_seed(f64::NEG_INFINITY).is_err());
    }
}
