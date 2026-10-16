#!/usr/bin/env python3
"""Streaming exact comparison and independent invariants, not a lifecycle evaluator."""
import argparse
from contextlib import ExitStack
import hashlib
import json
import math
from pathlib import Path
import struct
import tempfile
import fixtures


def f32(value):
    try:return struct.unpack('<f',struct.pack('<f',float(value)))[0]
    except OverflowError:return math.copysign(float('inf'),float(value))
def bits(value):return struct.unpack('<I',struct.pack('<f',f32(value)))[0]
def same(left,right):return bits(left)==bits(right)
def integer(value,low,high):return math.isfinite(value) and value==int(value) and low<=value<=high
def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
    return h.hexdigest()
def parse(line,expected_fields,input_row=False):
    if not line:raise ValueError('early EOF')
    tokens=line.split()
    if len(tokens)!=expected_fields:raise ValueError(f'expected {expected_fields} fields, got {len(tokens)}')
    out=[]
    for index,token in enumerate(tokens):
        is_integer=(index<5 or index>=69) if input_row else (index<4 or index>=68)
        if is_integer:
            value=float(token)
            if not math.isfinite(value) or value!=int(value):raise ValueError(f'field {index} must be an integer')
            out.append(int(value))
        else:out.append(f32(token))
    return out

def final_assignments(row):
    # Only independently tracks requested control assignments, never port
    # counts, clear extent or raw storage. Those come from the actual traces.
    mask=row[0];bypassed=False
    for op in row[70:]:
        if op<8:mask|=1<<op
        elif op<16:mask&=~(1<<(op-8))
        elif op==16:bypassed=not bypassed
        else:bypassed=op==18
    return mask,int(bypassed)

def validate(a,input_path,sdk_trace_path,rust_trace_path):
    report={'passed':False,'cases':0,'failed_cases':0,'protocol_failures':0,
            'first_failures':[],'compared_fields':0,'mismatched_fields':0,
            'compared_field_counts':{'counts':0,'raw_voltages':0,'bypassed':0,'consumer_mask':0},
            'independent_anchors_checked':0,'independent_invariants_checked':0,'max_absolute_error':0.,
            'input_sha256':digest(input_path),'sdk_trace_sha256':digest(sdk_trace_path),'rust_trace_sha256':digest(rust_trace_path),
            'expected_engine_dsp_frames':0,'engine_frame_observation_source':'Separate actual SDK adapter assertion required; not encoded in70-field trace',
            'qualification_basis':'Bounded producer-output lifecycle; exact full raw storage, no DSP/host/device equivalence claim',
            'trace_rows':{}}
    def failure(name,index,reasons):
        if len(report['first_failures'])<16:report['first_failures'].append({'case':name,'case_index':index,'reasons':reasons[:32]})
    with ExitStack() as stack:
        inp=stack.enter_context(Path(input_path).open())
        streams={label:stack.enter_context(Path(path).open()) for label,path in [('sdk',sdk_trace_path),('rust',rust_trace_path)]}
        row_counts={'input':0,'sdk':0,'rust':0}
        for c in fixtures.cases(a):
            index=report['cases'];report['cases']+=1;row=c['fields'];reasons=[];parsed={}
            def check(ok,reason):
                report['independent_invariants_checked']+=1
                if not ok:reasons.append(reason)
            line=inp.readline()
            if line:row_counts['input']+=1
            try:
                actual=parse(line,len(row),True)
                check(all((x==y if i<5 or i>=69 else same(x,y)) for i,(x,y) in enumerate(zip(actual,row))), 'input differs from frozen generated row')
            except (ValueError,OverflowError) as e:reasons.append('input: '+str(e));report['protocol_failures']+=1
            mask,bypassed=final_assignments(row)
            for label,stream in streams.items():
                line=stream.readline()
                if line:row_counts[label]+=1
                try:out=parse(line,70)
                except (ValueError,OverflowError) as e:reasons.append(label+': '+str(e));report['protocol_failures']+=1;continue
                parsed[label]=out
                check(integer(out[69],0,255) and out[69]==mask,f'{label}: actual consumer mask differs from requested final membership')
                check(out[68]==bypassed,f'{label}: actual bypass state differs from final assignment')
                check(all(math.isfinite(value) for value in out[4:68]),f'{label}: raw storage became nonfinite')
                for port in range(4):
                    count=out[port];initial_count=row[1+port];connected=bool(mask&(3<<(2*port)))
                    check(integer(count,0,16),f'{label}: port{port} invalid count')
                    check((count>0)==connected,f'{label}: port{port} count disagrees with consumers')
                    check(count in (0,1,initial_count),f'{label}: port{port} count is not reachable from declared count-changing operations')
                    for lane in range(16):
                        value=out[4+16*port+lane];seed=row[5+16*port+lane]
                        check(same(value,seed) or bits(value)==0,f'{label}: port{port}/lane{lane} neither retains seed nor clears to positive zero')
                        if lane>=max(1,initial_count):
                            check(same(value,seed),f'{label}: port{port}/lane{lane} beyond possible clear extent changed')
                anchor=c.get('anchor')
                if anchor:
                    for port in range(4):
                        report['independent_anchors_checked']+=1
                        check(out[port]==anchor['counts'][port],f'{label}: hand anchor count port{port}')
                        for lane in range(16):
                            report['independent_anchors_checked']+=1
                            expected=0. if lane<anchor['clear_prefixes'][port] else row[5+16*port+lane]
                            check(same(out[4+16*port+lane],expected),f'{label}: hand anchor raw port{port}/lane{lane}')
                    for field,key in [(68,'bypassed'),(69,'consumer_mask')]:
                        report['independent_anchors_checked']+=1
                        check(out[field]==anchor[key],f'{label}: hand anchor {key}')
            if len(parsed)==2:
                sdk,rust=parsed['sdk'],parsed['rust']
                for group,indices in [('counts',range(4)),('raw_voltages',range(4,68)),('bypassed',[68]),('consumer_mask',[69])]:
                    report['compared_field_counts'][group]+=len(indices)
                    for field in indices:
                        report['compared_fields']+=1
                        matches=(sdk[field]==rust[field]) if field<4 or field>=68 else same(sdk[field],rust[field])
                        if not matches:
                            report['mismatched_fields']+=1
                            if len(reasons)<32:reasons.append(f'field{field}: SDK={sdk[field]!r}, Rust={rust[field]!r}, exact mismatch')
                            if math.isfinite(sdk[field]) and math.isfinite(rust[field]):report['max_absolute_error']=max(report['max_absolute_error'],abs(sdk[field]-rust[field]))
            if reasons:report['failed_cases']+=1;failure(c['name'],index,reasons)
        for label,stream in {'input':inp,**streams}.items():
            extra=sum(1 for _ in stream);row_counts[label]+=extra
            if extra:
                report['protocol_failures']+=1;failure('EOF',report['cases'],[f'{label}: {extra} trailing rows'])
        report['trace_rows']=row_counts
    if report['cases']!=a['coverage']['cases']:
        report['protocol_failures']+=1;failure('declaration',report['cases'],['case total differs from acceptance'])
    report['passed']=report['failed_cases']==0 and report['protocol_failures']==0
    return report


def self_test():
    """Synthetic protocol/state mutations; no Engine or production execution."""
    original=fixtures.cases
    try:
        a={'coverage':{'cases':1}}
        c=fixtures.entry('synthetic_empty','synthetic',1,[7,0,0,0],1,[])
        out=[7,0,0,0,*fixtures.seed(1),0,1]
        fixtures.cases=lambda a:[c]
        with tempfile.TemporaryDirectory() as directory:
            paths=[Path(directory)/name for name in ('input','sdk','rust')]
            def reset(case=c,value=out):
                paths[0].write_text(fixtures.row_text(case['fields']))
                for path in paths[1:]:path.write_text(fixtures.row_text(value))
            def run():return validate(a,*paths)
            reset();assert run()['passed']
            for field,value in [(0,0),(1,2),(4,0.),(5,-0.),(19,0.),(68,1),(69,3),(4,float('nan'))]:
                changed=out[:];changed[field]=value;paths[2].write_text(fixtures.row_text(changed));assert not run()['passed'],field
            reset();paths[2].write_text(fixtures.row_text(out)*2);assert not run()['passed']
            reset();paths[2].write_text('');assert not run()['passed']
            reset();paths[0].write_text(fixtures.row_text(c['fields'])*2);assert not run()['passed']
            # Both traces agree on an independently wrong hand-anchor result.
            anchor={'counts':[0,0,0,0],'clear_prefixes':[0,0,0,0],'bypassed':1,'consumer_mask':0}
            c=fixtures.entry('synthetic_order','synthetic',1,[7,0,0,0],0,[fixtures.D(0),fixtures.B],anchor)
            fixtures.cases=lambda a:[c]
            correct=[0,0,0,0,*fixtures.seed(0),1,0];reset(c,correct);assert run()['passed']
            wrong=correct[:];wrong[4]=0.;reset(c,wrong);assert not run()['passed']
            # A correctly cleared negative-zero lane must be positive zero.
            anchor={'counts':[1,0,0,0],'clear_prefixes':[1,0,0,0],'bypassed':1,'consumer_mask':1}
            c=fixtures.entry('synthetic_signed_clear','synthetic',0,[0,0,0,0],1,[fixtures.C(0),fixtures.B],anchor)
            correct=[1,0,0,0,*fixtures.seed(1),1,1];correct[4]=0.;reset(c,correct);assert run()['passed']
            wrong=correct[:];wrong[4]=-0.;reset(c,wrong);assert not run()['passed']
    finally:fixtures.cases=original
    print('Synthetic lifecycle validator mutation checks passed; no SDK/Core/DSP execution.')

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--self-test',action='store_true');args=p.parse_args()
    if not args.self_test:p.error('Use --self-test; parent runner imports validate().')
    self_test()
