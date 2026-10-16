#!/usr/bin/env python3
"""Independent streaming StockSEQ3 validator. No DSP/reference mirror."""
from collections import Counter
from contextlib import ExitStack
import math
import struct
from fixtures import cases,rows,safety_cases,safety_rows

def bits(v):
    try:return struct.pack('<f',v)
    except (OverflowError,struct.error):return None

def exact(x,y):return bits(x) is not None and bits(x)==bits(y)
def f32(v):return struct.unpack('<f',struct.pack('<f',v))[0]
def read(stream,size,label,reasons,stats):
    line=stream.readline()
    if not line:reasons.append(f'{label}: missing row at EOF');return None
    stats[label]+=1;tokens=line.split()
    if len(tokens)!=size:reasons.append(f'{label}: expected{size}fields got{len(tokens)}');return None
    try:values=[float(t) for t in tokens]
    except ValueError:reasons.append(f'{label}: invalid numeric token');return None
    if any(math.isfinite(v) and bits(v) is None for v in values):reasons.append(f'{label}: finite token outsidef32');return None
    return [struct.unpack('<f',bits(v))[0] for v in values]

def initial():
    return [1.]*16+[0.]*16+[0.,0.,8.,1.]+[1.]*8+[0.]+[2.]*14+[0.]*3+[0.]*27+[0.]

def independent(label,v,row,previous,anchors,rate,stats,reasons):
    if v is None:return 0
    stats[f'{label}_independent_frames']+=1
    if any(not math.isfinite(x) for x in v):reasons.append(f'{label}: nonfinite meaningful output/state')
    if v[:16]!=[1.]*16:reasons.append(f'{label}: outputs must all be mono')
    if row[119]:
        if any(not exact(x,0.) for x in v[16:32]):reasons.append(f'{label}: bypass outputs notpositivezero')
        if previous is not None and any(not exact(x,y) for x,y in zip(v[32:],previous[32:])):reasons.append(f'{label}: bypass runtime mutated')
        return 0
    if not v[32]>=0:reasons.append(f'{label}: phase negative/nonfinite')
    for i,low,high in [(33,0,7),(34,1,8),(35,0,1),(44,0,1),(89,0,2047)]+[(i,0,1) for i in range(36,44)]+[(i,0,2) for i in range(45,59)]:
        if not math.isfinite(v[i]) or not v[i].is_integer() or not low<=v[i]<=high:reasons.append(f'{label}: invalid discrete state field{i}={v[i]}')
    dt=f32(1/rate)
    for i in range(59,62):
        if not -dt<=v[i]<=f32(.001):reasons.append(f'{label}: pulse[{i-59}] outside allowed f32 residue/remaining bounds')
    for i in range(62,89):
        if not 0<=v[i]<=1:reasons.append(f'{label}: lamp[{i-62}] outside finite[0,1]')
    if v[89]!=row[29]:reasons.append(f'{label}: heldmask doesnot equal receiving physicallevels')
    for i,bit in [(46,1),(47,2)]+[(48+j,3+j) for j in range(8)]:
        if v[i]!=int(bool(row[29]&(1<<bit))):reasons.append(f'{label}: processed Boolean detector field{i} wrong heldlevel')
    if v[35]==1 and v[45]!=int(bool(row[29]&1)):reasons.append(f'{label}: running clockbutton detector wrong heldlevel')
    if previous is not None:
        if v[35]==0 and any(not exact(v[i],previous[i]) for i in [45,56]):reasons.append(f'{label}: paused clock detectors mutated')
        if row[47]==0 and not exact(v[56],previous[56]):reasons.append(f'{label}: disconnected clock Schmitt mutated')
    index=int(v[33]) if math.isfinite(v[33]) and 0<=v[33]<=7 else None
    if index is not None:
        for i in range(8):
            if not exact(v[20+i],10. if i==index else 0.):reasons.append(f'{label}: step{i+1} output notonehot')
        for r in range(3):
            column=(index-row[118])%8 if row[0]==5 else index
            if not exact(v[16+r],row[5+r*8+column]):reasons.append(f'{label}: CVrow{r+1} doesnot select receivingcell')
        if not exact(v[19],v[29] if v[36+index] else 0.):reasons.append(f'{label}: trigger notclock AND selectedgate')
        if not exact(v[73+2*index],1.):reasons.append(f'{label}: selectedstep lamp not1')
    for i in [19,*range(20,28),29,30,31]:
        if v[i] not in (0.,10.):reasons.append(f'{label}: gate/output field{i} not0/10')
    if not exact(v[28],v[34]-1):reasons.append(f'{label}: stepsoutput notlengthminus1')
    if not exact(v[63],v[35]):reasons.append(f'{label}: runninglamp inconsistent')
    for i in range(8):
        if not exact(v[65+i],v[36+i]):reasons.append(f'{label}: gate{i+1} lamp inconsistent')
        if not exact(v[74+2*i],float(i>=v[34])):reasons.append(f'{label}: outofrange step{i+1} redlamp inconsistent')
    checked=0
    for anchor in anchors:
        checked+=1
        for key,wanted in anchor.items():
            if key=='frame':continue
            if key=='gates_mask':pairs=[(36+i,float(bool(wanted&(1<<i)))) for i in range(8)]
            elif key=='outputs':pairs=[(16+int(i),value) for i,value in wanted.items()]
            else:pairs=[({'phase':32,'index':33,'running':35,'passthrough':44}[key],wanted)]
            if any(not exact(v[i],value) for i,value in pairs):reasons.append(f'{label}: independent anchor {key} failed')
    return checked

def validate(rate,a,phase,input_path,sdk_trace_path_or_None,rust_trace_path):
    phase='reference' if phase=='normal' else phase
    if phase not in ('reference','safety'):raise ValueError('phase must be reference or safety')
    if (phase=='reference')!=(sdk_trace_path_or_None is not None):raise ValueError('SDK only/required for reference')
    anchors={c['name']:{x['frame']:x for x in c['anchors']} for c in (safety_cases if phase=='safety' else cases)(rate,a)}
    stats,traces=Counter(),Counter()
    report={'passed':False,'phase':phase,'rate':rate,'frames':0,'failed_frames':0,'protocol_failures':0,'first_failures':[],
            'max_absolute_error':0.,'mismatched_fields':0,'independent_anchors_checked':0,'oracle_claim':phase=='reference'}
    previous={'sdk':None,'rust':None}
    def record(name,frame,index,reasons,protocol):
        if not reasons:return
        report['failed_frames']+=1;report['protocol_failures']+=int(protocol)
        if len(report['first_failures'])<16:report['first_failures'].append({'case':name,'frame':frame,'row':index,'reason_count':len(reasons),'reasons':reasons[:32]})
    with ExitStack() as stack:
        streams={'input':stack.enter_context(open(input_path)),'rust':stack.enter_context(open(rust_trace_path))}
        if sdk_trace_path_or_None is not None:streams['sdk']=stack.enter_context(open(sdk_trace_path_or_None))
        for index,(name,frame,row) in enumerate((safety_rows if phase=='safety' else rows)(rate,a)):
            reasons=[];actual=read(streams['input'],120,'input',reasons,traces)
            if actual is not None and any(not (math.isnan(x) and math.isnan(y)) and not exact(x,y) for x,y in zip(actual,row)):reasons.append('input: differs from frozen generator')
            values={label:read(stream,90,label,reasons,traces) for label,stream in streams.items() if label!='input'}
            protocol=bool(reasons)
            if row[0]==1:previous={'sdk':initial(),'rust':initial()}
            for label,v in values.items():
                report['independent_anchors_checked']+=independent(label,v,row,previous[label],
                    [anchors[name][frame]] if frame in anchors[name] else [],rate,stats,reasons)
                previous[label]=v
            if phase=='reference' and values['sdk'] is not None and values['rust'] is not None:
                failures=[]
                for i,(x,y) in enumerate(zip(values['sdk'],values['rust'])):
                    stats['exact_f32_fields']+=1
                    if math.isfinite(x) and math.isfinite(y):report['max_absolute_error']=max(report['max_absolute_error'],abs(x-y))
                    if not math.isfinite(x) or not math.isfinite(y) or not exact(x,y):failures.append((i,x,y))
                report['mismatched_fields']+=len(failures)
                if failures:reasons.append(f'exact cross comparison: {len(failures)} mismatches; first(field,SDK,Rust)={failures[0]}')
            report['frames']+=1;record(name,frame,index,reasons,protocol)
        for label,stream in streams.items():
            extra=sum(1 for _ in stream);traces[label]+=extra
            if extra:record('<EOF>',None,report['frames'],[f'{label}: {extra} unexpected extra rows'],True)
    report.update({'passed':report['failed_frames']==0,'trace_rows':dict(traces),'compared_field_counts':dict(stats)})
    return report
