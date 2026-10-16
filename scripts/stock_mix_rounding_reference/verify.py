#!/usr/bin/env python3
"""No DSP model: reuse frozen full-snapshot validation, add supplement bookkeeping."""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BASE = HERE.parent / 'stock_mix_reference'
FILES = ('acceptance.json', 'fixtures.py', 'verify.py', 'README.md')
DEPENDENCIES = ('scripts/stock_mix_reference/acceptance.json',
                'scripts/stock_mix_reference/verify.py')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


base = module('stock_mix_frozen_verify', BASE / 'verify.py')
fixtures = module('stock_mix_rounding_inputs', HERE / 'fixtures.py')


def fixture_counts(path, declaration):
    actual = list(base.rows(path))
    base.equal(actual, list(fixtures.cases()), 'exact independent input declaration')
    counts = {rate: {'multiply': 0, 'divide': 0} for rate in declaration['sample_rates']}
    events = anchors = 0
    ids = set()
    for item in actual:
        if item['id'] in ids:
            raise ValueError('duplicate case id')
        ids.add(item['id'])
        counts[item['sample_rate']][item['id'].split('/')[1]] += 1
        for source in item['initial']['sources']:
            base.source_values(source)
        base.level_bits(item['initial']['level_bits'])
        base.equal([event['op'] for event in item['events']], ['step', 'step'], 'two samples')
        for event in item['events']:
            base.event_shape(event)
            events += 1
            anchors += len(event['expect'])
    for rate, families in counts.items():
        base.equal(families, declaration['families'], f'{rate}/families')
    result = {'cases': len(actual), 'events': events,
              'snapshots': len(actual) + events, 'hand_anchors': anchors}
    for key, field in (('cases', 'case_count'), ('events', 'event_count_excluding_initial'),
                       ('snapshots', 'snapshot_count'), ('hand_anchors', 'hand_anchor_count')):
        base.equal(result[key], declaration[field], field)
    return result


def frozen(lock_path, inputs):
    if lock_path is None:
        raise ValueError('external reviewed pre-execution lock required for trace validation')
    lock = json.loads(lock_path.read_text())
    if lock.get('reviewed_before_first_execution') is not True:
        raise ValueError('lock does not attest pre-execution review')
    base.equal(lock['files'], {name: base.digest(HERE / name) for name in FILES}, 'frozen files')
    base.equal(lock['dependencies'], {name: base.digest(ROOT / name) for name in DEPENDENCIES},
               'frozen original validator/schema')
    base.equal(lock['inputs_sha256'], base.digest(inputs), 'frozen inputs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--fixtures-only', action='store_true')
    parser.add_argument('--lock', type=Path)
    parser.add_argument('--sdk-trace', type=Path)
    parser.add_argument('--paired', type=Path)
    args = parser.parse_args()
    declaration = json.loads((HERE / 'acceptance.json').read_text())
    result = fixture_counts(args.inputs, declaration)
    if not args.fixtures_only:
        frozen(args.lock, args.inputs)
        if args.sdk_trace is None:
            raise ValueError('--sdk-trace required')
        schema = json.loads((BASE / 'acceptance.json').read_text())
        expected = (result['snapshots'], result['hand_anchors'])
        # Intentionally do not call the original large-matrix cardinality function.
        base.equal(base.validate(args.sdk_trace, args.inputs, schema), expected, 'SDK cardinality')
        if args.paired is not None:
            base.equal(base.validate(args.paired, args.inputs, schema), expected, 'Core cardinality')
            for index, (actual, reference) in enumerate(zip(base.rows(args.paired), base.rows(args.sdk_trace))):
                base.equal(actual, reference, f'complete snapshot {index}')
    result.update(passed=True, discarded_snapshots=0, inputs_sha256=base.digest(args.inputs),
                  sdk_trace_sha256=base.digest(args.sdk_trace) if args.sdk_trace else None,
                  paired_trace_sha256=base.digest(args.paired) if args.paired else None,
                  scope='fixtures only' if args.fixtures_only else
                  ('complete paired snapshots' if args.paired else 'SDK anchors only'))
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'passed': False, 'error': str(error)}, sort_keys=True), file=sys.stderr)
        sys.exit(1)
