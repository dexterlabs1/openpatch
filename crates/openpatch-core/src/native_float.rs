//! Local arithmetic for the bounded Rack Engine floating-point contract.
//!
//! Rack 2.6.6 Engine::stepBlock calls system::resetFpuFlags: nearest rounding,
//! denormal operands/results flushed to signed zero. The qualified ARM FZ
//! environment tests result tininess before rounding. Apply signed FTZ/DAZ with
//! bit classification around arithmetic, without modifying FPCR/MXCSR or raw
//! port copies. Used only by Stock Mix and canonical stacked-cable addition;
//! other legacy/native module arithmetic retains its separately qualified path.

pub(crate) fn flush_tiny(value: f32) -> f32 {
    let bits = value.to_bits();
    if bits & 0x7f80_0000 == 0 {
        f32::from_bits(bits & 0x8000_0000)
    } else {
        value
    }
}

pub(crate) fn rack_add(left: f32, right: f32) -> f32 {
    flush_tiny(flush_tiny(left) + flush_tiny(right))
}

pub(crate) fn rack_mul(left: f32, right: f32) -> f32 {
    let left = flush_tiny(left);
    let right = flush_tiny(right);
    let result = left * right;
    // A tiny exact product can round UP to minimum normal before post-flushing.
    // Only that rounded magnitude needs an extra check. A product of two f32
    // significands fits exactly in f64 (at most 48 bits), with ample exponent
    // range. Keep exactly-normal and above-normal products, even when the
    // latter round DOWN onto this same boundary.
    if result.to_bits() & 0x7fff_ffff == 0x0080_0000
        && (f64::from(left) * f64::from(right)).abs() < f64::from(f32::MIN_POSITIVE)
    {
        f32::from_bits(result.to_bits() & 0x8000_0000)
    } else {
        flush_tiny(result)
    }
}

pub(crate) fn rack_div(left: f32, right: f32) -> f32 {
    let left = flush_tiny(left);
    let right = flush_tiny(right);
    let result = left / right;
    // Compare the exact ratio with the boundary without rounding a quotient:
    // multiplying an f32 value by 2^-126 is exact and safely normal in f64.
    if result.to_bits() & 0x7fff_ffff == 0x0080_0000
        && f64::from(left).abs() < f64::from(right).abs() * f64::from(f32::MIN_POSITIVE)
    {
        f32::from_bits(result.to_bits() & 0x8000_0000)
    } else {
        flush_tiny(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplication_flushes_before_rounding_but_keeps_exact_and_above_boundary() {
        // Independent supplementary genuine-Engine anchors, including the
        // above-normal control that also rounds onto minimum normal.
        for (source, level, expected) in [
            (0x0080_0000, 0x3f7f_ffff, 0),
            (0x0080_0000, 0x3f80_0000, 0x0080_0000),
            (0x0080_0001, 0x3f7f_ffff, 0x0080_0000),
            (0x0080_0001, 0x3f80_0000, 0x0080_0001),
        ] {
            for sign in [0, 0x8000_0000] {
                assert_eq!(
                    rack_mul(f32::from_bits(source | sign), f32::from_bits(level)).to_bits(),
                    expected | sign
                );
            }
        }
    }

    #[test]
    fn division_flushes_before_rounding_without_clearing_normal_boundary_controls() {
        for (level, expected) in [
            (0x00ff_fffe, 0),
            (0x00ff_ffff, 0),
            (0x0100_0000, 0x0080_0000),
            (0x0100_0001, 0x0080_0001),
        ] {
            for left_sign in [0, 0x8000_0000] {
                for right in [2.0_f32, -2.0] {
                    assert_eq!(
                        rack_div(f32::from_bits(level | left_sign), right).to_bits(),
                        expected | (left_sign ^ (right.to_bits() & 0x8000_0000))
                    );
                }
            }
        }
    }
}
