#!/usr/bin/env python3
"""Independent source-defined inputs and rational/bitwise anchors; no DSP runs.

No production model import. Rational helpers implement IEEE encoding and the
two C++ conversion-domain checks, not a second stateful Octave processor.
"""
import argparse
from fractions import Fraction
import json
from pathlib import Path
import struct

RATES = (44100, 48000, 96000)
ZERO, NEG_ZERO, ONE = 0, 0x80000000, 0x3f800000
I32_MIN, I32_MAX = -(1 << 31), (1 << 31) - 1


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def rational(raw):
    exponent, mantissa = (raw >> 23) & 255, raw & 0x7fffff
    if exponent == 255:
        raise ValueError('nonfinite rational')
    significand = mantissa if exponent == 0 else (1 << 23) + mantissa
    power = -149 if exponent == 0 else exponent - 150
    value = Fraction(significand * (1 << max(power, 0)), 1 << max(-power, 0))
    return -value if raw >> 31 else value


def nearest_even(numerator, denominator):
    whole, remainder = divmod(numerator, denominator)
    return whole + (2 * remainder > denominator or
                    (2 * remainder == denominator and whole % 2 == 1))


def binary32(value):
    """Encode a rational using integer nearest-even rounding, independent of FPCR."""
    value = Fraction(value)
    sign = 0x80000000 if value < 0 else 0
    value = abs(value)
    if not value:
        return sign
    n, d = value.numerator, value.denominator
    exponent = n.bit_length() - d.bit_length()
    if value < Fraction(1 << max(exponent, 0), 1 << max(-exponent, 0)):
        exponent -= 1
    shift = max(exponent - 23, -149)
    significand = nearest_even(n << max(-shift, 0), d << max(shift, 0))
    if exponent < -126:
        return sign | significand
    if significand == 1 << 24:
        significand >>= 1
        exponent += 1
    if exponent > 127:
        return sign | 0x7f800000
    return sign | ((exponent + 127) << 23) | (significand - (1 << 23))


def away(value):
    value = Fraction(value)
    magnitude = abs(value)
    integer = (2 * magnitude.numerator + magnitude.denominator) // (2 * magnitude.denominator)
    return -integer if value < 0 else integer


def effective_octave(parameter, cv):
    """Source-defined conversion-domain check, including float sum before i32."""
    knob = away(rational(parameter))
    if not I32_MIN <= knob <= I32_MAX:
        raise ValueError('undefined rounded-parameter to i32 conversion')
    rounded_cv = away(rational(cv))
    # std::round(float) returns float. Large already-integral floats are exact.
    rounded = binary32(rounded_cv)
    combined = rational(binary32(rational(binary32(knob)) + rational(rounded)))
    result = int(combined)  # Fraction truncates toward zero, independently of FP.
    if not I32_MIN <= result <= I32_MAX:
        raise ValueError('undefined final float-sum to i32 conversion')
    return result


def legacy_bits(value, previous):
    if not isinstance(value, dict) or 'octave' not in value:
        return previous
    integer = value['octave'] if type(value['octave']) is int else 0
    if not -(1 << 63) <= integer < (1 << 63):
        raise ValueError('outside Jansson signed int64 domain')
    return binary32(integer)


def initial(pitch=None, cv=None, input_mask=3, output_mask=1, parameter=0, data=None):
    return {'sources': [pitch or [], cv or []], 'input_mask': input_mask,
            'output_mask': output_mask, 'parameter_bits': bits(parameter),
            'data': {} if data is None else data}


def step(expect=None):
    return {'op': 'step', **({'expect': expect} if expect else {})}


def connect(name, source, port, destination, input_port):
    return {'op': 'connect', 'id': name, 'source': source, 'port': port,
            'destination': destination, 'input': input_port}


def source(port, raw):
    return {'op': 'source', 'port': port, 'values_bits': raw}


def expect_output(numbers, count=None, last=None):
    result = {f'outputs.0.values_bits.{i}': bits(value) for i, value in enumerate(numbers)}
    if count is not None:
        result['outputs.0.channels'] = count
    if last is not None:
        result['last_octave'] = last
    return result


def case(rate, name, init, events):
    return {'id': f'{rate}/{name}', 'sample_rate': rate, 'initial': init, 'events': events}


def fresh_cases(rate):
    # 17 * 17 * 5 * 3 = 4,335. Width zero is genuinely uncabled here.
    for pitch in range(17):
        for cv in range(17):
            for knob in (-4, -1, 0, 1, 4):
                for consumers in (0, 1, 3):
                    init = initial([bits((i - 8) / 4) for i in range(pitch)],
                                   [bits((i % 7) - 3.5) for i in range(cv)],
                                   int(pitch > 0) | (int(cv > 0) << 1), consumers, knob)
                    yield case(rate, f'width/{pitch}/{cv}/{knob}/{consumers}', init, [step(), step()])
    # 10 signed CV ties * 3 neighbors * 4 knob ties * 3 neighbors = 360.
    for value in (-4.5, -3.5, -2.5, -1.5, -.5, .5, 1.5, 2.5, 3.5, 4.5):
        for cv_delta in (-1, 0, 1):
            cv = bits(value) + cv_delta
            for knob in (-3.5, -.5, .5, 3.5):
                for knob_delta in (-1, 0, 1):
                    param = bits(knob) + knob_delta
                    init = initial([bits(.25)], [cv]); init['parameter_bits'] = param
                    octave = effective_octave(param, cv)
                    expected = {'last_octave': octave, 'outputs.0.channels': 1,
                                'outputs.0.values_bits.0': binary32(Fraction(1, 4) + octave)}
                    yield case(rate, f'tie/{bits(value):08x}/{cv_delta}/{bits(knob):08x}/{knob_delta}',
                               init, [step(), step(expected)])
    # Every finite exponent, both signs, four mantissas, three knob settings.
    # 255 * 2 * 3 = 1,530 cases, four independent pitch lanes per case.
    for exponent in range(255):
        for sign in (0, 0x80000000):
            pitch = [sign | (exponent << 23) | m for m in (0, 1, 0x3fffff, 0x7fffff)]
            for knob in (-4, 0, 4):
                yield case(rate, f'finite-pitch/{exponent}/{sign}/{knob}',
                           initial(pitch, [0], parameter=knob), [step(), step()])
    # 158 finite CV exponents through 2^30, both signs/four mantissas/three knobs.
    # All 3,792 final conversions are defined, including the largest <2^31 float.
    for exponent in range(158):
        for sign in (0, 0x80000000):
            for mantissa in (0, 1, 0x3fffff, 0x7fffff):
                cv = sign | (exponent << 23) | mantissa
                for knob in (-4, 0, 4):
                    yield case(rate, f'conversion/{exponent}/{sign}/{mantissa}/{knob}',
                               initial([bits(.25)], [cv], parameter=knob), [step(), step()])


def explicit_cases(rate):
    yield case(rate, 'anchor/voice-isolation', initial([bits(1), bits(2), bits(3)], [bits(2)], parameter=-1),
               [step(expect_output([-1], 1, -1)), step(expect_output([2, 3, 4], 3, 1)),
                source(1, [bits(-1), bits(3)]), step(), step(expect_output([-1, 4, 2], 3, -2))])
    yield case(rate, 'anchor/cv-does-not-create-voices', initial([], [bits(2)] * 16, input_mask=2, parameter=1),
               [step(expect_output([1], 1, 1)), step(expect_output([3], 1, 3))])
    yield case(rate, 'anchor/connected-requested-zero', initial([], [], input_mask=3, parameter=4),
               [step({'inputs.0.channels': 1, 'inputs.1.channels': 1}), step(expect_output([4], 1, 4))])
    # Difference from integer addition/f64: 2^24 + 1 rounds back to 2^24 first.
    for param, cv, wanted in ((ONE, 0x4b800000, 16777216),
                              (bits(-1), 0xcb800000, -16777216),
                              (0xcf000000, 0x4f000000, 0),
                              (0xcf000000, 0x4f7fffff, 2147483392),
                              (0x4effffff, 0xcf000000, -128),
                              (ZERO, 0xcf000000, -2147483648)):
        init = initial([0], [cv]); init['parameter_bits'] = param
        yield case(rate, f'anchor/large-sum/{param:08x}/{cv:08x}', init,
                   [step(), step({'last_octave': wanted, 'outputs.0.values_bits.0': binary32(wanted)})])
    # Raw subnormal copies survive; arithmetic inputs use genuine Engine FZ/DAZ.
    tiny = [ZERO, NEG_ZERO, 1, 0x80000001, 0x007fffff, 0x807fffff, 0x00800000, 0x80800000]
    active_expected = [0, 0, 0, 0, 0, 0, 0x00800000, 0x80800000]
    copy_anchor = {f'inputs.0.values_bits.{i}': raw for i, raw in enumerate(tiny)}
    output_anchor = {f'outputs.0.values_bits.{i}': raw for i, raw in enumerate(active_expected)}
    yield case(rate, 'anchor/signedzero-subnormal-bypass', initial(tiny, [NEG_ZERO]),
               [step(copy_anchor), step(output_anchor), {'op': 'parameter', 'value_bits': bits(3)},
                step({'last_octave': 3}), {'op': 'bypass', 'value': True},
                step({**{f'outputs.0.values_bits.{i}': raw for i, raw in enumerate(tiny)}, 'last_octave': 3}),
                {'op': 'parameter', 'value_bits': bits(-2)}, step({'last_octave': 3}),
                {'op': 'bypass', 'value': False}, step({'last_octave': -2})])
    for connected in (False, True):
        events = [step(), step(expect_output([3] * 16, 16, 1))]
        if not connected:
            events.append({'op': 'disconnect', 'id': 'out0'})
        events += [source(0, [bits(5)]), step(), step()]
        wanted = [6] + ([0] * 15 if connected else [3] * 15)
        events[-1]['expect'] = expect_output(wanted, 1 if connected else 0, 1)
        events += [source(0, [bits(-2)] * 16), step(), step(),
                   {'op': 'disconnect', 'id': 'in0'}, step(),
                   connect('in0', 'source', 0, 'module', 0), step(), step()]
        yield case(rate, f'host/retirement/{int(connected)}',
                   initial([bits(2)] * 16, [bits(1)]), events)
    # Both bypass/connect orderings; genuine Module::processBypass owns copying.
    for consumers in range(4):
        for order in (0, 1):
            events = [step(), step(), source(0, [bits(3)]), step(), step(),
                      source(0, []), step(), step()]
            events += [{'op': 'disconnect', 'id': f'out{i}'} for i in range(2) if consumers & (1 << i)]
            transitions = [{'op': 'bypass', 'value': True}, connect('temp', 'module', 0, 'sink', 0)]
            events += transitions if order == 0 else list(reversed(transitions))
            events += [step(), {'op': 'bypass', 'value': True},
                       {'op': 'disconnect', 'id': 'temp'}, connect('temp', 'module', 0, 'sink', 0),
                       source(0, [bits(-1)] * 16), step(), step(),
                       {'op': 'bypass', 'value': False}, step(),
                       {'op': 'adopt', 'parameter_bits': bits(-3)}, step(),
                       {'op': 'disconnect', 'id': 'in0'}, step(),
                       {'op': 'bypass', 'value': True}, step(),
                       connect('in0', 'source', 0, 'module', 0), step(), step()]
            yield case(rate, f'host/consumer/{consumers}/{order}',
                       initial([bits(2)] * 16, [bits(2)], output_mask=consumers, parameter=1), events)
    yield case(rate, 'host/pitch-stack', initial([bits(2)], [bits(4)] * 3, input_mask=1, output_mask=3),
               [step(), step(expect_output([2], 1, 0)), connect('stack', 'source', 1, 'module', 0),
                step(), step(expect_output([6, 6, 6], 3, 0)), {'op': 'disconnect', 'id': 'stack'},
                step(expect_output([6, 6, 6], 3, 0)), step(expect_output([2], 1, 0)),
                {'op': 'adopt', 'parameter_bits': bits(3)}, step(expect_output([5], 1, 3))])
    # Loading writes raw Param, not bounded ParamQuantity. Invalid conversion
    # values are observed without processing, then reset before the next step.
    events = [step(), step({'last_octave': 3}), {'op': 'save'},
              {'op': 'data_load', 'value': {}, 'expect': {'parameter_bits': bits(2)}},
              {'op': 'adopt', 'parameter_bits': bits(-2), 'expect': {'parameter_bits': bits(-2), 'last_octave': 3}},
              step({'last_octave': -1})]
    for value in (None, False, True, 1.0, 1.5, '', '3', [], {}, {'x': 1}):
        events += [{'op': 'parameter', 'value_bits': bits(3)},
                   {'op': 'data_load', 'value': {'octave': value}, 'expect': {'parameter_bits': 0}}, step()]
    for root in (None, False, True, 0, 1, 'octave', [], [1]):
        events += [{'op': 'parameter', 'value_bits': bits(-3)},
                   {'op': 'data_load', 'value': root, 'expect': {'parameter_bits': bits(-3)}}, step()]
    for integer in (-2147483648, -16777217, -9, -4, 0, 4, 9, 16777217, 2147483520):
        events += [{'op': 'data_load', 'value': {'octave': integer},
                    'expect': {'parameter_bits': binary32(integer)}}, {'op': 'save'}, step()]
    for integer in (2147483647, 2147483648, -(1 << 63), (1 << 63) - 1):
        events += [{'op': 'data_load', 'value': {'octave': integer},
                    'expect': {'parameter_bits': binary32(integer)}}, {'op': 'save'},
                   {'op': 'reset', 'expect': {'parameter_bits': 0}}, step()]
    events += [{'op': 'parameter', 'value_bits': bits(4)}, step({'last_octave': 5}),
               {'op': 'reset', 'expect': {'parameter_bits': 0, 'last_octave': 5}},
               {'op': 'adopt', 'parameter_bits': bits(-1), 'expect': {'last_octave': 5}},
               {'op': 'recreate', 'expect': {'parameter_bits': 0, 'last_octave': 0,
                                            'input_cables': [0, 0], 'output_cables': [0]}},
               {'op': 'save'}, step(expect_output([0], 0, 0)),
               connect('newpitch', 'source', 0, 'module', 0), connect('newout', 'module', 0, 'sink', 1),
               step(), step()]
    yield case(rate, 'state/legacy-reset-adopt-recreate', initial([bits(.25)], [bits(1)], parameter=2), events)
    yield case(rate, 'state/initial-legacy-authority', initial([bits(1)], [0], parameter=-4, data={'octave': 9}),
               [step({'parameter_bits': bits(9), 'last_octave': 9}), step(expect_output([10], 1, 9)),
                {'op': 'adopt', 'parameter_bits': bits(2), 'expect': {'parameter_bits': bits(2)}},
                step(expect_output([3], 1, 2)), {'op': 'save'}])


def safety_records():
    for cv in (0x7f800000, 0xff800000, 0x7fc12345, 0x4f000000, 0xcf000001, 0x7f7fffff):
        yield {'id': f'undefined-cv/{cv:08x}', 'parameter_bits': 0, 'cv_bits': cv,
               'classification': 'nonfinite or undefined final float-sum to i32; never execute genuine DSP'}
    for parameter in (0x7f800000, 0xff800000, 0x7fc12345, 0x4f000000, 0xcf000001):
        yield {'id': f'undefined-parameter/{parameter:08x}', 'parameter_bits': parameter, 'cv_bits': 0,
               'classification': 'undefined first rounded-parameter to i32; never execute genuine DSP'}


def cases():
    for rate in RATES:
        yield from fresh_cases(rate)
        yield from explicit_cases(rate)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--safety-output', type=Path, required=True)
    args = parser.parse_args()
    for path, records in ((args.output, cases()), (args.safety_output, safety_records())):
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('x') as stream:
            for record in records:
                stream.write(json.dumps(record, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n')


if __name__ == '__main__':
    main()
