//! The `Math` transcendental functions, ported from V8's `ieee754.cc` (itself
//! Sun's fdlibm 5.3 plus a few FreeBSD msun routines).
//!
//! The platform libm is not a substitute: Rust's `f64::sin` and friends call
//! the OS `libm`, whose results differ from V8's in the last digit for a few
//! percent of inputs, and they differ between macOS and Linux. V8 carries its
//! own copy so `Math.cos(0.1)` is the same double everywhere; this module is
//! that copy.
//!
//! **Contraction.** Node built by clang for AArch64 compiles each fdlibm
//! expression with `-ffp-contract=on`: within ONE C expression, `a*b + c` and
//! `c - a*b` become a single fused multiply-add (`llvm.fmuladd`, which the
//! AArch64 backend lowers to `fmadd`/`fmsub`). The reference `node` on the
//! primary development machine therefore returns different last digits from an
//! unfused evaluation of the same source — measured: `Math.atan` differs on
//! 4 of 3600 samples unfused and on 0 fused. Every expression below is written
//! with `mul_add` exactly where clang fuses it (the multiply is the left
//! operand if there is one, else the right; statements are never fused across a
//! `;`), so the results are bit-identical to that node. `mul_add` is a true
//! fused operation on every target, so the results are also identical across
//! architectures.

use std::f64::consts::PI;

/// 64-bit view helpers for the fdlibm `GET_HIGH_WORD`/`SET_LOW_WORD` family.
fn hi(x: f64) -> i32 {
    (x.to_bits() >> 32) as i32
}

fn lo(x: f64) -> u32 {
    x.to_bits() as u32
}

fn from_words(hi: i32, lo: u32) -> f64 {
    f64::from_bits(((hi as u32 as u64) << 32) | lo as u64)
}

fn with_hi(x: f64, hi: i32) -> f64 {
    from_words(hi, lo(x))
}

fn with_lo(x: f64, lo: u32) -> f64 {
    from_words(self::hi(x), lo)
}

fn fma(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

const TWO24: f64 = 16777216.0;
const TWON24: f64 = 5.960464477539063e-8;

/// `2/pi` in 24-bit chunks (fdlibm `two_over_pi`).
const TWO_OVER_PI: [i32; 66] = [
    0xA2F983, 0x6E4E44, 0x1529FC, 0x2757D1, 0xF534DD, 0xC0DB62, 0x95993C, 0x439041, 0xFE5163,
    0xABDEBB, 0xC561B7, 0x246E3A, 0x424DD2, 0xE00649, 0x2EEA09, 0xD1921C, 0xFE1DEB, 0x1CB129,
    0xA73EE8, 0x8235F5, 0x2EBB44, 0x84E99C, 0x7026B4, 0x5F7E41, 0x3991D6, 0x398353, 0x39F49C,
    0x845F8B, 0xBDF928, 0x3B1FF8, 0x97FFDE, 0x05980F, 0xEF2F11, 0x8B5A0A, 0x6D1F6D, 0x367ECF,
    0x27CB09, 0xB74F46, 0x3F669E, 0x5FEA2D, 0x7527BA, 0xC7EBE5, 0xF17B3D, 0x0739F7, 0x8A5292,
    0xEA6BFB, 0x5FB11F, 0x8D5D08, 0x560330, 0x46FC7B, 0x6BABF0, 0xCFBC20, 0x9AF436, 0x1DA9E3,
    0x91615E, 0xE61B08, 0x659985, 0x5F14A0, 0x68408D, 0xFFD880, 0x4D7327, 0x310606, 0x1556CA,
    0x73A8C9, 0x60E27B, 0xC08C6B,
];

/// `pi/2` in 24-bit chunks (fdlibm `PIo2`).
const PIO2: [f64; 8] = [
    1.570_796_251_296_997,
    7.549_789_415_861_596e-8,
    5.390_302_529_957_765e-15,
    3.282_003_415_807_913e-22,
    1.270_655_753_080_676e-29,
    1.229_333_089_811_113_3e-36,
    2.733_700_538_164_645_6e-44,
    2.167_416_838_778_048_2e-51,
];

/// fdlibm `__kernel_rem_pio2` for double precision (`prec == 2`, `jk == 4`):
/// `y = x - n*pi/2` for a huge `x` given as 24-bit chunks `tx` with scaled
/// exponent `e0`. Returns `n & 7`.
fn kernel_rem_pio2(tx: &[f64; 3], y: &mut [f64; 2], e0: i32, nx: usize) -> i32 {
    const JK: usize = 4;
    let jp = JK;
    let mut iq = [0i32; 20];
    let mut f = [0f64; 20];
    let mut fq = [0f64; 20];
    let mut q = [0f64; 20];

    let jx = nx - 1;
    let jv = ((e0 - 3) / 24).max(0) as usize;
    let mut q0 = e0 - 24 * (jv as i32 + 1);

    let first = jv as i32 - jx as i32;
    for (k, fi) in f.iter_mut().take(jx + JK + 1).enumerate() {
        let j = first + k as i32;
        *fi = if j < 0 {
            0.0
        } else {
            TWO_OVER_PI[j as usize] as f64
        };
    }
    for i in 0..=JK {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw = fma(tx[j], f[jx + i - j], fw);
        }
        q[i] = fw;
    }

    let mut jz = JK;
    let mut z;
    let mut n;
    let mut ih;
    loop {
        // Distill q[] into iq[] reversingly.
        z = q[jz];
        for (i, j) in (1..=jz).rev().enumerate() {
            let fw = (TWON24 * z) as i32 as f64;
            iq[i] = fma(-TWO24, fw, z) as i32;
            z = q[j - 1] + fw;
        }
        // Compute n.
        z = scalbn(z, q0);
        z = fma(-8.0, (z * 0.125).floor(), z);
        n = z as i32;
        z -= n as f64;
        ih = 0;
        if q0 > 0 {
            let i = iq[jz - 1] >> (24 - q0);
            n += i;
            iq[jz - 1] -= i << (24 - q0);
            ih = iq[jz - 1] >> (23 - q0);
        } else if q0 == 0 {
            ih = iq[jz - 1] >> 23;
        } else if z >= 0.5 {
            ih = 2;
        }
        if ih > 0 {
            n += 1;
            let mut carry = 0;
            for e in iq.iter_mut().take(jz) {
                let j = *e;
                if carry == 0 {
                    if j != 0 {
                        carry = 1;
                        *e = 0x1000000 - j;
                    }
                } else {
                    *e = 0xffffff - j;
                }
            }
            if q0 > 0 {
                match q0 {
                    1 => iq[jz - 1] &= 0x7fffff,
                    2 => iq[jz - 1] &= 0x3fffff,
                    _ => {}
                }
            }
            if ih == 2 {
                z = 1.0 - z;
                if carry != 0 {
                    z -= scalbn(1.0, q0);
                }
            }
        }
        // Recompute when the leading terms cancelled completely.
        if z == 0.0 {
            let mut j = 0;
            for e in iq.iter().take(jz).skip(JK) {
                j |= *e;
            }
            if j == 0 {
                let mut k = 1;
                while iq[JK - k] == 0 {
                    k += 1;
                }
                for i in jz + 1..=jz + k {
                    f[jx + i] = TWO_OVER_PI[jv + i] as f64;
                    let mut fw = 0.0;
                    for j in 0..=jx {
                        fw = fma(tx[j], f[jx + i - j], fw);
                    }
                    q[i] = fw;
                }
                jz += k;
                continue;
            }
        }
        break;
    }

    // Chop off zero terms.
    if z == 0.0 {
        jz -= 1;
        q0 -= 24;
        while iq[jz] == 0 {
            jz -= 1;
            q0 -= 24;
        }
    } else {
        z = scalbn(z, -q0);
        if z >= TWO24 {
            let fw = (TWON24 * z) as i32 as f64;
            iq[jz] = fma(-TWO24, fw, z) as i32;
            jz += 1;
            q0 += 24;
            iq[jz] = fw as i32;
        } else {
            iq[jz] = z as i32;
        }
    }
    // Convert the integer "bit" chunks to floating point.
    let mut fw = scalbn(1.0, q0);
    for i in (0..=jz).rev() {
        q[i] = fw * iq[i] as f64;
        fw *= TWON24;
    }
    // Compute PIo2[0..jp] * q[jz..0].
    for i in (0..=jz).rev() {
        let mut fw = 0.0;
        let mut k = 0;
        while k <= jp && k <= jz - i {
            fw = fma(PIO2[k], q[i + k], fw);
            k += 1;
        }
        fq[jz - i] = fw;
    }
    // Compress fq[] into y[].
    let mut fw = 0.0;
    for i in (0..=jz).rev() {
        fw += fq[i];
    }
    y[0] = if ih == 0 { fw } else { -fw };
    fw = fq[0] - fw;
    for e in fq.iter().take(jz + 1).skip(1) {
        fw += *e;
    }
    y[1] = if ih == 0 { fw } else { -fw };
    n & 7
}

/// `x * 2^n` (fdlibm `scalbn`), exact for every `n` that keeps the result a
/// normal number or rounds once into the subnormal range.
fn scalbn(x: f64, n: i32) -> f64 {
    let p1023 = f64::from_bits(0x7fe0_0000_0000_0000);
    let m1022_p53 = f64::from_bits(0x0010_0000_0000_0000) * f64::from_bits(0x4340_0000_0000_0000);
    let mut x = x;
    let mut n = n;
    if n > 1023 {
        x *= p1023;
        n -= 1023;
        if n > 1023 {
            x *= p1023;
            n = (n - 1023).min(1023);
        }
    } else if n < -1022 {
        x *= m1022_p53;
        n += 1022 - 53;
        if n < -1022 {
            x *= m1022_p53;
            n = (n + 1022 - 53).max(-1022);
        }
    }
    x * f64::from_bits(((0x3ff + n) as u64) << 52)
}

/// High words of `n * pi/2`, `n = 1..=32` (fdlibm `npio2_hw`).
const NPIO2_HW: [i32; 32] = [
    0x3FF921FB, 0x400921FB, 0x4012D97C, 0x401921FB, 0x401F6A7A, 0x4022D97C, 0x4025FDBB, 0x402921FB,
    0x402C463A, 0x402F6A7A, 0x4031475C, 0x4032D97C, 0x40346B9C, 0x4035FDBB, 0x40378FDB, 0x403921FB,
    0x403AB41B, 0x403C463A, 0x403DD85A, 0x403F6A7A, 0x40407E4C, 0x404147AC, 0x4042106C, 0x4042D97C,
    0x4043A28C, 0x40446B9C, 0x404534AC, 0x4045FDBB, 0x4046C6CB, 0x40478FDB, 0x404858EB, 0x404921FB,
];

/// fdlibm `__ieee754_rem_pio2`: reduce `x` to `y[0] + y[1] = x - n*pi/2`
/// with `|y| <= pi/4`; returns `n`.
fn rem_pio2(x: f64, y: &mut [f64; 2]) -> i32 {
    const INVPIO2: f64 = std::f64::consts::FRAC_2_PI;
    const PIO2_1: f64 = 1.570_796_326_734_125_6;
    const PIO2_1T: f64 = 6.077_100_506_506_192e-11;
    const PIO2_2: f64 = 6.077_100_506_303_966e-11;
    const PIO2_2T: f64 = 2.022_266_248_795_950_6e-21;
    const PIO2_3: f64 = 2.022_266_248_711_166_5e-21;
    const PIO2_3T: f64 = 8.478_427_660_368_9e-32;

    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix <= 0x3fe921fb {
        y[0] = x;
        y[1] = 0.0;
        return 0;
    }
    if ix < 0x4002d97c {
        // |x| < 3pi/4, special case with n = +-1.
        if hx > 0 {
            let mut z = x - PIO2_1;
            if ix != 0x3ff921fb {
                y[0] = z - PIO2_1T;
                y[1] = (z - y[0]) - PIO2_1T;
            } else {
                z -= PIO2_2;
                y[0] = z - PIO2_2T;
                y[1] = (z - y[0]) - PIO2_2T;
            }
            return 1;
        }
        let mut z = x + PIO2_1;
        if ix != 0x3ff921fb {
            y[0] = z + PIO2_1T;
            y[1] = (z - y[0]) + PIO2_1T;
        } else {
            z += PIO2_2;
            y[0] = z + PIO2_2T;
            y[1] = (z - y[0]) + PIO2_2T;
        }
        return -1;
    }
    if ix <= 0x413921fb {
        // |x| <= 2^19 * (pi/2), medium size.
        let t = x.abs();
        let n = fma(t, INVPIO2, 0.5) as i32;
        let fn_ = n as f64;
        let mut r = fma(-fn_, PIO2_1, t);
        let mut w = fn_ * PIO2_1T;
        if n < 32 && ix != NPIO2_HW[(n - 1) as usize] {
            y[0] = r - w;
        } else {
            let j = ix >> 20;
            y[0] = r - w;
            let mut i = j - ((hi(y[0]) >> 20) & 0x7ff);
            if i > 16 {
                let t = r;
                w = fn_ * PIO2_2;
                r = t - w;
                w = fma(fn_, PIO2_2T, -((t - r) - w));
                y[0] = r - w;
                i = j - ((hi(y[0]) >> 20) & 0x7ff);
                if i > 49 {
                    let t = r;
                    w = fn_ * PIO2_3;
                    r = t - w;
                    w = fma(fn_, PIO2_3T, -((t - r) - w));
                    y[0] = r - w;
                }
            }
        }
        y[1] = (r - y[0]) - w;
        if hx < 0 {
            y[0] = -y[0];
            y[1] = -y[1];
            return -n;
        }
        return n;
    }
    if ix >= 0x7ff00000 {
        y[0] = f64::NAN;
        y[1] = y[0];
        return 0;
    }
    // Large arguments: split |x| into three 24-bit chunks.
    let e0 = (ix >> 20) - 1046;
    let mut z = from_words(ix - (e0 << 20), lo(x));
    let mut tx = [0f64; 3];
    for t in tx.iter_mut().take(2) {
        *t = z as i32 as f64;
        z = (z - *t) * TWO24;
    }
    tx[2] = z;
    let mut nx = 3;
    while tx[nx - 1] == 0.0 {
        nx -= 1;
    }
    let n = kernel_rem_pio2(&tx, y, e0, nx);
    if hx < 0 {
        y[0] = -y[0];
        y[1] = -y[1];
        return -n;
    }
    n
}

fn kernel_sin(x: f64, y: f64, iy: i32) -> f64 {
    const S1: f64 = -1.666_666_666_666_663_2e-1;
    const S2: f64 = 8.333_333_333_322_49e-3;
    const S3: f64 = -1.984_126_982_985_795e-4;
    const S4: f64 = 2.755_731_370_707_006_8e-6;
    const S5: f64 = -2.505_076_025_340_686_3e-8;
    const S6: f64 = 1.589_690_995_211_55e-10;
    let ix = hi(x) & 0x7fffffff;
    if ix < 0x3e400000 && x as i32 == 0 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = fma(z, fma(z, fma(z, fma(z, S6, S5), S4), S3), S2);
    if iy == 0 {
        fma(v, fma(z, r, S1), x)
    } else {
        x - fma(-v, S1, fma(z, fma(0.5, y, -(v * r)), -y))
    }
}

fn kernel_cos(x: f64, y: f64) -> f64 {
    const C1: f64 = 4.166_666_666_666_66e-2;
    const C2: f64 = -1.388_888_888_887_411e-3;
    const C3: f64 = 2.480_158_728_947_673e-5;
    const C4: f64 = -2.755_731_435_139_066_3e-7;
    const C5: f64 = 2.087_572_321_298_175e-9;
    const C6: f64 = -1.135_964_755_778_819_5e-11;
    let ix = hi(x) & 0x7fffffff;
    if ix < 0x3e400000 && x as i32 == 0 {
        return 1.0;
    }
    let z = x * x;
    let r = z * fma(z, fma(z, fma(z, fma(z, fma(z, C6, C5), C4), C3), C2), C1);
    if ix < 0x3FD33333 {
        1.0 - fma(0.5, z, -fma(z, r, -(x * y)))
    } else {
        let qx = if ix > 0x3fe90000 {
            0.28125
        } else {
            from_words(ix - 0x00200000, 0)
        };
        let hz = fma(0.5, z, -qx);
        let a = 1.0 - qx;
        a - (hz - fma(z, r, -(x * y)))
    }
}

fn kernel_tan(x: f64, y: f64, iy: i32) -> f64 {
    const T: [f64; 13] = [
        3.333_333_333_333_341e-1,
        1.333_333_333_332_012_4e-1,
        5.396_825_397_622_605e-2,
        2.186_948_829_485_954_2e-2,
        8.863_239_823_599_3e-3,
        3.592_079_107_591_312_4e-3,
        1.456_209_454_325_290_3e-3,
        5.880_412_408_202_641e-4,
        2.464_631_348_184_699e-4,
        7.817_944_429_395_571e-5,
        7.140_724_913_826_082e-5,
        -1.855_863_748_552_754_6e-5,
        2.590_730_518_636_337e-5,
    ];
    const PIO4: f64 = std::f64::consts::FRAC_PI_4;
    const PIO4LO: f64 = 3.061_616_997_868_383e-17;
    let mut x = x;
    let mut y = y;
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix < 0x3e300000 && x as i32 == 0 {
        if ((ix as u32 | lo(x)) | (iy + 1) as u32) == 0 {
            return 1.0 / x.abs();
        } else if iy == 1 {
            return x;
        }
        // -1/(x+y) computed carefully.
        let w = x + y;
        let z = with_lo(w, 0);
        let v = y - (z - x);
        let a = -1.0 / w;
        let t = with_lo(a, 0);
        let s = fma(t, z, 1.0);
        return fma(a, fma(t, v, s), t);
    }
    if ix >= 0x3FE59428 {
        if hx < 0 {
            x = -x;
            y = -y;
        }
        let z = PIO4 - x;
        let w = PIO4LO - y;
        x = z + w;
        y = 0.0;
    }
    let z = x * x;
    let w = z * z;
    let r = fma(
        w,
        fma(w, fma(w, fma(w, fma(w, T[11], T[9]), T[7]), T[5]), T[3]),
        T[1],
    );
    let v = z * fma(
        w,
        fma(w, fma(w, fma(w, fma(w, T[12], T[10]), T[8]), T[6]), T[4]),
        T[2],
    );
    let s = z * x;
    let mut r = fma(z, fma(s, r + v, y), y);
    r = fma(T[0], s, r);
    let w = x + r;
    if ix >= 0x3FE59428 {
        let v = iy as f64;
        let sign = (1 - ((hx >> 30) & 2)) as f64;
        return sign * fma(-2.0, x - (w * w / (w + v) - r), v);
    }
    if iy == 1 {
        return w;
    }
    let z = with_lo(w, 0);
    let v = r - (z - x);
    let a = -1.0 / w;
    let t = with_lo(a, 0);
    let s = fma(t, z, 1.0);
    fma(a, fma(t, v, s), t)
}

pub fn sin(x: f64) -> f64 {
    let ix = hi(x) & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return kernel_sin(x, 0.0, 0);
    }
    if ix >= 0x7ff00000 {
        return f64::NAN;
    }
    let mut y = [0.0; 2];
    let n = rem_pio2(x, &mut y);
    match n & 3 {
        0 => kernel_sin(y[0], y[1], 1),
        1 => kernel_cos(y[0], y[1]),
        2 => -kernel_sin(y[0], y[1], 1),
        _ => -kernel_cos(y[0], y[1]),
    }
}

pub fn cos(x: f64) -> f64 {
    let ix = hi(x) & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return kernel_cos(x, 0.0);
    }
    if ix >= 0x7ff00000 {
        return f64::NAN;
    }
    let mut y = [0.0; 2];
    let n = rem_pio2(x, &mut y);
    match n & 3 {
        0 => kernel_cos(y[0], y[1]),
        1 => -kernel_sin(y[0], y[1], 1),
        2 => -kernel_cos(y[0], y[1]),
        _ => kernel_sin(y[0], y[1], 1),
    }
}

pub fn tan(x: f64) -> f64 {
    let ix = hi(x) & 0x7fffffff;
    if ix <= 0x3fe921fb {
        return kernel_tan(x, 0.0, 1);
    }
    if ix >= 0x7ff00000 {
        return f64::NAN;
    }
    let mut y = [0.0; 2];
    let n = rem_pio2(x, &mut y);
    kernel_tan(y[0], y[1], 1 - ((n & 1) << 1))
}

pub fn exp(x: f64) -> f64 {
    const HALF: [f64; 2] = [0.5, -0.5];
    const O_THRESHOLD: f64 = 7.097_827_128_933_84e2;
    const U_THRESHOLD: f64 = -7.451_332_191_019_411e2;
    const LN2_HI: [f64; 2] = [6.931_471_803_691_238e-1, -6.931_471_803_691_238e-1];
    const LN2_LO: [f64; 2] = [1.908_214_929_270_587_7e-10, -1.908_214_929_270_587_7e-10];
    const INVLN2: f64 = std::f64::consts::LOG2_E;
    const P1: f64 = 1.666_666_666_666_660_2e-1;
    const P2: f64 = -2.777_777_777_701_559_3e-3;
    const P3: f64 = 6.613_756_321_437_934e-5;
    const P4: f64 = -1.653_390_220_546_525_2e-6;
    const P5: f64 = 4.138_136_797_057_238_5e-8;
    const E: f64 = std::f64::consts::E;
    const TWOM1000: f64 = 9.332_636_185_032_189e-302;

    let mut x = x;
    let hx = hi(x) as u32;
    let xsb = ((hx >> 31) & 1) as usize;
    let hx = hx & 0x7fffffff;
    if hx >= 0x40862E42 {
        if hx >= 0x7ff00000 {
            if ((hx & 0xfffff) | lo(x)) != 0 {
                return x + x;
            }
            return if xsb == 0 { x } else { 0.0 };
        }
        if x > O_THRESHOLD {
            return f64::INFINITY;
        }
        if x < U_THRESHOLD {
            return 0.0;
        }
    }
    let (reduced_hi, reduced_lo, k);
    if hx > 0x3fd62e42 {
        if hx < 0x3FF0A2B2 {
            // exp(1) is special-cased: the series below gets the last bit wrong.
            if x == 1.0 {
                return E;
            }
            reduced_hi = x - LN2_HI[xsb];
            reduced_lo = LN2_LO[xsb];
            k = 1 - xsb as i32 - xsb as i32;
        } else {
            k = fma(INVLN2, x, HALF[xsb]) as i32;
            let t = k as f64;
            reduced_hi = fma(-t, LN2_HI[0], x);
            reduced_lo = t * LN2_LO[0];
        }
        x = reduced_hi - reduced_lo;
    } else if hx < 0x3e300000 {
        return 1.0 + x;
    } else {
        k = 0;
        reduced_hi = 0.0;
        reduced_lo = 0.0;
    }
    let t = x * x;
    let c = fma(-t, fma(t, fma(t, fma(t, fma(t, P5, P4), P3), P2), P1), x);

    let y = if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    } else {
        1.0 - ((reduced_lo - (x * c) / (2.0 - c)) - reduced_hi)
    };
    if k >= -1021 {
        if k == 1024 {
            return y * 2.0 * 8.98846567431158e307;
        }
        y * from_words(0x3ff00000 + (k << 20), 0)
    } else {
        y * from_words(0x3ff00000 + ((k + 1000) << 20), 0) * TWOM1000
    }
}

pub fn expm1(x: f64) -> f64 {
    const O_THRESHOLD: f64 = 7.097_827_128_933_84e2;
    const LN2_HI: f64 = 6.931_471_803_691_238e-1;
    const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
    const INVLN2: f64 = std::f64::consts::LOG2_E;
    const Q1: f64 = -3.333_333_333_333_313e-2;
    const Q2: f64 = 1.587_301_587_254_814_6e-3;
    const Q3: f64 = -7.936_507_578_674_88e-5;
    const Q4: f64 = 4.008_217_827_329_362e-6;
    const Q5: f64 = -2.010_992_181_836_243_7e-7;

    let mut x = x;
    let hx = hi(x) as u32;
    let xsb = hx & 0x8000_0000;
    let hx = hx & 0x7fff_ffff;
    if hx >= 0x4043687A {
        if hx >= 0x40862E42 {
            if hx >= 0x7ff00000 {
                if ((hx & 0xfffff) | lo(x)) != 0 {
                    return x + x;
                }
                return if xsb == 0 { x } else { -1.0 };
            }
            if x > O_THRESHOLD {
                return f64::INFINITY;
            }
        }
        if xsb != 0 {
            return -1.0;
        }
    }
    let mut c = 0.0;
    let k: i32;
    if hx > 0x3fd62e42 {
        let (reduced_hi, reduced_lo);
        if hx < 0x3FF0A2B2 {
            if xsb == 0 {
                reduced_hi = x - LN2_HI;
                reduced_lo = LN2_LO;
                k = 1;
            } else {
                reduced_hi = x + LN2_HI;
                reduced_lo = -LN2_LO;
                k = -1;
            }
        } else {
            k = fma(INVLN2, x, if xsb == 0 { 0.5 } else { -0.5 }) as i32;
            let t = k as f64;
            reduced_hi = fma(-t, LN2_HI, x);
            reduced_lo = t * LN2_LO;
        }
        x = reduced_hi - reduced_lo;
        c = (reduced_hi - x) - reduced_lo;
    } else if hx < 0x3c900000 {
        return x;
    } else {
        k = 0;
    }
    let hfx = 0.5 * x;
    let hxs = x * hfx;
    let r1 = fma(
        hxs,
        fma(hxs, fma(hxs, fma(hxs, fma(hxs, Q5, Q4), Q3), Q2), Q1),
        1.0,
    );
    let t = fma(-r1, hfx, 3.0);
    let mut e = hxs * ((r1 - t) / fma(-x, t, 6.0));
    if k == 0 {
        return x - fma(x, e, -hxs);
    }
    let twopk = from_words(0x3ff00000 + (k << 20), 0);
    e = fma(x, e - c, -c);
    e -= hxs;
    if k == -1 {
        return fma(0.5, x - e, -0.5);
    }
    if k == 1 {
        if x < -0.25 {
            return -2.0 * (e - (x + 0.5));
        }
        return fma(2.0, x - e, 1.0);
    }
    if k <= -2 || k > 56 {
        let mut y = 1.0 - (e - x);
        if k == 1024 {
            y = y * 2.0 * 8.98846567431158e307;
        } else {
            y *= twopk;
        }
        return y - 1.0;
    }

    if k < 20 {
        let t = from_words(0x3ff00000 - (0x200000 >> k), 0);
        (t - (e - x)) * twopk
    } else {
        let t = from_words((0x3ff - k) << 20, 0);
        let mut yy = x - (e + t);
        yy += 1.0;
        yy * twopk
    }
}

pub fn log(x: f64) -> f64 {
    const LN2_HI: f64 = 6.931_471_803_691_238e-1;
    const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
    const TWO54: f64 = 1.801_439_850_948_198_4e16;
    const LG1: f64 = 6.666_666_666_666_735e-1;
    const LG2: f64 = 3.999_999_999_940_942e-1;
    const LG3: f64 = 2.857_142_874_366_239e-1;
    const LG4: f64 = 2.222_219_843_214_978_4e-1;
    const LG5: f64 = 1.818_357_216_161_805e-1;
    const LG6: f64 = 1.531_383_769_920_937_3e-1;
    const LG7: f64 = 1.479_819_860_511_658_6e-1;

    let mut x = x;
    let mut hx = hi(x);
    let lx = lo(x);
    let mut k = 0i32;
    if hx < 0x00100000 {
        if ((hx & 0x7fffffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY;
        }
        if hx < 0 {
            return f64::NAN;
        }
        k -= 54;
        x *= TWO54;
        hx = hi(x);
    }
    if hx >= 0x7ff00000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000fffff;
    let i = (hx + 0x95f64) & 0x100000;
    x = with_hi(x, hx | (i ^ 0x3ff00000));
    k += i >> 20;
    let f = x - 1.0;
    if (0x000fffff & (2 + hx)) < 3 {
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            let dk = k as f64;
            return fma(dk, LN2_HI, dk * LN2_LO);
        }
        let r = f * f * fma(-0.333_333_333_333_333_3, f, 0.5);
        if k == 0 {
            return f - r;
        }
        let dk = k as f64;
        return fma(dk, LN2_HI, -(fma(-dk, LN2_LO, r) - f));
    }
    let s = f / (2.0 + f);
    let dk = k as f64;
    let z = s * s;
    let i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * fma(w, fma(w, LG6, LG4), LG2);
    let t2 = z * fma(w, fma(w, fma(w, LG7, LG5), LG3), LG1);
    let i = i | j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            f - fma(-s, hfsq + r, hfsq)
        } else {
            fma(dk, LN2_HI, -((hfsq - fma(s, hfsq + r, dk * LN2_LO)) - f))
        }
    } else if k == 0 {
        fma(-s, f - r, f)
    } else {
        fma(dk, LN2_HI, -(fma(s, f - r, -(dk * LN2_LO)) - f))
    }
}

pub fn log1p(x: f64) -> f64 {
    const LN2_HI: f64 = 6.931_471_803_691_238e-1;
    const LN2_LO: f64 = 1.908_214_929_270_587_7e-10;
    const TWO54: f64 = 1.801_439_850_948_198_4e16;
    const LP1: f64 = 6.666_666_666_666_735e-1;
    const LP2: f64 = 3.999_999_999_940_942e-1;
    const LP3: f64 = 2.857_142_874_366_239e-1;
    const LP4: f64 = 2.222_219_843_214_978_4e-1;
    const LP5: f64 = 1.818_357_216_161_805e-1;
    const LP6: f64 = 1.531_383_769_920_937_3e-1;
    const LP7: f64 = 1.479_819_860_511_658_6e-1;

    let hx = hi(x);
    let ax = hx & 0x7fffffff;
    let mut k = 1i32;
    let mut f = 0.0;
    let mut c = 0.0;
    let mut hu = 0i32;
    if hx < 0x3FDA827A {
        if ax >= 0x3ff00000 {
            if x == -1.0 {
                return f64::NEG_INFINITY;
            }
            return f64::NAN;
        }
        if ax < 0x3e200000 {
            if TWO54 + x > 0.0 && ax < 0x3c900000 {
                return x;
            }
            return fma(-(x * x), 0.5, x);
        }
        if hx > 0 || hx <= 0xbfd2bec4u32 as i32 {
            k = 0;
            f = x;
            hu = 1;
        }
    }
    if hx >= 0x7ff00000 {
        return x + x;
    }
    if k != 0 {
        let u;
        if hx < 0x43400000 {
            u = 1.0 + x;
            hu = hi(u);
            k = (hu >> 20) - 1023;
            c = if k > 0 { 1.0 - (u - x) } else { x - (u - 1.0) };
            c /= u;
        } else {
            u = x;
            hu = hi(u);
            k = (hu >> 20) - 1023;
            c = 0.0;
        }
        hu &= 0x000fffff;
        let un;
        if hu < 0x6a09e {
            un = with_hi(u, hu | 0x3ff00000);
        } else {
            k += 1;
            un = with_hi(u, hu | 0x3fe00000);
            hu = (0x00100000 - hu) >> 2;
        }
        f = un - 1.0;
    }
    let hfsq = 0.5 * f * f;
    if hu == 0 {
        if f == 0.0 {
            if k == 0 {
                return 0.0;
            }
            c = fma(k as f64, LN2_LO, c);
            return fma(k as f64, LN2_HI, c);
        }
        let r = hfsq * fma(-0.666_666_666_666_666_6, f, 1.0);
        if k == 0 {
            return f - r;
        }
        let dk = k as f64;
        return fma(dk, LN2_HI, -((r - fma(dk, LN2_LO, c)) - f));
    }
    let s = f / (2.0 + f);
    let z = s * s;
    let r = z * fma(
        z,
        fma(
            z,
            fma(z, fma(z, fma(z, fma(z, LP7, LP6), LP5), LP4), LP3),
            LP2,
        ),
        LP1,
    );
    if k == 0 {
        f - fma(-s, hfsq + r, hfsq)
    } else {
        let dk = k as f64;
        fma(
            dk,
            LN2_HI,
            -((hfsq - fma(s, hfsq + r, fma(dk, LN2_LO, c))) - f),
        )
    }
}

/// FreeBSD `k_log1p`: `log(1+f) - f + f*f/2` for `f` in `~[sqrt(2)/2-1, sqrt(2)-1]`.
fn k_log1p(f: f64) -> f64 {
    const LG1: f64 = 6.666_666_666_666_735e-1;
    const LG2: f64 = 3.999_999_999_940_942e-1;
    const LG3: f64 = 2.857_142_874_366_239e-1;
    const LG4: f64 = 2.222_219_843_214_978_4e-1;
    const LG5: f64 = 1.818_357_216_161_805e-1;
    const LG6: f64 = 1.531_383_769_920_937_3e-1;
    const LG7: f64 = 1.479_819_860_511_658_6e-1;
    let s = f / (2.0 + f);
    let z = s * s;
    let w = z * z;
    let t1 = w * fma(w, fma(w, LG6, LG4), LG2);
    let t2 = z * fma(w, fma(w, fma(w, LG7, LG5), LG3), LG1);
    let r = t2 + t1;
    let hfsq = 0.5 * f * f;
    s * (hfsq + r)
}

/// Shared prologue of `log2`/`log10`: scale a subnormal, split off the
/// exponent. Returns `(x_normalized_high_word_state, k, early_return)`.
fn log_prologue(x: f64) -> Result<(f64, i32, i32), f64> {
    const TWO54: f64 = 1.801_439_850_948_198_4e16;
    let mut x = x;
    let mut hx = hi(x);
    let lx = lo(x);
    let mut k = 0;
    if hx < 0x00100000 {
        if ((hx & 0x7fffffff) as u32 | lx) == 0 {
            return Err(f64::NEG_INFINITY);
        }
        if hx < 0 {
            return Err(f64::NAN);
        }
        k -= 54;
        x *= TWO54;
        hx = hi(x);
    }
    if hx >= 0x7ff00000 {
        return Err(x + x);
    }
    if hx == 0x3ff00000 && lo(x) == 0 {
        return Err(0.0);
    }
    Ok((x, k, hx))
}

pub fn log2(x: f64) -> f64 {
    const IVLN2HI: f64 = 1.442_695_040_721_446_3;
    const IVLN2LO: f64 = 1.675_171_316_488_651_2e-10;
    let (mut x, mut k, mut hx) = match log_prologue(x) {
        Ok(v) => v,
        Err(r) => return r,
    };
    k += (hx >> 20) - 1023;
    hx &= 0x000fffff;
    let i = (hx + 0x95f64) & 0x100000;
    x = with_hi(x, hx | (i ^ 0x3ff00000));
    k += i >> 20;
    let y = k as f64;
    let f = x - 1.0;
    let hfsq = 0.5 * f * f;
    let r = k_log1p(f);
    let hi_ = with_lo(f - hfsq, 0);
    let lo_ = (f - hi_) - hfsq + r;
    let mut val_hi = hi_ * IVLN2HI;
    let mut val_lo = fma(lo_ + hi_, IVLN2LO, lo_ * IVLN2HI);
    let w = y + val_hi;
    val_lo += (y - w) + val_hi;
    val_hi = w;
    val_lo + val_hi
}

pub fn log10(x: f64) -> f64 {
    const IVLN10: f64 = std::f64::consts::LOG10_E;
    const LOG10_2HI: f64 = 3.010_299_956_636_117_7e-1;
    const LOG10_2LO: f64 = 3.694_239_077_158_931e-13;
    let (x, mut k, mut hx) = match log_prologue(x) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let lx = lo(x);
    k += (hx >> 20) - 1023;
    let i = ((k as u32 & 0x8000_0000) >> 31) as i32;
    hx = (hx & 0x000fffff) | ((0x3ff - i) << 20);
    let y = (k + i) as f64;
    let x = from_words(hx, lx);
    let z = fma(y, LOG10_2LO, IVLN10 * log(x));
    fma(y, LOG10_2HI, z)
}

pub fn atan(x: f64) -> f64 {
    const ATANHI: [f64; 4] = [
        4.636_476_090_008_061e-1,
        std::f64::consts::FRAC_PI_4,
        9.827_937_232_473_29e-1,
        std::f64::consts::FRAC_PI_2,
    ];
    const ATANLO: [f64; 4] = [
        2.269_877_745_296_168_7e-17,
        3.061_616_997_868_383e-17,
        1.390_331_103_123_099_8e-17,
        6.123_233_995_736_766e-17,
    ];
    const AT: [f64; 11] = [
        3.333_333_333_333_293e-1,
        -1.999_999_999_987_648_3e-1,
        1.428_571_427_250_346_6e-1,
        -1.111_111_040_546_235_6e-1,
        9.090_887_133_436_507e-2,
        -7.691_876_205_044_83e-2,
        6.661_073_137_387_531e-2,
        -5.833_570_133_790_573_5e-2,
        4.976_877_994_615_932_4e-2,
        -3.653_157_274_421_691_6e-2,
        1.628_582_011_536_578_2e-2,
    ];
    let mut x = x;
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix >= 0x44100000 {
        if x.is_nan() {
            return x + x;
        }
        return if hx > 0 {
            ATANHI[3] + ATANLO[3]
        } else {
            -ATANHI[3] - ATANLO[3]
        };
    }
    let id: i32;
    if ix < 0x3fdc0000 {
        if ix < 0x3e200000 {
            return x;
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3ff30000 {
            if ix < 0x3fe60000 {
                id = 0;
                x = fma(2.0, x, -1.0) / (2.0 + x);
            } else {
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x40038000 {
            id = 2;
            x = (x - 1.5) / fma(1.5, x, 1.0);
        } else {
            id = 3;
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    let s1 = z * fma(
        w,
        fma(
            w,
            fma(w, fma(w, fma(w, AT[10], AT[8]), AT[6]), AT[4]),
            AT[2],
        ),
        AT[0],
    );
    let s2 = w * fma(w, fma(w, fma(w, fma(w, AT[9], AT[7]), AT[5]), AT[3]), AT[1]);
    if id < 0 {
        return fma(-x, s1 + s2, x);
    }
    let id = id as usize;
    let z = ATANHI[id] - (fma(x, s1 + s2, -ATANLO[id]) - x);
    if hx < 0 {
        -z
    } else {
        z
    }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    const PI_O_4: f64 = std::f64::consts::FRAC_PI_4;
    const PI_O_2: f64 = std::f64::consts::FRAC_PI_2;
    const PI_LO: f64 = 1.224_646_799_147_353_2E-16;
    let hx = hi(x);
    let lx = lo(x);
    let ix = hx & 0x7fffffff;
    let hy = hi(y);
    let ly = lo(y);
    let iy = hy & 0x7fffffff;
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if (hx.wrapping_sub(0x3ff00000) as u32 | lx) == 0 {
        return atan(y);
    }
    let mut m = ((hy >> 31) & 1) | ((hx >> 30) & 2);
    if (iy as u32 | ly) == 0 {
        return match m {
            0 | 1 => y,
            2 => PI,
            _ => -PI,
        };
    }
    if (ix as u32 | lx) == 0 {
        return if hy < 0 { -PI_O_2 } else { PI_O_2 };
    }
    if ix == 0x7ff00000 {
        if iy == 0x7ff00000 {
            return match m {
                0 => PI_O_4,
                1 => -PI_O_4,
                2 => 3.0 * PI_O_4,
                _ => -3.0 * PI_O_4,
            };
        }
        return match m {
            0 => 0.0,
            1 => -0.0,
            2 => PI,
            _ => -PI,
        };
    }
    if iy == 0x7ff00000 {
        return if hy < 0 { -PI_O_2 } else { PI_O_2 };
    }
    let k = (iy - ix) >> 20;
    let z;
    if k > 60 {
        z = fma(0.5, PI_LO, PI_O_2);
        m &= 1;
    } else if hx < 0 && k < -60 {
        z = 0.0;
    } else {
        z = atan((y / x).abs());
    }
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

/// Rational approximation shared by `asin` and `acos`: returns `p(t)/q(t)`
/// written as the two numerator/denominator values.
fn asin_pq(t: f64) -> (f64, f64) {
    const PS0: f64 = 1.666_666_666_666_666_6e-1;
    const PS1: f64 = -3.255_658_186_224_009e-1;
    const PS2: f64 = 2.012_125_321_348_629_3e-1;
    const PS3: f64 = -4.005_553_450_067_941e-2;
    const PS4: f64 = 7.915_349_942_898_145e-4;
    const PS5: f64 = 3.479_331_075_960_212e-5;
    const QS1: f64 = -2.403_394_911_734_414;
    const QS2: f64 = 2.020_945_760_233_505_7;
    const QS3: f64 = -6.882_839_716_054_533e-1;
    const QS4: f64 = 7.703_815_055_590_194e-2;
    let p = t * fma(
        t,
        fma(t, fma(t, fma(t, fma(t, PS5, PS4), PS3), PS2), PS1),
        PS0,
    );
    let q = fma(t, fma(t, fma(t, fma(t, QS4, QS3), QS2), QS1), 1.0);
    (p, q)
}

pub fn asin(x: f64) -> f64 {
    const PIO2_HI: f64 = std::f64::consts::FRAC_PI_2;
    const PIO2_LO: f64 = 6.123_233_995_736_766e-17;
    const PIO4_HI: f64 = std::f64::consts::FRAC_PI_4;
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix >= 0x3ff00000 {
        if (ix.wrapping_sub(0x3ff00000) as u32 | lo(x)) == 0 {
            return fma(x, PIO2_HI, x * PIO2_LO);
        }
        return f64::NAN;
    } else if ix < 0x3fe00000 {
        if ix < 0x3e400000 {
            return x;
        }
        let t = x * x;
        let (p, q) = asin_pq(t);
        let w = p / q;
        return fma(x, w, x);
    }
    let w = 1.0 - x.abs();
    let t = w * 0.5;
    let (mut p, mut q) = asin_pq(t);
    let s = t.sqrt();
    let t = if ix >= 0x3FEF3333 {
        let w = p / q;
        PIO2_HI - fma(2.0, fma(s, w, s), -PIO2_LO)
    } else {
        let w = with_lo(s, 0);
        let c = fma(-w, w, t) / (s + w);
        let r = p / q;
        p = fma(2.0 * s, r, -fma(-2.0, c, PIO2_LO));
        q = fma(-2.0, w, PIO4_HI);
        PIO4_HI - (p - q)
    };
    if hx > 0 {
        t
    } else {
        -t
    }
}

pub fn acos(x: f64) -> f64 {
    const PIO2_HI: f64 = std::f64::consts::FRAC_PI_2;
    const PIO2_LO: f64 = 6.123_233_995_736_766e-17;
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix >= 0x3ff00000 {
        if (ix.wrapping_sub(0x3ff00000) as u32 | lo(x)) == 0 {
            if hx > 0 {
                return 0.0;
            }
            return fma(2.0, PIO2_LO, PI);
        }
        return f64::NAN;
    }
    if ix < 0x3fe00000 {
        if ix <= 0x3c600000 {
            return PIO2_HI + PIO2_LO;
        }
        let z = x * x;
        let (p, q) = asin_pq(z);
        let r = p / q;
        PIO2_HI - (x - fma(-x, r, PIO2_LO))
    } else if hx < 0 {
        let z = (1.0 + x) * 0.5;
        let (p, q) = asin_pq(z);
        let s = z.sqrt();
        let r = p / q;
        let w = fma(r, s, -PIO2_LO);
        fma(-2.0, s + w, PI)
    } else {
        let z = (1.0 - x) * 0.5;
        let s = z.sqrt();
        let df = with_lo(s, 0);
        let c = fma(-df, df, z) / (s + df);
        let (p, q) = asin_pq(z);
        let r = p / q;
        let w = fma(r, s, c);
        2.0 * (df + w)
    }
}

pub fn sinh(x: f64) -> f64 {
    const KSINH_OVERFLOW: f64 = 710.4758600739439;
    const TWO_M28: f64 = 3.725290298461914e-9;
    const LOG_MAXD: f64 = 709.7822265625;
    const SHUGE: f64 = 1.0e307;
    let h = if x < 0.0 { -0.5 } else { 0.5 };
    let ax = x.abs();
    if ax < 22.0 {
        if ax < TWO_M28 {
            return x;
        }
        let t = expm1(ax);
        if ax < 1.0 {
            return h * fma(2.0, t, -(t * t / (t + 1.0)));
        }
        return h * (t + t / (t + 1.0));
    }
    if ax < LOG_MAXD {
        return h * exp(ax);
    }
    if ax <= KSINH_OVERFLOW {
        let w = exp(0.5 * ax);
        let t = h * w;
        return t * w;
    }
    x * SHUGE
}

pub fn cosh(x: f64) -> f64 {
    const KCOSH_OVERFLOW: f64 = 710.4758600739439;
    let ix = hi(x) & 0x7fffffff;
    if ix < 0x3fd62e43 {
        let t = expm1(x.abs());
        let w = 1.0 + t;
        if ix < 0x3c800000 {
            return w;
        }
        return 1.0 + (t * t) / (w + w);
    }
    if ix < 0x40360000 {
        let t = exp(x.abs());
        return fma(0.5, t, 0.5 / t);
    }
    if ix < 0x40862e42 {
        return 0.5 * exp(x.abs());
    }
    if x.abs() <= KCOSH_OVERFLOW {
        let w = exp(0.5 * x.abs());
        let t = 0.5 * w;
        return t * w;
    }
    if ix >= 0x7ff00000 {
        return x * x;
    }
    f64::INFINITY
}

pub fn tanh(x: f64) -> f64 {
    let jx = hi(x);
    let ix = jx & 0x7fffffff;
    if ix >= 0x7ff00000 {
        return if jx >= 0 {
            1.0 / x + 1.0
        } else {
            1.0 / x - 1.0
        };
    }
    let z;
    if ix < 0x40360000 {
        if ix < 0x3e300000 {
            return x;
        }
        if ix >= 0x3ff00000 {
            let t = expm1(2.0 * x.abs());
            z = 1.0 - 2.0 / (t + 2.0);
        } else {
            let t = expm1(-2.0 * x.abs());
            z = -t / (t + 2.0);
        }
    } else {
        z = 1.0;
    }
    if jx >= 0 {
        z
    } else {
        -z
    }
}

pub fn asinh(x: f64) -> f64 {
    const LN2: f64 = std::f64::consts::LN_2;
    let hx = hi(x);
    let ix = hx & 0x7fffffff;
    if ix >= 0x7ff00000 {
        return x + x;
    }
    if ix < 0x3e300000 {
        return x;
    }

    let w = if ix > 0x41b00000 {
        log(x.abs()) + LN2
    } else if ix > 0x40000000 {
        let t = x.abs();
        log(fma(2.0, t, 1.0 / (fma(x, x, 1.0).sqrt() + t)))
    } else {
        let t = x * x;
        log1p(x.abs() + t / (1.0 + (1.0 + t).sqrt()))
    };
    if hx > 0 {
        w
    } else {
        -w
    }
}

pub fn acosh(x: f64) -> f64 {
    const LN2: f64 = std::f64::consts::LN_2;
    let hx = hi(x);
    let lx = lo(x);
    if hx < 0x3ff00000 {
        f64::NAN
    } else if hx >= 0x41b00000 {
        if hx >= 0x7ff00000 {
            x + x
        } else {
            log(x) + LN2
        }
    } else if (hx.wrapping_sub(0x3ff00000) as u32 | lx) == 0 {
        0.0
    } else if hx > 0x40000000 {
        let t = x * x;
        log(fma(2.0, x, -(1.0 / (x + (t - 1.0).sqrt()))))
    } else {
        let t = x - 1.0;
        log1p(t + fma(2.0, t, t * t).sqrt())
    }
}

pub fn atanh(x: f64) -> f64 {
    let hx = hi(x);
    let lx = lo(x);
    let ix = hx & 0x7fffffff;
    if (ix as u32 | ((lx | lx.wrapping_neg()) >> 31)) > 0x3ff00000 {
        return f64::NAN;
    }
    if ix == 0x3ff00000 {
        return x / 0.0;
    }
    if ix < 0x3e300000 {
        return x;
    }
    let x = with_hi(x, ix);

    let t = if ix < 0x3fe00000 {
        let t2 = x + x;
        0.5 * log1p(t2 + t2 * x / (1.0 - x))
    } else {
        0.5 * log1p((x + x) / (1.0 - x))
    };
    if hx >= 0 {
        t
    } else {
        -t
    }
}

pub fn cbrt(x: f64) -> f64 {
    const B1: u32 = 715094163;
    const B2: u32 = 696219795;
    const P0: f64 = 1.875_951_824_271_77;
    const P1: f64 = -1.884_979_795_433_771_7;
    const P2: f64 = 1.621_429_720_105_354_5;
    const P3: f64 = -0.758_397_934_778_766;
    const P4: f64 = 0.145_996_192_886_612_45;
    let hx = hi(x) as u32;
    let low = lo(x);
    let sign = hx & 0x8000_0000;
    let hx = hx ^ sign;
    if hx >= 0x7ff00000 {
        return x + x;
    }
    let mut t;
    if hx < 0x00100000 {
        if (hx | low) == 0 {
            return x;
        }
        t = from_words(0x43500000, 0);
        t *= x;
        let high = hi(t) as u32;
        t = from_words((sign | ((high & 0x7fffffff) / 3 + B2)) as i32, 0);
    } else {
        t = from_words((sign | (hx / 3 + B1)) as i32, 0);
    }
    let r = (t * t) * (t / x);
    t *= fma((r * r) * r, fma(r, P4, P3), fma(r, fma(r, P2, P1), P0));
    t = f64::from_bits(t.to_bits().wrapping_add(0x8000_0000) & 0xffff_ffff_c000_0000);
    let s = t * t;
    let r = x / s;
    let w = t + t;
    let r = (r - t) / (w + r);
    fma(t, r, t)
}
