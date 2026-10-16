#!/usr/bin/env python3
"""Frozen input schedules only: no oscillator implementation, SDK or subprocess.

rows(rate, acceptance) -> (case name, zero-based frame index, 76 numeric fields).
cases(rate, acceptance) retains compressed segments and independent anchors.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

HERE = Path(__file__).resolve().parent


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def float_bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def from_bits(value):
    return struct.unpack('<f', struct.pack('<I', value))[0]


LOW = f32(.1)
BELOW_LOW = from_bits(float_bits(LOW) - 1)
ABOVE_LOW = from_bits(float_bits(LOW) + 1)
BELOW_HIGH = from_bits(float_bits(2.) - 1)
ABOVE_HIGH = from_bits(float_bits(2.) + 1)
PARAMS = [1., 0., .5, 0., 1., 0.]


def port(width=0, values=0.):
    assert 0 <= width <= 16
    active = list(values) if isinstance(values, (list, tuple)) else [values] * width
    assert len(active) >= width
    return [width, *[f32(v) for v in active[:width]], *([0.] * (16 - width))]


def frame(params=None, fm=None, pwm=None, reset=None, clock=None, mask=15, event=0):
    return [event, *map(f32, PARAMS if params is None else params),
            *(port() if fm is None else fm), *(port() if pwm is None else pwm),
            *(port() if reset is None else reset), *(port() if clock is None else clock), mask]


def segment(count, **kwargs):
    assert count > 0
    return {'repeat': count, 'fields': frame(**kwargs)}


def case(name, segments, tags, anchors=None):
    # Fresh applies on the first expanded frame only, not every repetition.
    return {'name': name, 'tags': tags, 'segments': segments, 'anchors': anchors or []}


def modes():
    for offset in [0, 1]:
        for invert in [0, 1]:
            yield offset, invert


def cases(rate, acceptance):
    for offset, invert in modes():
        for frequency in acceptance['fixture_plan']['rate_values']:
            for width in acceptance['fixture_plan']['widths']:
                p = [frequency, 0, .5, 0, offset, invert]
                yield case(f'wave_o{offset}i{invert}_r{frequency}_w{width}',
                    [segment(acceptance['fixture_plan']['waveform_frames'], params=p,
                             fm=port(width, [(i - 8) / 2 for i in range(16)]))],
                    ['four_modes', 'frequency_range', 'widths', 'light_divider'],
                    [{'frame': 0, 'clock_frequency': 2., 'clock_timer': 0., 'lights': [0.] * 5}])
    for width in range(17):
        for amount in [-1, 0, .375, 1]:
            for frequency in [-8, 1, 10]:
                yield case(f'fm_w{width}_a{amount}_r{frequency}',
                    [segment(32, params=[frequency, amount, .5, 0, 1, 0],
                             fm=port(width, [(-1 if i % 2 else 1) * (i + 1) for i in range(16)]))],
                    ['fm_raw_lanes', 'all_widths', 'fractional_exponent'])
    for offset, invert in modes():
        for pw in [.01, .5, .99]:
            for amount in [-1, 0, 1]:
                for width in [0, 1, 3, 5, 16]:
                    yield case(f'pwm_o{offset}i{invert}_p{pw}_a{amount}_w{width}',
                        [segment(33, params=[10, 0, pw, amount, offset, invert],
                                 fm=port(5), pwm=port(width, [(-16 if i % 2 else 16) for i in range(16)]))],
                        ['pwm_clamps', 'mono_broadcast', 'short_poly_zero', 'four_modes'])
    levels = [2., 2., .5, ABOVE_LOW, BELOW_HIGH, 2., LOW, BELOW_HIGH,
              2., 2., ABOVE_LOW, 2., BELOW_LOW, ABOVE_HIGH, -16., 16.]
    for offset, invert in modes():
        for fm_width in [1, 3, 5, 16]:
            for reset_width in [0, 1, 2, 5, 16]:
                segments = []
                for tick in range(48):
                    values = [levels[(tick + lane * 3) % len(levels)] for lane in range(16)]
                    segments.append(segment(1, params=[10, .25, .5, 0, offset, invert],
                        fm=port(fm_width, [(i - 8) / 8 for i in range(16)]),
                        reset=port(reset_width, values)))
                anchors = []
                if reset_width == 1:
                    anchors = [{'frame': tick, 'phase_lanes_zero': list(range(((fm_width + 3) // 4) * 4))}
                               for tick in [8, 13, 15, 24, 29, 31, 40, 45, 47]]
                yield case(f'reset_o{offset}i{invert}_fm{fm_width}_reset{reset_width}', segments,
                    ['reset_thresholds', 'startup_high', 'advance_then_reset', 'mono_broadcast', 'short_poly_zero'], anchors)
    for width in [1, 3, 16]:
        for initial in [2., 0., .5]:
            # Other clock lanes deliberately disagree; only channel zero clocks.
            def clock(value):
                return port(width, [value, *[16. if i % 2 else -16. for i in range(15)]])
            segments = [segment(3, clock=clock(initial)), segment(1, clock=clock(LOW)),
                        segment(1, clock=clock(2.))]
            for period in [2, max(2, rate // 1000 - 1), math.ceil(rate / 1000) + 1,
                           rate // 200, rate // 100, rate // 400]:
                segments += [segment(period - 1, clock=clock(0.)), segment(1, clock=clock(2.))]
            segments += [segment(7, clock=clock(2.)), segment(31),
                         segment(3, clock=clock(2.)), segment(2, clock=clock(ABOVE_LOW)),
                         segment(1, clock=clock(BELOW_HIGH)), segment(1, clock=clock(2.)),
                         segment(1, clock=clock(LOW)), segment(rate // 100 - 1, clock=clock(0.)),
                         segment(1, clock=clock(ABOVE_HIGH))]
            yield case(f'clock_width{width}_startup{initial}', segments,
                ['clock_scalar', 'startup_arming', 'too_fast_rejection', 'valid_boundary', 'tempo_change',
                 'disconnect_resume', 'clock_channel_zero', 'threshold_hysteresis'])
    yield case('clock_missing_pulses', [segment(1, clock=port(1, 0.)),
        segment(1, clock=port(1, 2.)),
        segment(acceptance['fixture_plan']['long_missing_clock_frames'], clock=port(1, 0.)),
        segment(1, clock=port(1, 2.)), segment(32, clock=port(1, 2.))],
        ['long_timer_accumulation', 'missing_pulse', 'all_frames_observed'])
    extremes = [-127., -126.99999237060547, -126., -125.5, -1., -.5, -0., 0.,
                .5, 1., 10., 30., 127., 127.99998474121094, 128., 128.99998474121094]
    for offset, invert in modes():
        p = [0, 1, .5, 0, offset, invert]
        yield case(f'exp2_domain_o{offset}i{invert}', [segment(129, params=p, fm=port(16, extremes))],
            ['defined_exponent_boundaries', 'zero_exponent', 'delta_half_cap', 'ties_even_waveforms'])
        yield case(f'exp2_clock_overflow_o{offset}i{invert}',
            [segment(1, params=p, fm=port(16, extremes), clock=port(1, 0.)),
             segment(1, params=p, fm=port(16, extremes), clock=port(1, 2.)),
             segment(rate // 200 - 1, params=p, fm=port(16, extremes), clock=port(1, 0.)),
             segment(65, params=p, fm=port(16, extremes), clock=port(1, 2.))],
            ['defined_high_exponent', 'clock_scaled_frequency_overflow', 'delta_half_cap'])
    for offset, invert in modes():
        for widths in [(3, 1, 3), (5, 4, 5), (16, 1, 16)]:
            segments = []
            for stage, width in enumerate(widths):
                for tick in range(65):
                    segments.append(segment(1, params=[4, .25, .5, .5, offset, invert],
                        fm=port(width, [i / 8 for i in range(16)]),
                        pwm=port(16, [(i - 8) for i in range(16)]),
                        reset=port(16, [2. if (tick + stage * 7 + i) % 23 == 4 else 0. for i in range(16)])))
            yield case(f'quartet_o{offset}i{invert}_{widths[0]}_{widths[1]}_{widths[2]}', segments,
                ['quartet_retirement_regrowth', 'hidden_group_lanes', 'retained_reset_detectors'])
    for offset, invert in modes():
        for width in [1, 3, 5, 16]:
            segments = []
            for mask in [15, 0, 1, 2, 4, 8, 5, 10, 0, 15]:
                segments.append(segment(33, params=[7, .125, .3, .5, offset, invert],
                    fm=port(width, [i / 8 for i in range(16)]), pwm=port(1, 2.), mask=mask))
            yield case(f'consumers_o{offset}i{invert}_w{width}', segments,
                ['output_availability', 'phases_continue_unconsumed', 'first_reconnect_sample'])
    for width in [1, 5, 16]:
        segments = []
        for tick in range(97):
            # Initialize while HIGH and LOW; neither detector nor divider resets.
            init = tick in [1, 14, 15, 16, 17, 31, 48, 64, 80]
            high = tick % 11 in [0, 1, 2, 3]
            segments.append(segment(1, params=[3 + (tick // 24), .25, .2 + .2 * (tick // 32),
                -.5 if tick < 50 else .5, (tick // 16) % 2, (tick // 13) % 2],
                fm=port(width, [i / 8 for i in range(16)]), reset=port(1, 2. if high else 0.),
                clock=port(1, 2. if high else 0.), event=2 if init else 0))
        yield case(f'initialize_retained_detectors_w{width}', segments,
            ['runtime_initialize', 'retained_detectors', 'retained_divider_lights', 'receiving_parameters'])
    # Pulse comparator's strict < at exact phase .5, with neighboring f32 widths.
    for offset, invert in modes():
        for pw in [from_bits(float_bits(.5) - 1), .5, from_bits(float_bits(.5) + 1)]:
            yield case(f'pulse_boundary_o{offset}i{invert}_pw{pw}',
                [segment(33, params=[0, 1, pw, 0, offset, invert], fm=port(1, 30.))],
                ['strict_pulse_comparison', 'phase_half_and_zero', 'four_modes'])


def safety_cases(rate, acceptance):
    # No SDK execution for these schedules; inputs deliberately leave exp2 domain.
    values = [-1e6, -256., -128., from_bits(float_bits(-127.) + 1), 129., 255., 256., 1e6,
              float('nan'), float('inf'), -float('inf'), from_bits(0x7f7fffff),
              from_bits(0xff7fffff), -1., 16., -16.]
    for offset, invert in modes():
        yield case(f'safety_exponent_o{offset}i{invert}',
            [segment(65, params=[0, 1, .5, 0, offset, invert], fm=port(16, values)),
             segment(33, params=[0, 1, .5, 0, offset, invert], fm=port(1, 0.)),
             segment(33, params=[0, 1, .5, 0, offset, invert], fm=port(16, values), event=2)],
            ['rust_only', 'outside_reference_exponent_domain', 'finite_safety', 'recovery'])


def expand(all_cases):
    for item in all_cases:
        index = 0
        for part in item['segments']:
            for repeat in range(part['repeat']):
                row = list(part['fields'])
                row[0] = 1 if index == 0 else (row[0] if repeat == 0 else 0)
                yield item['name'], index, row
                index += 1


def rows(rate, acceptance):
    return expand(cases(rate, acceptance))


def safety_rows(rate, acceptance):
    return expand(safety_cases(rate, acceptance))


def row_text(row):
    def token(value):
        if isinstance(value, int):
            return str(value)
        if math.isnan(value):
            return 'NaN'
        if math.isinf(value):
            return '-inf' if value < 0 else 'inf'
        return format(value, '.9g')
    return ' '.join(map(token, row)) + '\n'


def fixture_text(rate, acceptance, safety=False):
    iterator = safety_rows if safety else rows
    return ''.join(row_text(row) for _, _, row in iterator(rate, acceptance))


def safety_fixture_text(rate, acceptance):
    return fixture_text(rate, acceptance, safety=True)


def validate_row(row, safety=False):
    assert len(row) == 76 and row[0] in [0, 1, 2] and 0 <= row[-1] <= 15
    assert -8 <= row[1] <= 10 and -1 <= row[2] <= 1 and f32(.01) <= row[3] <= f32(.99)
    assert -1 <= row[4] <= 1 and row[5] in [0, 1] and row[6] in [0, 1]
    for start in [7, 24, 41, 58]:
        width = row[start]
        assert isinstance(width, int) and 0 <= width <= 16
        assert all(float_bits(v) == 0 for v in row[start + 1 + width:start + 17])
    if not safety:
        assert all(math.isfinite(v) for v in row)
        channels = max(1, row[7])
        for lane in range(((channels + 3) // 4) * 4):
            pitch = f32(row[1] + f32(row[8 + lane] * row[2]))
            assert 0 <= f32(pitch + 127.) < 256., (pitch, row)


def digest_file(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest(acceptance):
    result = {'declaration': 'Frozen before any oracle comparison; no fitted tolerances or exclusions',
              'acceptance_sha256': digest_file(HERE / 'acceptance.json'),
              'generator_sha256': digest_file(HERE / 'fixtures.py'), 'rates': {}}
    for rate in acceptance['sample_rates']:
        fixed = acceptance['clock_reachability']['fixed_points_seconds'][str(rate)]
        assert f32(fixed + f32(1. / rate)) == fixed
        assert .5 * (from_bits(float_bits(float(fixed)) + 1) - fixed) > f32(1. / rate)
        result['rates'][str(rate)] = {}
        for name, factory, iterator in [('reference', cases, rows), ('safety', safety_cases, safety_rows)]:
            schedules = list(factory(rate, acceptance))
            assert len(set(item['name'] for item in schedules)) == len(schedules)
            hasher, count = hashlib.sha256(), 0
            for _, _, row in iterator(rate, acceptance):
                validate_row(row, name == 'safety')
                hasher.update(row_text(row).encode())
                count += 1
            if name == 'reference':
                assert count <= acceptance['fixture_plan']['maximum_reference_frames_per_rate']
            case_manifest = []
            cursor = 0
            for item in schedules:
                frames = sum(s['repeat'] for s in item['segments'])
                case_manifest.append({'name': item['name'], 'frames': frames,
                    'start_row': cursor, 'end_row_exclusive': cursor + frames,
                    'tags': item['tags'], 'anchors': item['anchors']})
                cursor += frames
            assert cursor == count
            result['rates'][str(rate)][name] = {'cases': len(schedules), 'processed_frames': count,
                'observed_frames': count, 'input_sha256': hasher.hexdigest(),
                'case_manifest': case_manifest}
    return result


def check_declaration():
    acceptance = json.loads((HERE / 'acceptance.json').read_text())
    expected = manifest(acceptance)
    assert json.loads((HERE / 'fixtures.json').read_text()) == expected, 'Frozen manifest differs'
    return expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    acceptance = json.loads((HERE / 'acceptance.json').read_text())
    expected = manifest(acceptance)
    target = HERE / 'fixtures.json'
    if args.freeze:
        with target.open('x') as output:
            output.write(json.dumps(expected, indent=2, sort_keys=True) + '\n')
    else:
        assert args.check, 'Specify --freeze or --check'
        assert json.loads(target.read_text()) == expected, 'Frozen manifest differs'
    print(json.dumps({'fixtures_sha256': digest_file(target), 'generator_sha256': expected['generator_sha256'],
                      'rates': {rate: {phase: {k: v for k, v in entry.items() if k != 'case_manifest'}
                                       for phase, entry in phases.items()} for rate, phases in expected['rates'].items()}}, indent=2))


if __name__ == '__main__':
    main()
