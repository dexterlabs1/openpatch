#!/usr/bin/env python3
"""Independent frozen StockSEQ3 stimuli, no oscillator/sequencer implementation."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
HERE=Path(__file__).resolve().parent

def f32(v):return struct.unpack('<f',struct.pack('<f',v))[0]
def bits(v):return struct.unpack('<I',struct.pack('<f',v))[0]
def raw(v):return struct.unpack('<f',struct.pack('<I',v))[0]
def port(width=0,value=0.):
    vs=list(value) if isinstance(value,(list,tuple)) else [value]*width
    assert len(vs)>=width and 0<=width<=16
    return [width,*map(f32,vs[:width]),*([0.]*(16-width))]
CELLS=[f32(-9.+r*6.+c*.625) for r in range(3) for c in range(8)]
def frame(event=0,tempo=1.,tempo_amount=1.,steps=8.,steps_amount=1.,cells=None,held=0,
          tempo_cv=None,clock=None,run=None,reset=None,steps_cv=None,durable=(-1,-1,-1),rotate=0,bypass=0):
    return [event,*map(f32,[tempo,tempo_amount,steps,steps_amount]),*map(f32,CELLS if cells is None else cells),held,
            *(port() if tempo_cv is None else tempo_cv),*(port() if clock is None else clock),
            *(port() if run is None else run),*(port() if reset is None else reset),
            *(port() if steps_cv is None else steps_cv),*durable,rotate,bypass]
def seg(repeat,**kw):return {'repeat':repeat,'fields':frame(**kw)}
def case(name,segments,tags,anchors=None):return {'name':name,'segments':segments,'tags':tags,'anchors':anchors or []}
def cases(rate,a):
    # Lowest knob complete cycles plus full pulse tails, not startup-only tests.
    for tempo in [-2.,-1.,0.,1.,2.,4.]:
        for passthrough in [0,1]:
            n=math.ceil(1.125*rate/(2**tempo))+math.ceil(rate*.003)+2
            yield case(f'internal_t{tempo}_pass{passthrough}',
                [seg(1,event=1,tempo=tempo),seg(1,event=4,tempo=tempo,durable=(1,255,passthrough)),seg(n,tempo=tempo)],
                ['full_internal_period','tempo_range','pulse_tails','phase_convention'],
                [{'frame':n+1,'index':1,'outputs':{'3':0.,'13':0.}}] if not passthrough else [{'frame':n+1,'index':1}])
    for steps in range(1,9):
        for passthrough in [0,1]:
            parts=[seg(1,event=1,steps=steps,clock=port(1,0)),
                   seg(1,event=4,steps=steps,clock=port(1,0),durable=(1,255,passthrough))]
            for tick in range(18):
                parts.extend([seg(1,steps=steps,clock=port(1,2)),seg(math.ceil(rate*.001)+2,steps=steps,clock=port(1,.1))])
            yield case(f'all_cells_length{steps}_pass{passthrough}',parts,['all24cells','all16outputs','length1','wrap','step_gates'])
    levels=[0.,raw(0x3dcccccc),f32(.1),raw(0x3dccccce),1.,raw(0x3fffffff),2.,raw(0x40000001),10.,0.,-10.]
    for target in ['clock','run','reset']:
        for width in [1,3,16]:
            yield case(f'thresholds_{target}_w{width}',
                [seg(1,clock=port(1,0),**({target:port(width,[level]+[16.]*15)} if target!='clock' else {}))
                 if target!='clock' else seg(1,clock=port(width,[level]+[16.]*15))
                 for level in levels*6],['threshold_neighbors','mono_first_lane','detector_hysteresis'])
    for held in [0,1,2,4,7,2047]:
        yield case(f'startup_held{held}',[seg(8,held=held,clock=port(1,10),run=port(1,10),reset=port(1,10)),
            seg(2,held=0,clock=port(1,0),run=port(1,0),reset=port(1,0)),
            seg(8,held=held,clock=port(1,10),run=port(1,10),reset=port(1,10)),seg(math.ceil(rate*.003),clock=port(1,0))],
            ['startup_uninitialized','held_controls','release_rearm','simultaneous_cv_button'])
    for gate in range(8):
        yield case(f'gate_and_clock_same_frame_{gate}',[
            seg(2,clock=port(1,0)),seg(1,held=1|(1<<(gate+3)),clock=port(1,2)),
            seg(1,event=2,held=1|(1<<(gate+3)),clock=port(1,2)),
            seg(2,clock=port(1,0)),seg(1,held=1|(1<<(gate+3)),clock=port(1,2)),
            seg(math.ceil(rate*.003),clock=port(1,0))],
            ['gate_clock_same_sample','initialize_held_detector_retention','selected_gate_during_pulse'])
    edge_cells=[-10. if (r+c)%2 else 10. for r in range(3) for c in range(8)]
    parts=[seg(2,cells=edge_cells,clock=port(1,0))]
    for _ in range(16):parts.extend([seg(1,cells=edge_cells,clock=port(1,2)),seg(1,cells=edge_cells,clock=port(1,0))])
    yield case('cell_voltage_endpoints',parts,['all24cells','both_voltage_endpoints'])
    for button in range(11):
        yield case(f'button{button}_hold',
            [seg(2,clock=port(1,0)),seg(5,held=1<<button,clock=port(1,0)),seg(2,clock=port(1,0)),
             seg(1,held=1<<button,clock=port(1,0)),seg(math.ceil(rate*.003),clock=port(1,0))],
            ['all_buttons','held_not_retriggered','exact_pulse_decay','gate_toggles'])
    for passthrough in [0,1]:
        for start in [0,1,5,7]:
            parts=[seg(2,clock=port(1,0)),seg(1,event=4,clock=port(1,0),durable=(1,255,passthrough))]
            for _ in range(start):parts.extend([seg(1,clock=port(1,2)),seg(1,clock=port(1,0))])
            for buttonmask,cvreset,cvclock in [(4,0,0),(5,0,2),(0,2,2),(1,2,2),(7,2,2),(0,0,0)]:
                parts.extend([seg(2,clock=port(1,0)),seg(1,held=buttonmask,reset=port(1,cvreset),clock=port(1,cvclock)),
                              seg(math.ceil(rate*.001)+2,clock=port(1,2))])
            yield case(f'reset_collisions_index{start}_pass{passthrough}',parts,['reset_index_pulse','manual_bypasses_guard','simultaneous_run_reset_clock','reset_window'])
    for external in [False,True]:
        for passthrough in [0,1]:
            clock=port(1,0) if external else port()
            yield case(f'stop_resume_ext{int(external)}_pass{passthrough}',
                [seg(2,clock=clock),seg(1,event=4,clock=clock,durable=(0,255,passthrough)),
                 seg(8,held=1,clock=port(1,2) if external else port()),
                 seg(1,event=4,held=1,clock=port(1,2) if external else port(),durable=(1,255,passthrough)),
                 seg(8,clock=clock),seg(1,held=1,clock=clock),seg(math.ceil(rate*.003),clock=clock)],
                ['stopped_detector_freeze','held_clock_resume','source_not_queue','passthrough'])
    for tempo in [-127.,-126.99999237060547,-20.,-.5,0.,.5,10.,20.,100.,127.,127.99998474121094]:
        yield case(f'effective_pitch_{tempo}',[seg(2,tempo=0,tempo_cv=port(1,tempo)),
            seg(1,tempo=0,tempo_cv=port(1,tempo),held=4),seg(math.ceil(rate*.002)+2,tempo=0,tempo_cv=port(1,tempo))],
            ['defined_scalar_exp_domain','zero_high_finite','reset_phase_accumulation'])
    for amount in [0.,.25,1.]:
        yield case(f'tempo_cv_amount{amount}',
            [seg(129,tempo=base,tempo_amount=amount,tempo_cv=port(width,cv))
             for base,width,cv in [(-2,0,0),(-2,1,-5),(4,1,8),(1,16,-.5),(1,0,0)]],
            ['tempo_cv_depth','first_lane','disconnect_internal'])
    for length in range(1,9):
        yield case(f'steps_boundaries_base{length}',
            [seg(1,steps=length,steps_amount=amount,steps_cv=port(width,cv),clock=port(1,2 if tick%2 else 0))
             for tick,(amount,width,cv) in enumerate((a,w,c) for a in [0.,.5,1.] for w in [0,1,16]
                 for c in [-16.,-1.500000119,-1.5,-1.499999881,-.5000000596,-.5,-.4999999702,0.,.4999999702,.5,.5000000596,16.])],
            ['length_rounding_half_neighbors','clamp','changing_length_while_clocked'])
    yield case('clock_reconnect_and_phase_retention',
        [seg(441,tempo=4),seg(17,tempo=4,clock=port(1,2)),seg(17,tempo=4,clock=port(1,0)),
         seg(441,tempo=4),seg(17,tempo=4,clock=port(1,2)),seg(441,tempo=4)],
        ['external_freezes_phase','disconnected_detector_retention','clock_width0'])
    for passthrough in [0,1]:
        parts=[seg(2,clock=port(1,0)),seg(1,clock=port(1,2)),
               seg(1,event=4,clock=port(1,0),durable=(1,85,passthrough)),
               seg(math.ceil(rate*.003),clock=port(1,0)),
               seg(1,event=4,clock=port(1,0),durable=(1,85,1-passthrough)),
               seg(math.ceil(rate*.003),clock=port(1,0)),
               seg(1,event=3,clock=port(1,0)),seg(1,event=2,clock=port(1,0)),seg(math.ceil(rate*.003),clock=port(1,0))]
        yield case(f'lifecycle_pulse_pass{passthrough}',parts,['pulse_remaining_frozen_passthrough','initialize_retained_history','explicit_adoption'])
    yield case('durable_missing_and_rotation',[
        seg(2,clock=port(1,0)),seg(1,event=4,clock=port(1,0),durable=(0,0,0)),
        seg(1,event=4,clock=port(1,0)),seg(1,event=4,clock=port(1,0),durable=(-1,129,-1)),
        seg(1,event=5,clock=port(1,0),rotate=1),seg(1,event=5,clock=port(1,0),rotate=-1),
        seg(1,event=3,clock=port(1,0)),seg(1,event=2,clock=port(1,0))],
        ['durable_running_gates_pass','missing_legacy_pass_true','rotation_all_rows_gates','initialize'])
    rotated=[CELLS[r*8+(c-1)%8] for r in range(3) for c in range(8)]
    yield case('rotation_composition_persistence',[
        seg(1,clock=port(1,0)),seg(1,event=4,clock=port(1,0),durable=(0,129,0)),
        seg(1,event=5,clock=port(1,0),rotate=1),seg(1,cells=rotated,clock=port(1,0)),
        seg(1,event=5,cells=rotated,clock=port(1,0),rotate=-1),seg(1,clock=port(1,0))],
        ['rotation_composition','persistent_cell_and_gate_rotation'],
        [{'frame':2,'gates_mask':3,'outputs':{str(r):CELLS[r*8+7] for r in range(3)}},
         {'frame':4,'gates_mask':129,'outputs':{str(r):CELLS[r*8] for r in range(3)}}])
    for gate_mask in [0,1,85,170,255]:
        parts=[seg(2,clock=port(1,0)),seg(1,event=4,clock=port(1,0),durable=(1,gate_mask,0))]
        for i in range(16):parts.extend([seg(1,clock=port(1,2)),seg(1,clock=port(1,0))])
        parts.extend([seg(1,event=3,clock=port(1,0)),seg(17,clock=port(1,0),bypass=1),seg(3,clock=port(1,0)),seg(1,event=2,clock=port(1,0))])
        yield case(f'gates_bypass_adopt_{gate_mask}',parts,['all_gate_patterns','bypass_host_shim','adoption','initialize'])
    yield case('analytic_static_stopped',[
        seg(1,clock=port(1,0)),seg(1,event=4,clock=port(1,0),durable=(0,255,0)),seg(65,clock=port(1,0))],
        ['independent_anchor'],[{'frame':66,'index':0,'running':0,'gates_mask':255,'passthrough':0,'phase':0.,'outputs':{str(i):CELLS[i*8] for i in range(3)}}])

def safety_cases(rate,a):
    vals=[float('nan'),float('inf'),-float('inf'),raw(0x7f7fffff),raw(0xff7fffff),-1e6,1e6,-128.,128.,255.]
    for i,v in enumerate(vals):
        yield case(f'safety_{i}',[seg(2),seg(65,tempo_cv=port(1,v),clock=port(1,v),run=port(1,v),reset=port(1,v),steps_cv=port(1,v)),
            seg(65,tempo_cv=port(1,v)),seg(65,event=3),seg(2,event=2)],['rust_only','nonfinite_extreme','recovery'])

def expand(items):
    for item in items:
        index=0
        for part in item['segments']:
            for i in range(part['repeat']):
                row=list(part['fields']);row[0]=1 if index==0 else row[0] if i==0 else 0
                yield item['name'],index,row;index+=1

def rows(rate,a):return expand(cases(rate,a))
def safety_rows(rate,a):return expand(safety_cases(rate,a))
def row_text(row):return ' '.join(str(v) if isinstance(v,int) else 'NaN' if math.isnan(v) else format(v,'.9g') for v in row)+'\n'
def fixture_text(rate,a,safety=False):return ''.join(row_text(r) for _,_,r in (safety_rows if safety else rows)(rate,a))
def safety_fixture_text(rate,a):return fixture_text(rate,a,True)
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def validate_row(row,safety):
    assert len(row)==120 and row[0] in range(6) and 0<=row[29]<2048 and row[119] in [0,1]
    assert -2<=row[1]<=4 and 0<=row[2]<=1 and 1<=row[3]<=8 and row[3].is_integer() and 0<=row[4]<=1
    assert all(-10<=v<=10 for v in row[5:29])
    assert row[115] in [-1,0,1] and -1<=row[116]<=255 and row[117] in [-1,0,1] and -1<=row[118]<=1
    for start in [30,47,64,81,98]:
        width=row[start];assert isinstance(width,int) and 0<=width<=16
        assert all(bits(v)==0 for v in row[start+1+width:start+17])
    if not safety:
        assert all(math.isfinite(v) for v in row)
        pitch=f32(row[1]+f32(row[31]*row[2]))
        assert 0<=f32(pitch+127)<255

def manifest(a):
    out={'schema':1,'declared_before_any_comparison':True,'acceptance_sha256':digest(HERE/'acceptance.json'),
         'generator_sha256':digest(HERE/'fixtures.py'),'rates':{}}
    for rate in a['sample_rates']:
        out['rates'][str(rate)]={}
        for phase,factory,iterator in [('reference',cases,rows),('safety',safety_cases,safety_rows)]:
            h=hashlib.sha256();n=0
            for _,_,row in iterator(rate,a):validate_row(row,phase=='safety');h.update(row_text(row).encode());n+=1
            items=[];cursor=0
            for item in factory(rate,a):
                length=sum(s['repeat'] for s in item['segments']);items.append({k:v for k,v in item.items() if k!='segments'}|{'frames':length,'start_row':cursor,'end_row_exclusive':cursor+length});cursor+=length
            assert n==cursor and len(items)==len({i['name'] for i in items})
            out['rates'][str(rate)][phase]={'processed_frames':n,'observed_frames':n,'cases':len(items),'input_sha256':h.hexdigest(),'case_manifest':items}
    return out

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
