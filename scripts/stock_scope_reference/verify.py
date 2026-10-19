#!/usr/bin/env python3
"""Input-only preflight, independent anchors and exact whole-Scope trace gate."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import sys
from fixtures import PARAM_DEFAULTS, RATES, cases as declared_cases, safety_records

HERE = Path(__file__).resolve().parent
PORTS = {'source_outputs': 3, 'inputs': 3, 'outputs': 2, 'sink_inputs': 2}
COUNTS = {'input_cables': 3, 'output_cables': 2, 'source_cables': 3, 'sink_cables': 2}
INF_POINT = {'min_bits': 0x7f800000, 'max_bits': 0xff800000}

def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''): h.update(chunk)
    return h.hexdigest()
def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result: raise ValueError('duplicate JSON key ' + key)
        result[key] = value
    return result
def rows(path):
    with Path(path).open() as stream:
        for line in stream:
            if not line.strip(): raise ValueError('blank JSONL line')
            yield json.loads(line, object_pairs_hook=unique,
                             parse_constant=lambda value: (_ for _ in ()).throw(ValueError(value)))
def equal(actual, expected, where=''):
    if type(actual) is not type(expected):
        raise ValueError(f'{where}: type {type(actual).__name__} != {type(expected).__name__}')
    if isinstance(actual, dict):
        if actual.keys() != expected.keys(): raise ValueError(f'{where}: keys {set(actual) ^ set(expected)}')
        for key in expected: equal(actual[key], expected[key], f'{where}.{key}')
    elif isinstance(actual, list):
        if len(actual) != len(expected): raise ValueError(f'{where}: length {len(actual)} != {len(expected)}')
        for index, (left, right) in enumerate(zip(actual, expected)):
            equal(left, right, f'{where}.{index}')
    elif actual != expected: raise ValueError(f'{where}: {actual!r} != {expected!r}')
def bounded(value, low, high):
    if type(value) is not int or not low <= value <= high:
        raise ValueError(f'integer outside {low}..{high}: {value!r}')
def raw(value): bounded(value, 0, 0xffffffff)
def finite(value):
    raw(value)
    if value & 0x7f800000 == 0x7f800000: raise ValueError('nonfinite ordinary control')
def float_of(value): return struct.unpack('<f', struct.pack('<I', value))[0]
def name(value):
    if type(value) is not str or not value: raise ValueError('nonempty name required')
def controls(values):
    if type(values) is not list or len(values) > 16: raise ValueError('source control width')
    for value in values: finite(value)
def params(values):
    if type(values) is not list or len(values) != 8: raise ValueError('eight source Params required')
    for value in values: finite(value)
    time_domain(values[4])
def time_domain(value):
    finite(value)
    time = float_of(value)
    if not -6.0 <= time <= 8.0: raise ValueError('ordinary TIME outside declared bounded domain')
def ftz_nonzero(value):
    # The genuine ARM Engine enables FZ. Finite subnormals compare equal to
    # zero for float-to-bool, while their stored raw Param bits remain intact.
    finite(value)
    return bool(value & 0x7f800000)
def json_domain(value):
    if value is None or type(value) is bool: return
    if type(value) is int: bounded(value, -(1 << 63), (1 << 63) - 1)
    elif type(value) is float:
        if not math.isfinite(value): raise ValueError('nonfinite JSON')
    elif type(value) is str:
        if '\0' in value: raise ValueError('Jansson NUL string')
    elif type(value) is list:
        for entry in value: json_domain(entry)
    elif type(value) is dict:
        for key, entry in value.items(): name(key); json_domain(entry)
    else: raise ValueError('unsupported JSON value')
def at(row, path):
    for component in path.split('.'):
        row = row[int(component)] if isinstance(row, list) else row[component]
    return row
def anchor_shape(path, value):
    if path in ('channels_x', 'channels_y'):
        bounded(value, 0, 16); return
    if path == 'buffer_index': bounded(value, 0, 256); return
    if path == 'frame_index': bounded(value, 0, 10000000); return
    match = re.fullmatch(r'(point_buffer|current_points)\.([0-9]+)\.([0-9]+)(?:\.([0-9]+))?\.(min_bits|max_bits)', path)
    if match:
        group, a, b, c, _ = match.groups()
        if group == 'point_buffer':
            bounded(int(a), 0, 255); bounded(int(b), 0, 1); bounded(int(c), 0, 15)
        else:
            if c is not None: raise ValueError('partial Point path')
            bounded(int(a), 0, 1); bounded(int(b), 0, 15)
        raw(value); return
    match = re.fullmatch(r'(source_outputs|inputs|outputs|sink_inputs)\.([0-9]+)\.(channels|values_bits\.([0-9]+))', path)
    if match:
        group, port, field, lane = match.groups(); bounded(int(port), 0, PORTS[group] - 1)
        if lane is None: bounded(value, 0, 16)
        else: bounded(int(lane), 0, 15); raw(value)
        return
    match = re.fullmatch(r'(parameter_bits|light_bits|triggers)\.([0-9]+)', path)
    if match:
        group, index = match.groups()
        bounded(int(index), 0, {'parameter_bits': 7, 'light_bits': 1, 'triggers': 15}[group])
        bounded(value, 0, 2) if group == 'triggers' else raw(value)
        return
    raise ValueError('undeclared hand anchor path: ' + path)
def event_shape(event):
    forms = {'advance': {'frames'}, 'source': {'port', 'values_bits'},
             'parameter': {'port', 'value_bits'}, 'data_load': {'value'},
             'save': set(), 'reset': set(), 'bypass': {'value'},
             'connect': {'id', 'source', 'port', 'destination', 'input'},
             'disconnect': {'id'}, 'adopt': {'parameter_bits'},
             'recreate': set(), 'rate': {'sample_rate'}}
    if type(event) is not dict or event.get('op') not in forms: raise ValueError('unknown event')
    required = {'op'} | forms[event['op']]
    if 'expect' in event:
        if type(event['expect']) is not dict: raise ValueError('anchor map required')
        for path, value in event['expect'].items(): anchor_shape(path, value)
        required.add('expect')
    equal(set(event), required, 'event keys')
    op = event['op']
    if op == 'advance': bounded(event['frames'], 0, 10000000)
    elif op == 'source': bounded(event['port'], 0, 2); controls(event['values_bits'])
    elif op == 'parameter':
        bounded(event['port'], 0, 7); finite(event['value_bits'])
        if event['port'] == 4: time_domain(event['value_bits'])
    elif op == 'data_load': json_domain(event['value'])
    elif op == 'bypass':
        if type(event['value']) is not bool: raise ValueError('bypass Boolean')
    elif op == 'adopt': params(event['parameter_bits'])
    elif op == 'rate':
        if event['sample_rate'] not in RATES: raise ValueError('undeclared sample rate')
    elif op == 'connect':
        name(event['id']); route = (event['source'], event['destination'])
        if route not in (('source', 'module'), ('module', 'sink')): raise ValueError('undeclared cable route')
        bounded(event['port'], 0, 2 if route[0] == 'source' else 1)
        bounded(event['input'], 0, 2 if route[1] == 'module' else 1)
    elif op == 'disconnect': name(event['id'])
def initial_cables(init):
    cables = {f'in{port}': ('source', port, 'module', port)
              for port in range(3) if init['input_mask'] & (1 << port)}
    cables.update({f'out{port}': ('module', port, 'sink', port)
                   for port in range(2) if init['output_mask'] & (1 << port)})
    return cables
def cable_update(cables, event):
    op = event['op']
    if op == 'connect':
        if event['id'] in cables: raise ValueError('duplicate cable identity')
        cables[event['id']] = (event['source'], event['port'], event['destination'], event['input'])
    elif op == 'disconnect':
        if event['id'] not in cables: raise ValueError('absent cable identity')
        del cables[event['id']]
    elif op == 'recreate':
        for key in list(cables):
            if 'module' in (cables[key][0], cables[key][2]): del cables[key]
def legacy_update(values, data):
    if type(data) is not dict: return
    for key, index in (('lissajous', 5), ('external', 7)):
        value = data.get(key)
        if type(value) is int and value != 0: values[index] = 0x3f800000
def controls_update(state, event):
    op = event['op']
    if op == 'source': state['source_config'][event['port']] = event['values_bits'].copy()
    elif op == 'parameter': state['parameter_bits'][event['port']] = event['value_bits']
    elif op == 'adopt': state['parameter_bits'] = event['parameter_bits'].copy()
    elif op == 'data_load': legacy_update(state['parameter_bits'], event['value'])
    elif op == 'bypass': state['bypassed'] = event['value']
    elif op == 'rate': state['sample_rate'] = event['sample_rate']
    elif op == 'reset': state['parameter_bits'] = list(PARAM_DEFAULTS)
    elif op == 'recreate':
        state['parameter_bits'] = list(PARAM_DEFAULTS); state['bypassed'] = False
def sample_time_bits(rate):
    # Independent nearest-even encoding of the rational number 1/rate.
    exponent = -rate.bit_length() if rate & (rate - 1) else 1 - rate.bit_length()
    quotient, remainder = divmod(1 << (23 - exponent), rate)
    quotient += int(2 * remainder > rate or (2 * remainder == rate and (quotient & 1)))
    return ((exponent + 127) << 23) | (quotient - (1 << 23))
def validate_fixtures(inputs, safety, declaration):
    equal(declaration['sample_rates'], list(RATES), 'declared rates')
    equal(list(rows(inputs)), list(declared_cases()), 'exact frozen input stream')
    equal(list(rows(safety)), list(safety_records()), 'frozen separate safety input records')
    ids = set(); total = events = anchors = 0
    families = {rate: {'width': 0, 'explicit': 0} for rate in RATES}
    for case in rows(inputs):
        equal(set(case), {'id', 'sample_rate', 'initial', 'events'}, 'case keys')
        json_domain(case); name(case['id'])
        if case['id'] in ids: raise ValueError('duplicate case id')
        ids.add(case['id']); rate = case['sample_rate']
        if rate not in RATES or not case['id'].startswith(f'{rate}/'): raise ValueError('case rate/id')
        family = 'width' if case['id'].split('/')[1] == 'width' else 'explicit'
        families[rate][family] += 1
        init = case['initial']
        equal(set(init), {'sources', 'input_mask', 'output_mask', 'parameter_bits', 'data'}, 'initial keys')
        if type(init['sources']) is not list or len(init['sources']) != 3: raise ValueError('source controls')
        for values in init['sources']: controls(values)
        bounded(init['input_mask'], 0, 7); bounded(init['output_mask'], 0, 3)
        params(init['parameter_bits']); json_domain(init['data'])
        if type(case['events']) is not list: raise ValueError('event array')
        cables = initial_cables(init)
        for event in case['events']:
            event_shape(event); cable_update(cables, event)
            anchors += len(event.get('expect', {}))
        total += 1; events += len(case['events'])
        if family == 'width':
            equal([event['op'] for event in case['events']], ['advance', 'advance'], 'width events')
            equal([event['frames'] for event in case['events']], [1, 1], 'width frames')
    equal(families, {rate: {'width': declaration['width_cases_per_rate'],
                            'explicit': declaration['explicit_cases_per_rate']} for rate in RATES}, 'family counts')
    equal(total, declaration['case_count'], 'case count')
    equal(events, declaration['event_count_excluding_initial'], 'event count')
    equal(total + events, declaration['snapshot_count'], 'snapshot count')
    equal(len(list(safety_records())), declaration['safety_record_count'], 'safety count')
    return {'cases': total, 'events': events, 'snapshots': total + events,
            'hand_anchors': anchors, 'families': families}
def point_shape(point):
    equal(set(point), {'min_bits', 'max_bits'}, 'Point keys')
    raw(point['min_bits']); raw(point['max_bits'])
def shape(row, declaration):
    equal(set(row), set(declaration['snapshot_fields']), 'snapshot keys')
    json_domain(row)
    for key, count in PORTS.items():
        if type(row[key]) is not list or len(row[key]) != count: raise ValueError(key + ' port count')
        for port in row[key]:
            equal(set(port), {'channels', 'values_bits'}, key + ' port keys')
            bounded(port['channels'], 0, 16)
            if type(port['values_bits']) is not list or len(port['values_bits']) != 16:
                raise ValueError(key + ' sixteen raw slots')
            for value in port['values_bits']: raw(value)
    for key, count in COUNTS.items():
        if type(row[key]) is not list or len(row[key]) != count: raise ValueError(key + ' counts')
        for value in row[key]: bounded(value, 0, 1024)
    params(row['parameter_bits'])
    if type(row['light_bits']) is not list or len(row['light_bits']) != 2: raise ValueError('two light bits')
    for value in row['light_bits']: raw(value)
    if type(row['point_buffer']) is not list or len(row['point_buffer']) != 256: raise ValueError('256 completed bins')
    for waves in row['point_buffer']:
        if type(waves) is not list or len(waves) != 2: raise ValueError('two completed waves')
        for lanes in waves:
            if type(lanes) is not list or len(lanes) != 16: raise ValueError('sixteen completed lanes')
            for point in lanes: point_shape(point)
    if type(row['current_points']) is not list or len(row['current_points']) != 2: raise ValueError('two partial waves')
    for lanes in row['current_points']:
        if type(lanes) is not list or len(lanes) != 16: raise ValueError('sixteen partial lanes')
        for point in lanes: point_shape(point)
    bounded(row['channels_x'], 0, 16); bounded(row['channels_y'], 0, 16)
    bounded(row['buffer_index'], 0, 256); bounded(row['frame_index'], 0, 10000000)
    if type(row['triggers']) is not list or len(row['triggers']) != 16: raise ValueError('all sixteen detectors')
    for state in row['triggers']: bounded(state, 0, 2)
    if type(row['source_config']) is not list or len(row['source_config']) != 3: raise ValueError('source controls')
    for values in row['source_config']: controls(values)
    if type(row['bypassed']) is not bool: raise ValueError('bypass Boolean')
    if row['sample_rate'] not in RATES: raise ValueError('rate')
    raw(row['sample_time_bits']); bounded(row['output_consumer_mask'], 0, 3)
    equal(row['saved_data'], None, 'Scope custom dataToJson')
def validate_trace(trace_path, input_path, declaration):
    trace = iter(rows(trace_path)); snapshots = anchors = invariants = 0
    for case in rows(input_path):
        init = case['initial']; cables = initial_cables(init)
        state = {'source_config': [values.copy() for values in init['sources']],
                 'parameter_bits': init['parameter_bits'].copy(), 'bypassed': False,
                 'sample_rate': case['sample_rate']}
        legacy_update(state['parameter_bits'], init['data'])
        frame = 0; previous = None
        for index, event in enumerate([{'op': 'initial'}] + case['events']):
            where = f'{case["id"]}/{index - 1}'
            try: row = next(trace)
            except StopIteration: raise ValueError('missing snapshot ' + where) from None
            shape(row, declaration); cable_update(cables, event); controls_update(state, event)
            if event['op'] == 'advance': frame += event['frames']
            counts = {key: [0] * count for key, count in COUNTS.items()}; registry = []
            for cable_id, (origin, port, target, input_port) in sorted(cables.items()):
                counts['source_cables' if origin == 'source' else 'output_cables'][port] += 1
                counts['input_cables' if target == 'module' else 'sink_cables'][input_port] += 1
                registry.append({'id': cable_id, 'source': origin, 'port': port,
                                 'destination': target, 'input': input_port})
            expected = {'case': case['id'], 'event': index - 1, 'op': event['op'],
                        'frame': frame, 'sample_rate': state['sample_rate'],
                        'sample_time_bits': sample_time_bits(state['sample_rate']),
                        'bypassed': state['bypassed'],
                        'parameter_bits': state['parameter_bits'],
                        'source_config': state['source_config'], 'cable_registry': registry,
                        'output_consumer_mask': sum((1 << port) for port, count in enumerate(counts['output_cables']) if count),
                        **counts}
            for key, value in expected.items(): equal(row[key], value, where + '/' + key); invariants += 1
            if event['op'] == 'initial':
                for bin_index, waves in enumerate(row['point_buffer']):
                    for wave_index, lanes in enumerate(waves):
                        for lane_index, point in enumerate(lanes):
                            equal(point, INF_POINT, f'{where}/constructor/{bin_index}/{wave_index}/{lane_index}')
                for wave_index, lanes in enumerate(row['current_points']):
                    for lane_index, point in enumerate(lanes):
                        equal(point, INF_POINT, f'{where}/partial/{wave_index}/{lane_index}')
                equal(row['triggers'], [2] * 16, where + '/tri-state constructor')
                equal(row['light_bits'], [0, 0], where + '/unprocessed lights')
                equal([row[key] for key in ('channels_x', 'channels_y', 'buffer_index', 'frame_index')],
                      [0, 0, 0, 0], where + '/constructor counters')
                for key, counted in (('source_outputs', 'source_cables'), ('inputs', 'input_cables'),
                                     ('outputs', 'output_cables'), ('sink_inputs', 'sink_cables')):
                    for port, count in zip(row[key], counts[counted]):
                        equal(port['values_bits'], [0] * 16, where + '/fresh raw values')
                        equal(port['channels'], int(count > 0), where + '/fresh connection channel')
            if previous is not None:
                if event['op'] not in ('advance', 'reset', 'recreate') or (event['op'] == 'advance' and (state['bypassed'] or event['frames'] == 0)):
                    for key in ('point_buffer', 'current_points', 'channels_x', 'channels_y',
                                'buffer_index', 'frame_index', 'triggers', 'light_bits'):
                        equal(row[key], previous[key], where + '/no callback writes/' + key)
                if event['op'] == 'reset':
                    equal(row['current_points'], previous['current_points'], where + '/reset partial retained')
                    for key in ('channels_x', 'channels_y', 'buffer_index', 'frame_index', 'triggers', 'light_bits'):
                        equal(row[key], previous[key], where + '/reset history retained/' + key)
                    for bin_index, waves in enumerate(row['point_buffer']):
                        for wave_index, lanes in enumerate(waves):
                            for lane_index, point in enumerate(lanes):
                                equal(point, INF_POINT, f'{where}/reset/{bin_index}/{wave_index}/{lane_index}')
                if event['op'] in ('source', 'parameter', 'data_load', 'save', 'reset', 'adopt', 'rate'):
                    for key in PORTS: equal(row[key], previous[key], where + '/no host port step/' + key)
                if event['op'] == 'advance' and event['frames'] > 0 and not state['bypassed']:
                    for output, count in enumerate(counts['output_cables']):
                        input_channels = (previous if event['frames'] == 1 else row)['inputs'][output]['channels']
                        equal(row['outputs'][output]['channels'],
                              max(1, input_channels) if count else 0,
                              where + '/source output setChannels')
                    equal(row['light_bits'],
                          [0x3f800000 if ftz_nonzero(state['parameter_bits'][5]) else 0,
                           0 if ftz_nonzero(state['parameter_bits'][7]) else 0x3f800000],
                          where + '/source mode lights')
            for path, value in event.get('expect', {}).items():
                equal(at(row, path), value, where + '/hand anchor/' + path); anchors += 1
            previous = row; snapshots += 1
    try: next(trace)
    except StopIteration: return snapshots, anchors, invariants
    raise ValueError('trailing snapshots')
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs', type=Path, required=True)
    parser.add_argument('--safety-inputs', type=Path, required=True)
    parser.add_argument('--sdk-trace', type=Path)
    parser.add_argument('--paired', type=Path)
    parser.add_argument('--fixtures-only', action='store_true')
    args = parser.parse_args(); declaration = json.loads((HERE / 'acceptance.json').read_text())
    result = validate_fixtures(args.inputs, args.safety_inputs, declaration)
    if not args.fixtures_only:
        if not args.sdk_trace: parser.error('--sdk-trace required')
        checked = validate_trace(args.sdk_trace, args.inputs, declaration)
        equal(checked[:2], (result['snapshots'], result['hand_anchors']), 'SDK trace cardinality')
        result['invariants_checked'] = checked[2]
        if args.paired:
            equal(validate_trace(args.paired, args.inputs, declaration), checked, 'paired trace cardinality')
            for index, (production, sdk) in enumerate(zip(rows(args.paired), rows(args.sdk_trace))):
                equal(production, sdk, f'full snapshot {index}')
    result.update(passed=True, discarded_snapshots=0,
                  inputs_sha256=digest(args.inputs), safety_inputs_sha256=digest(args.safety_inputs),
                  sdk_trace_sha256=digest(args.sdk_trace) if args.sdk_trace else None,
                  paired_trace_sha256=digest(args.paired) if args.paired else None)
    print(json.dumps(result, sort_keys=True))
if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'passed': False, 'error': {'type': type(error).__name__,
                     'message': str(error)}}, sort_keys=True), file=sys.stderr)
        sys.exit(1)
