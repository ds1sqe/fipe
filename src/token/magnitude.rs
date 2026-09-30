use alloc::vec::Vec;
use core::{cmp::Ordering, fmt};

/// An exact natural number of any size: the magnitude of an integer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Magnitude {
    /// Base 2^64 digits, least significant first, with no trailing zero.
    limbs: Vec<u64>,
}

/// The most decimal digits a limb step takes: 10^19 < 2^64.
const STEP: usize = 19;

impl Magnitude {
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// The value, when it fits a `u64`.
    #[must_use]
    pub const fn to_u64(&self) -> Option<u64> {
        match self.limbs.as_slice() {
            [] => Some(0),
            [value] => Some(*value),
            _ => None,
        }
    }

    /// Whether a binary float with this precision and exponent range
    /// represents this integer exactly.
    #[must_use]
    pub fn fits_binary_float(&self, precision: usize, exponent: usize) -> bool {
        let Some(high) = self.limbs.last() else {
            return true;
        };
        let bits = (self.limbs.len() - 1) * 64
            + usize::try_from(64 - high.leading_zeros()).unwrap_or(64);
        let zeros = self.limbs.iter().take_while(|limb| **limb == 0).count();
        let trailing = zeros * 64
            + usize::try_from(self.limbs[zeros].trailing_zeros()).unwrap_or(0);
        bits <= exponent && bits - trailing <= precision
    }

    /// Digits of `radix` 2, 8, 10 or 16, underscores skipped. `None` for
    /// another radix or a character that is not such a digit. Linear in
    /// the digits for a power of two, quadratic for decimal.
    #[must_use]
    pub fn parse(text: &str, radix: u32) -> Option<Self> {
        let digits = text
            .chars()
            .filter(|&ch| ch != '_')
            .map(|ch| ch.to_digit(radix))
            .collect::<Option<Vec<u32>>>()?;
        match radix {
            2 | 8 | 16 => {
                Some(Self::from_bits(&digits, radix.trailing_zeros()))
            }
            10 => Some(Self::from_decimal(&digits)),
            _ => None,
        }
    }

    /// Each digit is `width` bits.
    fn from_bits(digits: &[u32], width: u32) -> Self {
        let mut limbs = Vec::new();
        let (mut limb, mut filled) = (0_u128, 0);
        for &digit in digits.iter().rev() {
            limb |= u128::from(digit) << filled;
            filled += width;
            if filled >= 64 {
                limbs.push(low(limb));
                limb >>= 64;
                filled -= 64;
            }
        }
        limbs.push(low(limb));
        Self::trimmed(limbs)
    }

    /// Nineteen digits per multiplication.
    fn from_decimal(digits: &[u32]) -> Self {
        let mut number = Self::default();
        let head = digits.len() % STEP;
        let chunks = core::iter::once(&digits[..head])
            .chain(digits[head..].chunks(STEP))
            .filter(|chunk| !chunk.is_empty());
        for chunk in chunks {
            let scale = 10_u64.pow(u32::try_from(chunk.len()).unwrap_or(0));
            let value =
                chunk.iter().fold(0, |n, digit| n * 10 + u64::from(*digit));
            number.multiply_add(scale, value);
        }
        number
    }

    /// `self * scale + add`.
    fn multiply_add(&mut self, scale: u64, add: u64) {
        let mut carry = u128::from(add);
        for limb in &mut self.limbs {
            let product = u128::from(*limb) * u128::from(scale) + carry;
            *limb = low(product);
            carry = product >> 64;
        }
        if carry != 0 {
            self.limbs.push(low(carry));
        }
    }

    /// `self / divisor`, returning the remainder.
    fn divide(&mut self, divisor: u64) -> u64 {
        let mut remainder = 0_u128;
        for limb in self.limbs.iter_mut().rev() {
            let value = remainder << 64 | u128::from(*limb);
            *limb = low(value / u128::from(divisor));
            remainder = value % u128::from(divisor);
        }
        self.limbs = Self::trimmed(core::mem::take(&mut self.limbs)).limbs;
        low(remainder)
    }

    fn trimmed(mut limbs: Vec<u64>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self { limbs }
    }
}

/// The low 64 bits.
fn low(value: u128) -> u64 {
    u64::try_from(value & u128::from(u64::MAX)).unwrap_or(u64::MAX)
}

impl From<u64> for Magnitude {
    fn from(value: u64) -> Self {
        Self::trimmed(alloc::vec![value])
    }
}

impl Ord for Magnitude {
    fn cmp(&self, other: &Self) -> Ordering {
        self.limbs
            .len()
            .cmp(&other.limbs.len())
            .then_with(|| self.limbs.iter().rev().cmp(other.limbs.iter().rev()))
    }
}

impl PartialOrd for Magnitude {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Decimal digits.
impl fmt::Display for Magnitude {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const CHUNK: u64 = 10_000_000_000_000_000_000;
        let mut rest = self.clone();
        let mut chunks = Vec::new();
        while !rest.is_zero() {
            chunks.push(rest.divide(CHUNK));
        }
        let Some((first, others)) = chunks.split_last() else {
            return f.write_str("0");
        };
        write!(f, "{first}")?;
        for chunk in others.iter().rev() {
            write!(f, "{chunk:019}")?;
        }
        Ok(())
    }
}
