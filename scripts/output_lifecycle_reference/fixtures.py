#!/usr/bin/env python3
"""Independent input words for Rack producer-output lifecycle; no DSP or table mirror."""
import argparse
import hashlib
import itertools
import json
import math
from pathlib import Path
import struct
HERE=Path(__file__).resolve().parent
COUNTS=(1,3,7,16)

def f32(x):return struct.unpack('<f',struct.pack('<f',x))[0]
def bits(x):return struct.unpack('<I',struct.pack('<f',x))[0]
def digest(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def C(port,slot=0):return port*2+slot
def D(port,slot=0):return 8+port*2+slot
B,B0,B1=16,17,18

def seed(profile):
    result=[]
    for port in range(4):
        for lane in range(16):
            value=(port*16+lane+1)/8
            if (port+lane)%2:value=-value
            if profile==1 and lane%4==0:value=-0.
            if profile==1 and lane%4==1:value=0.
            result.append(value)
    return result

def fields(mask,counts,profile,word):return [mask,*counts,*seed(profile),len(word),*word]
def word_name(word):return ''.join(word) or 'empty'
def entry(name,group,mask,counts,profile,word,anchor=None):
    return {'name':name,'group':group,'profile':profile,'fields':fields(mask,counts,profile,word),'anchor':anchor}

def supplemental_scripts():
    # This finite script inventory is chosen before any execution. It exercises
    # physical 0<->positive edges, not merely changes to one cable's identity.
    scripts=[('empty',[]),('bypass',[B]),('double_bypass',[B,B]),('triple_bypass',[B,B,B]),
             ('same_false',[B0,B0]),('same_true',[B1,B1]),('explicit_roundtrip',[B1,B1,B0,B0]),
             ('connect_slot0_all',[C(p) for p in range(4)]),
             ('disconnect_slot0_all',[D(p) for p in range(4)]),
             ('connect_slot1_all',[C(p,1) for p in range(4)]),
             ('disconnect_slot1_all',[D(p,1) for p in range(4)]),
             ('connect_all_then_bypass',[C(p,s) for p in range(4) for s in range(2)]+[B]),
             ('bypass_disconnect_all_reconnect',[B]+[D(p,s) for p in range(4) for s in range(2)]+[C(p,1) for p in range(4)]),
             ('disconnect_all_bypass_connect_all',[D(p,s) for p in range(4) for s in range(2)]+[B]+[C(p,s) for p in range(4) for s in range(2)]),
             ('all_second_handover_bypass',[C(p,1) for p in range(4)]+[D(p) for p in range(4)]+[B]),
             ('all_last_disconnect_bypass',[C(p,1) for p in range(4)]+[D(p) for p in range(4)]+[D(p,1) for p in range(4)]+[B])]
    for p in range(4):
        for name,word in [
            ('second',[C(p,1)]),('second_noop',[C(p,1),C(p,1)]),
            ('handover',[C(p,1),D(p)]),('handover_back',[C(p,1),D(p),C(p),D(p,1)]),
            ('last_then_second',[D(p),C(p,1)]),('last_bypass_second',[D(p),B,C(p,1)]),
            ('second_bypass_first_remove',[C(p,1),B,D(p)]),('bypass_last_second',[B,D(p),C(p,1)]),
            ('last_second_bypass',[D(p),C(p,1),B]),('full_disconnect_reconnect',[C(p,1),D(p),D(p,1),C(p)]),
            ('handover_bypass_back',[C(p,1),D(p),B,C(p),D(p,1)]),
            ('false_then_true',[C(p,1),B0,D(p),B1]),
            ('disconnected_bypass_both',[D(p),D(p,1),B,C(p,1),C(p)]),
            ('second_remove_last_reconnect',[C(p,1),D(p,1),D(p),C(p)]),
            ('second_two_edges',[C(p,1),B,D(p,1),B]),
            ('bypass_reassert_after_connect',[D(p),B1,C(p,1),B1]),
            ('unbypass_after_connect',[D(p),B1,C(p,1),B0])]:scripts.append((f'p{p}_{name}',word))
    for p in range(4):
        for q in range(4):
            if p==q:continue
            scripts.extend([
                (f'cross_{p}_{q}_disconnect_before_bypass',[D(p),B,C(q,1),C(p)]),
                (f'cross_{p}_{q}_other_connect_before_bypass',[C(q,1),D(p),B,C(p)]),
                (f'cross_{p}_{q}_handover',[C(p,1),D(q),B,D(p),C(q)])])
    assert len({name for name,_ in scripts})==len(scripts)
    return scripts

def hand_anchors():
    # Literal, hand-derived final counts and cleared prefixes; this is not a
    # generic operation evaluator. Remaining raw bits must equal the seed.
    return [
        ('disconnect_then_bypass',1,[16,0,0,0],[D(0),B],[0,0,0,0],[0,0,0,0],1,0),
        ('bypass_then_disconnect',1,[16,0,0,0],[B,D(0)],[0,0,0,0],[16,0,0,0],1,0),
        ('reconnect_lowers_clear_extent',1,[16,0,0,0],[D(0),C(0),B],[1,0,0,0],[1,0,0,0],1,1),
        ('second_keeps_old_clear_extent',1,[16,0,0,0],[C(0,1),D(0),B],[1,0,0,0],[16,0,0,0],1,2),
        ('same_false_preserves',1,[16,0,0,0],[B0,B0],[16,0,0,0],[0,0,0,0],0,1),
        ('same_true_after_disconnected_connect',0,[0,0,0,0],[B1,C(0),B1],[1,0,0,0],[0,0,0,0],1,1),
        ('unbypass_is_clearing_edge',0,[0,0,0,0],[B1,C(0),B0],[1,0,0,0],[1,0,0,0],0,1),
        ('initially_empty_connect_bypass',0,[0,0,0,0],[C(0),B],[1,0,0,0],[1,0,0,0],1,1),
        ('two_initial_consumers_nonlast_remove',3,[7,0,0,0],[D(0)],[7,0,0,0],[0,0,0,0],0,2),
        ('two_initial_consumers_clear',3,[7,0,0,0],[D(0),B],[1,0,0,0],[7,0,0,0],1,2),
        ('idempotent_cables_preserve',1,[7,0,0,0],[C(0),C(0),D(0,1),D(0,1)],[7,0,0,0],[0,0,0,0],0,1),
        ('heterogeneous_disconnected_at_bypass',85,[1,3,7,16],[D(3),B,C(3)],[1,1,1,1],[1,3,7,0],1,85),
        ('heterogeneous_second_at_bypass',85,[1,3,7,16],[C(3,1),D(3),B],[1,1,1,1],[1,3,7,16],1,149),
        ('toggle_twice_is_not_identity',85,[1,3,7,16],[B,B],[1,1,1,1],[1,3,7,16],0,85)]

def cases(a=None):
    index=0
    for port in range(4):
        for count in range(17):
            counts=[0]*4;counts[port]=count;mask=(1<<(port*2)) if count else 0
            group=f'independent_p{port}_count{count:02d}'
            for length in range(8):
                for word in itertools.product('CDB',repeat=length):
                    opcodes=[C(port) if x=='C' else D(port) if x=='D' else B for x in word]
                    yield entry(f'{group}/{word_name(word)}',group,mask,counts,index%2,opcodes)
                    index+=1
    for initial in range(16):
        mask=sum(1<<(2*p) for p in range(4) if initial&(1<<p))
        counts=[COUNTS[p] if initial&(1<<p) else 0 for p in range(4)]
        group=f'multi_initial_{initial:01x}'
        for name,word in supplemental_scripts():
            yield entry(f'{group}/{name}',group,mask,counts,index%2,word);index+=1
    for name,mask,counts,word,final_counts,clear,bypassed,final_mask in hand_anchors():
        for profile in range(2):
            anchor={'counts':final_counts,'clear_prefixes':clear,'bypassed':bypassed,'consumer_mask':final_mask}
            yield entry(f'hand/{name}/profile{profile}','hand',mask,counts,profile,word,anchor);index+=1

def rows(a=None):
    for c in cases(a):yield c['name'],0,c['fields']
def row_text(row):return ' '.join(str(x) if isinstance(x,int) else format(x,'.9g') for x in row)+'\n'
def fixture_text(a=None):return ''.join(row_text(row) for _,_,row in rows(a))
def write_fixture(path,a=None):
    h=hashlib.sha256();count=0
    with Path(path).open('wb') as f:
        for _,_,row in rows(a):
            encoded=row_text(row).encode();f.write(encoded);h.update(encoded);count+=1
    return {'cases':count,'input_sha256':h.hexdigest()}
def validate_row(row):
    assert len(row)>=70 and len(row)==70+row[69] and 0<=row[0]<=255
    assert all(isinstance(v,int) and 0<=v<=16 for v in row[1:5])
    assert all((row[1+p]>0)==bool(row[0]&(3<<(p*2))) for p in range(4))
    assert all(math.isfinite(v) and f32(v)==v for v in row[5:69])
    assert all(isinstance(v,int) and 0<=v<=18 for v in row[70:])
    parsed=row_text(row).split();assert len(parsed)==len(row)
    assert all(bits(float(parsed[i]))==bits(row[i]) for i in range(5,69))

def manifest(a):
    h=hashlib.sha256();groups={};total=0;profiles=[0,0];max_operations=0;anchors=[]
    for c in cases(a):
        row=c['fields'];validate_row(row);h.update(row_text(row).encode())
        group=groups.setdefault(c['group'],{'name':c['group'],'start_row':total,'end_row_exclusive':total,'cases':0})
        group['end_row_exclusive']=total+1;group['cases']+=1
        total+=1;profiles[c['profile']]+=1;max_operations=max(max_operations,row[69])
        if c['anchor']:anchors.append({'row':total-1,'name':c['name'],'profile':c['profile'],'anchor':c['anchor']})
    assert total==223040+16*len(supplemental_scripts())+2*len(hand_anchors())
    assert all(group['cases']==3280 for name,group in groups.items() if name.startswith('independent'))
    out={'schema':1,'declared_before_first_comparison':True,'acceptance_sha256':digest(HERE/'acceptance.json'),
         'generator_sha256':digest(HERE/'fixtures.py'),'validator_sha256':digest(HERE/'verify.py'),'readme_sha256':digest(HERE/'README.md'),
         'cases':total,'observed_final_rows':total,'actual_engine_dsp_frames':0,'input_sha256':h.hexdigest(),
         'independent_cases':223040,'independent_words_per_port_count':3280,
         'independent_cases_by_word_length':{str(n):4*17*3**n for n in range(8)},
         'seed_profile_case_counts':profiles,'max_operations':max_operations,'groups':list(groups.values()),
         'supplemental_scripts':[{'name':name,'opcodes':word} for name,word in supplemental_scripts()],
         'hand_anchors':anchors}
    return json.loads(json.dumps(out))
def check_declaration():
    a=json.loads((HERE/'acceptance.json').read_text());generated=manifest(a)
    assert generated==json.loads((HERE/'fixtures.json').read_text()),'Frozen declaration changed'
    return generated

def main():
    p=argparse.ArgumentParser();p.add_argument('--freeze',action='store_true');p.add_argument('--check-fixtures',action='store_true');p.add_argument('--check',action='store_true');args=p.parse_args()
    a=json.loads((HERE/'acceptance.json').read_text());m=manifest(a)
    if args.freeze:
        with (HERE/'fixtures.json').open('x') as f:json.dump(m,f,indent=2,sort_keys=True);f.write('\n')
    else:assert args.check or args.check_fixtures;assert m==json.loads((HERE/'fixtures.json').read_text())
    print(json.dumps({k:v for k,v in m.items() if k not in ('groups','supplemental_scripts','hand_anchors')},indent=2))
    print('fixtures_sha256',digest(HERE/'fixtures.json'))
if __name__=='__main__':main()
