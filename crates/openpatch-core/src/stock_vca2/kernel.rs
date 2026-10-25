//! One VCA-2 channel, following the f32 operation order of the pinned
//! Fundamental `src/VCA.cpp` `processChannel` (GPL-3.0-or-later, see
//! LICENSE-THIRD-PARTY). Every add/sub/mul/div applies the Rack Engine's
//! flush-to-zero contract through `native_float`. Ports are raw 16-lane arrays
//! whose lanes at and above the channel count hold zero, as Rack cables leave
//! them. Self-contained apart from `crate::native_float`, so the reference
//! runner can compile it directly.
use crate::native_float::{flush_tiny, rack_add, rack_div, rack_mul, rack_sub};

/// One port as a module sees it: channel count plus 16 raw lanes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vca2Port {
    pub channels: u8,
    pub values: [f32; 16],
}

/// `logf(50.f)` as Fundamental's `simd::pow(50.f, cv)` evaluates it.
pub const LN_50_BITS: u32 = 0x407a_5e96;

/// SSE `max_ps(x, b)`: the second operand wins unless `x > b`.
fn sse_max(x: f32, b: f32) -> f32 {
    if x > b {
        x
    } else {
        b
    }
}

/// SSE `min_ps(x, b)`: the second operand wins unless `x < b`.
fn sse_min(x: f32, b: f32) -> f32 {
    if x < b {
        x
    } else {
        b
    }
}

/// `simd::clamp(x, 0, 1)` = `fmin(fmax(x, 0), 1)` with SSE select semantics.
fn simd_clamp_unit(x: f32) -> f32 {
    sse_min(sse_max(x, 0.0), 1.0)
}

/// `math::clamp(x, 0, 1)` = `std::fmax(std::fmin(x, 1), 0)`. Unlike
/// `f32::clamp`, `fmax(-0, +0)` returns +0 here, as the source's libm call does.
#[allow(clippy::manual_clamp)]
fn scalar_clamp_unit(x: f32) -> f32 {
    x.min(1.0).max(0.0)
}

/// `float` from a C double literal, rounded as `_mm_set_ps1` receives it.
fn c_float(literal: f64) -> f32 {
    literal as f32
}

/// Altered Rust port of `sse_mathfun_exp_ps` from Rack's
/// `include/simd/sse_mathfun.h` (zlib licence, Julien Pommier; see
/// LICENSE-THIRD-PARTY). One lane, same operation order and constants, with
/// the Rack flush-to-zero contract applied to every arithmetic operation.
/// The decimal literals are the source's, rounded as the C code rounds them.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
pub fn sse_mathfun_exp(x: f32) -> f32 {
    let one = 1.0_f32;
    let mut x = sse_min(flush_tiny(x), c_float(88.3762626647949));
    x = sse_max(x, c_float(-88.3762626647949));
    // exp(x) = exp(g + n*log(2))
    let mut fx = rack_mul(x, c_float(1.44269504088896341));
    fx = rack_add(fx, c_float(0.5));
    // floorf via truncation, minus one where truncation rounded up.
    let truncated = fx as i32;
    let tmp = truncated as f32;
    let mask = if tmp > fx { one } else { 0.0 };
    fx = rack_sub(tmp, mask);
    let tmp = rack_mul(fx, c_float(0.693359375));
    let z = rack_mul(fx, c_float(-2.12194440e-4));
    x = rack_sub(x, tmp);
    x = rack_sub(x, z);
    let z = rack_mul(x, x);
    let mut y = c_float(1.9875691500E-4);
    y = rack_mul(y, x);
    y = rack_add(y, c_float(1.3981999507E-3));
    y = rack_mul(y, x);
    y = rack_add(y, c_float(8.3334519073E-3));
    y = rack_mul(y, x);
    y = rack_add(y, c_float(4.1665795894E-2));
    y = rack_mul(y, x);
    y = rack_add(y, c_float(1.6666665459E-1));
    y = rack_mul(y, x);
    y = rack_add(y, c_float(5.0000001201E-1));
    y = rack_mul(y, z);
    y = rack_add(y, x);
    y = rack_add(y, one);
    // 2^n from the exponent field.
    let n = (fx as i32).wrapping_add(0x7f).wrapping_shl(23);
    rack_mul(y, f32::from_bits(n as u32))
}

/// `rescale(x, 1, 50, 0, 1)` in the source's order:
/// `yMin + (x - xMin) / (xMax - xMin) * (yMax - yMin)`.
fn rescale_exponential(x: f32) -> f32 {
    rack_add(
        0.0,
        rack_mul(
            rack_div(rack_sub(x, 1.0), rack_sub(50.0, 1.0)),
            rack_sub(1.0, 0.0),
        ),
    )
}

/// Exponential gain for a mono CV (`std::pow(50.f, cv)`).
pub fn mono_exponential_gain(cv: f32) -> f32 {
    rescale_exponential(50.0_f32.powf(cv))
}

/// Exponential gain for one lane of a polyphonic CV (`simd::pow(50.f, cv)`).
pub fn poly_exponential_gain(cv: f32) -> f32 {
    rescale_exponential(sse_mathfun_exp(rack_mul(cv, f32::from_bits(LN_50_BITS))))
}

/// One amplifier channel. Returns the output channel count; lanes at and
/// above it are zero in `out`, as a Rack cable would carry them.
pub fn process_channel(
    input: &Vca2Port,
    level: f32,
    lin: &Vca2Port,
    exp: &Vca2Port,
    out: &mut [f32; 16],
) -> u8 {
    let channels = input.channels.clamp(1, 16);
    let width = usize::from(channels);
    *out = [0.0; 16];
    for (lane, slot) in out.iter_mut().enumerate().take(width) {
        // Knob gain.
        let mut v = rack_mul(input.values[lane], level);
        // Linear CV.
        if lin.channels > 1 {
            let cv = simd_clamp_unit(rack_div(lin.values[lane], 10.0));
            v = rack_mul(v, cv);
        } else if lin.channels == 1 {
            let cv = scalar_clamp_unit(rack_div(lin.values[0], 10.0));
            v = rack_mul(v, cv);
        }
        // Exponential CV.
        if exp.channels > 1 {
            let cv = simd_clamp_unit(rack_div(exp.values[lane], 10.0));
            v = rack_mul(v, poly_exponential_gain(cv));
        } else if exp.channels == 1 {
            let cv = scalar_clamp_unit(rack_div(exp.values[0], 10.0));
            v = rack_mul(v, mono_exponential_gain(cv));
        }
        *slot = v;
    }
    channels
}
