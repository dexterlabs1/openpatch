# SPDX-License-Identifier: MIT
"""Original deterministic host stimuli; no upstream DSP implementation."""
import hashlib
import json
import math
from pathlib import Path
import struct
from functools import lru_cache

HERE = Path(__file__).resolve().parent
ACCEPTANCE = json.loads((HERE / 'acceptance.json').read_text())
PARAM_NAMES = ['rate', 'shape', 'unipolar', 'unused_mode', 'probability', 'spread',
               'rate_cv', 'shape_cv', 'probability_cv', 'spread_cv']
INPUT_NAMES = ['rate', 'shape', 'trigger', 'external', 'probability', 'spread']
OUTPUT_NAMES = ['stepped', 'linear', 'smooth', 'exponential', 'trigger']
INPUT_FIELDS = 115
OUTPUT_FIELDS = 129


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def adjacent(value, offset):
    bits = struct.unpack('<I', struct.pack('<f', value))[0]
    return struct.unpack('<f', struct.pack('<I', bits + offset))[0]


def defaults():
    return [1., 1., 0., 0., 1., 1., 0., 0., 0., 0.]


def frame_count(name, rate):
    return rate + 512 if name == 'source_defaults' else ACCEPTANCE['frames_per_case']


@lru_cache(maxsize=8192)
def token(value):
    return format(f32(value), '.9g')


def rows(name, rate):
    for frame in range(frame_count(name, rate)):
        event, params = 0, defaults()
        ports = [[0, *[991. - lane for lane in range(16)]] for _ in range(6)]
        # Vary both slots even if unused. These are already representable f32s.
        draws = [((frame * 73 + 19) % 1024) / 1024,
                 ((frame * 151 + 401) % 1024) / 1024]

        def port(index, values, width=1):
            values = values if isinstance(values, list) else [values] * width
            assert len(values) >= width
            ports[index] = [width, *values[:width], *[991. - lane for lane in range(width, 16)]]

        def external(width=5):
            port(3, [(-1 if lane % 2 else 1) * (0.25 + lane * 0.47 + (frame // 97 % 7) * 0.13)
                     for lane in range(width)], width)

        def trigger(period=193):
            # A low first sample deliberately initializes the source detector.
            port(2, 10. if frame % period in [1, 2, 3] else 0.)

        trigger()
        external()
        params[1] = 0.673
        if name == 'source_defaults':
            params = defaults()
            port(2, [], 0)
            port(3, [], 0)
        elif name == 'external_voltage_probability_one':
            external(16)
        elif name == 'startup_positive':
            port(2, 10. if frame < 37 or frame % 193 in [1, 2, 3] else 0.)
        elif name == 'startup_between_thresholds':
            port(2, 1. if frame < 37 else (2. if frame < 71 else (0.1 if frame < 101 else (2. if frame % 137 == 1 else 0.))))
        elif name == 'schmitt_thresholds':
            levels = [0.1, adjacent(0.1, 1), 1., adjacent(2., -1), 2., 5., 0.2, adjacent(0.1, 1), 0.1, 2., 0.]
            port(2, levels[frame // 7 % len(levels)])
        elif name == 'internal_clock':
            port(2, [], 0)
            params[0] = [math.log2(2000), 7., math.log2(0.002), 10.][frame // 1024 % 4]
        elif name == 'internal_rate_cv':
            port(2, [], 0)
            params[0], params[6] = 8.37, [-1., -0.5, 0., 0.5, 1.][frame // 97 % 5]
            port(0, [-3.25, 7., -7.], 3)
        elif name == 'clock_switch_gaps':
            params[0] = 9.25
            block = frame // 317 % 6
            if block in [0, 4]:
                port(2, [], 0)
            elif block == 2:
                port(2, 10.)
            elif block == 3:
                port(2, 0.)
            else:
                trigger(53 if block == 1 else 271)
        elif name == 'rejected_clock_updates':
            params[4] = [0., 0.2, 1.][frame // 257 % 3]
            trigger(53 if frame < 2048 else 271)
            draws[0] = 0.75
        elif name == 'probability_draw_boundaries':
            params[4] = [0., 0.25, 0.5, adjacent(1., -1), 1.][frame // 157 % 5]
            draws[0] = [0., adjacent(0.25, -1), 0.25, adjacent(0.25, 1), 0.5, adjacent(1., -1)][frame // 11 % 6]
            trigger(11)
            if frame // 503 % 2:
                port(3, [], 0)
        elif name == 'random_target_draws':
            port(3, [], 0)
            trigger(29)
            draws = [[0., 0.25, 0.5, 0.75, adjacent(1., -1)][frame // 29 % 5], 0.375]
        elif name == 'spread_zero_one':
            port(3, [], 0)
            params[5] = [0., 1., 0.2, 0.637][frame // 251 % 4]
            trigger(31)
        elif name == 'bipolar_unipolar':
            port(3, [], 0)
            params[2] = float(frame // 251 % 2)
            trigger(31)
        elif name == 'shape_zero_tiny_one':
            params[1] = [0., 1e-12, 1e-7, 0.2, 1.][frame // 127 % 5]
        elif name == 'shape_linear_threshold':
            params[1] = [adjacent(1e-6, -1), f32(1e-6), adjacent(1e-6, 1)][frame // 67 % 3]
        elif name == 'shape_exponential_thresholds':
            low, high = f32(1e-20 ** (1 / 8)), f32(0.999 ** (1 / 8))
            params[1] = [adjacent(low, -1), low, adjacent(low, 1), adjacent(high, -1), high, adjacent(high, 1), 1.][frame // 71 % 7]
        elif name == 'stepped_ceil_boundaries':
            boundary = f32(math.sqrt((1 + frame // 113 % 14) / 15))
            params[1] = adjacent(boundary, frame // 37 % 3 - 1)
            trigger(113)
        elif name == 'held_external_changes':
            external(16)
            trigger(509)
        elif name == 'width_shrink_regrow':
            external([16, 1, 5, 0, 3, 16][frame // 257 % 6])
            trigger(47)
        elif name == 'poisoned_padding_raw_zero':
            widths = [0, 1, 3, 5, 16]
            external(widths[frame // 83 % 5])
            for index in [0, 1, 4, 5]:
                width = widths[(frame // 29 + index) % 5]
                port(index, [(-1 if lane % 2 else 1) * (lane + 1) for lane in range(width)], width)
            params[6:] = [0.3, -0.7, 0.6, -0.4]
        elif name == 'mono_cv_first_lane':
            external(16)
            params[6:] = [0.5, -0.75, 0.3, 0.8]
            for index in [0, 1, 4, 5]:
                port(index, [2., -100., 100., -50., 50.], 1 if frame // 193 % 2 == 0 else 5)
            # Trigger CV is monophonic even with multiple active input lanes.
            port(2, [10. if frame % 193 in [1, 2, 3] else 0., 10., 0.], 3)
        elif name == 'cv_saturation':
            params[7:] = [1., 1., -1.]
            params[4] = params[5] = 0.5
            params[1] = 0.5
            for index in [1, 4, 5]:
                port(index, [-1e6, -10., 0., 10., 1e6][(frame // 79 + index) % 5])
            if frame // 317 % 2:
                port(3, [], 0)
        elif name == 'source_parameter_reset_retains_runtime':
            params[0], params[1], params[2], params[4] = 8.5, 0.31, 1., 0.75
            if frame >= 1029:
                params = defaults()
            if frame == 1029:
                event = 1
        elif name == 'fresh_identity_reset_state':
            external(16)
            if frame in [257, 1029, 2307]:
                event = 2
        elif name == 'host_parameter_adoption':
            external([5, 16, 1, 16][frame // 317 % 4])
            params[1] = [0., 0.371, 1.][frame // 109 % 3]
            params[2] = float(frame // 227 % 2)
            params[4] = [0., 0.5, 1.][frame // 173 % 3]
            if frame % 109 == 0:
                event = 3
        else:
            raise ValueError(name)
        flat = [event, *params, *[x for p in ports for x in p], *draws]
        assert len(flat) == INPUT_FIELDS
        assert all(math.isfinite(x) for x in flat)
        # Avoid Rack's undefined out-of-domain exp2Floor conversion/shift paths.
        rate_cv = ports[0][1] if ports[0][0] else 0.
        assert -120 <= params[0] + rate_cv * params[6] <= 120
        assert all(0 <= x < 1 for x in draws)
        yield ' '.join(map(token, flat))


def inputs():
    for rate in ACCEPTANCE['sample_rates']:
        for name in ACCEPTANCE['cases']:
            yield rate, name, '\n'.join(rows(name, rate)) + '\n'


def manifest():
    return {
        'declaration': 'Frozen before first Rust comparison, 2026-10-15',
        'acceptance_sha256': hashlib.sha256((HERE / 'acceptance.json').read_bytes()).hexdigest(),
        'generator_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'input_fields': INPUT_FIELDS,
        'output_fields': OUTPUT_FIELDS,
        'frame_counts': {f'{rate}:{name}': frame_count(name, rate)
                         for rate in ACCEPTANCE['sample_rates'] for name in ACCEPTANCE['cases']},
        'input_sha256': {f'{rate}:{name}': hashlib.sha256(data.encode()).hexdigest()
                         for rate, name, data in inputs()},
    }
