#!/usr/bin/env python3
"""Original numerical MinBLEP design; no Rack code or data is used.

Brandt, 'Hard Sync Without Aliasing' (ICMC 2001), sections 6.1–6.3:
https://www.cs.cmu.edu/~eli/papers/icmc01-hardsync.pdf

A 32-zero-crossing sinc uses the standard four-term Blackman–Harris window.
Real-cepstral spectral factorization moves its energy to minimum phase; a
cumulative integral gives a unit step. By default design arithmetic is double
precision and playback coefficients are rounded once to IEEE f32. An explicit
float32 sampling option measures how initial design rounding changes the result;
the FFT and spectral factorization remain double precision in both cases. The table
has 16 points per audio frame and supports a 32-frame correction tail. This is
an independent implementation of the mathematical method, not copied module
source or extracted SDK coefficients.
"""
from pathlib import Path
import cmath
import math
import struct
import argparse
import json

N = 512

def fft(values, inverse=False):
    """Iterative radix-two complex transform, normalized only on inverse."""
    result = list(map(complex, values))
    bits = (len(result)-1).bit_length()
    for index in range(len(result)):
        reverse = int(f'{index:0{bits}b}'[::-1], 2)
        if index < reverse:
            result[index], result[reverse] = result[reverse], result[index]
    width = 2
    while width <= len(result):
        turn = cmath.exp((2j if inverse else -2j)*math.pi/width)
        for start in range(0, len(result), width):
            rotation = 1+0j
            for offset in range(width//2):
                left = result[start+offset]
                right = rotation*result[start+offset+width//2]
                result[start+offset] = left+right
                result[start+offset+width//2] = left-right
                rotation *= turn
        width *= 2
    return [value/len(result) for value in result] if inverse else result

def f32(value):
    """Round a finite scalar to IEEE binary32, independently of native SIMD."""
    return struct.unpack('<f', struct.pack('<f', value))[0]


def sampled_pulse(sampling='float64'):
    if sampling not in ('float64', 'float32'):
        raise ValueError('unknown sampling precision')
    rounded = f32 if sampling == 'float32' else float
    pulse = []
    for i in range(N):
        fraction = rounded(i / (N - 1))
        position = rounded(rounded(32. * fraction) - 16.)
        # Standard four-term Blackman-Harris cosine series. Explicit rounding
        # makes the float32 experiment reproducible without SDK dependencies.
        window = 0.
        for harmonic, weight in enumerate((.35875, -.48829, .14128, -.01168)):
            angle = rounded(rounded(2. * harmonic * rounded(math.pi)) * fraction)
            cosine = rounded(math.cos(angle))
            term = rounded(rounded(weight) * cosine)
            window = rounded(window + term)
        angle = rounded(math.pi * position)
        sinc = rounded(rounded(math.sin(angle)) / angle) if angle else 1.
        pulse.append(rounded(sinc * window))
    return pulse


def coefficients(sampling='float64', retain_midpoint=True):
    pulse = sampled_pulse(sampling)
    cepstrum = fft([math.log(max(abs(value), math.exp(-30.))) for value in fft(pulse)], inverse=True)
    # Causal cepstral lifter: keep DC, double strictly positive quefrencies,
    # keep the self-conjugate midpoint once, and discard negative quefrencies.
    # See the even-length rceps construction cited by Brandt:
    # https://www.mathworks.com/help/signal/ref/rceps.html
    # Dropping the midpoint is exposed only to diagnose the former generator.
    midpoint = cepstrum[N//2].real if retain_midpoint else 0.
    causal = [cepstrum[0].real]+[2*value.real for value in cepstrum[1:N//2]]+[midpoint]+[0.]*(N//2-1)
    minimum_phase = fft([cmath.exp(value) for value in fft(causal)], inverse=True)
    integrated = []
    total = 0.
    for value in minimum_phase:
        total += value.real
        integrated.append(total)
    return [value/total for value in integrated]+[1.]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sampling', choices=('float64', 'float32'), default='float64')
    parser.add_argument('--drop-midpoint', action='store_true', help='diagnose the former nonstandard cepstral lifter')
    parser.add_argument('--json', action='store_true', help='print generated values without writing production output')
    parser.add_argument('--output', type=Path, help='write Rust coefficients to this path instead of production')
    args = parser.parse_args()
    values = coefficients(args.sampling, not args.drop_midpoint)
    if args.json:
        print(json.dumps(values, allow_nan=False))
        return
    bits = [struct.unpack('<I',struct.pack('<f',value))[0] for value in values]
    body = '\n'.join('    '+', '.join(f'f32::from_bits(0x{value:08x})' for value in bits[i:i+4])+',' for i in range(0,len(bits),4))
    destination = args.output or Path('/tmp/openpatch-independent-minblep.rs')
    precision = f'//! Sampling: {args.sampling}; transform/factorization: float64; midpoint retained: {not args.drop_midpoint}.\n'
    destination.write_text('//! Original numerical MinBLEP design from Brandt (ICMC 2001), sections 6.1–6.3.\n//! Reproduce with scripts/wavetable_kernel/generate.py; no SDK code/data used.\n'+precision+'\npub(crate) const SYNC_BLEP: [f32; 513] = [\n'+body+'\n];\n')
    print(destination)

if __name__ == '__main__':
    main()
