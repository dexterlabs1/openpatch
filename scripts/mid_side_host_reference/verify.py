#!/usr/bin/env python3
"""Independent exact host snapshot validator; never computes MidSide DSP."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).resolve().parent

def bits(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f'invalid finite float: {value!r}')
    return struct.pack('<f', value)

def at(row, path):
    for key in path.split('.'):
        row = row[int(key)] if isinstance(row, list) else row[key]
    return row

def equal(a, b, path=''):
    if isinstance(a, bool) or isinstance(b, bool):
        if type(a) is not type(b) or a != b: raise ValueError(f'{path}: {a!r} != {b!r}')
    elif isinstance(a, dict):
        if not isinstance(b, dict) or a.keys() != b.keys(): raise ValueError(f'{path}: dictionary keys differ')
        for k in a: equal(a[k], b[k], f'{path}.{k}')
    elif isinstance(a, list):
        if not isinstance(b, list) or len(a) != len(b): raise ValueError(f'{path}: array lengths differ')
        for i, (x,y) in enumerate(zip(a,b)): equal(x,y,f'{path}.{i}')
    elif isinstance(a, (int,float)) and isinstance(b, (int,float)):
        is_float = '.values.' in path or '.params.' in path or '.source_config.' in path
        if (bits(a) != bits(b)) if is_float else (a != b): raise ValueError(f'{path}: {a!r} != {b!r}')
    elif type(a) is not type(b) or a != b:
        raise ValueError(f'{path}: {a!r} != {b!r}')

def rows(path):
    with path.open() as f:
        for line in f:
            if not line.strip(): raise ValueError('blank trace line')
            yield json.loads(line)

def validate(path, cases):
    trace=iter(rows(path)); count=0; anchors=0
    keys={'case','event','op','frame','bypassed','inputs','outputs','sink_inputs','source_outputs','params','source_config','input_cables','output_cables'}
    for case in cases:
        frame=0
        for i,event in enumerate(case['events']):
            try: row=next(trace)
            except StopIteration: raise ValueError(f'missing {case["id"]} event {i}') from None
            if set(row)!=keys: raise ValueError(f'{case["id"]}/{i}: unexpected snapshot keys')
            if event['op']=='step': frame+=1
            for key,expected in [('case',case['id']),('event',i),('op',event['op']),('frame',frame)]: equal(row[key],expected,key)
            if not isinstance(row['bypassed'],bool): raise ValueError('bypass must be boolean')
            for key,n in [('inputs',6),('outputs',4),('sink_inputs',8),('source_outputs',6)]:
                ports=row[key]
                if not isinstance(ports,list) or len(ports)!=n: raise ValueError(f'{key}: wrong port count')
                for port in ports:
                    if set(port)!={'channels','values'}: raise ValueError('wrong port keys')
                    channels=port['channels']; values=port['values']
                    if type(channels) is not int or not 0<=channels<=16 or len(values)!=16: raise ValueError('invalid physical port shape')
                    for value in values: bits(value)
                    if key in ('inputs','sink_inputs') and any(bits(v)!=bits(0) for v in values[channels:]): raise ValueError('inactive input slots must be positive zero')
            for key,n in [('input_cables',6),('output_cables',4)]:
                if len(row[key])!=n or any(type(v) is not int or v<0 for v in row[key]): raise ValueError('invalid physical cable counts')
            if len(row['params'])!=2 or len(row['source_config'])!=6: raise ValueError('invalid configuration shape')
            for value in row['params']: bits(value)
            for values in row['source_config']:
                if not isinstance(values,list) or len(values)>16: raise ValueError('invalid source configuration')
                for value in values: bits(value)
            for path,expected in event.get('expect',{}).items():
                equal(at(row,path),expected,'.'+path);anchors+=1
            count+=1
    try: next(trace)
    except StopIteration: return count,anchors
    raise ValueError('trailing unrequested snapshot')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('sdk_trace',type=Path)
    parser.add_argument('--paired',type=Path,help='actual production Engine host snapshots in identical schema/order')
    parser.add_argument('--cases',type=Path,default=ROOT/'cases.json')
    args=parser.parse_args()
    declaration=json.loads((ROOT/'acceptance.json').read_text())
    cases=json.loads(args.cases.read_text())['cases']
    if len(cases)!=declaration['case_count'] or sum(len(c['events']) for c in cases)!=declaration['event_count']: raise ValueError('declaration/case counts differ')
    count,anchors=validate(args.sdk_trace,cases)
    if args.paired:
        validate(args.paired,cases)
        for i,(sdk,production) in enumerate(zip(rows(args.sdk_trace),rows(args.paired))):equal(sdk,production,f'row{i}')
    digest=lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    print(json.dumps({'passed':True,'scope':'paired genuine host snapshots' if args.paired else 'SDK hand-anchor validation only; no production equivalence claim','cases':len(cases),'snapshots':count,'hand_anchors':anchors,'discarded_snapshots':0,'cases_sha256':digest(args.cases),'acceptance_sha256':digest(ROOT/'acceptance.json'),'sdk_trace_sha256':digest(args.sdk_trace),'paired_trace_sha256':digest(args.paired) if args.paired else None},sort_keys=True))

if __name__=='__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'passed':False,'error':str(error)},sort_keys=True),file=sys.stderr)
        sys.exit(1)
