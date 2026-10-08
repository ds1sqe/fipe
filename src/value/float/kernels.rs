//! The functions `core` lacks without `std`, written over `f64` bits:
//! the correctly rounded square root, the length without overflow,
//! and msun's sine, cosine and arctangent kernels.

/// `mantissa * 2^exponent` as an `f64`, for a 53-bit mantissa with its
/// leading bit set and a normal result.
fn compose(mantissa: u64, exponent: i32) -> f64 {
    debug_assert_eq!(mantissa >> 52, 1);
    let biased = u64::try_from(exponent + 52 + 1023).expect("a normal result");
    debug_assert!((1..2047).contains(&biased));
    f64::from_bits((biased << 52) | (mantissa & ((1 << 52) - 1)))
}

/// The integer square root of a `u128`.
const fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = 1_u128 << ((128 - n.leading_zeros()).div_ceil(2));
    loop {
        let y = x.midpoint(n / x);
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// The correctly rounded square root: the exact root of the integer
/// mantissa scaled by an even power of two, rounded from its remainder.
///
/// # Panics
/// Never: every shift and exponent of a finite argument is in range.
#[must_use]
pub fn sqrt64(x: f64) -> f64 {
    if x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mut mantissa, mut exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    // Normalize to a 53-bit mantissa, then make the exponent even: the
    // root of m * 2^e is sqrt(m) * 2^(e/2). Scaling m by 2^60 gives an
    // integer root of 56 or 57 bits: 53 to keep, the rest and the
    // remainder deciding the rounding.
    while mantissa.leading_zeros() > 11 {
        mantissa <<= 1;
        exponent -= 1;
    }
    if exponent % 2 != 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    let scaled = u128::from(mantissa) << 60;
    let root = isqrt(scaled);
    let remainder = scaled - root * root;
    let width = 128 - root.leading_zeros();
    let drop = width - 53;
    let kept = u64::try_from(root >> drop).expect("fifty-three bits");
    let rest = root & ((1_u128 << drop) - 1);
    let half = 1_u128 << (drop - 1);
    let up = rest > half || (rest == half && (remainder > 0 || kept & 1 == 1));
    let mut kept = kept + u64::from(up);
    let mut result_exponent =
        exponent / 2 - 30 + i32::try_from(drop).expect("a small shift");
    if kept >> 53 == 1 {
        kept >>= 1;
        result_exponent += 1;
    }
    compose(kept, result_exponent)
}

/// The length without overflow or underflow: the squares split into
/// exact halves, as FreeBSD's msun does.
#[must_use]
pub fn hypot64(x: f64, y: f64) -> f64 {
    let (mut x, mut y) = (x.abs(), y.abs());
    if x < y {
        core::mem::swap(&mut x, &mut y);
    }
    if y == 0.0 {
        return x;
    }
    let (ex, ey) = (exponent(x), exponent(y));
    if ex - ey > 64 {
        return x + y;
    }
    let mut z = 1.0;
    if ex > 0x3ff + 510 {
        z = f64::from_bits((0x3ff_u64 + 700) << 52);
        let down = f64::from_bits((0x3ff_u64 - 700) << 52);
        x *= down;
        y *= down;
    } else if ey < 0x3ff - 450 {
        z = f64::from_bits((0x3ff_u64 - 700) << 52);
        let up = f64::from_bits((0x3ff_u64 + 700) << 52);
        x *= up;
        y *= up;
    }
    let (hx, lx) = square(x);
    let (hy, ly) = square(y);
    z * sqrt64(ly + lx + hy + hx)
}

/// `x * x` as a high part and the error of that rounding.
fn square(x: f64) -> (f64, f64) {
    let split = 134_217_729.0; // 2^27 + 1
    let xc = x * split;
    let xh = x - xc + xc;
    let xl = x - xh;
    let hi = x * x;
    let lo = xh * xh - hi + 2.0 * xh * xl + xl * xl;
    (hi, lo)
}

const PIO2_1: f64 = 1.570_796_326_734_125_6;
const PIO2_1T: f64 = 6.077_100_506_506_192e-11;
const PIO2_2: f64 = 6.077_100_506_303_966e-11;
const PIO2_2T: f64 = 2.022_266_248_795_950_6e-21;
const PIO2_3: f64 = 2.022_266_248_711_166_5e-21;
const PIO2_3T: f64 = 8.478_427_660_368_9e-32;
const INV_PIO2: f64 = core::f64::consts::FRAC_2_PI;

/// The argument reduced to `[-pi/4, pi/4]` as a head and a tail, with
/// the quadrant: a three-part `pi/2` as in msun, exact for arguments
/// below about `2^20 * pi/2`; beyond, the tail loses bits.
fn reduce(x: f64) -> (i32, f64, f64) {
    let quadrants =
        (x * INV_PIO2 + 6_755_399_441_055_744.0) - 6_755_399_441_055_744.0;
    let exponent_x = exponent(x);
    let mut rest = x - quadrants * PIO2_1;
    let mut tail = quadrants * PIO2_1T;
    let mut head = rest - tail;
    if exponent_x - exponent(head) > 16 {
        let kept = rest;
        tail = quadrants * PIO2_2;
        rest = kept - tail;
        tail = quadrants * PIO2_2T - ((kept - rest) - tail);
        head = rest - tail;
        if exponent_x - exponent(head) > 49 {
            let kept = rest;
            tail = quadrants * PIO2_3;
            rest = kept - tail;
            tail = quadrants * PIO2_3T - ((kept - rest) - tail);
            head = rest - tail;
        }
    }
    let low = (rest - head) - tail;
    (quadrant(quadrants), head, low)
}

/// The biased exponent of a float's bits.
fn exponent(x: f64) -> i32 {
    i32::try_from((x.to_bits() >> 52) & 0x7ff).expect("eleven bits")
}

/// The quadrant of a whole number of quarter turns.
fn quadrant(turns: f64) -> i32 {
    // The count is small and whole: the truncating cast is exact.
    #[allow(clippy::cast_possible_truncation)]
    let count = turns as i64;
    i32::try_from(count & 3).expect("two bits")
}

const S1: f64 = -0.166_666_666_666_666_32;
const S2: f64 = 0.008_333_333_333_322_49;
const S3: f64 = -0.000_198_412_698_298_579_5;
const S4: f64 = 2.755_731_370_707_006_8e-06;
const S5: f64 = -2.505_076_025_340_686_3e-08;
const S6: f64 = 1.589_690_995_211_55e-10;

/// The sine on `[-pi/4, pi/4]` of a head and tail, msun's kernel.
fn sin_kernel(head: f64, low: f64, tail: bool) -> f64 {
    let square = head * head;
    let fourth = square * square;
    let poly =
        S2 + square * (S3 + square * S4) + square * fourth * (S5 + square * S6);
    let cube = square * head;
    if tail {
        head - ((square * (0.5 * low - cube * poly) - low) - cube * S1)
    } else {
        head + cube * (S1 + square * poly)
    }
}

const C1: f64 = 0.041_666_666_666_666_6;
const C2: f64 = -0.001_388_888_888_887_411;
const C3: f64 = 2.480_158_728_947_673e-05;
const C4: f64 = -2.755_731_435_139_066_3e-07;
const C5: f64 = 2.087_572_321_298_175e-09;
const C6: f64 = -1.135_964_755_778_819_5e-11;

/// The cosine on `[-pi/4, pi/4]` of a head and tail, msun's kernel.
fn cos_kernel(head: f64, low: f64) -> f64 {
    let square = head * head;
    let fourth = square * square;
    let poly = square * (C1 + square * (C2 + square * C3))
        + fourth * fourth * (C4 + square * (C5 + square * C6));
    let half_square = 0.5 * square;
    let one_less = 1.0 - half_square;
    one_less + (((1.0 - one_less) - half_square) + (square * poly - head * low))
}

/// Below `pi/4` no reduction is needed; below `2^-26` the sine is `x`.
const PIO4: f64 = core::f64::consts::FRAC_PI_4;

#[must_use]
pub fn sin64(x: f64) -> f64 {
    if x.abs() <= PIO4 {
        if x.abs() < 1.490_116_119_384_765_6e-08 {
            return x;
        }
        return sin_kernel(x, 0.0, false);
    }
    let (n, y0, y1) = reduce(x);
    match n {
        0 => sin_kernel(y0, y1, true),
        1 => cos_kernel(y0, y1),
        2 => -sin_kernel(y0, y1, true),
        _ => -cos_kernel(y0, y1),
    }
}

#[must_use]
pub fn cos64(x: f64) -> f64 {
    if x.abs() <= PIO4 {
        if x.abs() < 1.490_116_119_384_765_6e-08 {
            return 1.0;
        }
        return cos_kernel(x, 0.0);
    }
    let (n, y0, y1) = reduce(x);
    match n {
        0 => cos_kernel(y0, y1),
        1 => -sin_kernel(y0, y1, true),
        2 => -cos_kernel(y0, y1),
        _ => sin_kernel(y0, y1, true),
    }
}

const ATAN_HI: [f64; 4] = [
    0.463_647_609_000_806_1,
    core::f64::consts::FRAC_PI_4,
    0.982_793_723_247_329,
    core::f64::consts::FRAC_PI_2,
];
const ATAN_LO: [f64; 4] = [
    2.269_877_745_296_168_7e-17,
    3.061_616_997_868_383e-17,
    1.390_331_103_123_099_8e-17,
    6.123_233_995_736_766e-17,
];
const AT: [f64; 11] = [
    0.333_333_333_333_329_3,
    -0.199_999_999_998_764_83,
    0.142_857_142_725_034_66,
    -0.111_111_104_054_623_56,
    0.090_908_871_334_365_07,
    -0.076_918_762_050_448_3,
    0.066_610_731_373_875_31,
    -0.058_335_701_337_905_735,
    0.049_768_779_946_159_324,
    -0.036_531_572_744_216_916,
    0.016_285_820_115_365_782,
];

/// The arctangent, msun's: the argument reduced to one of four
/// intervals, then a polynomial in its square.
#[must_use]
pub fn atan64(x: f64) -> f64 {
    let sign = x.is_sign_negative();
    let ax = x.abs();
    if ax >= 4_503_599_627_370_496.0 {
        // The half pi rounded up by a tiny amount, as msun writes it.
        let angle = ATAN_HI[3] + f64::from_bits((1023 - 120) << 52);
        return if sign { -angle } else { angle };
    }
    let (id, x) = if ax < 0.4375 {
        if ax < 7.450_580_596_923_828e-09 {
            return x;
        }
        (None, ax)
    } else if ax < 0.6875 {
        (Some(0), (2.0 * ax - 1.0) / (2.0 + ax))
    } else if ax < 1.1875 {
        (Some(1), (ax - 1.0) / (ax + 1.0))
    } else if ax < 2.4375 {
        (Some(2), (ax - 1.5) / (1.0 + 1.5 * ax))
    } else {
        (Some(3), -1.0 / ax)
    };
    let z = x * x;
    let w = z * z;
    let s1 = z
        * (AT[0]
            + w * (AT[2]
                + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let s2 = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
    let result = id.map_or_else(
        || x - x * (s1 + s2),
        |id| ATAN_HI[id] - ((x * (s1 + s2) - ATAN_LO[id]) - x),
    );
    if sign {
        -result
    } else {
        result
    }
}

const PI: f64 = core::f64::consts::PI;
const PI_LO: f64 = 1.224_646_799_147_353_2e-16;

/// The angle of the point `(x, y)`, msun's, on finite arguments.
#[must_use]
pub fn atan2_64(y: f64, x: f64) -> f64 {
    if x.to_bits() == 1.0_f64.to_bits() {
        return atan64(y);
    }
    let quadrant_bits =
        u8::from(y.is_sign_negative()) | (u8::from(x.is_sign_negative()) << 1);
    if y == 0.0 {
        return match quadrant_bits {
            0 | 1 => y,
            2 => PI,
            _ => -PI,
        };
    }
    if x == 0.0 {
        return if y.is_sign_negative() {
            -PI / 2.0
        } else {
            PI / 2.0
        };
    }
    let gap = exponent(y) - exponent(x);
    let angle = if gap > 60 {
        PI / 2.0 + 0.5 * PI_LO
    } else if x.is_sign_negative() && gap < -60 {
        0.0
    } else {
        atan64((y / x).abs())
    };
    match quadrant_bits {
        0 => angle,
        1 => -angle,
        2 => PI - (angle - PI_LO),
        _ => (angle - PI_LO) - PI,
    }
}
