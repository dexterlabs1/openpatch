#!/usr/bin/env python3
"""Independent streaming invariants/hand anchors, not a quantizer implementation."""
import argparse
from contextlib import ExitStack
import hashlib
import json
import math
from pathlib import Path
import struct
import tempfile
import fixtures


def f32(x):
    try:return struct.unpack('<f',struct.pack('<f',float(x)))[0]
    except OverflowError:return math.copysign(float('inf'),float(x))
def bits(x):return struct.unpack('<I',struct.pack('<f',f32(x)))[0]
def same(a,b):return (math.isnan(a) and math.isnan(b)) or bits(a)==bits(b)
def integer(x,lo,hi):return math.isfinite(x) and x==int(x) and lo<=x<=hi
def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
    return h.hexdigest()
def parse(line,n):
    if not line:raise ValueError('early EOF')
    fields=line.split()
    if len(fields)!=n:raise ValueError(f'expected {n} fields, got {len(fields)}')
    integers=({0,2,19,20,21,22,23} if n==24 else {0,*range(17,44)})
    values=[]
    for index,token in enumerate(fields):
        if index in integers:
            value=float(token)
            if not math.isfinite(value) or value!=int(value):raise ValueError(f'field {index} must be an exact integer')
            values.append(value)
        else:values.append(f32(token))
    return values
def update_mask(mask,row):
    event=row[0]
    if event in (1,2):return 4095
    if event==4 and row[19]>=0:
        prefix=(1<<min(row[19],12))-1
        return (mask&~prefix)|(row[20]&prefix)
    if event==5:
        delta=row[21]%12
        return ((mask<<delta)|(mask>>(12-delta)))&4095
    if event==6:return row[20]
    return mask


def validate(rate,a,phase,input_path,sdk_trace_path_or_None,rust_trace_path):
    if rate not in a['sample_rates']:raise ValueError('undeclared sample rate')
    if phase not in ('reference','safety'):raise ValueError('phase must be reference or safety')
    if (phase=='reference') != (sdk_trace_path_or_None is not None):raise ValueError('reference requires SDK; safety forbids SDK')
    factory=fixtures.cases if phase=='reference' else fixtures.safety_cases
    report={'passed':False,'rate':rate,'phase':phase,'frames':0,'failed_frames':0,'protocol_failures':0,
            'first_failures':[],'mismatched_fields':0,'max_absolute_error':0.,
            'compared_field_counts':{'active_outputs':0,'counts':0,'enabled_masks':0,'ranges':0,'playing_valid':0,'playing_masks':0,'offsets':0},
            'independent_anchors_checked':0,'independent_invariants_checked':0,
            'oracle_claim':phase=='reference','trace_rows':{},
            'input_sha256':digest(input_path),'rust_trace_sha256':digest(rust_trace_path),
            'sdk_trace_sha256':digest(sdk_trace_path_or_None) if sdk_trace_path_or_None else None,
            'qualification_basis':'frozen original declaration; exact meaningful reference fields' if phase=='reference' else 'Rust safety extension only; no SDK numerical claim'}
    expected_mask=4095;expected_valid=False;previous_playing={};trace_rows={}
    def protocol_failure(case_name,frame_index,reason):
        report['protocol_failures']+=1
        if len(report['first_failures'])<16:report['first_failures'].append({'case':case_name,'frame':frame_index,'reasons':[reason]})
    with ExitStack() as stack:
        inp=stack.enter_context(Path(input_path).open())
        streams={'rust':stack.enter_context(Path(rust_trace_path).open())}
        if sdk_trace_path_or_None is not None:streams['sdk']=stack.enter_context(Path(sdk_trace_path_or_None).open())
        trace_rows={label:0 for label in streams};input_rows=0
        for case in factory(rate,a):
            anchors={anchor['frame']:anchor for anchor in case['anchors']}
            for frame_index,row in enumerate(case['frames']):
                report['frames']+=1;reasons=[]
                def check(ok,reason):
                    report['independent_invariants_checked']+=1
                    if not ok:reasons.append(reason)
                input_line=inp.readline()
                if input_line:input_rows+=1
                try:
                    actual_input=parse(input_line,24)
                    check(all(same(x,y) for x,y in zip(actual_input,row)),'input row differs from immutable generator')
                except (ValueError,OverflowError) as e:reasons.append('input: '+str(e));report['protocol_failures']+=1
                if row[0]==1:
                    expected_valid=False;previous_playing={label:0 for label in streams}
                expected_mask=update_mask(expected_mask,row)
                if not row[23]:expected_valid=True
                expected_count=max(1,row[2]) if row[22] else 0
                parsed={}
                for label,stream in streams.items():
                    line=stream.readline()
                    if line:trace_rows[label]+=1
                    try:out=parse(line,45)
                    except (ValueError,OverflowError) as e:
                        reasons.append(label+': '+str(e));report['protocol_failures']+=1;continue
                    parsed[label]=out
                    check(out[0]==expected_count,f'{label}: count {out[0]} != {expected_count}')
                    check(integer(out[17],0,4095) and out[17]==expected_mask,f'{label}: enabled mask {out[17]} != {expected_mask}')
                    check(out[42]==int(expected_valid),f'{label}: playing validity mismatch')
                    check(integer(out[43],0,4095),f'{label}: playing mask invalid')
                    check(same(out[44],row[1]),f'{label}: offset differs from receiving parameter')
                    valid_ranges=all(integer(value,-12,24) for value in out[18:42])
                    check(valid_ranges,f'{label}: cached range out of integer domain')
                    if valid_ranges:
                        if expected_mask:
                            check(all(expected_mask&(1<<(int(value)%12)) for value in out[18:42]),f'{label}: range selects a disabled note')
                        if expected_mask in (0,4095):
                            # Independent chromatic/empty-scale closed-form anchor only.
                            check(out[18:42]==[(i+1)//2 for i in range(24)],f'{label}: empty/full chromatic range anchor')
                        if expected_mask==1:
                            check(out[18:42]==[0]*13+[12]*11,f'{label}: single-C tie range anchor')
                    if not expected_valid:
                        check(out[43]==0,f'{label}: invalid playing state must use zero wire sentinel')
                    elif row[23]:
                        check(out[43]==previous_playing.get(label,0),f'{label}: bypass changed playing state')
                    else:
                        check(out[43]>0,f'{label}: process must mark at least one actual playing note')
                        if integer(out[43],0,4095) and expected_mask:
                            check(int(out[43])&~expected_mask==0,f'{label}: process marks a disabled note')
                    if label=='rust':
                        check(all(bits(v)==0 for v in out[1+expected_count:17]),'rust: inactive/disconnected output padding is not positive zero')
                    active=out[1:1+expected_count]
                    if row[23]:
                        expected_raw=row[3:3+row[2]] if row[2] else [0.]
                        check(all(same(value,expected_raw[lane]) for lane,value in enumerate(active)),f'{label}: bypass did not copy raw active pitch')
                    else:
                        check(all(math.isfinite(v) for v in active),f'{label}: non-bypassed output is nonfinite')
                        if active and all(math.isfinite(v) and abs(v)<1024 for v in active):
                            notes=[round(v*12) for v in active]
                            check(all(same(v,f32(note/12)) for v,note in zip(active,notes)),f'{label}: musical-range output is not a semitone')
                            playing=0
                            for note in notes:playing|=1<<(note%12)
                            check(out[43]==playing,f'{label}: actual playing mask differs from active-output pitch-class union')
                    anchor=anchors.get(frame_index)
                    if anchor:
                        for key,index in [('enabled_mask',17),('playing_mask',43),('offset',44),('count',0)]:
                            if key in anchor:
                                report['independent_anchors_checked']+=1
                                check(same(out[index],anchor[key]),f'{label}: independent {key} anchor')
                        for lane,value in anchor.get('outputs',{}).items():
                            report['independent_anchors_checked']+=1
                            check(int(lane)<expected_count and same(out[1+int(lane)],value),f'{label}: independent output anchor lane {lane}')
                    previous_playing[label]=out[43]
                if 'sdk' in parsed and 'rust' in parsed:
                    sdk,rust=parsed['sdk'],parsed['rust']
                    groups={'active_outputs':list(range(1,1+expected_count)),'counts':[0],'enabled_masks':[17],
                            'ranges':list(range(18,42)),'playing_valid':[42],'playing_masks':[43],'offsets':[44]}
                    for group,indices in groups.items():
                        report['compared_field_counts'][group]+=len(indices)
                        for i in indices:
                            # Normal domain is finite, and all comparisons retain signed-zero bits.
                            if not same(sdk[i],rust[i]):
                                report['mismatched_fields']+=1
                                if len(reasons)<32:reasons.append(f'field {i}: SDK={sdk[i]!r}, Rust={rust[i]!r}; exact f32 mismatch')
                                if math.isfinite(sdk[i]) and math.isfinite(rust[i]):report['max_absolute_error']=max(report['max_absolute_error'],abs(sdk[i]-rust[i]))
                if reasons:
                    report['failed_frames']+=1
                    if len(report['first_failures'])<16:report['first_failures'].append({'case':case['name'],'frame':frame_index,'global_row':report['frames']-1,'reasons':reasons[:32]})
        for label,stream in {'input':inp,**streams}.items():
            extra=sum(1 for _ in stream)
            if label=='input':input_rows+=extra
            else:trace_rows[label]+=extra
            if extra:protocol_failure('EOF',report['frames'],f'{label}: {extra} trailing rows')
        report['trace_rows']={'input':input_rows,**trace_rows}
    report['passed']=report['failed_frames']==0 and report['protocol_failures']==0
    return report


def self_test():
    """Synthetic protocol mutations only; never invokes either DSP implementation."""
    original_cases=fixtures.cases;original_safety=fixtures.safety_cases
    try:
        a={'sample_rates':[48000]}
        row=fixtures.frame(event=1,values=[0.])
        chromatic=[(i+1)//2 for i in range(24)]
        out=[1,*([0.]*16),4095,*chromatic,1,1,0.]
        fixtures.cases=lambda rate,a:[fixtures.case('synthetic_zero',[row],['synthetic'],[{'frame':0,'outputs':{0:0.},'playing_mask':1}])]
        with tempfile.TemporaryDirectory() as directory:
            paths=[Path(directory)/name for name in ('input','sdk','rust')]
            paths[0].write_text(fixtures.row_text(row));paths[1].write_text(fixtures.row_text(out));paths[2].write_text(fixtures.row_text(out))
            def run(phase='reference'):return validate(48000,a,phase,paths[0],paths[1] if phase=='reference' else None,paths[2])
            assert run()['passed']
            for index,value in [(0,0),(1,1.),(2,-0.),(17,0),(18,1),(42,0),(43,2),(44,.1),(1,float('nan'))]:
                changed=out[:];changed[index]=value;paths[2].write_text(fixtures.row_text(changed));assert not run()['passed'],index
            paths[2].write_text(fixtures.row_text(out)*2);assert not run()['passed']
            paths[2].write_text('');assert not run()['passed']
            paths[2].write_text(fixtures.row_text(out));paths[0].write_text(fixtures.row_text(row)*2);assert not run()['passed']
            # Fresh bypass must not read uninitialized source playing booleans.
            bypass=fixtures.frame(event=1,values=[-.0],bypassed=1)
            frozen=[1,-.0,*([0.]*15),4095,*chromatic,0,0,0.]
            fixtures.cases=lambda rate,a:[fixtures.case('synthetic_bypass',[bypass],['synthetic'])]
            paths[0].write_text(fixtures.row_text(bypass));paths[1].write_text(fixtures.row_text(frozen));paths[2].write_text(fixtures.row_text(frozen));assert run()['passed']
            # Nonfinite bypass is an intentionally raw Rust safety contract.
            unsafe=fixtures.frame(event=1,values=[float('nan'),float('inf'),-float('inf')],bypassed=1)
            raw=[3,float('nan'),float('inf'),-float('inf'),*([0.]*13),4095,*chromatic,0,0,0.]
            fixtures.safety_cases=lambda rate,a:[fixtures.case('synthetic_raw_bypass',[unsafe],['synthetic'])]
            paths[0].write_text(fixtures.row_text(unsafe));paths[2].write_text(fixtures.row_text(raw));assert run('safety')['passed']
            raw[2]=0.;paths[2].write_text(fixtures.row_text(raw));assert not run('safety')['passed']
    finally:fixtures.cases=original_cases;fixtures.safety_cases=original_safety
    print('Synthetic validator mutation checks passed; no DSP/oracle execution.')

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--self-test',action='store_true');args=parser.parse_args()
    if not args.self_test:parser.error('Use --self-test; production runner calls validate().')
    self_test()
