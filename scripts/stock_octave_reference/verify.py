#!/usr/bin/env python3
"""Independent exact whole-trace gate and input-only C++ conversion preflight.

No production import or output DSP implementation. Sparse fixture anchors,
actual source controls, registry, state and pre-process conversion domains are
checked independently of paired equality. Never assume inactive raw tails zero.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import sys
from fixtures import effective_octave, legacy_bits, safety_records

HERE = Path(__file__).resolve().parent
PORTS = {'source_outputs': 2, 'inputs': 2, 'outputs': 1, 'sink_inputs': 2}
COUNTS = {'input_cables': 2, 'output_cables': 1, 'source_cables': 2, 'sink_cables': 2}


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def rows(path):
    opener = gzip.open if str(path).endswith('.gz') else open
    with opener(path, 'rt') as stream:
        for line in stream:
            if not line.strip():
                raise ValueError('blank JSONL row')
            yield json.loads(line)


def equal(a, b, path=''):
    if type(a) is not type(b):
        raise ValueError(f'{path}: types differ')
    if isinstance(a, dict):
        if a.keys() != b.keys():
            raise ValueError(f'{path}: keys differ')
        for key in a:
            equal(a[key], b[key], f'{path}.{key}')
    elif isinstance(a, (list, tuple)):
        if len(a) != len(b):
            raise ValueError(f'{path}: lengths differ')
        for index, (x, y) in enumerate(zip(a, b)):
            equal(x, y, f'{path}.{index}')
    elif a != b:
        raise ValueError(f'{path}: {a!r} != {b!r}')


def at(row, path):
    for part in path.split('.'):
        row = row[int(part)] if isinstance(row, list) else row[part]
    return row


def bounded(value, low, high):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f'integer outside [{low},{high}]')


def finite(value):
    bounded(value, 0, 0xffffffff)
    if value & 0x7f800000 == 0x7f800000:
        raise ValueError('nonfinite control in ordinary domain')


def source_values(values):
    if type(values) is not list or len(values) > 16:
        raise ValueError('source array shape')
    for value in values:
        finite(value)


def named(value):
    if type(value) is not str or not value:
        raise ValueError('nonempty string required')


def event_shape(event):
    fields = {'step': set(), 'reset': set(), 'save': set(), 'recreate': set(),
              'source': {'port', 'values_bits'}, 'parameter': {'value_bits'},
              'data_load': {'value'}, 'bypass': {'value'},
              'connect': {'id', 'source', 'port', 'destination', 'input'},
              'disconnect': {'id'}, 'adopt': {'parameter_bits'}}
    if type(event) is not dict or event.get('op') not in fields:
        raise ValueError('invalid operation')
    op = event['op']; expected = fields[op] | {'op'}
    if 'expect' in event:
        if type(event['expect']) is not dict:
            raise ValueError('anchor map required')
        for path in event['expect']:
            named(path)
        expected.add('expect')
    equal(set(event), expected, 'event fields')
    if op == 'source':
        bounded(event['port'], 0, 1); source_values(event['values_bits'])
    elif op == 'parameter':
        finite(event['value_bits'])
    elif op == 'adopt':
        finite(event['parameter_bits'])
    elif op == 'bypass':
        if type(event['value']) is not bool:
            raise ValueError('Boolean bypass required')
    elif op == 'connect':
        named(event['id'])
        route = (event['source'], event['destination'])
        if route not in (('source', 'module'), ('module', 'sink')):
            raise ValueError('undeclared feedback/role topology')
        bounded(event['port'], 0, 1 if route[0] == 'source' else 0)
        bounded(event['input'], 0, 1)
        if route == ('source', 'module') and event['input'] == 1 and event['port'] != 1:
            raise ValueError('CV is supplied only by source1 in this declaration')
    elif op == 'disconnect':
        named(event['id'])


def control_update(state, event):
    op = event['op']
    if op == 'parameter':
        state['parameter_bits'] = event['value_bits']
    elif op == 'reset':
        state['parameter_bits'] = 0
    elif op == 'data_load':
        state['parameter_bits'] = legacy_bits(event['value'], state['parameter_bits'])
    elif op == 'adopt':
        state['parameter_bits'] = event['parameter_bits']
    elif op == 'bypass':
        state['bypassed'] = event['value']
    elif op == 'recreate':
        state.update(parameter_bits=0, bypassed=False)


def initial_cables(initial):
    cables = {f'in{p}': ('source', p, 'module', p) for p in range(2) if initial['input_mask'] & (1 << p)}
    cables.update({f'out{p}': ('module', 0, 'sink', p) for p in range(2) if initial['output_mask'] & (1 << p)})
    return cables


def cable_update(cables, event):
    op = event['op']
    if op == 'connect':
        if event['id'] in cables:
            raise ValueError('duplicate cable identity')
        route = (event['source'], event['port'], event['destination'], event['input'])
        if route[2:] == ('module', 1) and any(c[2:] == ('module', 1) for c in cables.values()):
            raise ValueError('CV stacking outside declared conversion preflight')
        cables[event['id']] = route
    elif op == 'disconnect':
        del cables[event['id']]
    elif op == 'recreate':
        for name in list(cables):
            if cables[name][0] == 'module' or cables[name][2] == 'module':
                del cables[name]


def validate_fixtures(path, declaration):
    ids = set(); families = {rate: {} for rate in declaration['sample_rates']}
    snapshots = events = anchors = domain_checks = 0
    for case in rows(path):
        equal(set(case), {'id', 'sample_rate', 'initial', 'events'}, 'case fields')
        named(case['id'])
        if case['id'] in ids:
            raise ValueError('duplicate case')
        ids.add(case['id']); rate = case['sample_rate']
        if rate not in families or not case['id'].startswith(str(rate) + '/'):
            raise ValueError('undeclared/mismatched rate')
        family = case['id'].split('/')[1]
        family = family if family in declaration['fresh_families_per_rate'] else 'explicit'
        families[rate][family] = families[rate].get(family, 0) + 1
        initial = case['initial']
        equal(set(initial), {'sources', 'input_mask', 'output_mask', 'parameter_bits', 'data'}, 'initial fields')
        if type(initial['sources']) is not list or len(initial['sources']) != 2:
            raise ValueError('two source buses required')
        for values in initial['sources']:
            source_values(values)
        bounded(initial['input_mask'], 0, 3); bounded(initial['output_mask'], 0, 3)
        finite(initial['parameter_bits'])
        if type(case['events']) is not list:
            raise ValueError('events list required')
        # Conservative, input-only conversion proof: every CV value that could
        # be retained at any point in this case, including constructor zero,
        # must be safe with the actual control on every nonbypassed step.
        possible_cv = {0, *initial['sources'][1]}
        for event in case['events']:
            event_shape(event)
            if event['op'] == 'source' and event['port'] == 1:
                possible_cv.update(event['values_bits'])
        state = {'parameter_bits': legacy_bits(initial['data'], initial['parameter_bits']), 'bypassed': False}
        cables = initial_cables(initial)
        for event in case['events']:
            control_update(state, event); cable_update(cables, event)
            if event['op'] == 'step' and not state['bypassed']:
                for cv in possible_cv:
                    effective_octave(state['parameter_bits'], cv)
                    domain_checks += 1
            anchors += len(event.get('expect', {}))
        if family != 'explicit':
            equal([e['op'] for e in case['events']], ['step', 'step'], 'fresh two steps')
        events += len(case['events']); snapshots += 1 + len(case['events'])
    for rate, found in families.items():
        equal(found, {**declaration['fresh_families_per_rate'], 'explicit': declaration['explicit_cases_per_rate']}, f'families/{rate}')
    equal(len(ids), declaration['case_count'], 'case cardinality')
    equal(events, declaration['event_count_excluding_initial'], 'event cardinality')
    equal(snapshots, declaration['snapshot_count'], 'snapshot cardinality')
    return {'cases': len(ids), 'snapshots': snapshots, 'events': events, 'hand_anchors': anchors,
            'conversion_domain_checks_before_execution': domain_checks, 'families': families}


def shape(row, declaration):
    equal(set(row), set(declaration['snapshot_fields']), 'snapshot fields')
    for key, count in PORTS.items():
        if type(row[key]) is not list or len(row[key]) != count:
            raise ValueError(f'{key} shape')
        for port in row[key]:
            equal(set(port), {'channels', 'values_bits'}, 'port fields')
            bounded(port['channels'], 0, 16)
            if type(port['values_bits']) is not list or len(port['values_bits']) != 16:
                raise ValueError('sixteen raw slots required')
            for raw in port['values_bits']:
                bounded(raw, 0, 0xffffffff)
    for key, count in COUNTS.items():
        if type(row[key]) is not list or len(row[key]) != count:
            raise ValueError('physical count shape')
        for value in row[key]:
            bounded(value, 0, 1024)
    if type(row['bypassed']) is not bool:
        raise ValueError('bypassed Boolean required')
    finite(row['parameter_bits']); bounded(row['last_octave'], -(1 << 31), (1 << 31) - 1)
    equal(row['saved_data'], None, 'actual default dataToJson is null')
    if type(row['source_config']) is not list or len(row['source_config']) != 2:
        raise ValueError('source config shape')
    for values in row['source_config']:
        source_values(values)
    bounded(row['output_consumer_mask'], 0, 1)


def validate(trace_path, fixture_path, declaration):
    trace = iter(rows(trace_path)); total = anchors = invariants = 0
    for case in rows(fixture_path):
        initial = case['initial']; frame = 0; previous = None
        state = {'parameter_bits': legacy_bits(initial['data'], initial['parameter_bits']), 'bypassed': False}
        configs = [v.copy() for v in initial['sources']]; cables = initial_cables(initial)
        for index, event in enumerate([{'op': 'initial'}] + case['events']):
            try:
                row = next(trace)
            except StopIteration:
                raise ValueError(f'missing {case["id"]}/{index - 1}') from None
            shape(row, declaration); op = event['op']
            if op == 'step':
                frame += 1
            if op == 'source':
                configs[event['port']] = event['values_bits']
            control_update(state, event); cable_update(cables, event)
            counts = {key: [0] * size for key, size in COUNTS.items()}
            registry = []
            for name, (src, port, dst, input_port) in sorted(cables.items()):
                counts['source_cables' if src == 'source' else 'output_cables'][port] += 1
                counts['input_cables' if dst == 'module' else 'sink_cables'][input_port] += 1
                registry.append({'id': name, 'source': src, 'port': port, 'destination': dst, 'input': input_port})
            expected = {'case': case['id'], 'event': index - 1, 'op': op, 'frame': frame,
                        **state, **counts, 'source_config': configs, 'cable_registry': registry,
                        'output_consumer_mask': int(counts['output_cables'][0] > 0)}
            for key, want in expected.items():
                equal(row[key], want, f'{case["id"]}/{index - 1}/{key}'); invariants += 1
            if op in ('initial', 'recreate'):
                equal(row['last_octave'], 0, 'fresh runtime display state')
            if op == 'initial':
                for key, connections in (('source_outputs', 'source_cables'), ('inputs', 'input_cables'),
                                         ('outputs', 'output_cables'), ('sink_inputs', 'sink_cables')):
                    for port, number in zip(row[key], counts[connections]):
                        equal(port['values_bits'], [0] * 16, 'constructor has no DSP writes')
                        equal(port['channels'], int(number > 0), 'constructor connected count')
            if previous is not None:
                if op not in ('step', 'recreate') or (op == 'step' and state['bypassed']):
                    equal(row['last_octave'], previous['last_octave'], 'runtime changes only in active process/recreation')
                if op in ('source', 'parameter', 'data_load', 'save', 'reset', 'adopt'):
                    for key in PORTS:
                        equal(row[key], previous[key], f'{op} cannot process/mutate raw ports')
                if op == 'step' and not state['bypassed']:
                    cv = previous['inputs'][1]['values_bits'][0]
                    wanted = effective_octave(state['parameter_bits'], cv)
                    equal(row['last_octave'], wanted, 'channel-zero conversion order/rational anchor')
                    count = max(previous['inputs'][0]['channels'], 1) if counts['output_cables'][0] else 0
                    equal(row['outputs'][0]['channels'], count, 'only pitch creates voices')
            for path, want in event.get('expect', {}).items():
                equal(at(row, path), want, f'{case["id"]}/{index - 1}/anchor/{path}'); anchors += 1
            previous = row; total += 1
    try:
        next(trace)
    except StopIteration:
        return total, anchors, invariants
    raise ValueError('trailing snapshot')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--safety-inputs', type=Path, required=True)
    parser.add_argument('--sdk-trace', type=Path)
    parser.add_argument('--paired', type=Path)
    parser.add_argument('--fixtures-only', action='store_true')
    args = parser.parse_args(); declaration = json.loads((HERE / 'acceptance.json').read_text())
    equal(list(rows(args.safety_inputs)), list(safety_records()), 'separate undefined-domain records')
    result = validate_fixtures(args.inputs, declaration)
    if not args.fixtures_only:
        if not args.sdk_trace:
            parser.error('--sdk-trace required')
        counts = validate(args.sdk_trace, args.inputs, declaration)
        equal(counts[:2], (result['snapshots'], result['hand_anchors']), 'trace cardinality')
        result['invariants_checked'] = counts[2]
        if args.paired:
            equal(validate(args.paired, args.inputs, declaration), counts, 'paired cardinality')
            for index, (actual, reference) in enumerate(zip(rows(args.paired), rows(args.sdk_trace))):
                equal(actual, reference, f'snapshot {index}')
    result.update(passed=True, discarded_snapshots=0, inputs_sha256=digest(args.inputs),
                  safety_inputs_sha256=digest(args.safety_inputs),
                  sdk_trace_sha256=digest(args.sdk_trace) if args.sdk_trace else None,
                  paired_trace_sha256=digest(args.paired) if args.paired else None,
                  scope='input-only domain/structure proof' if args.fixtures_only else
                  ('complete paired host snapshots' if args.paired else 'SDK anchors only; production unqualified'))
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'passed': False, 'error': str(error)}, sort_keys=True), file=sys.stderr)
        sys.exit(1)
