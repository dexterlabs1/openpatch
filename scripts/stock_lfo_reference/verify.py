#!/usr/bin/env python3
"""Streaming validator for the frozen stock LFO contract; no DSP implementation.

validate(rate, acceptance, phase, input_path, sdk_trace_path_or_None,
         rust_trace_path) returns evidence and never converts numeric failure
into an exception or stops inspecting later frames. phase is 'reference' (or
'normal') or 'safety'. File/configuration errors remain exceptions for the caller.
"""
from collections import Counter
from contextlib import ExitStack
import math
import struct

from fixtures import cases, rows, safety_cases, safety_rows


COUNTS = slice(0, 4)
PHASES = slice(68, 84)
RESET = slice(84, 100)
CLOCK_TRIGGER = 100
CLOCK_FREQUENCY = 101
CLOCK_TIMER = 102
DIVIDER = 103
LIGHTS = slice(104, 109)
WAVES = ('sine', 'triangle', 'saw', 'square')


def _bits(value):
    try:
        return struct.pack('<f', value)
    except (OverflowError, struct.error):
        return None


ZERO = _bits(0.)


def _integer(value, minimum, maximum):
    return math.isfinite(value) and value.is_integer() and minimum <= value <= maximum


def _display(value):
    packed = _bits(value)
    return f'{value!r} (f32={packed.hex() if packed is not None else "out-of-range"})'


def _read(stream, size, label, reasons, trace_rows):
    line = stream.readline()
    if not line:
        reasons.append(f'{label}: missing row at EOF')
        return None
    trace_rows[label] += 1
    tokens = line.split()
    if len(tokens) != size:
        reasons.append(f'{label}: expected {size} fields, got {len(tokens)}')
        return None
    try:
        values = []
        for token in tokens:
            value = float(token)
            packed = _bits(value)
            # Decimal trace fields encode f32 values. Range checks and the sine
            # absolute error use those f32 values, not their decimal rounding.
            values.append(struct.unpack('<f', packed)[0] if packed is not None else value)
    except ValueError as error:
        reasons.append(f'{label}: invalid numeric field: {error}')
        return None
    return values


def _exact(actual, expected):
    left, right = _bits(actual), _bits(expected)
    return left is not None and right is not None and left == right


def _check_input(actual, expected, reasons):
    if actual is None:
        return
    for index, (left, right) in enumerate(zip(actual, expected)):
        # The safety wire protocol has a plain NaN token, not a NaN payload gate.
        if math.isnan(left) and math.isnan(right):
            continue
        if not _exact(left, right):
            reasons.append(f'input: field {index} differs from frozen generated row: '
                           f'{_display(left)} != {_display(right)}')


def _check_independent(label, values, row, frame_index, previous_lights, anchors,
                       tolerance, counts, reasons):
    if values is None:
        return None, 0, 0
    counts[f'{label}_independent_frames'] += 1
    width = max(1, row[7])
    mask = row[-1]
    offset, invert = row[5], row[6]
    for wave in range(4):
        expected_count = width if mask & (1 << wave) else 0
        count = values[wave]
        if not _integer(count, 0, 16) or count != expected_count:
            reasons.append(f'{label}: {WAVES[wave]} count {count!r}, expected {expected_count}')
        counts[f'{label}_independent_output_counts'] += 1
        for lane in range(16):
            voltage = values[4 + wave * 16 + lane]
            if lane < expected_count:
                margin = tolerance if wave == 0 else 0.
                low, high = (0., 10.) if offset else (-5., 5.)
                if not math.isfinite(voltage) or not low - margin <= voltage <= high + margin:
                    reasons.append(f'{label}: active {WAVES[wave]}[{lane}]={voltage!r} '
                                   f'outside finite mode bounds [{low},{high}] + {margin}')
                counts[f'{label}_independent_active_voltage_ranges'] += 1
            elif label == 'rust':
                if _bits(voltage) != ZERO:
                    reasons.append(f'rust: inactive {WAVES[wave]}[{lane}] is not positive zero: '
                                   f'{_display(voltage)}')
                counts['rust_positive_zero_padding_values'] += 1
    for lane, value in enumerate(values[PHASES]):
        if not math.isfinite(value) or not 0. <= value < 1.:
            reasons.append(f'{label}: phase[{lane}]={value!r} is not finite in [0,1)')
    for lane, value in enumerate(values[RESET]):
        if not _integer(value, 0, 1):
            reasons.append(f'{label}: reset_high[{lane}]={value!r} is not boolean')
    if not _integer(values[CLOCK_TRIGGER], 0, 2):
        reasons.append(f'{label}: invalid scalar clock state {values[CLOCK_TRIGGER]!r}')
    frequency = values[CLOCK_FREQUENCY]
    if not math.isfinite(frequency) or not .001 <= frequency <= 1000.:
        reasons.append(f'{label}: invalid/nonfinite clock frequency {frequency!r}')
    timer = values[CLOCK_TIMER]
    if not math.isfinite(timer) or timer < 0.:
        reasons.append(f'{label}: invalid/nonfinite clock timer {timer!r}')
    expected_divider = (frame_index + 1) % 16
    if not _integer(values[DIVIDER], 0, 15) or values[DIVIDER] != expected_divider:
        reasons.append(f'{label}: light divider {values[DIVIDER]!r}, expected {expected_divider}; '
                       'runtime Initialize must retain divider history')
    lamps = values[LIGHTS]
    for index, value in enumerate(lamps):
        if not math.isfinite(value) or not 0. <= value <= 1.:
            reasons.append(f'{label}: light[{index}]={value!r} is not finite in [0,1]')
    if expected_divider == 0:
        if not _exact(lamps[3], invert) or not _exact(lamps[4], offset):
            reasons.append(f'{label}: mode lights {lamps[3:]!r} do not equal '
                           f'[invert,offset]={[invert, offset]!r} on divider tick')
        if width > 1:
            if not all(_exact(value, expected) for value, expected in zip(lamps[:3], [0., 0., 1.])):
                reasons.append(f'{label}: poly phase lights {lamps[:3]!r} != [0,0,1]')
        elif not _exact(lamps[0], lamps[1]) or not _exact(lamps[2], 0.):
            reasons.append(f'{label}: mono phase lights require identical R/G and positive-zero B: '
                           f'{lamps[:3]!r}')
    else:
        baseline = [0.] * 5 if frame_index == 0 else previous_lights
        if baseline is not None and not all(_exact(x, y) for x, y in zip(lamps, baseline)):
            reasons.append(f'{label}: lights changed between divider ticks, including Initialize: '
                           f'{lamps!r} != {baseline!r}')
    counts[f'{label}_independent_state_frames'] += 1
    checked, fields = 0, 0
    for anchor in anchors:
        checked += 1
        for key, expected in anchor.items():
            if key == 'frame':
                continue
            if key in ('clock_frequency', 'clock_timer'):
                index = CLOCK_FREQUENCY if key == 'clock_frequency' else CLOCK_TIMER
                if not _exact(values[index], expected):
                    reasons.append(f'{label}: independent anchor {key}: '
                                   f'{_display(values[index])} != {_display(expected)}')
                fields += 1
            elif key == 'lights':
                for lane, (value, wanted) in enumerate(zip(lamps, expected)):
                    if not _exact(value, wanted):
                        reasons.append(f'{label}: independent anchor light[{lane}]: '
                                       f'{_display(value)} != {_display(wanted)}')
                    fields += 1
            elif key == 'phase_lanes_zero':
                for lane in expected:
                    if not _exact(values[68 + lane], 0.):
                        reasons.append(f'{label}: independent reset anchor phase[{lane}] '
                                       f'is not positive zero: {_display(values[68 + lane])}')
                    fields += 1
            else:
                raise ValueError(f'Unsupported frozen independent anchor: {key}')
    return lamps, checked, fields


def _compare(sdk, rust, row, tolerance, counts, reasons):
    if sdk is None or rust is None:
        return None
    for wave in range(4):
        counts['output_counts'] += 1
        if sdk[wave] != rust[wave]:
            reasons.append(f'{WAVES[wave]} output count: SDK={sdk[wave]!r}, Rust={rust[wave]!r}')
    max_error = 0.
    width = max(1, row[7])
    for wave in range(4):
        if not row[-1] & (1 << wave):
            continue
        for lane in range(width):
            index = 4 + wave * 16 + lane
            left, right = sdk[index], rust[index]
            counts[f'active_{WAVES[wave]}_voltages'] += 1
            if wave == 0:
                if not math.isfinite(left) or not math.isfinite(right):
                    reasons.append(f'sine[{lane}]: nonfinite SDK={left!r}, Rust={right!r}')
                    continue
                error = abs(left - right)
                max_error = max(max_error, error)
                if error > tolerance:
                    reasons.append(f'sine[{lane}]: absolute error {error!r} > {tolerance!r}; '
                                   f'SDK={left!r}, Rust={right!r}')
            elif not _exact(left, right):
                reasons.append(f'{WAVES[wave]}[{lane}]: SDK={_display(left)}, Rust={_display(right)}')
    groups = [('phases', range(68, 84)), ('reset_flags', range(84, 100)),
              ('clock_trigger', [100]), ('clock_frequency', [101]), ('clock_timer', [102]),
              ('light_divider', [103]), ('lights', range(104, 109))]
    for name, indices in groups:
        for index in indices:
            counts[name] += 1
            if not _exact(sdk[index], rust[index]):
                reasons.append(f'{name} field {index}: SDK={_display(sdk[index])}, '
                               f'Rust={_display(rust[index])}')
    return max_error


def validate(rate, a, phase, input_path, sdk_trace_path_or_None, rust_trace_path):
    """Validate every frozen frame with O(one case's metadata + one row) memory.

    ``failed_frames`` counts expected rows with failures. Unexpected trailing
    records are separately counted in ``protocol_failures`` and also fail the
    result. ``first_failures`` retains at most 16 failing-frame summaries, each
    with at most 32 reasons and the full reason count. All remaining rows are
    still read and checked. Counts in ``compared_field_counts`` are attempted
    checks; malformed/missing trace rows cannot contribute comparison counts.
    """
    if phase not in ('reference', 'normal', 'safety'):
        raise ValueError(f'Unknown qualification phase: {phase}')
    safety = phase == 'safety'
    if safety and sdk_trace_path_or_None is not None:
        raise ValueError('Safety inputs must never be compared to an SDK oracle trace')
    if not safety and sdk_trace_path_or_None is None:
        raise ValueError('Reference phase requires the SDK trace')
    if rate not in a['sample_rates']:
        raise ValueError(f'Undeclared sample rate: {rate}')
    tolerance = a['numeric_gate']['sine_absolute_tolerance_volts']
    factory = safety_cases if safety else cases
    iterator = safety_rows if safety else rows
    # Store anchors only, not segment arrays or expanded traces.
    anchor_table = {}
    for item in factory(rate, a):
        by_frame = {}
        for anchor in item['anchors']:
            by_frame.setdefault(anchor['frame'], []).append(anchor)
        anchor_table[item['name']] = by_frame
    fields_checked, trace_rows = Counter(), Counter()
    failures = []
    failed_frames = frames = protocol_failures = anchor_count = anchor_fields = 0
    max_sine_error = None if safety else 0.
    previous_case = None
    previous_lights = {'sdk': None, 'rust': None}

    def failure(case_name, local_frame, global_row, reasons):
        if len(failures) < 16:
            failures.append({'case': case_name, 'frame': local_frame, 'row': global_row,
                             'reasons': reasons[:32], 'reason_count': len(reasons)})

    with ExitStack() as stack:
        streams = {'input': stack.enter_context(open(input_path, encoding='utf-8')),
                   'rust': stack.enter_context(open(rust_trace_path, encoding='utf-8'))}
        if not safety:
            streams['sdk'] = stack.enter_context(open(sdk_trace_path_or_None, encoding='utf-8'))
        for name, frame_index, row in iterator(rate, a):
            if name != previous_case:
                previous_case = name
                previous_lights = {'sdk': None, 'rust': None}
            reasons = []
            actual_input = _read(streams['input'], 76, 'input', reasons, trace_rows)
            _check_input(actual_input, row, reasons)
            traces = {label: _read(streams[label], 109, label, reasons, trace_rows)
                      for label in ('rust',) if label in streams}
            if not safety:
                traces['sdk'] = _read(streams['sdk'], 109, 'sdk', reasons, trace_rows)
            for label, trace in traces.items():
                previous_lights[label], checked, checked_fields = _check_independent(
                    label, trace, row, frame_index, previous_lights[label],
                    anchor_table[name].get(frame_index, []), tolerance, fields_checked, reasons)
                anchor_count += checked
                anchor_fields += checked_fields
            if not safety:
                error = _compare(traces['sdk'], traces['rust'], row, tolerance, fields_checked, reasons)
                if error is not None:
                    max_sine_error = max(max_sine_error, error)
            if reasons:
                failed_frames += 1
                failure(name, frame_index, frames, reasons)
            frames += 1
        for label, stream in streams.items():
            extra = sum(1 for _ in stream)
            if extra:
                protocol_failures += extra
                trace_rows[label] += extra
                failure('<eof>', None, frames, [f'{label}: {extra} unexpected trailing records'])
    return {
        'passed': failed_frames == 0 and protocol_failures == 0,
        'phase': 'safety' if safety else 'reference',
        'sample_rate': rate,
        'oracle_compared': not safety,
        'claim': ('Rust-only finite-safety checks; no numerical oracle parity claim' if safety else
                  'Bounded frozen module reference comparison; no full-host or hardware claim'),
        'frames': frames,
        'failed_frames': failed_frames,
        'protocol_failures': protocol_failures,
        'trace_rows': dict(trace_rows),
        'first_failures': failures,
        'max_sine_error': max_sine_error,
        'compared_field_counts': dict(sorted(fields_checked.items())),
        'independent_anchors_checked': anchor_count,
        'independent_anchor_fields_checked': anchor_fields,
        'fresh_initial_snapshot_checks': ('Performed before processing by the Rust adapter' if safety else
                                          'Performed before processing by both adapters') +
                                         '; not inferred from post-process traces',
    }
