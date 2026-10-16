#!/usr/bin/env python3
"""Frozen input declaration for Fundamental Process; no DSP implementation."""
import argparse
import json
from pathlib import Path
import struct

RATES = (44100, 48000, 96000)
ZERO, ONE = 0, 0x3f800000

def bits(x): return struct.unpack('<I', struct.pack('<f', x))[0]
def initial(signal=None, gate=None, slew=None, input_mask=None, output_mask=63, params=None):
    sources = [slew if slew is not None else [], signal if signal is not None else [], gate if gate is not None else []]
    if input_mask is None: input_mask = sum((1 << p) for p, a in enumerate(sources) if a)
    return {'sources': sources, 'input_mask': input_mask, 'output_mask': output_mask,
            'parameter_bits': params if params is not None else [bits(-10), ZERO], 'data': {}}
def case(rate, name, init, events): return {'id': f'{rate}/{name}', 'sample_rate': rate, 'initial': init, 'events': events}
def step(expect=None): return {'op': 'step', **({'expect': expect} if expect else {})}
def source(port, values): return {'op': 'source', 'port': port, 'values_bits': values}
def parameter(port, value): return {'op': 'parameter', 'port': port, 'value_bits': value}
def connect(name, src, port, dst, input_port): return {'op': 'connect', 'id': name, 'source': src, 'port': port, 'destination': dst, 'input': input_port}

def cases():
    for rate in RATES:
        for width in range(17):
            for gate_width in (0, 1, 3, 16):
                for slew_width in (0, 1, 5, 16):
                    for mask in (0, 1, 63):
                        sig = [bits((i % 9 - 4) * .5) for i in range(width)]
                        gate = [bits((0, .1, 2, 5)[i % 4]) for i in range(gate_width)]
                        cv = [bits((i % 5 - 2) * .25) for i in range(slew_width)]
                        init = initial(sig, gate, cv, output_mask=mask, params=[bits(0), ZERO])
                        yield case(rate, f'width/{width}/{gate_width}/{slew_width}/{mask}', init, [step(), step()])
        for mask in range(64):
            for variant in range(3):
                sig = [bits((i + 1) / 8) for i in range((1, 7, 16)[variant])]
                gate = [bits(2 if i % 2 else 0) for i in range((0, 3, 16)[variant])]
                cv = [bits(i / 8) for i in range((16, 0, 5)[variant])]
                yield case(rate, f'mask/{mask}/{variant}', initial(sig, gate, cv, output_mask=mask, params=[bits(0), ZERO]), [step(), step()])
        # Sparse independent anchors derive only from source branches: hard-left
        # slew jump, exact input/gate values, and no floating point approximation.
        sig = [bits(3)]
        yield case(rate, 'anchor/initial-untriggered', initial(sig, [], [], params=[bits(-10), ZERO]),
                   [step(), step({'outputs.2.values_bits.0': bits(3), 'outputs.3.values_bits.0': ZERO,
                                  'outputs.4.values_bits.0': bits(3), 'outputs.5.values_bits.0': bits(3),
                                  'engines.0.state': False})])
        yield case(rate, 'anchor/trigger-manual-button', initial(sig, [], [], params=[bits(-10), ZERO]),
                   [step(), step(), parameter(1, ONE), step({'outputs.0.values_bits.0': bits(3), 'outputs.1.values_bits.0': ZERO,
                                  'outputs.2.values_bits.0': bits(3), 'outputs.3.values_bits.0': bits(3),
                                  'engines.0.sample1_bits': bits(3), 'engines.0.state': True,
                                  'light_bits.0': ONE})])
        yield case(rate, 'anchor/hard-left-filter-cancellation',
                   initial([bits(16)], [], [], params=[bits(-10), ZERO]),
                   [step(), step({'outputs.4.values_bits.0': bits(16)}),
                    source(1, [bits(2**-24)]), step(),
                    step({'inputs.1.values_bits.0': bits(2**-24),
                          'outputs.4.values_bits.0': ZERO,
                          'engines.0.slew_filter_bits': ZERO})])
        tiny = (ZERO, 0x80000000, 1, 0x80000001, 0x007fffff,
                0x807fffff, 0x00800000, 0x80800000)
        yield case(rate, 'anchor/raw-signed-zero-subnormal',
                   initial(list(tiny), [], [], params=[bits(-10), ZERO]),
                   [step({f'inputs.1.values_bits.{i}': raw for i, raw in enumerate(tiny)}), step()])
        yield case(rate, 'anchor/manual-light-no-voltage',
                   initial([], [], [], input_mask=0, output_mask=0,
                           params=[bits(-10), ONE]),
                   [step({'light_bits.0': ONE, 'engines.0.state': False}), step()])
        for button in (0x80000000, ZERO, 1, bits(.25), ONE, bits(-1)):
            yield case(rate, f'manual-gate-raw/{button:08x}',
                       initial([bits(1)], [], [], params=[bits(-10), button]), [step(), step()])
        threshold = (0x3dcccccc, 0x3dcccccd, 0x3dccccce, 0x3fffffff, 0x40000000, 0x40000001)
        for raw in threshold:
            init = initial([bits(1.25)], [raw], [], params=[bits(0), ZERO])
            yield case(rate, f'gate-threshold/{raw:08x}', init, [step(), step()])
            primed = initial([bits(1.25)], [bits(2)], [], params=[bits(0), ZERO])
            yield case(rate, f'gate-fall-threshold/{raw:08x}', primed,
                       [step(), step(), source(2, [raw]), step(), step()])
        for gate_width in (0, 1, 3, 16):
            for slew_width in (0, 1, 5, 16):
                sig = [bits(i / 4 - 2) for i in range(16)]
                gate = [bits(2 if i % 2 else 0) for i in range(gate_width)]
                cv = [bits((i % 5) / 4 - .5) for i in range(slew_width)]
                init = initial(sig, gate, cv, params=[bits(0), ZERO])
                events = [step(), step(), source(1, [bits(-i / 4) for i in range(16)]), step(), step(),
                          source(2, []), step(), parameter(1, ONE), step(), parameter(1, ZERO), step(),
                          source(2, [bits(2)] * gate_width), step(), source(2, [ZERO] * gate_width), step()]
                yield case(rate, f'gate-slew-independent/{gate_width}/{slew_width}', init, events)
        # The edge is strict >1ms after cumulative binary32 sampleTime sums.
        for slew_raw in (bits(-10), bits(-9), bits(0), bits(3)):
            boundary = (rate + 999) // 1000
            events = [step() for _ in range(boundary - 3)]
            for n in range(8):
                events += [source(1, [bits(n + 2)]), step()]
            events += [source(2, [ZERO]), step(), step()]
            yield case(rate, f'one-ms/{slew_raw:08x}', initial([bits(1)], [bits(2)], [ZERO], params=[slew_raw, ZERO]), events)
        for width in (0, 1, 3, 16):
            init = initial([bits(i + 1) for i in range(16)], [bits(2)] * 16, [ZERO] * 16,
                           params=[bits(0), ZERO])
            events = [step(), step(), source(1, [bits(-3)] * width), step(), step(),
                      source(1, []), step(), step(), source(1, [bits(i + 1) for i in range(16)]), step(), step(),
                      {'op': 'disconnect', 'id': 'in1'}, step(), connect('in1', 'source', 1, 'module', 1), step(), step()]
            yield case(rate, f'freeze-shrink-regrow/{width}', init, events)
        for port in range(3):
            init = initial([ONE] * 3, [bits(2)] * 3, [ZERO] * 3, params=[bits(0), ZERO])
            events = [step(), step(), {'op': 'disconnect', 'id': f'in{port}'}, step(),
                      connect(f'in{port}', 'source', port, 'module', port), step(),
                      connect('stack', 'source', (port + 1) % 3, 'module', port), step(),
                      {'op': 'disconnect', 'id': 'stack'}, step()]
            yield case(rate, f'input-topology/{port}', init, events)
        init = initial([bits(1)] * 16, [bits(2)] * 16, [ZERO] * 16, params=[bits(0), ZERO])
        events = [step(), step(), {'op': 'bypass', 'value': True}, step(), {'op': 'save'},
                  {'op': 'reset'}, step(), {'op': 'bypass', 'value': False}, step(),
                  {'op': 'adopt', 'parameter_bits': [bits(1), ONE]}, step(),
                  {'op': 'rate', 'sample_rate': 96000 if rate != 96000 else 44100}, step(),
                  {'op': 'recreate'}, step(), connect('newin', 'source', 1, 'module', 1),
                  connect('newout', 'module', 5, 'sink', 5), step(), step()]
        yield case(rate, 'lifecycle/reset-adopt-recreate-rate', init, events)
        for mask in range(64):
            init = initial([bits(2)] * 16, [bits(2)] * 16, [ZERO] * 16, output_mask=mask, params=[bits(0), ZERO])
            events = [step(), step(), source(1, [bits(-3)]), step(), step(),
                      connect('extra', 'module', mask % 6, 'sink', (mask + 1) % 6), step(),
                      {'op': 'disconnect', 'id': 'extra'}, step()]
            yield case(rate, f'retained-tail/{mask}', init, events)
        # +8 is intentionally a finite immediate-Param extension beyond the
        # configured UI maximum; exp2Floor remains in its defined exponent band.
        for raw in (bits(-10), bits(-9.965784), bits(-9.965783), bits(-5), bits(0),
                    bits(3.3219278), bits(3.321928094887362), bits(8)):
            for cv in (bits(-5), ZERO, bits(5)):
                init = initial([bits(4), bits(-4)], [bits(2)], [cv], params=[raw, ZERO])
                yield case(rate, f'slew-domain/{raw:08x}/{cv:08x}', init, [step(), step()])
        yield case(rate, 'no-sample-no-consumers', initial([ONE], [bits(2)], [ZERO], output_mask=0),
                   [{'op': 'save'}, parameter(1, ONE), source(1, [bits(7)]),
                    {'op': 'adopt', 'parameter_bits': [bits(0), ZERO]},
                    {'op': 'rate', 'sample_rate': 96000 if rate != 96000 else 44100},
                    {'op': 'reset'}, {'op': 'recreate'}])

def safety_records():
    for role in ('signal', 'gate', 'slew-cv', 'slew-param'):
        for raw in (0x7f800000, 0xff800000, 0x7fc00001, 0xffc00001, 0x7f7fffff, 0xff7fffff):
            yield {'id': f'{role}/{raw:08x}', 'role': role, 'bits': raw,
                   'classification': 'local Rust safety, no source-defined SDK oracle'}
    for value in (-200, -130, 160, 200):
        yield {'id': f'exp2-floor-outside/{value}', 'slew_pitch': value,
               'classification': 'outside declared exp2_taylor5 exponent domain; local Rust safety'}

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--safety-output', type=Path, required=True)
    a = p.parse_args()
    for path, rows in ((a.output, cases()), (a.safety_output, safety_records())):
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('x') as out:
            for row in rows: out.write(json.dumps(row, sort_keys=True, separators=(',', ':'), allow_nan=False) + '\n')
if __name__ == '__main__': main()
