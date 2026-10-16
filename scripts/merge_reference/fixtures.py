#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Deterministic fixture generator; no SDK, compiler, subprocess or oracle access."""
import struct


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def patterns(acceptance):
    for slot in range(16):
        for width in range(17):
            ports = [[0, *[991.0 + lane for lane in range(16)]] for _ in range(16)]
            ports[slot] = [width, *[
                f32(((-1 if (slot + lane) % 2 else 1) * (slot + lane + 1)) / 2)
                if lane < width else 991.0 + lane for lane in range(16)
            ]]
            yield f'slot{slot + 1:02}_width{width:02}', ports
    state = acceptance['mixed_seed']

    def draw():
        nonlocal state
        state ^= (state << 13) & 0xffffffff
        state ^= state >> 17
        state ^= (state << 5) & 0xffffffff
        state &= 0xffffffff
        return state

    for case in range(acceptance['seeded_mixed_cases']):
        ports = []
        for slot in range(16):
            width = draw() % 17
            values = [f32((draw() % 32769 - 16384) / 1024) for _ in range(16)]
            if (case + slot) % 4 == 0:
                values[0] = 0.0
            # Deterministically ensure empty ports, mono ports and zero-volt
            # occupied high slots occur in the mixed collection.
            if slot == case % 16:
                width = 0
            if slot == (case + 7) % 16:
                width, values[0] = 1, 0.0
            ports.append([width, *[
                value if lane < width else 991.0 + lane
                for lane, value in enumerate(values)
            ]])
        yield f'mixed{case:02}', ports


def rows(acceptance):
    for selection in acceptance['wire_configurations']:
        for name, ports in patterns(acceptance):
            flat = [selection, *[value for port in ports for value in port]]
            assert len(flat) == acceptance['fields_per_input_row']
            yield f'channels{selection}_{name}', flat


def fixture_text(acceptance):
    return ''.join(' '.join(format(f32(v), '.9g') for v in row) + '\n' for _, row in rows(acceptance))
