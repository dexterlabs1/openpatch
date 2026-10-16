#!/usr/bin/env python3
"""Independent input/topology/state anchors and exact whole-trace gate.

No production imports and no second VCMixer DSP implementation. Retains every
constructor and event. The finite-input domain permits internally nonfinite bits.
"""
import argparse
import gzip
import hashlib
import json
import math
import re
from pathlib import Path
import sys
from fixtures import safety_records

HERE=Path(__file__).resolve().parent
PORTS={'source_outputs':9,'inputs':9,'outputs':5,'sink_inputs':5}
COUNTS={'input_cables':9,'output_cables':5,'source_cables':9,'sink_cables':5}
ONE=0x3f800000

def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda:stream.read(1024*1024),b''): h.update(chunk)
    return h.hexdigest()
def rows(path):
    opener=gzip.open if str(path).endswith('.gz') else open
    def unique(pairs):
        result={}
        for key,value in pairs:
            if key in result: raise ValueError('duplicate JSON key: '+key)
            result[key]=value
        return result
    with opener(path,'rt') as stream:
        for line in stream:
            if not line.strip(): raise ValueError('blank JSONL row')
            yield json.loads(line,object_pairs_hook=unique,parse_constant=lambda x:(_ for _ in ()).throw(ValueError('nonfinite JSON token')))
def equal(a,b,path=''):
    if type(a) is not type(b): raise ValueError(f'{path}: types differ {type(a).__name__} vs {type(b).__name__}')
    if isinstance(a,dict):
        if a.keys()!=b.keys(): raise ValueError(f'{path}: keys differ {set(a)^set(b)}')
        for key in a: equal(a[key],b[key],f'{path}.{key}')
    elif isinstance(a,(list,tuple)):
        if len(a)!=len(b): raise ValueError(f'{path}: lengths differ')
        for i,(x,y) in enumerate(zip(a,b)): equal(x,y,f'{path}.{i}')
    elif a!=b: raise ValueError(f'{path}: actual={a!r}, expected={b!r}')
def at(row,path):
    for part in path.split('.'): row=row[int(part)] if isinstance(row,list) else row[part]
    return row
def bounded(v,lo,hi):
    if type(v) is not int or not lo<=v<=hi: raise ValueError(f'integer outside [{lo},{hi}]: {v!r}')
def raw(v): bounded(v,0,0xffffffff)
def finite(v):
    raw(v)
    if v&0x7f800000==0x7f800000: raise ValueError('nonfinite ordinary input/control')
def source_values(v):
    if type(v) is not list or len(v)>16: raise ValueError('source controls shape')
    for x in v: finite(x)
def parameters(v):
    if type(v) is not list or len(v)!=5: raise ValueError('five raw parameters required')
    for x in v: finite(x)
def named(v):
    if type(v) is not str or not v: raise ValueError('nonempty string required')
def json_domain(v):
    if v is None or type(v) is bool: return
    if type(v) is int:
        if not -(1<<63)<=v<(1<<63): raise ValueError('Jansson int64 overflow')
    elif type(v) is float:
        if not math.isfinite(v): raise ValueError('nonfinite/overflow JSON real')
    elif type(v) is str:
        if '\x00' in v: raise ValueError('Jansson default JSON string cannot contain NUL')
    elif type(v) is list:
        for x in v: json_domain(x)
    elif type(v) is dict:
        for k,x in v.items():
            if type(k) is not str: raise ValueError('JSON string key')
            json_domain(k);json_domain(x)
    else: raise ValueError('unsupported JSON-domain type')
def sample_time_bits(rate):
    """Integer nearest-even encoding of exact 1/rate, no host FP arithmetic."""
    exponent=1-rate.bit_length() if rate&(rate-1)==0 else -rate.bit_length()
    q,r=divmod(1<<(23-exponent),rate)
    q+=int(2*r>rate or (2*r==rate and q&1))
    return ((exponent+127)<<23)|(q-(1<<23))
def load_flags(state,value):
    if isinstance(value,dict):
        for key,target in (('chExp','ch_exp'),('mixExp','mix_exp')):
            if key in value: state[target]=value[key] is True

def anchor_shape(path,value):
    """Whitelist independent sparse anchor destinations before any execution."""
    m=re.fullmatch(r'(source_outputs|inputs|outputs|sink_inputs)\.(\d+)\.(channels|values_bits(?:\.\d+)?)',path)
    if m:
        key,p,field=m.groups();bounded(int(p),0,PORTS[key]-1)
        if field=='channels': bounded(value,0,16)
        elif field=='values_bits':
            if type(value) is not list or len(value)!=16: raise ValueError('anchor raw port shape')
            for v in value: raw(v)
        else: bounded(int(field.split('.')[-1]),0,15);raw(value)
        return
    m=re.fullmatch(r'meters\.([0-3])\.(v_bits|mode|lambda_bits)',path)
    if m:
        raw(value)
        if m[2]=='mode': equal(value,0,'anchor PEAK')
        elif m[2]=='lambda_bits': equal(value,0x41f00000,'anchor lambda')
        return
    if path=='light_bits':
        if type(value) is not list or len(value)!=4: raise ValueError('anchor four lights')
        for v in value: raw(v)
        return
    m=re.fullmatch(r'light_bits\.([0-3])',path)
    if m: raw(value);return
    if path=='sample_rate': bounded(value,44100,96000);return
    if path=='sample_time_bits': finite(value);return
    if path=='divider_clock': bounded(value,0,511);return
    if path=='divider_division': equal(value,512,'anchor divider');return
    if path in ('bypassed','ch_exp','mix_exp'):
        if type(value) is not bool: raise ValueError('anchor Boolean')
        return
    m=re.fullmatch(r'parameter_bits\.([0-4])',path)
    if m: finite(value);return
    raise ValueError('undeclared anchor destination: '+path)

def event_shape(e):
    fields={'step':set(),'reset':set(),'save':set(),'recreate':set(),
            'source':{'port','values_bits'},'parameter':{'port','value_bits'},
            'data_load':{'value'},'bypass':{'value'},
            'connect':{'id','source','port','destination','input'},'disconnect':{'id'},
            'adopt':{'parameter_bits','data'}}
    if type(e) is not dict or e.get('op') not in fields: raise ValueError('unknown event')
    want=fields[e['op']]|{'op'}
    if 'expect' in e:
        if type(e['expect']) is not dict: raise ValueError('anchor map required')
        for path,value in e['expect'].items(): named(path);anchor_shape(path,value)
        want.add('expect')
    equal(set(e),want,'event fields'); op=e['op']
    if op=='source': bounded(e['port'],0,8); source_values(e['values_bits'])
    elif op=='parameter': bounded(e['port'],0,4); finite(e['value_bits'])
    elif op=='adopt': parameters(e['parameter_bits']);json_domain(e['data'])
    elif op=='data_load': json_domain(e['value'])
    elif op=='bypass':
        if type(e['value']) is not bool: raise ValueError('Boolean bypass required')
    elif op=='connect':
        named(e['id']); route=(e['source'],e['destination'])
        if route not in (('source','module'),('module','sink')): raise ValueError('undeclared feedback/role topology')
        bounded(e['port'],0,8 if route[0]=='source' else 4)
        bounded(e['input'],0,8 if route[1]=='module' else 4)
    elif op=='disconnect': named(e['id'])
def initial_cables(init):
    cables={f'in{p}':('source',p,'module',p) for p in range(9) if init['input_mask']&(1<<p)}
    cables.update({f'out{p}':('module',p,'sink',p) for p in range(5) if init['output_mask']&(1<<p)})
    return cables
def cable_update(cables,e):
    op=e['op']
    if op=='connect':
        if e['id'] in cables: raise ValueError('duplicate cable identity')
        cables[e['id']]=(e['source'],e['port'],e['destination'],e['input'])
    elif op=='disconnect': del cables[e['id']]
    elif op=='recreate':
        for name in list(cables):
            if cables[name][0]=='module' or cables[name][2]=='module': del cables[name]
def control_update(s,e):
    op=e['op']
    if op=='parameter': s['parameter_bits'][e['port']]=e['value_bits']
    elif op=='data_load': load_flags(s,e['value'])
    elif op=='adopt': s['parameter_bits']=e['parameter_bits'].copy();load_flags(s,e['data'])
    elif op=='reset': s.update(parameter_bits=[ONE]*5,ch_exp=False,mix_exp=False)
    elif op=='bypass': s['bypassed']=e['value']
    elif op=='recreate': s.update(parameter_bits=[ONE]*5,ch_exp=False,mix_exp=False,bypassed=False)
def initial_state(init):
    state={'parameter_bits':init['parameter_bits'].copy(),'ch_exp':False,'mix_exp':False,'bypassed':False}
    load_flags(state,init['data']);return state

def validate_fixtures(path,declaration):
    ids=set();families={r:{} for r in declaration['sample_rates']};snapshots=events=anchors=finite_controls=0
    for c in rows(path):
        equal(set(c),{'id','sample_rate','initial','events'},'case fields');json_domain(c);named(c['id'])
        if c['id'] in ids: raise ValueError('duplicate case')
        ids.add(c['id']);rate=c['sample_rate']
        if type(rate) is not int or rate not in families or not c['id'].startswith(str(rate)+'/'): raise ValueError('undeclared rate')
        family=c['id'].split('/')[1];family=family if family in declaration['fresh_families_per_rate'] else 'explicit'
        families[rate][family]=families[rate].get(family,0)+1; init=c['initial']
        equal(set(init),{'sources','input_mask','output_mask','parameter_bits','data'},'initial fields')
        if type(init['sources']) is not list or len(init['sources'])!=9: raise ValueError('nine source buses required')
        for v in init['sources']: source_values(v);finite_controls+=len(v)
        bounded(init['input_mask'],0,511);bounded(init['output_mask'],0,31);parameters(init['parameter_bits']);finite_controls+=5;json_domain(init['data'])
        if type(c['events']) is not list: raise ValueError('event list required')
        cables=initial_cables(init);state=initial_state(init)
        for e in c['events']:
            event_shape(e);control_update(state,e);cable_update(cables,e);anchors+=len(e.get('expect',{}))
            if e['op']=='source': finite_controls+=len(e['values_bits'])
            elif e['op']=='parameter': finite_controls+=1
            elif e['op']=='adopt': finite_controls+=5
        if family!='explicit': equal([e['op'] for e in c['events']],['step','step'],'fresh two steps')
        events+=len(c['events']);snapshots+=1+len(c['events'])
    for rate,found in families.items(): equal(found,{**declaration['fresh_families_per_rate'],'explicit':declaration['explicit_cases_per_rate']},f'families/{rate}')
    equal(len(ids),declaration['case_count'],'case cardinality');equal(events,declaration['event_count_excluding_initial'],'event cardinality');equal(snapshots,declaration['snapshot_count'],'snapshot cardinality')
    return {'cases':len(ids),'snapshots':snapshots,'events':events,'hand_anchors':anchors,'finite_controls_checked_before_execution':finite_controls,'families':families}

def shape(row,d):
    equal(set(row),set(d['snapshot_fields']),'snapshot fields');json_domain(row)
    if type(row['sample_rate']) is not int or row['sample_rate'] not in d['sample_rates']: raise ValueError('actual Engine sample rate')
    finite(row['sample_time_bits'])
    for key,n in PORTS.items():
        if type(row[key]) is not list or len(row[key])!=n: raise ValueError(key+' port shape')
        for p in row[key]:
            equal(set(p),{'channels','values_bits'},'raw port fields');bounded(p['channels'],0,16)
            if type(p['values_bits']) is not list or len(p['values_bits'])!=16: raise ValueError('all sixteen raw slots required')
            for v in p['values_bits']: raw(v)
    for key,n in COUNTS.items():
        if type(row[key]) is not list or len(row[key])!=n: raise ValueError(key+' count shape')
        for v in row[key]: bounded(v,0,1024)
    parameters(row['parameter_bits'])
    for key in ('bypassed','ch_exp','mix_exp'):
        if type(row[key]) is not bool: raise ValueError(key+' Boolean required')
    equal(row['saved_data'],{'chExp':row['ch_exp'],'mixExp':row['mix_exp']},'actual serialized flags')
    if type(row['source_config']) is not list or len(row['source_config'])!=9: raise ValueError('source config shape')
    for v in row['source_config']: source_values(v)
    bounded(row['output_consumer_mask'],0,31);bounded(row['divider_clock'],0,511);equal(row['divider_division'],512,'actual divider division')
    if type(row['meters']) is not list or len(row['meters'])!=4: raise ValueError('four actual meters required')
    for meter in row['meters']:
        equal(set(meter),{'v_bits','mode','lambda_bits'},'meter fields');raw(meter['v_bits']);equal(meter['mode'],0,'actual PEAK mode');equal(meter['lambda_bits'],0x41f00000,'actual lambda30')
    if type(row['light_bits']) is not list or len(row['light_bits'])!=4: raise ValueError('four actual lights required')
    for v in row['light_bits']: raw(v)

def validate(trace_path,input_path,d):
    trace=iter(rows(trace_path));total=anchors=invariants=0
    for c in rows(input_path):
        init=c['initial'];state=initial_state(init);configs=[v.copy() for v in init['sources']];cables=initial_cables(init);frame=clock=0;previous=None
        for index,e in enumerate([{'op':'initial'}]+c['events']):
            context=f'{c["id"]}/{index-1}';op=e['op']
            try: row=next(trace)
            except StopIteration: raise ValueError('missing '+context) from None
            shape(row,d)
            control_update(state,e);cable_update(cables,e)
            if op=='step':
                frame+=1
                if not state['bypassed']: clock=(clock+1)%512
            elif op=='source': configs[e['port']]=e['values_bits']
            elif op=='recreate': clock=0
            counts={key:[0]*n for key,n in COUNTS.items()};registry=[]
            for name,(src,p,dst,i) in sorted(cables.items()):
                counts['source_cables' if src=='source' else 'output_cables'][p]+=1
                counts['input_cables' if dst=='module' else 'sink_cables'][i]+=1
                registry.append({'id':name,'source':src,'port':p,'destination':dst,'input':i})
            expected={'case':c['id'],'event':index-1,'op':op,'frame':frame,'sample_rate':c['sample_rate'],'sample_time_bits':sample_time_bits(c['sample_rate']),**state,**counts,'source_config':configs,'cable_registry':registry,'output_consumer_mask':sum((1<<p) for p,n in enumerate(counts['output_cables']) if n),'divider_clock':clock}
            for key,want in expected.items(): equal(row[key],want,context+'/'+key);invariants+=1
            if op in ('initial','recreate'):
                equal([m['v_bits'] for m in row['meters']],[0]*4,context+'/fresh meters');equal(row['light_bits'],[0]*4,context+'/fresh lights')
            if op=='initial':
                for key,connections in (('source_outputs','source_cables'),('inputs','input_cables'),('outputs','output_cables'),('sink_inputs','sink_cables')):
                    for p,n in zip(row[key],counts[connections]):
                        equal(p['values_bits'],[0]*16,context+'/constructor no DSP writes');equal(p['channels'],int(n>0),context+'/constructor actual connected count')
            if previous is not None:
                if op not in ('step','recreate') or (op=='step' and state['bypassed']):
                    equal(row['meters'],previous['meters'],context+'/runtime meters preserved');equal(row['light_bits'],previous['light_bits'],context+'/runtime lights preserved')
                if op in ('source','parameter','data_load','save','reset','adopt'):
                    for key in PORTS: equal(row[key],previous[key],context+'/'+op+' no raw DSP writes')
                if op=='step' and not state['bypassed']:
                    # Source-defined width invariant reads the exact previous actual
                    # module inputs, matching Engine processing-before-transfer.
                    width=max(1,*[previous['inputs'][p]['channels'] for p in range(1,5)])
                    for p,n in enumerate(counts['output_cables']): equal(row['outputs'][p]['channels'],width if n else 0,context+'/signal-only width')
                    if clock!=0: equal(row['light_bits'],previous['light_bits'],context+'/lights frozen between divider triggers')
            for path,want in e.get('expect',{}).items(): equal(at(row,path),want,context+'/anchor/'+path);anchors+=1
            previous=row;total+=1
    try: next(trace)
    except StopIteration: return total,anchors,invariants
    raise ValueError('trailing snapshot')

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--inputs',type=Path,required=True);p.add_argument('--safety-inputs',type=Path,required=True);p.add_argument('--sdk-trace',type=Path);p.add_argument('--paired',type=Path);p.add_argument('--fixtures-only',action='store_true');a=p.parse_args()
    d=json.loads((HERE/'acceptance.json').read_text());equal(list(rows(a.safety_inputs)),list(safety_records()),'separate safety records');result=validate_fixtures(a.inputs,d)
    if not a.fixtures_only:
        if not a.sdk_trace: p.error('--sdk-trace required')
        counts=validate(a.sdk_trace,a.inputs,d);equal(counts[:2],(result['snapshots'],result['hand_anchors']),'trace cardinality');result['invariants_checked']=counts[2]
        if a.paired:
            equal(validate(a.paired,a.inputs,d),counts,'paired cardinality')
            for i,(actual,reference) in enumerate(zip(rows(a.paired),rows(a.sdk_trace))): equal(actual,reference,f'full snapshot {i}')
    result.update(passed=True,discarded_snapshots=0,inputs_sha256=digest(a.inputs),safety_inputs_sha256=digest(a.safety_inputs),sdk_trace_sha256=digest(a.sdk_trace) if a.sdk_trace else None,paired_trace_sha256=digest(a.paired) if a.paired else None,scope='input-only declaration preflight' if a.fixtures_only else ('complete exact paired host snapshots' if a.paired else 'SDK source+host anchors; production unqualified'))
    print(json.dumps(result,sort_keys=True))
if __name__=='__main__':
    try: main()
    except Exception as error:
        print(json.dumps({'passed':False,'error':{'type':type(error).__name__,'message':str(error)}},sort_keys=True),file=sys.stderr);sys.exit(1)
