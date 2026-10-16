#!/usr/bin/env python3
"""Independent deterministic inputs and sparse hand anchors; never runs DSP.

The combinatorial matrix contains inputs only. Expected arithmetic is confined
to explicitly named, source-derived hand examples below; no production import.
"""
import argparse
import json
from pathlib import Path
import struct

RATES = (44100, 48000, 96000)
ZERO = 0
NEG_ZERO = 0x80000000
ONE = 0x3f800000


def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]


def initial(sources=None, input_mask=0, output_mask=0, level=1, data=None):
    return {'sources': sources or [[] for _ in range(6)],
            'input_mask': input_mask, 'output_mask': output_mask,
            'level_bits': bits(level), 'data': {} if data is None else data}


def flags(code):
    return {'invert': bool(code & 1), 'average': bool(code & 2)}


def values(width, position=0):
    return [bits((1 if (lane + position) % 2 == 0 else -1) * (lane + 1) / 4)
            for lane in range(width)]


def step(expect=None):
    return {'op': 'step', **({'expect': expect} if expect else {})}


def connect(name, source, port, destination, input_port):
    return {'op': 'connect', 'id': name, 'source': source, 'port': port,
            'destination': destination, 'input': input_port}


def disconnect(name):
    return {'op': 'disconnect', 'id': name}


def source(port, numbers):
    return {'op': 'source', 'port': port, 'values_bits': [bits(v) for v in numbers]}


def row_expect(numbers, channels=None):
    result = {f'outputs.0.values_bits.{i}': bits(value) for i, value in enumerate(numbers)}
    if channels is not None:
        result['outputs.0.channels'] = channels
    return result


def fresh_cases(rate):
    # Exactly 17*6*4*3 = 1,224. Width zero is genuinely uncabled here.
    for width in range(17):
        for position in range(6):
            for flag in range(4):
                for li, level in enumerate((0, .5, 1)):
                    held = [[] for _ in range(6)]
                    held[position] = values(width, position)
                    yield {'id': f'{rate}/width/{width}/{position}/{flag}/{li}',
                           'sample_rate': rate,
                           'initial': initial(held, (1 << position) if width else 0, 1, level, flags(flag)),
                           'events': [step(), step()]}
    # Exactly 64*4*5 = 1,280. Distinct counts exercise effective jack counting.
    for mask in range(64):
        for flag in range(4):
            for li, level in enumerate((0, 1/3, .5, .875, 1)):
                held = [values(w, p) if mask & (1 << p) else []
                        for p, w in enumerate((1, 2, 3, 4, 8, 16))]
                yield {'id': f'{rate}/mask/{mask}/{flag}/{li}', 'sample_rate': rate,
                       'initial': initial(held, mask, 1, level, flags(flag)),
                       'events': [step(), step()]}
    # Exactly 17*17*3*4*2 = 6,936; pair positions expose source summation order.
    for left in range(17):
        for right in range(17):
            for p, q in ((0, 1), (0, 5), (2, 4)):
                for flag in range(4):
                    for li, level in enumerate((.5, 1)):
                        held = [[] for _ in range(6)]
                        held[p], held[q] = values(left, p), values(right, q)
                        mask = ((1 << p) if left else 0) | ((1 << q) if right else 0)
                        yield {'id': f'{rate}/pair/{left}/{right}/{p}-{q}/{flag}/{li}',
                               'sample_rate': rate, 'initial': initial(held, mask, 1, level, flags(flag)),
                               'events': [step(), step()]}


def explicit_cases(rate):
    # Four exact voice-isolation/flag anchors; all values are dyadic.
    for flag, expected in enumerate(([6, 6, 8], [-6, -6, -8], [3, 3, 4], [-3, -3, -4])):
        held = [[bits(2)], [bits(4), bits(6), bits(8)], [], [], [], []]
        events = [step(row_expect([0, 0, 0, 0], 1)),
                  step(row_expect(expected, 3))]
        # Inverted first-sample zero is -0 in the computed quartet.
        if flag & 1:
            events[0]['expect'].update(row_expect([-0.0] * 4))
        yield {'id': f'{rate}/anchor/voices/{flag}', 'sample_rate': rate,
               'initial': initial(held, 3, 1, 1, flags(flag)), 'events': events}

    held = [[bits(2)], [bits(4), bits(6), bits(8)], [], [], [], []]
    # Connected zero raises denominator 2→3; source desired0 is connected mono0.
    yield {'id': f'{rate}/anchor/denominator', 'sample_rate': rate,
           'initial': initial(held, 3, 1, 1, flags(2)), 'events': [step(), step(),
               connect('zero', 'source', 2, 'module', 2),
               step(row_expect([2, 2, struct.unpack('<f', struct.pack('<I', 0x402aaaab))[0]], 3)),
               disconnect('zero'), step(row_expect([3, 3, 4], 3)),
               {'op': 'level', 'value_bits': bits(.5)},
               {'op': 'data_load', 'value': {'average': False}},
               step(row_expect([3, 3, 4], 3)),
               source(0,[1]),source(1,[1]),source(2,[1]),
               connect('third','source',2,'module',2),
               {'op':'level','value_bits':0x3dcccccd},
               {'op':'data_load','value':{'average':True}},step(),
               # f32(f32(0.1)/3)*3 is 0x3dccccce; dividing the final
               # f32(3*0.1) by three instead gives 0x3dcccccd.
               step({'outputs.0.channels':1,'outputs.0.values_bits.0':0x3dccccce})]}

    # 16 + 2^-21 rounds back to 16 in f32 before adding -16.
    held = [[bits(16)], [bits(2**-21)], [bits(-16)], [], [], []]
    yield {'id': f'{rate}/anchor/f32-order', 'sample_rate': rate,
           'initial': initial(held, 7, 1), 'events': [step(), step(row_expect([0], 1)),
               {'op': 'data_load', 'value': {'invert': True}}, step(row_expect([-0.0], 1))]}

    # No module amplitude clipping; six connected +16 V inputs produce +96 V.
    yield {'id': f'{rate}/anchor/unclipped', 'sample_rate': rate,
           'initial': initial([[bits(16)] for _ in range(6)], 63, 1),
           'events': [step(), step(row_expect([96], 1)),
                      {'op': 'data_load', 'value': flags(3)}, step(row_expect([-16], 1))]}

    # Connected retirement clears AFTER quartet writes; disconnected does not.
    for consumer in (False, True):
        events = [step(), step(row_expect([2] * 16, 16)),
                  *([] if consumer else [disconnect('out0')]),
                  disconnect('in0'), {'op': 'data_load', 'value': {'invert': True}}]
        expected = [-0.0] + [0.0] * 15 if consumer else [-0.0] * 4 + [2.0] * 12
        events.append(step(row_expect(expected, 1 if consumer else 0)))
        yield {'id': f'{rate}/anchor/retirement/{int(consumer)}', 'sample_rate': rate,
               'initial': initial([[bits(2)] * 16, [], [], [], [], []], 1, 1), 'events': events}

    # Actual Jansson semantics: absent/nonobject retain; present nonboolean false.
    data_events = [step(), step(), {'op': 'data_load', 'value': flags(3)},
                   {'op': 'level', 'value_bits': bits(.25)},
                   {'op': 'reset', 'expect': {'level_bits': ONE, 'average': True, 'invert': True}},
                   {'op': 'save'}, {'op': 'data_load', 'value': {'average': False},
                    'expect': {'average': False, 'invert': True}},
                   {'op': 'data_load', 'value': {'average': True}}]
    for key in ('average', 'invert'):
        for malformed in (None, 0, 1, -1, 0.5, '', 'true', [], {}, {'x': True}):
            expected = {'average': True, 'invert': True}
            expected[key] = False
            data_events += [{'op': 'data_load', 'value': {key: malformed}, 'expect': expected},
                            {'op': 'data_load', 'value': {key: True}}]
    for root in (None, False, True, 0, 1, 'invert', [], [True]):
        data_events.append({'op': 'data_load', 'value': root,
                            'expect': {'average': True, 'invert': True}})
    data_events += [{'op': 'data_load', 'value': {'invert': None},
                     'expect': {'average': True, 'invert': False}},
                    {'op': 'adopt', 'level_bits': bits(.875), 'invert': True, 'average': False,
                     'expect': {'level_bits': bits(.875), 'invert': True, 'average': False}},
                    {'op': 'save'}, step(), {'op': 'recreate',
                     'expect': {'level_bits': ONE, 'average': False, 'invert': False,
                                'outputs.0.channels': 0, 'input_cables': [0]*6,
                                'output_cables': [0]}}, {'op': 'save'}, step()]
    yield {'id': f'{rate}/state/data-reset-adopt', 'sample_rate': rate,
           'initial': initial([[bits(3)] * 5, [], [], [], [], []], 1, 1), 'events': data_events}

    # Distinct consumer masks and both bypass/connection orderings.
    for mask in range(4):
        for order in (0, 1):
            events = [step(), step(), source(0, [3]), step(), step(), source(0, []),
                      step(), step(), source(0, [4] * 16), step(), step()]
            for sink in range(2):
                if mask & (1 << sink): events.append(disconnect(f'out{sink}'))
            transitions = [{'op': 'bypass', 'value': True},
                           connect('temp', 'module', 0, 'sink', 0)]
            events.extend(transitions if order == 0 else reversed(transitions))
            events += [step(), {'op': 'bypass', 'value': True}, disconnect('temp'),
                       connect('temp', 'module', 0, 'sink', 0), step(),
                       {'op': 'bypass', 'value': False}, step(), step(),
                       disconnect('in0'), connect('in0', 'source', 0, 'module', 0), step(), step(),
                       {'op': 'adopt', 'level_bits': bits(.5), 'invert': True, 'average': True}, step()]
            yield {'id': f'{rate}/host/consumer/{mask}/{order}', 'sample_rate': rate,
                   'initial': initial([[bits(2)] * 16, [], [], [], [], []], 1, mask), 'events': events}

    # Partial stack removal retains aggregate through the next process.
    held = [[bits(2)], [bits(4)] * 3, [bits(-1)], [], [], []]
    yield {'id': f'{rate}/host/stack', 'sample_rate': rate,
           'initial': initial(held, 1, 3), 'events': [step(), step(),
               connect('stack', 'source', 1, 'module', 0), step(),
               step(row_expect([6, 6, 6], 3)), disconnect('stack'),
               step(row_expect([6, 6, 6], 3)), step(row_expect([2], 1)),
               connect('zero', 'source', 3, 'module', 1),
               {'op': 'data_load', 'value': {'average': True}}, step(row_expect([1], 1)),
               connect('negative', 'source', 2, 'module', 0), step(), step(),
               disconnect('in0'), step(), disconnect('negative'),
               step(row_expect([0], 1)), disconnect('out0'), step(),
               disconnect('out1'), {'op': 'bypass', 'value': True},
               connect('newout', 'module', 0, 'sink', 1), step()]}

    # Finite boundaries and signed zeros, with source widths across quartets.
    boundaries = [0x00000001, 0x80000001, 0x00800000, 0x00800001, 0x3f7fffff,
                  ONE, 0x3f800001, 0x417fffff, bits(-16), bits(-2**-21),
                  bits(2**-20), bits(-2**-20), 0x007fffff, 0x807fffff, ZERO, NEG_ZERO]
    events = [step(), step()]
    for width in (15, 13, 12, 9, 8, 5, 4, 3, 2, 1, 0, 16):
        events += [{'op': 'source', 'port': 0, 'values_bits': boundaries[:width]}, step(), step()]
    for level in (0, 0x00800000, 0x3eaaaaaa, 0x3eaaaaab, 0x3eaaaaac, 0x3f7fffff, ONE):
        events += [{'op': 'level', 'value_bits': level},
                   {'op': 'data_load', 'value': flags(3)}, step(),
                   {'op': 'data_load', 'value': flags(0)}, step()]
    yield {'id': f'{rate}/precision/boundaries', 'sample_rate': rate,
           'initial': initial([boundaries, [bits(1)]*7, [bits(-1)]*4, [], [], []], 7, 1),
           'events': events}

    # Engine single-cable copies preserve subnormal payloads. Stacked adds use
    # actual Rack FTZ/DAZ, including signed flushed cancellation residuals.
    a = [0x00000001, 0x80000001, 0x007fffff, 0x807fffff, 0x00800000, 0x80800000]
    b = [0x00000001, 0x00000001, 0x807fffff, 0x807fffff, 0x80800001, 0x00800001]
    single = {f'inputs.0.values_bits.{i}': value for i,value in enumerate(a)}
    stacked = {f'inputs.0.values_bits.{i}': value for i,value in enumerate([0,0,0,0,NEG_ZERO,0])}
    yield {'id': f'{rate}/precision/stack-ftz', 'sample_rate': rate,
           'initial': initial([a,b,[],[],[],[]],1,1),
           'events': [step(single),step(),connect('tiny','source',1,'module',0),
                      step(stacked),step(),disconnect('tiny'),step(single),step(),
                      {'op':'level','value_bits':0x00800000},step(),
                      {'op':'data_load','value':flags(3)},step(),
                      disconnect('in0'),connect('in0','source',0,'module',0),step(single),step()]}


def cases():
    for rate in RATES:
        yield from fresh_cases(rate)
        yield from explicit_cases(rate)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('refusing to overwrite fixture artifact')
    with args.output.open('w') as out:
        for case in cases():
            out.write(json.dumps(case, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n')


if __name__ == '__main__':
    main()
