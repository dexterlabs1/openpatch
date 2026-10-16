#!/usr/bin/env python3
"""Independent Stock Noise input/state/paired validator; no DSP implementation import."""
import argparse
import collections
import json
import math
from pathlib import Path
import struct
import sys
sys.dont_write_bytecode=True
from fixtures import RATES, TAPE_LENGTH, PROFILES, UNIFORM_TAPE

HERE = Path(__file__).resolve().parent
FIELDS = tuple(json.loads((HERE/'acceptance.json').read_text())['snapshot_fields'])
RUNTIME = ('red_x_bits','red_y_bits','last_white_bits','last_pink_bits',
           'pink_frame','pink_values_bits','gray_frame','gray_input_bits',
           'gray_output_bits','black_divider_clock','black_divider_division','light_bits')
GRAY_RAW_ABS = 2.5e-4
GRAY_VOLT_ABS = 5e-4
GRAY_REL = 3e-5

def rows(path):
    def unique(pairs):
        value={}
        for k,v in pairs:
            if k in value: raise ValueError(f'duplicate JSON key {k}')
            value[k]=v
        return value
    with Path(path).open() as stream:
        for line in stream:
            if not line.strip(): raise ValueError('blank JSONL row')
            yield json.loads(line,object_pairs_hook=unique,
                             parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))
def u32(x):
    if type(x) is not int or not 0 <= x <= 0xffffffff: raise ValueError(f'u32 required: {x!r}')
def integer(x,lo,hi):
    if type(x) is not int or not lo <= x <= hi: raise ValueError(f'integer {lo}..{hi} required: {x!r}')
def f32(bits): return struct.unpack('<f',struct.pack('<I',bits))[0]
def sample_time_bits(rate):
    exponent = 1-rate.bit_length() if rate & (rate-1) == 0 else -rate.bit_length()
    q,r=divmod(1 << (23-exponent),rate)
    q += int(2*r > rate or (2*r == rate and q&1))
    return ((exponent+127)<<23)|(q-(1<<23))
def check_case(c):
    if set(c) != {'id','sample_rate','initial','events'}: raise ValueError('case shape')
    if type(c['id']) is not str or not c['id']: raise ValueError('case id')
    if c['sample_rate'] not in RATES: raise ValueError('sample rate')
    if set(c['initial']) != {'output_mask','normal_profile','normal_bits','uniform_bits'}: raise ValueError('initial shape')
    integer(c['initial']['output_mask'],0,127)
    profile=c['initial']['normal_profile']
    if profile not in PROFILES or tuple(c['initial']['normal_bits'])!=PROFILES[profile]:
        raise ValueError('normal tape/profile drift')
    if tuple(c['initial']['uniform_bits'])!=UNIFORM_TAPE:raise ValueError('uniform tape drift')
    if len(c['initial']['normal_bits'])!=TAPE_LENGTH or len(c['initial']['uniform_bits'])!=TAPE_LENGTH:
        raise ValueError('controlled tape length')
    if type(c['events']) is not list: raise ValueError('events shape')
    registry={f'out{p}':('module',p,'sink',p) for p in range(7)
              if c['initial']['output_mask']&(1<<p)}
    for e in c['events']:
        if type(e) is not dict or type(e.get('op')) is not str: raise ValueError('event shape')
        op=e['op']; keys={
            'step':{'op'}, 'reset':{'op'}, 'adopt':{'op'},'recreate':{'op'},
            'bypass':{'op','value'},'disconnect':{'op','id'},
            'connect':{'op','id','source','port','destination','input'}}
        if op not in keys or (set(e)!=keys[op] and not(op=='step' and set(e)=={'op','frames'})):
            raise ValueError(f'event fields: {e}')
        if op=='step':integer(e.get('frames',1),1,2048)
        if op=='bypass' and type(e['value']) is not bool: raise ValueError('bypass bool')
        if op=='connect':
            if e['id'] in registry or type(e['id']) is not str or not e['id']: raise ValueError('cable id')
            if e['source']!='module' or e['destination']!='sink': raise ValueError('cable roles')
            integer(e['port'],0,6);integer(e['input'],0,6)
            registry[e['id']]=('module',e['port'],'sink',e['input'])
        if op=='disconnect':
            if e['id'] not in registry: raise ValueError('unknown cable')
            del registry[e['id']]
        if op=='recreate': registry.clear()
def check_inputs(inputs):
    ids=set();families=collections.Counter();cases=events=0
    for c in rows(inputs):
        check_case(c)
        if c['id'] in ids: raise ValueError('duplicate case id')
        ids.add(c['id']);cases+=1;events+=len(c['events'])
        _,family=c['id'].split('/',1);families[(c['sample_rate'],family.split('/')[0])]+=1
        if family.startswith('masks/'):
            expected_events=4 if c['initial']['output_mask']&(1<<5) else 2
            if int(family.split('/')[1]) != c['initial']['output_mask'] or len(c['events'])!=expected_events:
                raise ValueError('mask fixture drift')
    for rate in RATES:
        if families[(rate,'masks')]!=128: raise ValueError('all 128 consumer masks required per rate')
        for name in ('all-boundaries','gray-solo-boundaries','gray-freeze','voss-wrap',
                     'source-family-freezes','cable-bypass-reset-adopt-recreate',
                     'post-fft-topology'):
            if families[(rate,name)]!=1: raise ValueError(f'missing {rate}/{name}')
        if families[(rate,'gray-profile')]!=6+(rate==96000):raise ValueError('gray diagnostic profile coverage')
    if cases!=424: raise ValueError(f'expected 424 cases, got {cases}')
    return {'passed':True,'cases':cases,'events':events,'snapshots':cases+events,
            'output_consumer_masks_per_rate':128,'sample_rates':list(RATES),
            'discarded_snapshots':0}
def expected_registry(registry):
    return [{'id':i,'source':src,'port':port,'destination':dst,'input':inp}
            for i,(src,port,dst,inp) in sorted(registry.items())]
def port_shape(value):
    if type(value) is not dict or set(value)!={'channels','values_bits'}: raise ValueError('port shape')
    integer(value['channels'],0,16)
    if type(value['values_bits']) is not list or len(value['values_bits'])!=16: raise ValueError('port slots')
    for v in value['values_bits']: u32(v)
def trace_check(inputs,trace):
    trace_rows=rows(trace);observed=0; dft_rates=set();dft_profiles=set()
    for c in rows(inputs):
        registry={f'out{p}':('module',p,'sink',p) for p in range(7)
                  if c['initial']['output_mask']&(1<<p)}
        frame=normal=uniform=0;last_normal=last_uniform=0;pink_frame=-1;gray_frame=0
        normal_tape=c['initial']['normal_bits'];uniform_tape=c['initial']['uniform_bits']
        bypassed=False;gray_completed=False;previous=None;incarnation=0
        for index,event in [(-1,{'op':'initial'})]+list(enumerate(c['events'])):
            op=event['op'];draw_events=[]
            if op=='connect':registry[event['id']]=(event['source'],event['port'],event['destination'],event['input'])
            if op=='disconnect':del registry[event['id']]
            if op=='recreate':
                registry.clear();pink_frame=-1;gray_frame=0;gray_completed=False;bypassed=False
                incarnation+=1
            if op=='bypass':bypassed=event['value']
            counts=[0]*7; sink_counts=[0]*7
            for _,port,_,inp in registry.values(): counts[port]+=1;sink_counts[inp]+=1
            mask=sum((1<<p) for p,n in enumerate(counts) if n)
            if op=='step':
                for _ in range(event.get('frames',1)):
                    draw_events=[];frame+=1
                    if not bypassed:
                        if mask & 0b0101101: # white/red/violet/gray: bits 0,2,3,5
                            b=normal_tape[normal%TAPE_LENGTH];normal+=1;last_normal=b
                            draw_events.append({'kind':'normal','bits':b})
                        if mask & ((1<<1)|(1<<4)):
                            next_frame=(pink_frame+1)&255
                            count=((pink_frame^next_frame)&255).bit_count()
                            pink_frame=next_frame
                            for _ in range(count):
                                b=uniform_tape[uniform%TAPE_LENGTH];uniform+=1;last_uniform=b
                                draw_events.append({'kind':'pink','bits':b})
                        if mask&(1<<5):
                            gray_frame=(gray_frame+1)%1024
                            if gray_frame==0:gray_completed=True
                        if mask&(1<<6):
                            b=uniform_tape[uniform%TAPE_LENGTH];uniform+=1;last_uniform=b
                            draw_events.append({'kind':'black','bits':b})
            try: row=next(trace_rows)
            except StopIteration: raise ValueError(f'missing row {c["id"]}:{index}')
            observed+=1
            if type(row) is not dict or set(row)!=set(FIELDS): raise ValueError(f'snapshot fields {c["id"]}:{index}')
            scalar={'case':c['id'],'event':index,'op':op,'frame':frame,
                    'module_incarnation':incarnation,'sample_rate':c['sample_rate'],
                    'sample_time_bits':sample_time_bits(c['sample_rate']),'bypassed':bypassed,
                    'cable_registry':expected_registry(registry),'output_cables':counts,
                    'sink_cables':sink_counts,'output_consumer_mask':mask,
                    'normal_draws':normal,'uniform_draws':uniform,
                    'last_normal_bits':last_normal,'last_uniform_bits':last_uniform,
                    'draw_events':draw_events,'pink_frame':pink_frame,'gray_frame':gray_frame,
                    'black_divider_clock':0,'black_divider_division':1,'light_bits':[]}
            for key,value in scalar.items():
                if row[key]!=value: raise ValueError(f'{c["id"]}:{index} {key}: {row[key]!r} != {value!r}')
            for key in ('outputs','sink_inputs','sink_seen'):
                if type(row[key]) is not list or len(row[key])!=7: raise ValueError(key)
                for port in row[key]:port_shape(port)
            for key,n in (('pink_values_bits',8),('gray_input_bits',1024),('gray_output_bits',1024)):
                if type(row[key]) is not list or len(row[key])!=n: raise ValueError(key)
                for v in row[key]:u32(v)
            for key in ('red_x_bits','red_y_bits','last_white_bits','last_pink_bits'):u32(row[key])
            if index == -1 or op=='recreate':
                for key in ('red_x_bits','red_y_bits','last_white_bits','last_pink_bits'):
                    if row[key]!=0: raise ValueError(f'initial {key}')
                if any(row['pink_values_bits']) or any(row['gray_input_bits']) or any(row['gray_output_bits']):
                    raise ValueError('initial state buffers')
            if previous is not None and op!='recreate':
                frozen=(op!='step' or bypassed)
                if frozen:
                    for key in RUNTIME:
                        if row[key]!=previous[key]: raise ValueError(f'{c["id"]}:{index} {key} changed without processing')
                elif op=='step':
                    for family,keys in (((1<<2),('red_x_bits','red_y_bits')),
                                        ((1<<3),('last_white_bits',)),
                                        (((1<<1)|(1<<4)),('pink_frame','pink_values_bits')),
                                        ((1<<4),('last_pink_bits',)),
                                        ((1<<5),('gray_frame','gray_input_bits','gray_output_bits'))):
                        if not mask&family:
                            for key in keys:
                                if row[key]!=previous[key]: raise ValueError(f'{c["id"]}:{index} frozen {key}')
            if previous is not None:
                if op!='step' and row['sink_seen']!=previous['sink_seen']:
                    raise ValueError(f'{c["id"]}:{index} sink_seen changed without processing')
                if op=='step' and event.get('frames',1)==1 and row['sink_seen']!=previous['sink_inputs']:
                    raise ValueError(f'{c["id"]}:{index} process-time sink tap lacks one-frame delay')
            if not gray_completed and any(row['gray_output_bits']): raise ValueError('gray pre-block output not zero')
            if not gray_completed and mask&(1<<5) and op=='step' and not bypassed:
                if row['outputs'][5]['values_bits'][0]!=0: raise ValueError('gray startup voltage')
            if op=='step' and not bypassed and mask&(1<<6) and not mask&((1<<1)|(1<<4)) and uniform==1:
                if row['outputs'][6]['values_bits'][0]!=0xc0a00000: raise ValueError('black -5V hand anchor')
            if op=='step' and not bypassed and mask&(1<<0) and normal==1:
                if row['outputs'][0]['values_bits'][0]!=0: raise ValueError('white zero hand anchor')
            if (c['id'].endswith('masks/127') and frame==1 and op=='step'):
                # First normal is +0. The Voss first call replaces all eight
                # registers; their exact dyadic sum is -1/2. Black consumes
                # uniform draw nine (=1/16) after those eight pink draws.
                first=(0x00000000,0xc00aa5fd,0x00000000,0x00000000,
                       0xc044aa15,0x00000000,0xc08c0000)
                for p,expected in enumerate(first):
                    if row['outputs'][p]['values_bits'][0]!=expected:
                        raise ValueError(f'first all-colors hand anchor output {p}')
            # One independent mathematical DFT anchor per rate: first completed
            # gray-solo block, five selected outputBuffer positions.
            if (c['id'].endswith('gray-solo-boundaries') and gray_completed and
                    gray_frame==0 and c['sample_rate'] not in dft_rates):
                dft_gray(row,c['sample_rate']);dft_rates.add(c['sample_rate'])
            if (c['id'].split('/')[1:2]==['gray-profile'] and gray_completed and
                    gray_frame==0 and (c['sample_rate'],c['initial']['normal_profile']) not in dft_profiles):
                if row['gray_input_bits']!=normal_tape[:1024]:
                    raise ValueError('diagnostic gray input differs from controlled stream')
                dft_gray(row,c['sample_rate'])
                dft_profiles.add((c['sample_rate'],c['initial']['normal_profile']))
            previous=row
    try:extra=next(trace_rows)
    except StopIteration:extra=None
    if extra is not None: raise ValueError('extra trace rows')
    if dft_rates!=set(RATES) or len(dft_profiles)!=19:
        raise ValueError('independent DFT checks per rate/profile required')
    return {'passed':True,'rows':observed,'dft_checks':len(dft_rates)+len(dft_profiles),'discarded_snapshots':0}
def near_bits(a,b,absolute):
    x=f32(a);y=f32(b)
    if not (math.isfinite(x) and math.isfinite(y)):return False
    return abs(x-y)<=absolute+GRAY_REL*max(abs(x),abs(y))
def dft_gray(row,rate):
    # Canonical f64 DFT of actual controlled white input, using the exact
    # source's HALF physical-bin frequency mapping, zero DC/Nyquist and N=1024.
    samples=[f32(v) for v in row['gray_input_bits']]
    spec=[];N=1024
    for k in range(1,N//2):
        re=im=0.0
        for n,x in enumerate(samples):
            angle=-2*math.pi*k*n/N
            re+=x*math.cos(angle);im+=x*math.sin(angle)
        freq=rate*k/(2*N)
        if 80 <= freq <= 20000:
            f2=freq*freq
            amp=((424.36+f2)*math.sqrt((11599.3+f2)*(544496+f2))*(148693636+f2))/(148693636*f2*f2)
        else:amp=0.0
        spec.append((re*amp,im*amp))
    for n in (0,1,255,511,1023):
        expected=2/N*sum(re*math.cos(2*math.pi*k*n/N)-im*math.sin(2*math.pi*k*n/N)
                         for k,(re,im) in enumerate(spec,1))
        actual=f32(row['gray_output_bits'][n])
        if not math.isfinite(actual) or abs(actual-expected)>GRAY_RAW_ABS+GRAY_REL*max(abs(actual),abs(expected)):
            raise ValueError(f'f64 DFT gray anchor rate={rate} n={n} got={actual} expected={expected}')
def paired_check(inputs,sdk,production):
    cases=rows(inputs);left=rows(sdk);right=rows(production);count=0
    for c in cases:
        registry={f'out{p}':('module',p,'sink',p) for p in range(7)
                  if c['initial']['output_mask']&(1<<p)}
        gray_steps=0;bypassed=False;gray_output_numeric=False
        sink_numeric=[False]*7;seen_numeric=[False]*7
        for index,event in [(-1,{'op':'initial'})]+list(enumerate(c['events'])):
            op=event['op']
            if op=='connect':registry[event['id']]=(event['source'],event['port'],event['destination'],event['input'])
            if op=='disconnect':
                removed=registry.pop(event['id'])
                if not any(inp==removed[3] for _,_,_,inp in registry.values()):
                    sink_numeric[removed[3]]=False # Rack clears all 16 sink slots.
            if op=='recreate':
                registry.clear();gray_steps=0;bypassed=False;gray_output_numeric=False
                sink_numeric=[False]*7 # All incident sink cables removed.
            if op=='bypass':
                if event['value']!=bypassed and any(port==5 for _,port,_,_ in registry.values()):
                    gray_output_numeric=False # Engine::bypassModule setChannels(0).
                bypassed=event['value']
            if op=='step':
                for _ in range(event.get('frames',1)):
                    seen_numeric=sink_numeric.copy() # Sink::process sees prior cable propagation.
                    if not bypassed and any(port==5 for _,port,_,_ in registry.values()):
                        gray_steps+=1
                        gray_output_numeric=gray_steps>=1024
                    sink_numeric=[gray_output_numeric and any(port==5 and inp==p
                                  for _,port,_,inp in registry.values()) for p in range(7)]
            try:a=next(left);b=next(right)
            except StopIteration:raise ValueError('paired trace missing rows')
            count+=1
            if set(a)!=set(b):raise ValueError(f'paired fields {c["id"]}:{index}')
            gray_warm=gray_steps>=1024
            for key in FIELDS:
                if key=='gray_output_bits' and gray_warm:
                    if len(a[key])!=1024 or len(b[key])!=1024:raise ValueError('gray buffer shape')
                    for i,(x,y) in enumerate(zip(a[key],b[key])):
                        if not near_bits(x,y,GRAY_RAW_ABS):raise ValueError(f'{c["id"]}:{index} gray_output_bits[{i}]')
                elif key in ('outputs','sink_inputs','sink_seen'):
                    if len(a[key])!=7 or len(b[key])!=7:raise ValueError(f'{key} port count')
                    for p,(ap,bp) in enumerate(zip(a[key],b[key])):
                        if ap['channels']!=bp['channels']:raise ValueError(f'{key}[{p}] channels')
                        if len(ap['values_bits'])!=16 or len(bp['values_bits'])!=16:
                            raise ValueError(f'{key}[{p}] lane count')
                        for lane,(x,y) in enumerate(zip(ap['values_bits'],bp['values_bits'])):
                            allowed=lane==0 and ((key=='outputs' and p==5 and gray_output_numeric) or
                                                 (key=='sink_inputs' and sink_numeric[p]) or
                                                 (key=='sink_seen' and seen_numeric[p]))
                            if (allowed and not near_bits(x,y,GRAY_VOLT_ABS)) or (not allowed and x!=y):
                                raise ValueError(f'{c["id"]}:{index} {key}[{p}][{lane}]')
                elif a[key]!=b[key]:raise ValueError(f'{c["id"]}:{index} {key}')
    for stream in (left,right):
        try:extra=next(stream)
        except StopIteration:extra=None
        if extra is not None:raise ValueError('paired extra rows')
    return {'passed':True,'paired_rows':count,'discarded_snapshots':0}
def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--inputs',type=Path,required=True)
    p.add_argument('--sdk-trace',type=Path)
    p.add_argument('--paired',type=Path)
    p.add_argument('--fixtures-only',action='store_true')
    args=p.parse_args()
    report=check_inputs(args.inputs)
    if not args.fixtures_only:
        if args.sdk_trace is None:raise ValueError('--sdk-trace required')
        report['sdk']=trace_check(args.inputs,args.sdk_trace)
        if args.paired:
            report['production']=trace_check(args.inputs,args.paired)
            report['paired']=paired_check(args.inputs,args.sdk_trace,args.paired)
    print(json.dumps(report,sort_keys=True))
if __name__=='__main__':
    try:main()
    except Exception as error:
        print(f'Stock Noise reference validation failed: {error}',file=sys.stderr)
        raise SystemExit(1)
