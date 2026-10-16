#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Deterministic finite Split fixtures; no compiler, network, subprocess or oracle."""
import struct


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def rows(acceptance):
    for width in acceptance['input_widths']:
        for hot in range(16):
            values = [0.0] * 16
            if hot < width:
                values[hot] = f32((hot + 1) * (-1 if hot % 2 else 1))
            yield f'width{width:02}_hot{hot + 1:02}', [width, *values]
    for name in ['all_zero', 'signed_zero', 'finite_extremes']:
        for width in acceptance['input_widths']:
            values = [0.0] * 16
            for lane in range(width):
                values[lane] = (0.0 if name == 'all_zero' else
                                (-0.0 if lane % 2 else 0.0) if name == 'signed_zero' else
                                (-16.0 if lane % 2 else 16.0))
            yield f'width{width:02}_{name}', [width, *values]
    state = acceptance['mixed_seed']

    def draw():
        nonlocal state
        state ^= (state << 13) & 0xffffffff
        state ^= state >> 17
        state ^= (state << 5) & 0xffffffff
        state &= 0xffffffff
        return state

    for case in range(acceptance['seeded_mixed_cases']):
        width = draw() % 17
        values = [f32((draw() % 32769 - 16384) / 1024) if lane < width else 0.0
                  for lane in range(16)]
        if width and case % 4 == 0:
            values[case % width] = -0.0 if case % 8 == 0 else 0.0
        yield f'mixed{case:02}', [width, *values]


def fixture_text(acceptance):
    return ''.join(' '.join(format(f32(value), '.9g') for value in row) + '\n'
                   for _, row in rows(acceptance))


def raw_rows(acceptance):
    for width in acceptance['input_widths']:
        for rotation in acceptance['raw_rotations']:
            base = acceptance['raw_value_bits']
            values = [base[(lane + rotation) % 16] for lane in range(16)]
            yield f'raw_width{width:02}_rotate{rotation:02}', [width, *values]


def raw_fixture_text(acceptance):
    return ''.join(' '.join(str(value) for value in row) + '\n'
                   for _, row in raw_rows(acceptance))
