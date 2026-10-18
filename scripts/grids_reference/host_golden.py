#!/usr/bin/env python3
"""Author a host-control corpus, then explicitly freeze genuine source outputs.

No expected DSP values are computed here. The original Grids C++ helper emits
them; this script only selects its state/next-step columns. Frozen files are
never updated by the test runner.
"""
import argparse
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def corpus():
    base = json.loads((HERE / 'cases.json').read_text())
    original = next(case for case in base['cases'] if case['id'] == 'native_scan_composite')
    case = dict(original, id='native_host_scan')
    operations = []
    previous_resolution = None
    for row in original['operations']:
        operations.append(row)
        if row[0] == 0:
            resolution = row[2] & 7
            if previous_resolution is not None and resolution != previous_resolution:
                # The genuine firmware option menu resets the pattern when
                # editing resolution; the raw Options::unpack API does not.
                operations.append([5, 1])
            previous_resolution = resolution
    case['operations'] = operations
    base['cases'].append(case)
    return base, case


def segments(case):
    result = []
    params = {'seed': case['seed']}
    for row in case['operations']:
        op, repeat, *args = row
        if op == 0:
            packed, *controls = args
            params = dict(seed=case['seed'], mode=int(bool(packed & 64)),
                          resolution=packed & 7, gate_mode=int(not packed & 128),
                          alternate=int(bool(packed & 32)), swing=int(not packed & 8),
                          tap_tempo=int(bool(packed & 16)))
            params.update({name: value / 255 for name, value in
                           zip(['x', 'y', 'chaos', 'fill1', 'fill2', 'fill3'], controls)})
        elif op == 10:
            params.update(tempo=args[0], resolution=args[1])
        elif op == 40:
            if not result or result[-1]['params'] != params:
                result.append({'params': dict(params), 'inputs': []})
            result[-1]['inputs'].append([repeat, *args])
        elif op != 5:
            raise ValueError(f'unsupported golden setup operation {op}')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    cases = sub.add_parser('cases')
    cases.add_argument('output', type=Path)
    freeze = sub.add_parser('freeze')
    freeze.add_argument('attempt', type=Path)
    freeze.add_argument('output', type=Path)
    args = parser.parse_args()
    combined, case = corpus()
    if args.command == 'cases':
        args.output.write_text(json.dumps(combined, indent=2) + '\n')
        print(json.dumps({'cases': len(combined['cases']), 'output': str(args.output)}))
        return
    report = json.loads((args.attempt / 'result.json').read_text())
    if not report['passed']:
        raise ValueError('refuse to freeze an unsuccessful original-source attempt')
    retained = json.loads((args.attempt / 'cases.json').read_text())
    assert next(c for c in retained['cases'] if c['id'] == case['id']) == case
    trace = (args.attempt / case['id'] / 'source.stdout').read_bytes()
    outputs = []
    last = None
    for line in trace.decode().splitlines():
        row = json.loads(line)
        if isinstance(row[0], int):
            last = row
        elif row[0] == 's':
            assert last is not None and last[0] == row[1]
            outputs.append([last[1], last[3]])
    authored = segments(case)
    assert len(outputs) == sum(row[0] for s in authored for row in s['inputs'])
    result = {'source': 'pichenettes/eurorack:f4cfa7697095a28d292bff758beb51deb49b5a86/grids',
              'case': case['id'], 'sample_rate': 8000,
              'source_trace_sha256': hashlib.sha256(trace).hexdigest(),
              'segments': authored, 'state_and_next_step': outputs}
    args.output.write_text(json.dumps(result, separators=(',', ':')) + '\n')
    print(json.dumps({'frames': len(outputs), 'output': str(args.output)}))


if __name__ == '__main__':
    main()
