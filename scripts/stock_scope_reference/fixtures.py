#!/usr/bin/env python3
"""Input-only Stock Scope declaration. Never imports or evaluates a DSP model."""
import argparse
import json
import math
from pathlib import Path
import struct

RATES = (44100, 48000, 96000)
WIDTHS = tuple(range(17))
OUTPUT_MASKS = (0, 1, 2, 3)
ZERO, ONE, NEG_ZERO = 0, 0x3f800000, 0x80000000

def bits(value): return struct.unpack('<I', struct.pack('<f', value))[0]
def float_of(raw): return struct.unpack('<f', struct.pack('<I', raw))[0]
TIME_MIN = bits(-math.log2(50.0))
TIME_DEFAULT = ONE
TIME_MAX = bits(-math.log2(0.005))
PARAM_DEFAULTS = (ZERO, ZERO, ZERO, ZERO, TIME_DEFAULT, ZERO, ZERO, ONE)

def initial(x=(), y=(), trigger=(), input_mask=None, output_mask=3, params=None, data=None):
    sources = [list(x), list(y), list(trigger)]
    if input_mask is None:
        input_mask = sum((1 << port) for port, values in enumerate(sources) if values)
    return {'sources': sources, 'input_mask': input_mask, 'output_mask': output_mask,
            'parameter_bits': list(params or PARAM_DEFAULTS), 'data': {} if data is None else data}

def case(rate, name, init, events):
    return {'id': f'{rate}/{name}', 'sample_rate': rate, 'initial': init, 'events': events}
def advance(frames, expect=None):
    return {'op': 'advance', 'frames': frames, **({'expect': expect} if expect else {})}
def source(port, values): return {'op': 'source', 'port': port, 'values_bits': list(values)}
def param(port, value): return {'op': 'parameter', 'port': port, 'value_bits': value}
def connect(name, origin, port, target, input_port):
    return {'op': 'connect', 'id': name, 'source': origin, 'port': port,
            'destination': target, 'input': input_port}
def disconnect(name): return {'op': 'disconnect', 'id': name}
def modes(*changes):
    values = list(PARAM_DEFAULTS)
    for index, value in changes: values[index] = value
    return values
def nominal_count(rate, time_bits):
    # This chooses input event positions using ideal real exp2, not Rack's
    # exp2_taylor5, whose ceil result is the independent oracle target.
    return max(1, math.ceil(rate * 2.0 ** (-float_of(time_bits)) / 256.0))
def adjacent(raw):
    if raw == ZERO: return (0x80000001, ZERO, 1)
    return (raw + 1, raw, raw - 1) if raw & 0x80000000 else (raw - 1, raw, raw + 1)

def cases():
    for rate in RATES:
        # Pair coverage gives every X, Y and external-trigger width 0..16 in
        # the declared set, with four independent asymmetric rotations. All
        # four output consumer masks are orthogonal to those widths.
        for xi, xwidth in enumerate(WIDTHS):
            for rotation in range(4):
                ywidth = WIDTHS[(xi + (1, 3, 6, 9)[rotation]) % len(WIDTHS)]
                twidth = WIDTHS[(xi + (5, 7, 2, 8)[rotation]) % len(WIDTHS)]
                x = [bits((lane + 1) / 8) for lane in range(xwidth)]
                y = [bits(-(lane + 1) / 16) for lane in range(ywidth)]
                t = [bits(-1 if lane % 3 else 2) for lane in range(twidth)]
                for mask in OUTPUT_MASKS:
                    yield case(rate, f'width/{xi}/{rotation}/{mask}',
                               initial(x, y, t, output_mask=mask,
                                       params=modes((4, TIME_MAX))),
                               [advance(1), advance(1)])

        # Integer exponents are independent exact binary-rational width
        # anchors. Fractional and configured endpoints deliberately straddle
        # ceil boundaries without taking timings from any implementation.
        for name, time_bits in (
            ('configured-min-50s', TIME_MIN), ('default-half-second', TIME_DEFAULT),
            ('configured-max-5ms', TIME_MAX),
            ('integer-minus4', bits(-4)), ('integer-minus2', bits(-2)),
            ('integer-zero', ZERO), ('integer-two', bits(2)),
            ('integer-four', bits(4)), ('integer-seven', bits(7)),
            ('fraction-half', bits(0.5)), ('fraction-three-halves', bits(1.5)),
        ):
            nominal = nominal_count(rate, time_bits)
            if name.startswith('integer-') or name == 'default-half-second':
                # exp2_taylor5 is exact for integer exponents. These frame
                # counts follow the exact binary power and rational rate/256.
                timing = [advance(nominal - 1, {'buffer_index': 0,
                                                'frame_index': nominal - 1}),
                          advance(1, {'buffer_index': 1, 'frame_index': 0}),
                          advance(1, {'buffer_index': 1, 'frame_index': 1})]
            else:
                timing = [advance(max(0, nominal - 1)), advance(1), advance(1)]
            yield case(rate, f'time/{name}',
                       initial([bits(2)], [bits(-3)], [], params=modes((4, time_bits))),
                       timing)

        # In exp2Floor, adding 127.f to -TIME rounds the first adjacent value
        # to exactly 128.f (ties to even), but the next one to 128+2^-16.
        # The resulting exp2_taylor5 values are exactly 2 and just above 2;
        # the declared widths below are rational ceil results, not trace data.
        for name, time_raw, count in (
            ('exact', 0xbf800040, {44100: 345, 48000: 375, 96000: 750}[rate]),
            ('next-f32', 0xbf800041, {44100: 345, 48000: 376, 96000: 751}[rate]),
        ):
            yield case(rate, f'ceil-adjacent/{name}',
                       initial([bits(2)], [], [], params=modes((4, time_raw))),
                       [advance(count - 1, {'buffer_index': 0,
                                            'frame_index': count - 1}),
                        advance(1, {'buffer_index': 1, 'frame_index': 0}),
                        advance(1, {'buffer_index': 1, 'frame_index': 1})])

        # Complete 256-bin capture at the source-configured 50 s endpoint.
        # Trigger is enabled and the X input remains high: the first high
        # observation only arms UNINITIALIZED, leaving the completed sweep
        # waiting rather than immediately starting a second one.
        yield case(rate, 'full-sweep/50s',
                   initial([bits(2)], [bits(-2)], [], params=modes((4, TIME_MIN), (7, ZERO))),
                   [advance(1), advance(rate * 50 + 1024), {'op': 'save',
                     'expect': {'buffer_index': 256, 'frame_index': 0}}])

        # Long enough to finish a 256-bin 5 ms sweep at all three rates.
        # Each trigger profile then issues predetermined single-frame edges;
        # no advance count is derived from a prior observed state.
        for profile in range(12):
            width = WIDTHS[profile % len(WIDTHS)]
            trig = [bits(2 if lane == width - 1 else -2) for lane in range(width)]
            x = [bits(2 if lane == 15 else -2) for lane in range(16)]
            threshold = (bits(-2), ZERO, bits(2))[profile % 3]
            xy = (ZERO, ONE, bits(-0.25), bits(0.25))[profile % 4]
            init = initial(x, [bits(-lane / 8) for lane in range(7)], trig,
                           input_mask=3 | (4 if profile % 2 == 0 else 0),
                           params=modes((4, TIME_MAX), (5, xy), (6, threshold), (7, ZERO)))
            events = [advance(1024), source(2, [bits(-10)] * width), advance(1),
                      source(2, trig), advance(1), advance(1),
                      param(6, bits(0.5)), advance(1), param(5, ZERO), advance(1)]
            yield case(rate, f'trigger/profile-{profile}', init, events)

        for threshold in (bits(-10), ZERO, bits(10)):
            high = bits(float_of(threshold) + float_of(bits(0.001)))
            middle = bits(float_of(threshold) + float_of(bits(0.0005)))
            for voltage in (*adjacent(threshold), *adjacent(high)):
                yield case(rate, f'threshold/{threshold:08x}/{voltage:08x}',
                           initial([bits(-2)], [], [bits(-2)],
                                   params=modes((4, TIME_MAX), (6, threshold), (7, ZERO))),
                           [advance(1024), source(2, [voltage]), advance(2),
                            source(2, [middle]), advance(2),
                            source(2, [bits(-10)]), advance(2),
                            source(2, [bits(10)]), advance(2)])

        # Source-derived hand anchors: no implementation trace supplies these.
        # The 5 ms TIME endpoint needs at most two frames per point here.
        yield case(rate, 'anchor/constructor', initial(), [
            {'op': 'save', 'expect': {'buffer_index': 0, 'frame_index': 0,
                                     'channels_x': 0, 'channels_y': 0}}])
        yield case(rate, 'anchor/connected-empty-source',
                   initial([], [], [], input_mask=7, output_mask=3,
                           params=modes((4, TIME_MAX))),
                   [advance(1), advance(1,
                     {'inputs.0.channels': 1, 'inputs.1.channels': 1,
                      'outputs.0.channels': 1, 'outputs.1.channels': 1})])
        yield case(rate, 'anchor/disconnected-output-zero',
                   initial([bits(2)], [bits(-2)], [], output_mask=0,
                           params=modes((4, TIME_MAX))),
                   [advance(1), advance(1,
                     {'outputs.0.channels': 0, 'outputs.1.channels': 0})])
        yield case(rate, 'anchor/two-sample-extrema',
                   initial([bits(2)], [bits(-3)], [], params=modes((4, TIME_MAX))),
                   [advance(2), source(0, [bits(-4)]), source(1, [bits(5)]),
                    advance(2, {'point_buffer.1.0.0.min_bits': bits(-4),
                                'point_buffer.1.0.0.max_bits': bits(2),
                                'point_buffer.1.1.0.min_bits': bits(-3),
                                'point_buffer.1.1.0.max_bits': bits(5)} if rate == 96000 else
                               {'point_buffer.3.0.0.min_bits': bits(-4),
                                'point_buffer.3.1.0.max_bits': bits(5)})])
        yield case(rate, 'anchor/signed-zero',
                   initial([NEG_ZERO], [ZERO], [], params=modes((4, TIME_MAX))),
                   [advance(2), source(0, [ZERO]), source(1, [NEG_ZERO]),
                    advance(2, {'point_buffer.1.0.0.min_bits': NEG_ZERO,
                                'point_buffer.1.0.0.max_bits': NEG_ZERO,
                                'point_buffer.1.1.0.min_bits': ZERO,
                                'point_buffer.1.1.0.max_bits': ZERO} if rate == 96000 else
                               {'point_buffer.2.0.0.min_bits': NEG_ZERO,
                                'point_buffer.3.0.0.min_bits': ZERO,
                                'point_buffer.2.1.0.max_bits': ZERO,
                                'point_buffer.3.1.0.max_bits': NEG_ZERO})])
        yield case(rate, 'anchor/subnormal-direct-copy',
                   initial([1, 0x80000001], [0x007fffff], [],
                           params=modes((4, TIME_MAX))),
                   [advance(1, {'source_outputs.0.values_bits.0': 1,
                                'source_outputs.0.values_bits.1': 0x80000001,
                                'source_outputs.1.values_bits.0': 0x007fffff,
                                'inputs.0.values_bits.0': 1,
                                'inputs.0.values_bits.1': 0x80000001,
                                'inputs.1.values_bits.0': 0x007fffff}),
                    advance(1, {'outputs.0.values_bits.0': 1,
                                'outputs.0.values_bits.1': 0x80000001,
                                'outputs.1.values_bits.0': 0x007fffff}),
                    advance(2, {'point_buffer.1.0.0.min_bits': 1,
                                'point_buffer.1.0.0.max_bits': 1,
                                'point_buffer.1.0.1.min_bits': 0x80000001,
                                'point_buffer.1.0.1.max_bits': 0x80000001,
                                'point_buffer.1.1.0.min_bits': 0x007fffff,
                                'point_buffer.1.1.0.max_bits': 0x007fffff})])
        # FTZ comparisons see +subnormal and -subnormal as equal zero. A
        # within-bin tie retains the first raw operand in std::min/max.
        signed_bin = {44100: 6, 48000: 6, 96000: 12}[rate]
        yield case(rate, 'anchor/signed-subnormal-same-bin',
                   initial([1], [0x80000001], [], params=modes((4, bits(5)))),
                   [advance(signed_bin), source(0, [0x80000001]), source(1, [1]),
                    advance(signed_bin,
                            {'point_buffer.1.0.0.min_bits': 1,
                             'point_buffer.1.0.0.max_bits': 1,
                             'point_buffer.1.1.0.min_bits': 0x80000001,
                             'point_buffer.1.1.0.max_bits': 0x80000001,
                             'outputs.0.values_bits.0': 0x80000001,
                             'outputs.1.values_bits.0': 1})])
        for control, name in ((5, 'xy'), (7, 'trigger')):
            events = []
            for raw_value in (1, 0x80000001, 0x00800000, 0x80800000):
                active = bool(raw_value & 0x7f800000)
                events.append(param(control, raw_value))
                events.append(advance(1, {
                    f'parameter_bits.{control}': raw_value,
                    'light_bits.0': ONE if control == 5 and active else ZERO,
                    'light_bits.1': ONE if control == 7 and not active else ZERO,
                }))
            yield case(rate, f'control/{name}-ftz-boundaries',
                       initial([bits(1)], [bits(-1)], [], params=modes((4, TIME_MAX))),
                       events)
        # Finite binary32 extrema remain within the source's defined direct
        # copies and ordered std::min/std::max. The second pair lands in the
        # same bin at 96 kHz and in successive bins at the lower two rates.
        maximum = 0x7f7fffff
        minimum = 0xff7fffff
        yield case(rate, 'anchor/finite-extreme-amplitudes',
                   initial([maximum], [minimum], [], params=modes((4, TIME_MAX))),
                   [advance(2), source(0, [minimum]), source(1, [maximum]),
                    advance(2, {'point_buffer.1.0.0.min_bits': minimum,
                                'point_buffer.1.0.0.max_bits': maximum,
                                'point_buffer.1.1.0.min_bits': minimum,
                                'point_buffer.1.1.0.max_bits': maximum} if rate == 96000 else
                               {'point_buffer.2.0.0.max_bits': maximum,
                                'point_buffer.3.0.0.min_bits': minimum,
                                'point_buffer.2.1.0.min_bits': minimum,
                                'point_buffer.3.1.0.max_bits': maximum})])
        yield case(rate, 'anchor/lane-retire-regrow',
                   initial([bits(lane + 1) for lane in range(16)], [bits(-lane - 1) for lane in range(16)],
                           [], params=modes((4, TIME_MAX))),
                   [advance(3), source(0, [bits(-3)]), source(1, []), advance(3),
                    source(0, [bits(lane + 1) for lane in range(16)]), advance(3)])
        yield case(rate, 'anchor/first-high-only-arms',
                   initial([bits(2)], [], [], params=modes((4, TIME_MAX), (7, ZERO))),
                   [advance(1024, {'buffer_index': 256}), advance(1),
                    source(0, [bits(-2)]), advance(2), source(0, [bits(2)]),
                    advance(2, {'buffer_index': 1 if rate != 96000 else 0})])
        yield case(rate, 'anchor/all-sixteen-detectors-and-highest-lane',
                   initial([bits(-2)] * 16, [bits(1)], [bits(2)] * 16,
                           params=modes((4, TIME_MAX), (7, ZERO))),
                   [advance(1024, {'buffer_index': 256,
                                   'triggers.0': 1, 'triggers.15': 1}),
                    source(2, [bits(-2)] * 16),
                    advance(2, {'triggers.0': 0, 'triggers.15': 0}),
                    source(2, [bits(-2)] * 15 + [bits(2)]),
                    advance(2, {'triggers.0': 2, 'triggers.15': 2,
                                'buffer_index': 1 if rate != 96000 else 0})])
        yield case(rate, 'anchor/trigger-sample-included',
                   initial([bits(-2)], [], [bits(-2)], params=modes((4, TIME_MAX), (7, ZERO))),
                   [advance(1024), source(2, [bits(2)]), source(0, [bits(3)]),
                    advance(1), advance(1,
                      {'buffer_index': 1 if rate != 96000 else 0,
                       'point_buffer.0.0.0.min_bits' if rate != 96000 else
                       'current_points.0.0.min_bits': bits(3)})])
        yield case(rate, 'anchor/bin255-and-next-sweep',
                   initial([bits(1)], [], [], params=modes((4, TIME_MAX))),
                   [advance(255 * (2 if rate == 96000 else 1),
                            {'buffer_index': 255, 'frame_index': 0}),
                    advance((2 if rate == 96000 else 1) - 1,
                            {'buffer_index': 255, 'frame_index': (2 if rate == 96000 else 1) - 1}),
                    advance(1, {'buffer_index': 256, 'frame_index': 0}),
                    advance(1, {'buffer_index': 0 if rate == 96000 else 1})])

        # Display-only controls are still native Params: retain their raw
        # values and reset them through the genuine Module event.
        yield case(rate, 'control/display-param-reset',
                   initial([bits(1)], [bits(-1)], [], params=modes(
                       (0, bits(8)), (1, bits(-10)), (2, bits(4)),
                       (3, bits(10)), (4, TIME_MAX))),
                   [advance(2), param(0, ZERO), param(1, bits(-3)),
                    param(2, ONE), param(3, bits(3)),
                    {'op': 'reset', 'expect': {'parameter_bits.0': ZERO,
                                               'parameter_bits.1': ZERO,
                                               'parameter_bits.2': ZERO,
                                               'parameter_bits.3': ZERO}},
                    advance(1)])

        # Distinct live lifecycle states: partial, completed/waiting and fresh.
        base = initial([bits(2)], [bits(-2)], [bits(-2)],
                       params=modes((4, TIME_MAX), (7, ZERO)))
        partial_base = initial([bits(2)], [bits(-2)], [bits(-2)],
                               params=modes((4, ZERO), (7, ZERO)))
        yield case(rate, 'life/initialize-partial', partial_base,
                   [advance(3), {'op': 'reset',
                     'expect': {'buffer_index': 0, 'frame_index': 3,
                                'current_points.0.0.min_bits': ZERO,
                                'current_points.0.0.max_bits': bits(2),
                                'current_points.1.0.min_bits': bits(-2),
                                'current_points.1.0.max_bits': ZERO}},
                    advance(1), {'op': 'save'}])
        yield case(rate, 'life/initialize-waiting', base,
                   [advance(1024), source(2, [bits(2)]), advance(1),
                    {'op': 'reset', 'expect': {'buffer_index': 256,
                                              'triggers.0': 0}},
                    param(7, ZERO),
                    advance(1, {'buffer_index': 0, 'triggers.0': 2})])
        yield case(rate, 'life/adopt-receiving-controls', partial_base,
                   [advance(3), {'op': 'adopt', 'parameter_bits': modes((4, bits(2)),
                                                                     (5, ONE), (7, ONE))},
                    advance(1)])
        yield case(rate, 'life/rate-partial-and-waiting', partial_base,
                   [advance(3), {'op': 'rate', 'sample_rate': 96000 if rate != 96000 else 44100,
                                 'expect': {'frame_index': 3, 'buffer_index': 0}},
                    advance(1), param(4, TIME_MAX), advance(1024),
                    {'op': 'rate', 'sample_rate': rate}, advance(1)])
        yield case(rate, 'life/bypass-partial', partial_base,
                   [advance(3), {'op': 'bypass', 'value': True,
                                 'expect': {'outputs.0.channels': 1,
                                            'outputs.1.channels': 1,
                                            'outputs.0.values_bits.0': ZERO,
                                            'outputs.1.values_bits.0': ZERO}}, advance(3),
                    {'op': 'bypass', 'value': False}, advance(1)])
        yield case(rate, 'life/bypass-waiting', base,
                   [advance(1024), {'op': 'bypass', 'value': True}, advance(2),
                    {'op': 'bypass', 'value': False}, advance(1)])
        yield case(rate, 'life/recreate', partial_base,
                   [advance(3), {'op': 'recreate'}, {'op': 'save'},
                    connect('new_x', 'source', 0, 'module', 0),
                    connect('new_output', 'module', 0, 'sink', 0), advance(2)])
        yield case(rate, 'life/time-threshold-edit-within-bin', partial_base,
                   [advance(3), param(4, bits(2)), param(6, bits(0.75)),
                    source(0, [bits(-5)]), advance(1), param(4, TIME_MAX), advance(2)])
        yield case(rate, 'life/cable-replace-while-waiting', base,
                   [advance(1024, {'buffer_index': 256}), disconnect('in2'),
                    connect('in2', 'source', 1, 'module', 2), advance(2),
                    disconnect('in2'), connect('in2', 'source', 0, 'module', 2), advance(2)])

        # Physical output fanout/last-consumer and input replacement are
        # separate from source held controls and the one-frame cable delay.
        for port in range(3):
            yield case(rate, f'topology/input-{port}', base,
                       [advance(1), disconnect(f'in{port}'), advance(1),
                        connect(f'in{port}', 'source', port, 'module', port), advance(1),
                        connect('stack', 'source', (port + 1) % 3, 'module', port),
                        advance(1), disconnect('stack'), advance(1)])
        for output in range(2):
            yield case(rate, f'topology/output-{output}', base,
                       [advance(1), connect('fanout_middle', 'module', output, 'sink', 1 - output),
                        connect('fanout_last', 'module', output, 'sink', output), advance(1),
                        disconnect('fanout_middle'), advance(1),
                        disconnect(f'out{output}'), advance(1),
                        disconnect('fanout_last'), advance(1),
                        connect(f'out{output}', 'module', output, 'sink', output), advance(1)])

        # Jansson json_integer_value: nonzero integers set a parameter to 1;
        # zero, missing fields, booleans and floating numbers do not clear it.
        for index, data in enumerate(({}, {'lissajous': 0.5, 'external': -0.5}, {'lissajous': -1},
                                      {'external': 2}, {'lissajous': 0, 'external': 0},
                                      {'lissajous': True, 'external': 1})):
            if index == 4:
                yield case(rate, f'legacy-json/{index}',
                           initial(params=modes((7, ZERO))),
                           [{'op': 'data_load', 'value': {'lissajous': -1, 'external': 2}},
                            {'op': 'data_load', 'value': data,
                             'expect': {'parameter_bits.5': ONE, 'parameter_bits.7': ONE}},
                            {'op': 'data_load', 'value': {},
                             'expect': {'parameter_bits.5': ONE, 'parameter_bits.7': ONE}},
                            {'op': 'save'}, advance(1)])
            else:
                yield case(rate, f'legacy-json/{index}', initial(data=data),
                           [{'op': 'data_load', 'value': data,
                             **({'expect': {'parameter_bits.5': ZERO,
                                            'parameter_bits.7': ONE}} if index == 1 else {})},
                            {'op': 'save'}, advance(1)])

def safety_records():
    for role in ('x', 'y', 'trigger', 'time', 'threshold', 'xy', 'trigger-mode'):
        for raw in (0x7f800000, 0xff800000, 0x7fc00001, 0xffc00001,
                    0x7f7fffff, 0xff7fffff):
            if raw & 0x7f800000 == 0x7f800000:
                classification = ('nonfinite input/control outside the declared finite '
                                  'SDK comparison domain; local Rust safety only')
            elif role == 'time':
                classification = ('finite TIME outside the declared bounded domain; '
                                  'float-to-int32 conversion in exp2Floor exceeds its range')
            else:
                classification = ('finite extreme outside this role-specific safety '
                                  'record\'s sampled SDK matrix; local Rust safety only; '
                                  'selected XY extrema also have ordinary exact anchors')
            yield {'id': f'{role}/{raw:08x}', 'role': role, 'bits': raw,
                   'classification': classification}
    for raw in (-200, -130, 160, 200):
        classification = ('outside declared bounded TIME domain; positive exponent '
                          'path not classified as proven undefined by this gate'
                          if raw < 0 else
                          'outside declared bounded TIME domain; exp2Floor shifts '
                          'a negative signed exponent, which is undefined in C++17')
        yield {'id': f'time-exp2-floor-outside/{raw}', 'time_raw': raw,
               'classification': classification + '; local Rust safety only'}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--safety-output', type=Path, required=True)
    args = parser.parse_args()
    for path, rows in ((args.output, cases()), (args.safety_output, safety_records())):
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('x') as target:
            for row in rows:
                target.write(json.dumps(row, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n')

if __name__ == '__main__': main()
