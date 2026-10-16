#!/usr/bin/env python3
"""Independent Stock Quantizer inputs; never computes quantized output tables."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
HERE=Path(__file__).resolve().parent

def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def bits(x):return struct.unpack('<I',struct.pack('<f',x))[0]
def raw(x):return struct.unpack('<f',struct.pack('<I',x))[0]
def neighbor(x,direction):
    x=f32(x)
    if x==0:return raw(1 if direction>0 else 0x80000001)
    return raw(bits(x)+(1 if (x>0)==(direction>0) else -1))
def scaled(x,offset=0.):return f32(f32(x+offset)*24.)
def transition(k,offset=0.):
    # Input arithmetic only. Binary search ordered f32 encodings so cancellation
    # plateaus near input zero cannot require millions of adjacent-float steps.
    # The fixture's octave/offset limits put every threshold inside [-64,64].
    def ordered(x):
        b=bits(x);return b^(0xffffffff if b&0x80000000 else 0x80000000)
    def value(key):return raw(key^(0x80000000 if key&0x80000000 else 0xffffffff))
    low,high=ordered(-64.),ordered(64.)
    assert scaled(value(low),offset)<k<=scaled(value(high),offset)
    while low+1<high:
        mid=(low+high)//2
        if scaled(value(mid),offset)>=k:high=mid
        else:low=mid
    previous,current=value(low),value(high)
    assert scaled(previous,offset)<k<=scaled(current,offset)
    return previous,current
def points(octave,offset=0.):
    result=[]
    for i in range(24):
        boundary=f32(octave+i/24-offset)
        result.extend([f32(octave+(i+.5)/24-offset),neighbor(boundary,-1),boundary,neighbor(boundary,1),*transition(octave*24+i,offset)])
    return result

def frame(event=0,offset=0.,values=(),width=None,array_length=-1,mask=4095,rotate=0,connected=1,bypassed=0):
    values=list(values);width=len(values) if width is None else width
    assert 0<=width<=16 and len(values)>=width
    return [event,f32(offset),width,*map(f32,values[:width]),*([0.]*(16-width)),array_length,mask,rotate,connected,bypassed]
def case(name,frames,tags,anchors=None):return {'name':name,'frames':frames,'tags':tags,'anchors':anchors or []}
def packed(values,offset=0.,mask=4095,first_load=False):
    return [frame(event=4 if first_load and start==0 else 0,offset=offset,values=values[start:start+16],array_length=12,mask=mask)
            for start in range(0,len(values),16)]
SELECTED=sorted(set([0,4095,5,129,85,170,0x249,0x555,0xaaa]+[1<<n for n in range(12)]+[4095^(1<<n) for n in range(12)]))

def cases(rate,a):
    main=points(0)+points(-1)
    for mask in range(4096):
        yield case(f'exhaustive_mask_{mask:03x}',[frame(event=1)]+packed(main,mask=mask,first_load=True),
                   ['all4096masks','all24ranges','sixpoints','positive_negative_octaves','16voice_packing'])
    for mask in SELECTED:
        for offset in [-1.,-.5,f32(-1/12),f32(.1),f32(1/12),.5,1.]:
            values=[]
            for octave in [-16,-2,1,16]:values.extend(points(octave,offset))
            yield case(f'offset_mask{mask:03x}_{offset}',[frame(event=1,offset=offset)]+packed(values,offset,mask,True),
                       ['offset_f32_cancellation','negative_octaves','large_musical_octaves'])
    for mask in SELECTED:
        frames=[frame(event=1),frame(event=4,array_length=12,mask=mask)]
        for connected in [1,0,1]:
            for width in range(17):
                frames.append(frame(values=[f32((i-8)/12) for i in range(16)],width=width,connected=connected))
        for width in [16,0,1,3,16,1,0]:
            frames.append(frame(values=[f32((i%3)/12) for i in range(16)],width=width))
        yield case(f'widths_consumers_{mask:03x}',frames,['width0..16','actual_consumer_context','playing_union_replacement','duplicate_notes','shrink_regrow'])
    for offset in [-1.,0.,1.]:
        for mask in SELECTED:
            yield case(f'int_domain_edges_{mask:03x}_{offset}',[frame(event=1,offset=offset),
                frame(event=4,offset=offset,array_length=12,mask=mask,values=[-89478480.,-89478472.,-1048576.,-65536.,-0.,0.,65536.,1048576.,89478472.,89478480.])],
                ['finite_int_safe_domain_edges','large_octave_output_precision','signedzero'])
    # C/D source mapping deliberately delays the C->D and D->nextC thresholds.
    values=[0.1,neighbor(.125,-1),.125,neighbor(.125,1),.6,neighbor(.625,-1),.625,
            -.9,neighbor(-.875,-1),-.875,neighbor(-.875,1),-.4,neighbor(-.375,-1),-.375]
    expected=[0.,0.,f32(2/12),f32(2/12),f32(2/12),f32(2/12),1.,-1.,-1.,f32(-10/12),f32(-10/12),f32(-10/12),f32(-10/12),0.]
    yield case('hand_anchor_C_D_delayed_thresholds',[frame(event=1),frame(event=4,array_length=12,mask=5,values=values)],
               ['independent_hand_anchor','not_nearest_continuous_pitch'],[{'frame':1,'outputs':dict(enumerate(expected)),'enabled_mask':5,'playing_mask':5}])
    yield case('hand_anchor_binary_offset',[frame(event=1),frame(event=4,offset=-.125,array_length=12,mask=5,values=[.25,.75])],
               ['independent_offset_anchor'],[{'frame':1,'outputs':{0:f32(2/12),1:1.},'playing_mask':5}])
    yield case('hand_anchor_single_C_ties',[frame(event=1),frame(event=4,array_length=12,mask=1,values=[.5,-.5])],
               ['independent_gap_tie_anchor'],[{'frame':1,'outputs':{0:0.,1:-1.},'playing_mask':1}])
    for seed in [0,5,129,0x555,4095]:
        for length in [-1,0,1,2,6,11,12,13,14]:
            changed=0xaaa
            effective=seed if length<0 else (seed&~((1<<min(length,12))-1))|(changed&((1<<min(length,12))-1))
            yield case(f'partial_json_seed{seed}_length{length}',[
                frame(event=1),frame(event=4,array_length=12,mask=seed,values=[0.]),
                frame(event=4,array_length=length,mask=changed,values=[.1,.625]),
                frame(event=3,values=[.1,.625]),frame(event=2,offset=.5,values=[0.])],
                ['actual_json_missing_short_long_arrays','partial_tail_retention','same_saved_adoption','runtime_reset_offset_retained'],
                [{'frame':2,'enabled_mask':effective},{'frame':3,'enabled_mask':effective},
                 {'frame':4,'enabled_mask':4095,'offset':.5,'outputs':{0:.5},'playing_mask':64}])
    for mask in SELECTED:
        frames=[frame(event=1),frame(event=4,array_length=12,mask=mask,values=[.1,.625])]
        for delta in [1,-1,12,-12]:frames.append(frame(event=5,rotate=delta,values=[.1,.625]))
        frames += [frame(event=3,offset=.125,values=[0.,.5]),frame(event=6,offset=-.125,mask=5,values=[.25,.75]),
                   frame(event=2,offset=.5,values=[0.])]
        yield case(f'rotate_adopt_{mask:03x}',frames,['both_rotations','rotation_composition','explicit_receiving_mask','source_reset'],
            [{'frame':3,'enabled_mask':mask},{'frame':5,'enabled_mask':mask},
             {'frame':7,'enabled_mask':5,'offset':-.125,'outputs':{0:f32(2/12),1:1.},'playing_mask':5}])
    for mask in [0,5,1,4095]:
        frames=[frame(event=1,bypassed=1,offset=.5),frame(event=4,array_length=12,mask=mask,bypassed=1,values=[.2,.7]),
                frame(values=[.1,.625],connected=0)]
        for connected in [1,0,1]:
            for width in [16,1,0,3]:
                frames.append(frame(offset=.5,values=[f32((i-7)/7) for i in range(16)],width=width,connected=connected,bypassed=1))
        frames += [frame(event=5,rotate=1,bypassed=1),frame(event=2,bypassed=1),frame(event=3,bypassed=1),
                   frame(values=[0.,.1,.625]),frame(connected=0,values=[.5]),frame(values=[-0.])]
        yield case(f'bypass_lifecycle_{mask}',frames,['actual_SDK_bypass','fresh_playing_invalid','playing_freeze','editing_during_bypass','consumer_reconnect'])

def safety_cases(rate,a):
    bad=[float('nan'),float('inf'),-float('inf'),raw(0x7f7fffff),raw(0xff7fffff),89478488.,-89478488.,89478496.,-89478496.,1e20,-1e20,0.,-0.,.1,.125,.625]
    for mask in [0,1,5,0xaaa,4095]:
        for offset in [-1.,0.,1.]:
            anchors=[{'frame':6,'outputs':{0:0.,1:0.,2:0.},'enabled_mask':4095,'playing_mask':1}]
            if mask in (0,4095):
                # Closed-form saturation anchors: +/-2**30 notes divided by 12.
                # These rule out replacing the declared extension with a small CV clamp.
                anchors.append({'frame':1,'outputs':{0:offset,1:offset,2:offset,
                    3:89478488.,4:-89478488.,5:89478488.,6:-89478488.,
                    7:89478488.,8:-89478488.,9:89478488.,10:-89478488.}})
            yield case(f'safety_{mask}_{offset}',[frame(event=1),frame(event=4,array_length=12,mask=mask,offset=offset,values=bad),
                frame(offset=offset,values=bad,connected=0),frame(offset=offset,values=bad,bypassed=1),
                frame(offset=offset,values=bad,bypassed=1,connected=0),frame(event=3,offset=offset,values=[0.,.1,.625]),
                frame(event=2,values=[float('nan'),float('inf'),-float('inf')])],
                ['Rust_only','undefined_source_cast_or_overflow','defined_saturating_extension','raw_bypass_nonfinite_copy','recovery'],
                anchors)

def expand(items):
    for c in items:
        for i,row in enumerate(c['frames']):yield c['name'],i,row

def rows(rate,a):return expand(cases(rate,a))
def safety_rows(rate,a):return expand(safety_cases(rate,a))
def row_text(row):return ' '.join(str(x) if isinstance(x,int) else 'NaN' if math.isnan(x) else format(x,'.9g') for x in row)+'\n'
def fixture_text(rate,a,safety=False):return ''.join(row_text(row) for _,_,row in (safety_rows if safety else rows)(rate,a))
def safety_fixture_text(rate,a):return fixture_text(rate,a,True)
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def validate_row(row,safety=False):
    assert len(row)==24 and row[0] in range(7) and -1<=row[1]<=1 and isinstance(row[2],int) and 0<=row[2]<=16
    assert -1<=row[19]<=14 and 0<=row[20]<=4095 and -12<=row[21]<=12 and row[22] in [0,1] and row[23] in [0,1]
    assert all(bits(x)==0 for x in row[3+row[2]:19])
    if not safety:
        assert all(math.isfinite(x) for x in row)
        for x in row[3:3+max(1,row[2])]:
            q=scaled(x,row[1]);assert math.isfinite(q) and -2147483640<=math.floor(q)<=2147483647

def manifest(a):
    out={'schema':1,'declared_before_first_comparison':True,'acceptance_sha256':digest(HERE/'acceptance.json'),
         'generator_sha256':digest(HERE/'fixtures.py'),'validator_sha256':digest(HERE/'verify.py'),
         'readme_sha256':digest(HERE/'README.md'),'rates':{}}
    for rate in a['sample_rates']:
        out['rates'][str(rate)]={}
        for phase,factory,iterator in [('reference',cases,rows),('safety',safety_cases,safety_rows)]:
            h=hashlib.sha256();count=0
            for _,_,row in iterator(rate,a):validate_row(row,phase=='safety');h.update(row_text(row).encode());count+=1
            items=[];cursor=0
            for c in factory(rate,a):
                length=len(c['frames']);items.append({k:v for k,v in c.items() if k!='frames'}|{'frames':length,'start_row':cursor,'end_row_exclusive':cursor+length});cursor+=length
            assert count==cursor and len(items)==len({i['name'] for i in items})
            out['rates'][str(rate)][phase]={'processed_frames':count,'observed_frames':count,'cases':len(items),'input_sha256':h.hexdigest(),'case_manifest':items}
    # JSON object keys are strings; normalize anchors before immutable equality checks.
    return json.loads(json.dumps(out))

def check_declaration():
    a=json.loads((HERE/'acceptance.json').read_text());m=manifest(a)
    assert m==json.loads((HERE/'fixtures.json').read_text()),'Frozen declaration changed'
    return m

def main():
    p=argparse.ArgumentParser();p.add_argument('--freeze',action='store_true');p.add_argument('--check-fixtures',action='store_true');p.add_argument('--check',action='store_true');args=p.parse_args()
    a=json.loads((HERE/'acceptance.json').read_text());m=manifest(a)
    if args.freeze:
        with (HERE/'fixtures.json').open('x') as f:json.dump(m,f,indent=2,sort_keys=True);f.write('\n')
    else:assert args.check or args.check_fixtures;assert m==json.loads((HERE/'fixtures.json').read_text())
    print(json.dumps({'fixtures_sha256':digest(HERE/'fixtures.json'),'generator_sha256':m['generator_sha256'],'rates':{r:{p:{k:v for k,v in s.items() if k!='case_manifest'} for p,s in phases.items()} for r,phases in m['rates'].items()}},indent=2))
if __name__=='__main__':main()
