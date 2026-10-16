#!/usr/bin/env python3
"""Independent whole-snapshot validator. Never implements the Mixer DSP loop."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PORTS = {'source_outputs': 6, 'inputs': 6, 'outputs': 1, 'sink_inputs': 2}
COUNTS = {'input_cables': 6, 'output_cables': 1, 'source_cables': 6, 'sink_cables': 2}


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024*1024), b''): h.update(chunk)
    return h.hexdigest()


def rows(path):
    opener = gzip.open if str(path).endswith('.gz') else open
    with opener(path, 'rt') as stream:
        for line in stream:
            if not line.strip(): raise ValueError('blank JSONL row')
            yield json.loads(line)


def equal(a, b, path=''):
    if type(a) is not type(b): raise ValueError(f'{path}: types differ')
    if isinstance(a, dict):
        if a.keys() != b.keys(): raise ValueError(f'{path}: keys differ')
        for key in a: equal(a[key], b[key], f'{path}.{key}')
    elif isinstance(a, list):
        if len(a) != len(b): raise ValueError(f'{path}: lengths differ')
        for index, (x,y) in enumerate(zip(a,b)): equal(x,y,f'{path}.{index}')
    elif a != b: raise ValueError(f'{path}: {a!r} != {b!r}')


def at(row, path):
    for part in path.split('.'): row = row[int(part)] if isinstance(row, list) else row[part]
    return row


def u32(value):
    if type(value) is not int or not 0 <= value <= 0xffffffff: raise ValueError('invalid u32 float bits')


def finite(value):
    u32(value)
    if value & 0x7f800000 == 0x7f800000: raise ValueError('nonfinite in ordinary finite domain')


def source_values(values):
    if type(values)is not list or len(values)>16: raise ValueError('source array shape')
    for value in values:
        finite(value)
        if value & 0x7fffffff > 0x41800000: raise ValueError('source outside real [-16,16] bounds')


def level_bits(value):
    finite(value)
    if value!=0x80000000 and not 0<=value<=0x3f800000: raise ValueError('level outside [0,1]')


def bounded_integer(value,lo,hi):
    if type(value)is not int or not lo<=value<=hi: raise ValueError('bounded integer required')


def named(value):
    if type(value)is not str or not value: raise ValueError('nonempty name required')


def event_shape(event):
    fields={'step':set(),'reset':set(),'save':set(),'recreate':set(),
            'source':{'port','values_bits'},'level':{'value_bits'},
            'data_load':{'value'},'bypass':{'value'},
            'connect':{'id','source','port','destination','input'},
            'disconnect':{'id'},'adopt':{'level_bits','average','invert'}}
    if type(event)is not dict or event.get('op') not in fields: raise ValueError('invalid event operation')
    op=event['op']; expected=fields[op]|{'op'}
    if 'expect' in event:
        if type(event['expect'])is not dict: raise ValueError('anchor map required')
        for path in event['expect']: named(path)
        expected.add('expect')
    equal(set(event),expected,'event fields')
    if op=='source': bounded_integer(event['port'],0,5);source_values(event['values_bits'])
    elif op=='level': level_bits(event['value_bits'])
    elif op=='bypass':
        if type(event['value'])is not bool: raise ValueError('Boolean bypass required')
    elif op=='adopt':
        level_bits(event['level_bits'])
        if type(event['average'])is not bool or type(event['invert'])is not bool: raise ValueError('Boolean receiving flags required')
    elif op=='connect':
        named(event['id'])
        if event['source'] not in ('source','module') or event['destination'] not in ('module','sink'): raise ValueError('invalid port role')
        bounded_integer(event['port'],0,5 if event['source']=='source' else 0)
        bounded_integer(event['input'],0,5 if event['destination']=='module' else 1)
    elif op=='disconnect': named(event['id'])


def control_update(state, event):
    op = event['op']
    if op == 'level': state['level_bits'] = event['value_bits']
    elif op == 'reset': state['level_bits'] = 0x3f800000
    elif op == 'bypass': state['bypassed'] = event['value']
    elif op == 'data_load':
        value = event['value']
        if isinstance(value, dict):
            for key in ('average', 'invert'):
                if key in value: state[key] = value[key] is True
    elif op == 'adopt':
        for key in ('level_bits', 'average', 'invert'): state[key] = event[key]
    elif op == 'recreate':
        state.update(level_bits=0x3f800000, average=False, invert=False, bypassed=False)


def validate_fixture_count(path, declaration):
    ids=set(); rates={rate: {'fresh':0,'explicit':0} for rate in declaration['sample_rates']}
    families={rate:{'width':0,'mask':0,'pair':0} for rate in declaration['sample_rates']}
    snapshots=events=anchors=0
    for case in rows(path):
        if set(case) != {'id','sample_rate','initial','events'}: raise ValueError('case keys differ')
        if case['id'] in ids: raise ValueError('duplicate case id')
        ids.add(case['id']); rate=case['sample_rate']
        if rate not in rates: raise ValueError('undeclared rate')
        family=case['id'].split('/')[1]
        rates[rate]['fresh' if family in ('width','mask','pair') else 'explicit'] += 1
        if family in families[rate]:
            families[rate][family]+=1
            equal(case['events'],[{'op':'step'},{'op':'step'}],'fresh first-sample and settled snapshots')
        init=case['initial']
        if set(init) != {'sources','input_mask','output_mask','level_bits','data'}: raise ValueError('initial keys differ')
        if len(init['sources']) != 6: raise ValueError('six source controls required')
        if type(init['input_mask']) is not int or not 0<=init['input_mask']<64: raise ValueError('input mask')
        if type(init['output_mask']) is not int or not 0<=init['output_mask']<4: raise ValueError('output mask')
        for source in init['sources']: source_values(source)
        level_bits(init['level_bits'])
        for event in case['events']:
            if event['op'] not in declaration['host_events']: raise ValueError('undeclared event')
            event_shape(event)
            anchors += len(event.get('expect',{}))
        events += len(case['events']); snapshots += 1+len(case['events'])
    equal(len(ids),declaration['case_count'],'case count')
    equal(events,declaration['event_count_excluding_initial'],'event count')
    equal(snapshots,declaration['snapshot_count'],'snapshot count')
    for rate, counts in rates.items():
        equal(counts,{'fresh':declaration['fresh_cases_per_rate'],'explicit':declaration['explicit_cases_per_rate']},str(rate))
        equal(families[rate],{'width':1224,'mask':1280,'pair':6936},f'{rate}/fresh families')
    return {'cases':len(ids),'snapshots':snapshots,'hand_anchors':anchors,'rates':rates}


def shape(row, declaration):
    equal(sorted(row),sorted(declaration['snapshot_fields']),'snapshot fields')
    for key,n in PORTS.items():
        if type(row[key]) is not list or len(row[key])!=n: raise ValueError(f'{key} count')
        for port in row[key]:
            if set(port)!={'channels','values_bits'}: raise ValueError('port fields')
            count=port['channels']
            if type(count)is not int or not 0<=count<=16 or len(port['values_bits'])!=16: raise ValueError('port shape')
            for value in port['values_bits']: finite(value)
            if key in ('inputs','sink_inputs'):
                equal(port['values_bits'][count:],[0]*(16-count),'inactive input padding')
    for key,n in COUNTS.items():
        if len(row[key])!=n or any(type(v)is not int or v<0 for v in row[key]): raise ValueError('cable count shape')
    for key in ('average','invert','bypassed'):
        if type(row[key])is not bool: raise ValueError('boolean state required')
    finite(row['level_bits'])
    equal(row['saved_data'],{'average':row['average'],'invert':row['invert']},'actual saved data')
    if len(row['source_config'])!=6: raise ValueError('source config shape')
    for source in row['source_config']:
        if not isinstance(source,list) or len(source)>16: raise ValueError('source config width')
        for value in source: finite(value)


def validate(trace_path, fixture_path, declaration):
    trace=iter(rows(trace_path)); total=anchors=0
    for case in rows(fixture_path):
        initial=case['initial']; frame=0; previous=None
        state={'level_bits':initial['level_bits'],'average':False,'invert':False,'bypassed':False}
        control_update(state,{'op':'data_load','value':initial['data']})
        configs=[source.copy() for source in initial['sources']]
        cables={f'in{p}':('source',p,'module',p) for p in range(6) if initial['input_mask']&(1<<p)}
        cables.update({f'out{p}':('module',0,'sink',p) for p in range(2) if initial['output_mask']&(1<<p)})
        events=[{'op':'initial'}]+case['events']
        for index,event in enumerate(events):
            try: row=next(trace)
            except StopIteration: raise ValueError(f'missing {case["id"]}/{index-1}') from None
            shape(row,declaration); op=event['op']
            if op=='step': frame+=1
            if op=='source': configs[event['port']]=event['values_bits']
            control_update(state,event)
            if op=='connect':
                if event['id'] in cables: raise ValueError('duplicate declared cable')
                cables[event['id']]=(event['source'],event['port'],event['destination'],event['input'])
            elif op=='disconnect': del cables[event['id']]
            elif op=='recreate': cables={key:value for key,value in cables.items() if value[0]!='module' and value[2]!='module'}
            counts={key:[0]*size for key,size in COUNTS.items()}
            for src,port,dst,input_port in cables.values():
                if src=='source': counts['source_cables'][port]+=1
                elif src=='module': counts['output_cables'][port]+=1
                else: raise ValueError('unknown declared cable source')
                if dst=='module': counts['input_cables'][input_port]+=1
                elif dst=='sink': counts['sink_cables'][input_port]+=1
                else: raise ValueError('unknown declared cable destination')
            for key,want in counts.items(): equal(row[key],want,f'physical registry/{key}')
            for key,want in {'case':case['id'],'event':index-1,'op':op,'frame':frame,**state,'source_config':configs}.items():
                equal(row[key],want,f'{case["id"]}/{index-1}/{key}')
            if op=='initial':
                for key,connections in (('source_outputs','source_cables'),('inputs','input_cables'),('outputs','output_cables'),('sink_inputs','sink_cables')):
                    for port,number in zip(row[key],counts[connections]):
                        equal(port['values_bits'],[0]*16,'constructor has no DSP writes')
                        equal(port['channels'],int(number>0),'constructor connected count')
            if previous is not None and op in ('source','level','data_load','save','reset','adopt'):
                for key in (*PORTS,*COUNTS): equal(row[key],previous[key],f'{op} must not mutate ports/{key}')
            for path,want in event.get('expect',{}).items():
                equal(at(row,path),want,f'{case["id"]}/{index-1}/anchor/{path}');anchors+=1
            previous=row;total+=1
    try: next(trace)
    except StopIteration: return total,anchors
    raise ValueError('trailing snapshot')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs',type=Path,required=True)
    parser.add_argument('--sdk-trace',type=Path)
    parser.add_argument('--paired',type=Path)
    parser.add_argument('--fixtures-only',action='store_true')
    args=parser.parse_args(); declaration=json.loads((HERE/'acceptance.json').read_text())
    result=validate_fixture_count(args.inputs,declaration)
    if not args.fixtures_only:
        if not args.sdk_trace: parser.error('--sdk-trace required')
        counts=validate(args.sdk_trace,args.inputs,declaration)
        equal(counts,(result['snapshots'],result['hand_anchors']),'trace cardinality')
        if args.paired:
            equal(validate(args.paired,args.inputs,declaration),counts,'paired cardinality')
            for index,(actual,reference) in enumerate(zip(rows(args.paired),rows(args.sdk_trace))):
                equal(actual,reference,f'snapshot {index}')
    result.update(passed=True,discarded_snapshots=0,inputs_sha256=digest(args.inputs),
                  sdk_trace_sha256=digest(args.sdk_trace) if args.sdk_trace else None,
                  paired_trace_sha256=digest(args.paired) if args.paired else None,
                  scope='fixture structure only' if args.fixtures_only else ('complete paired host snapshots' if args.paired else 'SDK anchors only; production unqualified'))
    print(json.dumps(result,sort_keys=True))


if __name__=='__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'passed':False,'error':str(error)},sort_keys=True),file=sys.stderr);sys.exit(1)
