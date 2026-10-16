#!/usr/bin/env python3
"""Supplemental input-only generator with eight hand-derived magnitude rows."""
import argparse
import json
from pathlib import Path

RATES = (44100, 48000, 96000)
SIGN = 0x80000000
ONE = 0x3f800000

# name, source magnitude, level, expected output magnitude.
# With Average, double source and connect a second zero source: same exact product.
MULTIPLY = (
    ('below-round-up', 0x00800000, 0x3f7fffff, 0x00000000),
    ('exact-min-normal', 0x00800000, 0x3f800000, 0x00800000),
    ('above-round-down', 0x00800001, 0x3f7fffff, 0x00800000),
    ('exact-above-neighbor', 0x00800001, 0x3f800000, 0x00800001),
)
# name, level, expected with Average, expected without Average.
# Two same-sign unit inputs sum to +/-2. Average first divides level by two.
DIVIDE = (
    ('below-exact', 0x00fffffe, 0x00000000, 0x017ffffe),
    ('below-round-up', 0x00ffffff, 0x00000000, 0x017fffff),
    ('exact-min-normal', 0x01000000, 0x01000000, 0x01800000),
    ('exact-above-neighbor', 0x01000001, 0x01000001, 0x01800001),
)


def case(rate, family, name, negative, invert, average, held, mask, level, magnitude):
    result = magnitude | (SIGN if negative ^ invert else 0)
    tail = SIGN if invert else 0
    settled = {'outputs.0.channels': 1,
               'outputs.0.values_bits.0': result,
               'outputs.0.values_bits.1': tail,
               'outputs.0.values_bits.2': tail,
               'outputs.0.values_bits.3': tail,
               'sink_inputs.0.values_bits.0': result,
               'sink_inputs.1.values_bits.0': result}
    return {'id': f'{rate}/{family}/{name}/{int(negative)}/{int(invert)}/{int(average)}',
            'sample_rate': rate,
            'initial': {'sources': held, 'input_mask': mask, 'output_mask': 3,
                        'level_bits': level, 'data': {'invert': invert, 'average': average}},
            'events': [
                {'op': 'step', 'expect': {'outputs.0.channels': 1,
                                        'outputs.0.values_bits.0': tail}},
                {'op': 'step', 'expect': settled}]}


def cases():
    for rate in RATES:
        for negative in (False, True):
            for invert in (False, True):
                for average in (False, True):
                    sign = SIGN if negative else 0
                    for name, source, level, expected in MULTIPLY:
                        # Exact power-of-two encoding shift for these normal operands.
                        held = [[(source + (0x00800000 if average else 0)) | sign],
                                [0] if average else [], [], [], [], []]
                        yield case(rate, 'multiply', name, negative, invert, average,
                                   held, 3 if average else 1, level, expected)
                    for name, level, averaged, unaveraged in DIVIDE:
                        held = [[ONE | sign], [ONE | sign], [], [], [], []]
                        yield case(rate, 'divide', name, negative, invert, average,
                                   held, 3, level, averaged if average else unaveraged)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('x') as stream:
        for item in cases():
            stream.write(json.dumps(item, sort_keys=True, separators=(',', ':')) + '\n')


if __name__ == '__main__':
    main()
