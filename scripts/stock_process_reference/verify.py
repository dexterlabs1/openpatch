#!/usr/bin/env python3
"""Input preflight, source-order anchors, exact complete SDK/production trace gate."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import sys
from fixtures import safety_records

HERE = Path(__file__).resolve().parent
PORTS = {'source_outputs': 3, 'inputs': 3, 'outputs': 6, 'sink_inputs': 6}
COUNTS = {'input_cables': 3, 'output_cables': 6, 'source_cables': 3, 'sink_cables': 6}
ENGINE = {'state', 'on_time_bits', 'sample1_bits', 'sample2_bits', 'sample1_filter_bits',
          'sample2_filter_bits', 'hold_bits', 'slew_filter_bits', 'glide_filter_bits'}
DEFAULT_PARAMETERS = [0xc11f73da, 0]

def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''): h.update(chunk)
    return h.hexdigest()
def unique(pairs):
    out = {}
    for key, value in pairs:
        if key in out: raise ValueError('duplicate JSON key ' + key)
        out[key] = value
    return out
def rows(path):
    with Path(path).open() as f:
        for line in f:
            if not line.strip(): raise ValueError('blank JSONL line')
            yield json.loads(line, object_pairs_hook=unique,
                             parse_constant=lambda x: (_ for _ in ()).throw(ValueError('nonfinite JSON token')))
def equal(actual, expected, where=''):
    if type(actual) is not type(expected): raise ValueError(f'{where}: type {type(actual).__name__} != {type(expected).__name__}')
    if isinstance(actual, dict):
        if actual.keys() != expected.keys(): raise ValueError(f'{where}: keys {set(actual) ^ set(expected)}')
        for key in expected: equal(actual[key], expected[key], f'{where}.{key}')
    elif isinstance(actual, list):
        if len(actual) != len(expected): raise ValueError(f'{where}: length {len(actual)} != {len(expected)}')
        for i, (a, b) in enumerate(zip(actual, expected)): equal(a, b, f'{where}.{i}')
    elif actual != expected: raise ValueError(f'{where}: {actual!r} != {expected!r}')
def bounded(v, low, high):
    if type(v) is not int or not low <= v <= high: raise ValueError(f'integer outside {low}..{high}: {v!r}')
def raw(v): bounded(v, 0, 0xffffffff)
def finite(v):
    raw(v)
    if v & 0x7f800000 == 0x7f800000: raise ValueError('nonfinite ordinary control')
def float_of(v): return struct.unpack('<f', struct.pack('<I', v))[0]
def controls(a):
    if type(a) is not list or len(a) > 16: raise ValueError('source controls shape')
    for v in a: finite(v)
def parameters(a):
    if type(a) is not list or len(a) != 2: raise ValueError('two parameters required')
    for v in a: finite(v)
def name(v):
    if type(v) is not str or not v: raise ValueError('nonempty name required')
def json_domain(v):
    if v is None or type(v) is bool: return
    if type(v) is int: bounded(v, -(1 << 63), (1 << 63) - 1)
    elif type(v) is float:
        if not math.isfinite(v): raise ValueError('nonfinite JSON number')
    elif type(v) is str:
        if '\0' in v: raise ValueError('Jansson NUL string')
    elif type(v) is list:
        for x in v: json_domain(x)
    elif type(v) is dict:
        for key, x in v.items(): name(key); json_domain(x)
    else: raise ValueError('unsupported JSON value')
def at(row, path):
    for part in path.split('.'): row = row[int(part)] if isinstance(row, list) else row[part]
    return row
def anchor_shape(path, value):
    m = re.fullmatch(r'(source_outputs|inputs|outputs|sink_inputs)\.([0-9]+)\.(channels|values_bits\.[0-9]+)', path)
    if m:
        group, p, field = m.groups(); bounded(int(p), 0, PORTS[group] - 1)
        if field == 'channels': bounded(value, 0, 16)
        else: bounded(int(field.split('.')[-1]), 0, 15); raw(value)
        return
    m = re.fullmatch(r'engines\.([0-9]+)\.([a-z0-9_]+)', path)
    if m:
        bounded(int(m[1]), 0, 15)
        if m[2] not in ENGINE: raise ValueError('unknown engine anchor')
        if m[2] == 'state':
            if type(value) is not bool: raise ValueError('state Boolean anchor')
        else: raw(value)
        return
    if path == 'light_bits.0': raw(value); return
    raise ValueError('undeclared anchor path ' + path)
def event_shape(e):
    forms = {'step': set(), 'save': set(), 'reset': set(), 'recreate': set(),
             'source': {'port', 'values_bits'}, 'parameter': {'port', 'value_bits'},
             'data_load': {'value'}, 'bypass': {'value'}, 'adopt': {'parameter_bits'},
             'rate': {'sample_rate'}, 'connect': {'id', 'source', 'port', 'destination', 'input'},
             'disconnect': {'id'}}
    if type(e) is not dict or e.get('op') not in forms: raise ValueError('unknown event')
    want = forms[e['op']] | {'op'}
    if 'expect' in e:
        if type(e['expect']) is not dict: raise ValueError('anchor map')
        for path, value in e['expect'].items(): anchor_shape(path, value)
        want.add('expect')
    equal(set(e), want, 'event keys')
    op = e['op']
    if op == 'source': bounded(e['port'], 0, 2); controls(e['values_bits'])
    elif op == 'parameter': bounded(e['port'], 0, 1); finite(e['value_bits'])
    elif op == 'adopt': parameters(e['parameter_bits'])
    elif op == 'data_load': json_domain(e['value'])
    elif op == 'bypass':
        if type(e['value']) is not bool: raise ValueError('bypass Boolean')
    elif op == 'rate':
        if e['sample_rate'] not in (44100, 48000, 96000): raise ValueError('undeclared rate')
    elif op == 'connect':
        name(e['id']); route = (e['source'], e['destination'])
        if route not in (('source', 'module'), ('module', 'sink')): raise ValueError('undeclared route')
        bounded(e['port'], 0, 2 if route[0] == 'source' else 5)
        bounded(e['input'], 0, 2 if route[1] == 'module' else 5)
    elif op == 'disconnect': name(e['id'])
def initial_cables(init):
    d = {f'in{p}': ('source', p, 'module', p) for p in range(3) if init['input_mask'] & (1 << p)}
    d.update({f'out{p}': ('module', p, 'sink', p) for p in range(6) if init['output_mask'] & (1 << p)})
    return d
def cable_update(cables, e):
    op = e['op']
    if op == 'connect':
        if e['id'] in cables: raise ValueError('duplicate cable identity')
        cables[e['id']] = (e['source'], e['port'], e['destination'], e['input'])
    elif op == 'disconnect':
        if e['id'] not in cables: raise ValueError('absent cable')
        del cables[e['id']]
    elif op == 'recreate':
        for key in list(cables):
            if 'module' in (cables[key][0], cables[key][2]): del cables[key]
def sample_time_bits(rate):
    # Independent integer nearest-even encoding of rational 1/rate.
    e = -rate.bit_length() if rate & (rate - 1) else 1 - rate.bit_length()
    q, r = divmod(1 << (23 - e), rate)
    q += int(2 * r > rate or (2 * r == rate and (q & 1)))
    return ((e + 127) << 23) | (q - (1 << 23))
def controls_update(s, e):
    if e['op'] == 'source': s['source_config'][e['port']] = e['values_bits']
    elif e['op'] == 'parameter': s['parameter_bits'][e['port']] = e['value_bits']
    elif e['op'] == 'adopt': s['parameter_bits'] = e['parameter_bits'].copy()
    elif e['op'] == 'bypass': s['bypassed'] = e['value']
    elif e['op'] == 'rate': s['sample_rate'] = e['sample_rate']
    elif e['op'] == 'reset': s['parameter_bits'] = DEFAULT_PARAMETERS.copy()
    elif e['op'] == 'recreate': s['bypassed'] = False
    if e['op'] == 'recreate': s['parameter_bits'] = DEFAULT_PARAMETERS.copy()
def validate_fixtures(input_path, safety_path, d):
    equal(list(rows(safety_path)), list(safety_records()), 'separate safety records')
    ids = set(); total = events = anchors = 0; families = {rate: {} for rate in d['sample_rates']}
    for c in rows(input_path):
        equal(set(c), {'id', 'sample_rate', 'initial', 'events'}, 'case keys'); json_domain(c)
        name(c['id'])
        if c['id'] in ids: raise ValueError('duplicate case')
        ids.add(c['id'])
        rate = c['sample_rate']
        if rate not in families or not c['id'].startswith(f'{rate}/'): raise ValueError('undeclared rate/id')
        family = c['id'].split('/')[1]
        family = family if family in d['fresh_families_per_rate'] else 'explicit'
        families[rate][family] = families[rate].get(family, 0) + 1
        init = c['initial']; equal(set(init), {'sources', 'input_mask', 'output_mask', 'parameter_bits', 'data'}, 'initial keys')
        if type(init['sources']) is not list or len(init['sources']) != 3: raise ValueError('three sources')
        for a in init['sources']: controls(a)
        bounded(init['input_mask'], 0, 7); bounded(init['output_mask'], 0, 63)
        parameters(init['parameter_bits']); json_domain(init['data']); equal(init['data'], {}, 'Process no custom data')
        if type(c['events']) is not list: raise ValueError('events array')
        cables = initial_cables(init)
        for e in c['events']:
            event_shape(e); cable_update(cables, e); anchors += len(e.get('expect', {}))
            if e['op'] == 'data_load' and e['value'] not in ({}, None): raise ValueError('custom data excluded')
        # Conservative all-pairs check includes held controls before/after every
        # event, even when disconnected. It does not evaluate the DSP equation.
        slew_params = [init['parameter_bits'][0], DEFAULT_PARAMETERS[0]]
        cvs = init['sources'][0].copy() + [0]
        for e in c['events']:
            if e['op'] == 'parameter' and e['port'] == 0: slew_params.append(e['value_bits'])
            elif e['op'] == 'adopt': slew_params.append(e['parameter_bits'][0])
            elif e['op'] == 'source' and e['port'] == 0: cvs.extend(e['values_bits'])
        for param in slew_params:
            p = float_of(param)
            if abs(p) > 64: raise ValueError('slew parameter outside predeclared ordinary bound')
            for cv in cvs:
                v = float_of(cv)
                if abs(v) > 64: raise ValueError('slew CV outside predeclared ordinary bound')
                x = 30 - (p + v)
                if not -126 <= x <= 127: raise ValueError('exp2_taylor5 floor conversion outside defined exponent band')
        events += len(c['events']); total += 1
        if family != 'explicit': equal([e['op'] for e in c['events']], ['step', 'step'], 'fresh steps')
    for rate, found in families.items():
        equal(found, {**d['fresh_families_per_rate'], 'explicit': d['explicit_cases_per_rate']}, f'families/{rate}')
    equal(total, d['case_count'], 'case count'); equal(events, d['event_count_excluding_initial'], 'event count')
    equal(total + events, d['snapshot_count'], 'snapshot count')
    return {'cases': total, 'events': events, 'snapshots': total + events, 'hand_anchors': anchors, 'families': families}
def shape(row, d):
    equal(set(row), set(d['snapshot_fields']), 'snapshot keys'); json_domain(row)
    for key, n in PORTS.items():
        if type(row[key]) is not list or len(row[key]) != n: raise ValueError(key + ' port count')
        for p in row[key]:
            equal(set(p), {'channels', 'values_bits'}, key + ' port keys'); bounded(p['channels'], 0, 16)
            if type(p['values_bits']) is not list or len(p['values_bits']) != 16: raise ValueError(key + ' sixteen raw slots')
            for value in p['values_bits']: raw(value)
    for key, n in COUNTS.items():
        if type(row[key]) is not list or len(row[key]) != n: raise ValueError(key + ' counts')
        for value in row[key]: bounded(value, 0, 1024)
    parameters(row['parameter_bits'])
    if type(row['light_bits']) is not list or len(row['light_bits']) != 1: raise ValueError('one light')
    raw(row['light_bits'][0])
    if type(row['engines']) is not list or len(row['engines']) != 16: raise ValueError('sixteen engines')
    for e in row['engines']:
        equal(set(e), ENGINE, 'engine fields')
        if type(e['state']) is not bool: raise ValueError('engine bool')
        for key in ENGINE - {'state'}: raw(e[key])
    if type(row['source_config']) is not list or len(row['source_config']) != 3: raise ValueError('source config')
    for a in row['source_config']: controls(a)
    if type(row['bypassed']) is not bool: raise ValueError('bypass bool')
    if row['sample_rate'] not in d['sample_rates']: raise ValueError('rate')
    raw(row['sample_time_bits']); bounded(row['output_consumer_mask'], 0, 63)
    equal(row['saved_data'], None, 'Process has no custom data')
def validate_trace(trace_path, input_path, d):
    trace = iter(rows(trace_path)); count = anchors = invariants = 0
    for c in rows(input_path):
        init = c['initial']; cables = initial_cables(init)
        s = {'source_config': [a.copy() for a in init['sources']], 'parameter_bits': init['parameter_bits'].copy(),
             'bypassed': False, 'sample_rate': c['sample_rate']}
        frame = 0; previous = None
        for index, e in enumerate([{'op': 'initial'}] + c['events']):
            where = f'{c["id"]}/{index-1}'
            try: row = next(trace)
            except StopIteration: raise ValueError('missing ' + where) from None
            shape(row, d); cable_update(cables, e); controls_update(s, e)
            if e['op'] == 'step': frame += 1
            counts = {key: [0] * n for key, n in COUNTS.items()}; registry = []
            for cable_id, (src, p, dst, i) in sorted(cables.items()):
                counts['source_cables' if src == 'source' else 'output_cables'][p] += 1
                counts['input_cables' if dst == 'module' else 'sink_cables'][i] += 1
                registry.append({'id': cable_id, 'source': src, 'port': p, 'destination': dst, 'input': i})
            expected = {'case': c['id'], 'event': index - 1, 'op': e['op'], 'frame': frame,
                        'sample_rate': s['sample_rate'], 'sample_time_bits': sample_time_bits(s['sample_rate']),
                        'bypassed': s['bypassed'], 'parameter_bits': s['parameter_bits'],
                        'source_config': s['source_config'], 'cable_registry': registry,
                        'output_consumer_mask': sum((1 << p) for p, n in enumerate(counts['output_cables']) if n), **counts}
            for key, value in expected.items(): equal(row[key], value, where + '/' + key); invariants += 1
            if e['op'] == 'initial':
                for engine in row['engines']:
                    equal(engine, {key: False if key == 'state' else 0 for key in ENGINE}, where + '/fresh engine')
                equal(row['light_bits'], [0], where + '/fresh light')
                for key, counted in (('source_outputs', 'source_cables'), ('inputs', 'input_cables'),
                                     ('outputs', 'output_cables'), ('sink_inputs', 'sink_cables')):
                    for port, n in zip(row[key], counts[counted]):
                        equal(port['values_bits'], [0] * 16, where + '/fresh raw slots')
                        equal(port['channels'], int(n > 0), where + '/fresh channel')
            if previous is not None:
                if e['op'] not in ('step', 'recreate') or (e['op'] == 'step' and s['bypassed']):
                    equal(row['engines'], previous['engines'], where + '/no DSP engine writes')
                if e['op'] in ('source', 'parameter', 'data_load', 'save', 'reset', 'adopt', 'rate'):
                    for key in PORTS: equal(row[key], previous[key], where + '/no DSP port writes')
                if e['op'] == 'step' and not s['bypassed']:
                    width = max(1, previous['inputs'][1]['channels'])
                    for p, n in enumerate(counts['output_cables']):
                        equal(row['outputs'][p]['channels'], width if n else 0, where + '/signal width')
            for path, value in e.get('expect', {}).items(): equal(at(row, path), value, where + '/anchor/' + path); anchors += 1
            previous = row; count += 1
    try: next(trace)
    except StopIteration: return count, anchors, invariants
    raise ValueError('trailing snapshot')
def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--inputs', type=Path, required=True); p.add_argument('--safety-inputs', type=Path, required=True)
    p.add_argument('--sdk-trace', type=Path); p.add_argument('--paired', type=Path); p.add_argument('--fixtures-only', action='store_true')
    a = p.parse_args(); d = json.loads((HERE / 'acceptance.json').read_text())
    result = validate_fixtures(a.inputs, a.safety_inputs, d)
    if not a.fixtures_only:
        if not a.sdk_trace: p.error('--sdk-trace required')
        actual = validate_trace(a.sdk_trace, a.inputs, d)
        equal(actual[:2], (result['snapshots'], result['hand_anchors']), 'SDK trace cardinality')
        result['invariants_checked'] = actual[2]
        if a.paired:
            equal(validate_trace(a.paired, a.inputs, d), actual, 'paired trace cardinality')
            for i, (prod, sdk) in enumerate(zip(rows(a.paired), rows(a.sdk_trace))): equal(prod, sdk, f'full snapshot {i}')
    result.update(passed=True, discarded_snapshots=0, inputs_sha256=digest(a.inputs),
                  safety_inputs_sha256=digest(a.safety_inputs),
                  sdk_trace_sha256=digest(a.sdk_trace) if a.sdk_trace else None,
                  paired_trace_sha256=digest(a.paired) if a.paired else None)
    print(json.dumps(result, sort_keys=True))
if __name__ == '__main__':
    try: main()
    except Exception as exc:
        print(json.dumps({'passed': False, 'error': {'type': type(exc).__name__, 'message': str(exc)}}, sort_keys=True), file=sys.stderr)
        sys.exit(1)
