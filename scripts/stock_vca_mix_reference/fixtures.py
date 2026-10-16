#!/usr/bin/env python3
"""Source-only bounded fixture declaration. No production import or DSP model."""
import argparse
import json
from pathlib import Path
import struct

RATES=(44100,48000,96000)
ONE=0x3f800000
SIGNALS=(1,-2,4,8)
CV_LANES=(10,20,5,0,-5,15,2.5)
CONTROLS=(0,0x80000000,1,0x80000001,0x3effffff,0x3f000000,0x3f000001,ONE,0x3fb504f3,0x40000000,0xbf800000,0x7f7fffff)

def bits(x): return struct.unpack('<I',struct.pack('<f',x))[0]
def step(expect=None): return {'op':'step',**({'expect':expect} if expect else {})}
def source(p,values): return {'op':'source','port':p,'values_bits':values}
def connect(name,src,p,dst,i): return {'op':'connect','id':name,'source':src,'port':p,'destination':dst,'input':i}
def flags(n): return {'chExp':bool(n&1),'mixExp':bool(n&2)}
def initial(widths=None,mask=31,flag=0,params=None):
    widths=widths or [1]*9
    return {'sources':[[bits(SIGNALS[p-1]*(i%5+1)*(-1 if i%3==2 else 1)) if p in (1,2,3,4) else bits(CV_LANES[i%len(CV_LANES)]) for i in range(w)] for p,w in enumerate(widths)],
            'input_mask':sum(1<<p for p,w in enumerate(widths) if w), 'output_mask':mask,
            'parameter_bits':[ONE]*5 if params is None else params,'data':flags(flag)}
def case(rate,name,init,events): return {'id':f'{rate}/{name}','sample_rate':rate,'initial':init,'events':events}

def nominal_witness(init):
    """Exact lane-zero anchors ONLY for unity-gain/unity-or-absent CV cases.

    Integer-valued distinct strips make the ordered unity sum exact; this is
    not a VCMixer processor and is never used for arbitrary gains/CV controls.
    """
    live=[bool(init['input_mask']&(1<<p)) and bool(init['sources'][p]) for p in range(9)]
    strips=[SIGNALS[i] if live[i+1] and (not(init['input_mask']&(1<<(i+5))) or live[i+5]) else (-0.0 if live[i+1] and SIGNALS[i]<0 else 0) for i in range(4)]
    width=max(1,*[len(init['sources'][p]) if init['input_mask']&(1<<p) else 0 for p in range(1,5)])
    mix=sum(strips) if (not(init['input_mask']&1) or live[0]) and init['output_mask']&1 else 0
    return {**{f'outputs.{p}.values_bits.0':bits(v) for p,v in enumerate([mix]+strips)},
            **{f'outputs.{p}.channels':width if init['output_mask']&(1<<p) else 0 for p in range(5)}}

def nominal_steps(init): return [step(),step(nominal_witness(init))]

def fresh_cases(rate):
    for p in range(9):
        for w in range(17):
            for f in range(4):
                for mask in (0,1,31):
                    widths=[1]*9; widths[p]=w
                    init=initial(widths,mask,f)
                    yield case(rate,f'width/{p}/{w}/{f}/{mask}',init,nominal_steps(init))
    patterns=((0,0,0,0,0,0,0,0,0),(16,0,0,0,0,16,16,16,16),
              (0,1,2,3,4,16,15,1,0),(1,16,15,9,5,0,1,2,3),
              (2,3,4,5,6,7,8,9,10),(16,1,1,1,1,1,16,0,2),
              (3,4,7,8,11,12,15,16,1),(15,16,1,0,2,3,4,5,6),
              (1,0,16,0,0,1,1,1,1),(4,5,0,9,0,16,0,16,0),
              (5,9,13,16,1,2,3,4,5),(16,16,16,16,16,16,16,16,16))
    for n,widths in enumerate(patterns):
        for f in range(4):
            for mask in range(32):
                init=initial(widths,mask,f)
                yield case(rate,f'asym/{n}/{f}/{mask}',init,nominal_steps(init))
    for p in range(5):
        for raw in CONTROLS:
            for f in range(4):
                params=[ONE]*5;params[p]=raw
                init=initial([4]*9,31,f,params)
                witness=4 if p!=3 else 8;out=3 if p!=3 else 4
                yield case(rate,f'gain/{p}/{raw:08x}/{f}',init,[step(),step({f'outputs.{out}.values_bits.0':bits(witness),'outputs.0.channels':4})])
    # Independent 17x17 signal/CV widths for each strip, plus a global signal
    # witness against mix CV. All other strips remain active and distinct.
    for role in range(5):
        signal_port=role+1 if role<4 else 1
        cv_port=role+5 if role<4 else 0
        for signal_width in range(17):
            for cv_width in range(17):
                for f in range(4):
                    widths=[1]*9;widths[signal_port]=signal_width;widths[cv_port]=cv_width
                    init=initial(widths,31,f)
                    yield case(rate,f'pair/{role}/{signal_width}/{cv_width}/{f}',init,nominal_steps(init))
    for mask in range(512):
        for f in range(4):
            init=initial([1]*9,31,f);init['input_mask']=mask
            yield case(rate,f'input-mask/{mask}/{f}',init,nominal_steps(init))
    for role in ('signal','cv','parameter'):
        for exponent in range(255):
            for sign in (0,0x80000000):
                raw=[sign|(exponent<<23)|m for m in (0,1,0x3fffff,0x7fffff)]
                init=initial([4]*9,31,3)
                if role=='signal': init['sources'][1]=raw
                elif role=='cv': init['sources'][5]=raw
                else: init['parameter_bits'][1]=raw[-1]
                yield case(rate,f'finite/{role}/{exponent}/{sign}',init,[step(),step({'outputs.3.values_bits.0':bits(4),'outputs.0.channels':4})])

def explicit_cases(rate):
    init=initial([0,1,0,0,0,0,0,0,0]);init['sources'][1]=[bits(1.25)]
    anchor={**{f'outputs.{p}.values_bits.{i}':bits(1.25) for p in (0,1) for i in range(4)},'meters.0.v_bits':ONE,'outputs.0.channels':1,'divider_clock':2}
    yield case(rate,'anchor/padded-mono',init,[step(),step(anchor)])
    init=initial([0,4,0,0,0,0,0,0,0]);init['sources'][1]=[bits(x) for x in (1,-1,2,-2)]
    yield case(rate,'anchor/signed-cancellation',init,[step(),step({'meters.0.v_bits':0})])
    init=initial([0,4,4,4,4,0,0,0,0]);init['sources']=[[],[bits(16777216)]*4,[ONE]*4,[bits(-16777216)]*4,[ONE]*4,[],[],[],[]]
    yield case(rate,'anchor/strip-add-order',init,[step(),step({'outputs.0.values_bits.0':ONE})])
    tiny=[0,0x80000000,1,0x80000001,0x007fffff,0x807fffff,0x00800000,0x80800000]
    init=initial([0,8,0,0,0,0,0,0,0]);init['sources'][1]=tiny
    yield case(rate,'anchor/subnormal-copy',init,[step({f'inputs.1.values_bits.{i}':v for i,v in enumerate(tiny)}),step(),{'op':'bypass','value':True},step(),{'op':'bypass','value':False},step()])
    for mask in (0,1,30,31):
        # A sole signal is essential: other live width16 strips would prevent
        # the global output width from shrinking and hide output retirement.
        init=initial([0,16,0,0,0,0,0,0,0],mask,0)
        init['sources'][1]=[bits(i+1) for i in range(16)]
        strip_consumed=bool(mask&2);mix_consumed=bool(mask&1)
        retired_strip=[bits(3)]+([0]*15 if strip_consumed else [bits(3)]*3+[bits(i+1) for i in range(4,16)])
        retired_mix=[bits(3)]+[0]*15 if mix_consumed else [0]*16
        anchor={'outputs.1.values_bits':retired_strip,'outputs.0.values_bits':retired_mix,
                **{f'outputs.{p}.channels':1 if mask&(1<<p) else 0 for p in range(5)}}
        events=[step(),step(),source(1,[bits(3)]),step(),step(anchor)]
        for values in ([bits(1),bits(-2),bits(4)],[],[bits(i+1) for i in range(5)],[bits(i+1) for i in range(16)]):
            events += [source(1,values),step(),step()]
        yield case(rate,f'host/sole-signal-global-retirement/{mask}',init,events)
        init=initial([0,1,0,0,0,0,0,0,0],mask,0);init['sources'][1]=[bits(3)]
        fresh={'outputs.1.values_bits':[bits(3)]*4+[0]*12,
               'outputs.0.values_bits':([bits(3)]*4+[0]*12) if mix_consumed else [0]*16,
               **{f'outputs.{p}.channels':1 if mask&(1<<p) else 0 for p in range(5)}}
        yield case(rate,f'host/sole-signal-fresh-mono/{mask}',init,[step(),step(fresh)])
    for mask in (0,1,30,31):
        init=initial([16]*9,mask,3)
        events=[step(),step(),source(1,[bits(3)]),step(),step(),source(1,[]),step(),step(),source(1,[bits(-2)]*16),step(),step(),{'op':'disconnect','id':'in1'},step(),connect('in1','source',1,'module',1),step(),step()]
        # Actual connection/disconnection before a sample, plus duplicate consumers.
        events += [connect('extra','module',0,'sink',4),{'op':'bypass','value':True},step(),{'op':'reset'},step(),{'op':'disconnect','id':'extra'},{'op':'bypass','value':False},step(),{'op':'adopt','parameter_bits':[bits(.5)]*5,'data':flags(1)},step(),{'op':'recreate'},step(),connect('newin','source',1,'module',1),connect('newout','module',0,'sink',0),step(),step()]
        yield case(rate,f'host/lifecycle/{mask}',init,events)
    init=initial([0]*9);init['input_mask']=511
    yield case(rate,'host/connected-zero',init,[step(),step(),source(1,[ONE]*16),step(),step(),source(1,[]),step(),step()])
    for port in range(9):
        init=initial([1]*9);init['sources'][port]=[];init['input_mask']=511
        emptied=nominal_witness(init)
        restored=initial([1]*9)
        values=restored['sources'][port]
        events=[step({f'inputs.{port}.channels':1,f'inputs.{port}.values_bits.0':0}),step(emptied),source(port,values),step(),step(nominal_witness(restored)),source(port,[]),step(),step(emptied)]
        yield case(rate,f'host/isolated-connected-zero/{port}',init,events)
    init=initial([1]*9);events=[step(),step(nominal_witness(init))]
    for port in range(5):
        for raw in CONTROLS:
            events += [{'op':'parameter','port':port,'value_bits':raw,'expect':{f'parameter_bits.{port}':raw}}, {'op':'save'},step(),
                       {'op':'adopt','parameter_bits':[raw]*5,'data':flags(port%4),'expect':{f'parameter_bits.{p}':raw for p in range(5)}},{'op':'save'},step()]
    events += [{'op':'adopt','parameter_bits':[ONE]*5,'data':flags(0)},{'op':'save'},step(nominal_witness(init))]
    yield case(rate,'state/all-raw-controls-roundtrip',init,events)
    init=initial([1]*9);events=[step(),step()]
    for p in range(9): events += [connect(f'stack{p}','source',(p+1)%9,'module',p),step(),step()]
    for p in range(9): events += [{'op':'disconnect','id':f'stack{p}'},step(),step()]
    events += [connect('fanout','module',4,'sink',0),step(),{'op':'disconnect','id':'fanout'},step()]
    yield case(rate,'host/stack-multiplicity',init,events)
    init=initial([16]*9,31,3);events=[step(),step()]
    for p in range(9):
        events += [source(p,[bits(3)]),step(),step(),source(p,[]),step(),step(),source(p,[bits(-2)]*16),step(),step(),{'op':'disconnect','id':f'in{p}'},step(),connect(f'in{p}','source',p,'module',p),step(),step()]
    yield case(rate,'host/all-port-retirement',init,events)
    init=initial([4]*9,31,0)
    for p,value in ((0,16777216),(1,1),(2,-16777216),(3,1),(5,10),(6,10),(7,10),(8,10)): init['sources'][p]=[bits(value)]*4
    events=[step(),step()]
    for p in (0,2,3): events += [connect(f'many{p}','source',p,'module',1),step(),step()]
    for p in range(5): events += [connect(f'fan{p}','module',p,'sink',(p+1)%5),step(),step()]
    for p in (0,2,3): events += [{'op':'disconnect','id':f'many{p}'},step(),step()]
    for p in range(5): events += [{'op':'disconnect','id':f'fan{p}'},step(),step()]
    yield case(rate,'host/many-cable-cancellation',init,events)

    for order in range(2):
        init=initial([16]*9,0,3)
        transitions=[{'op':'bypass','value':True},connect('late','module',0,'sink',0)]
        events=[step(),step()]+(transitions if order==0 else list(reversed(transitions)))
        events += [step(),{'op':'bypass','value':True},{'op':'disconnect','id':'late'},connect('late','module',0,'sink',0),step(),{'op':'bypass','value':False},step()]
        yield case(rate,f'host/bypass-connect-order/{order}',init,events)

    events=[step(),step(),{'op':'save'}]
    for value in (None,False,True,0,1,1.0,'true',[],{}, {'x':1}):
        events += [{'op':'data_load','value':flags(3)},{'op':'data_load','value':{'chExp':value}},{'op':'save'},step(),{'op':'data_load','value':{'mixExp':value}},step()]
    for value in (None,False,True,0,1,'flags',[],[1],{}): events += [{'op':'data_load','value':flags(3)},{'op':'data_load','value':value},step()]
    events += [{'op':'reset'}, {'op':'adopt','parameter_bits':[bits(-1)]*5,'data':flags(2)},step(),{'op':'save'},{'op':'recreate'},{'op':'save'},step()]
    yield case(rate,'state/json-reset-adopt',initial([1]*9,31,0),events)
    init=initial([0,1,0,0,0,0,0,0,0]);init['sources'][1]=[bits(1.25)]
    events=[step({'divider_clock':(n+1)%512,'light_bits':[ONE,0,0,0] if n>=511 else [0]*4}) if n in (510,511,512) else step() for n in range(513)]
    events += [source(1,[0]),*[step() for _ in range(513)],{'op':'bypass','value':True},*[step() for _ in range(3)],{'op':'reset'},{'op':'bypass','value':False},step(),{'op':'adopt','parameter_bits':[ONE]*5,'data':flags(3)},step(),{'op':'recreate'},step()]
    yield case(rate,'meter/attack-release-refresh',init,events)
    # Excite every actual lamp at refreshes: strips2/4 are strictly inside
    # the brightness interval at frame512, strip3 attacks to saturation later.
    init=initial([0,1,1,1,1,0,0,0,0])
    for port,value in enumerate((1.25,.3125,.078125,.625),1): init['sources'][port]=[bits(value)]
    first_wrap={**{f'meters.{p}.v_bits':bits(v) for p,v in enumerate((1,.25,.0625,.5))},
                'light_bits.0':ONE,'light_bits.2':0,'divider_clock':0}
    events=[step(first_wrap) if n==511 else step() for n in range(513)]
    for port,value in enumerate((0,.125,1.25,0),1): events.append(source(port,[bits(value)]))
    events += [step({'light_bits.2':ONE,'divider_clock':0}) if n==510 else step() for n in range(513)]
    yield case(rate,'meter/four-strip-mapping-refresh',init,events)
    init=initial([0,16,0,0,0,0,0,0,0]);init['sources'][1]=[bits(.15625)]*16
    events=[step() for _ in range(513)]+[source(1,[bits(x) for x in (1,-1,2,-2)]),*[step() for _ in range(513)]]
    yield case(rate,'meter/poly-cancellation-refresh',init,events)
    init=initial([0,1,0,0,0,0,0,0,0],0);init['sources'][1]=[bits(1.25)]
    events=[step() for _ in range(511)]+[{'op':'bypass','value':True},step(),{'op':'reset'},step(),{'op':'bypass','value':False},step(),connect('late','module',0,'sink',0),step(),{'op':'disconnect','id':'late'},step()]
    yield case(rate,'meter/bypass-phase511',init,events)

def safety_records():
    for role in ('source','parameter'):
        for raw in (0x7f800000,0xff800000,0x7fc12345,0xffc12345):
            yield {'id':f'nonfinite-input/{role}/{raw:08x}','role':role,'bits':raw,'classification':'separate local safety obligation; no invented SDK result'}
def cases():
    for rate in RATES:
        yield from fresh_cases(rate)
        yield from explicit_cases(rate)
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=Path,required=True);p.add_argument('--safety-output',type=Path,required=True);a=p.parse_args()
    for path,values in ((a.output,cases()),(a.safety_output,safety_records())):
        path.parent.mkdir(parents=True,exist_ok=True)
        with path.open('x') as stream:
            for value in values: stream.write(json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n')
if __name__=='__main__': main()
