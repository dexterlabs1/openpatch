#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify original stock_vcf Rust DSP against the actual pinned SDK class."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent
PIN='10dd0160c664770910e5584b7b00498cc48d9ddd'
URL=f'https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/VCF.cpp'
ACCEPTANCE=json.loads((HERE/'acceptance.json').read_text())
SEEDS=ACCEPTANCE['self_oscillation']['seeds']
CASES={name:8192 for name in ['impulse','dc','drive_extremes','signed_cv_extremes','audio_rate_cv','mono_cv_broadcast','short_cv','width_1','width_4','width_5','width_16','shrink_regrow','connectivity_reconnect','parameter_adoption']}
RESONANCE_FIXTURE=json.loads((HERE/'resonance_fixture.json').read_text())
CASES[RESONANCE_FIXTURE['name']]=RESONANCE_FIXTURE['frames']
CASES['self_oscillation']=ACCEPTANCE['self_oscillation']['frames']
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def f32(x):return struct.unpack('f',struct.pack('f',x))[0]
def vectors(name,frames,rate):
    for frame in range(frames):
        mask=3;p=[0.5,0.0,0.0,0.0,0.0,0.0]
        ports=[[0,*[997.0-i*3 for i in range(16)]] for _ in range(4)]
        def port(i,value,width=1):
            values=value if isinstance(value,list) else [value]*width
            ports[i]=[width,*values[:width],*[997.0-i*3 for i in range(width,16)]]
        def audio(width):port(3,[3*math.sin(2*math.pi*(103+lane*31)*frame/rate) for lane in range(width)],width)
        audio(1)
        if name=='impulse':port(3,5.0 if frame==0 else 0.0)
        elif name=='dc':port(3,[-5.,0.,5.,20.,-20.][frame//1639%5])
        elif name=='drive_extremes':
            p[4]=[-1.,0.,1.][frame//683%3];port(3,1e6 if frame%37<19 else -1e6,5)
        elif name=='signed_cv_extremes':
            p=[0.5,0.0,1.,0.,0.,-1.];audio(16)
            values=[-1e6,-1000.,-128.,-20.,0.,10.,128.,1000.,1e6]
            port(0,[values[(frame//71+lane)%len(values)] for lane in range(16)],16)
            port(1,1e6);port(2,values[frame//41%len(values)])
        elif name=='audio_rate_cv':
            p=[0.5,0.2,0.3,0.1,0.,0.7];audio(5)
            for i in range(3):port(i,4*math.sin(2*math.pi*(1093+i*733)*frame/rate))
        elif name=='mono_cv_broadcast':
            p=[0.5,0.15,0.5,0.2,0.,0.4];audio(16)
            for i in range(3):port(i,(-1 if i==1 else 1)*math.sin(frame/17)*3)
        elif name=='short_cv':
            p=[0.5,0.1,0.5,0.2,0.,0.4];audio(16)
            for i,width in enumerate([3,5,7]):port(i,[(lane-2)*0.7 for lane in range(width)],width)
        elif name.startswith('width_'):audio(int(name[6:]));p[1]=0.25
        elif name=='shrink_regrow':
            width=[16,1,5,16,4,0,16][frame//613%7];audio(width);p[1]=0.2;port(0,2.0);p[2]=0.25
        elif name=='connectivity_reconnect':
            mask=[3,1,0,2,3,0,1,2,3][frame//499%9];audio(5);p[1]=0.2
        elif name=='parameter_adoption':
            cycle=frame//127;p=[0.2+(cycle%8)*0.09,(cycle%4)*0.1,0.5,-0.25,[-1.,0.,1.][cycle%3],0.5];audio(5);port(0,2.0);port(1,2.0);port(2,-2.0)
        elif name=='resonance_cv_extremes':
            fixture=RESONANCE_FIXTURE
            segment=fixture['segments'][frame//fixture['segment_frames']%len(fixture['segments'])]
            p=[fixture['frequency'],fixture['raw_resonance'],0.,segment['resonance_cv_amount'],fixture['drive'],0.]
            port(1,segment['resonance_cv_volts'])
            port(3,fixture['audio_volts'],fixture['audio_channels'])
        elif name=='self_oscillation':p=[0.6,1.,0.,0.,0.,0.];port(3,0.0)
        else:raise ValueError(name)
        yield ' '.join(format(f32(v),'.9g') for v in [mask,*p,*[v for port_values in ports for v in port_values]])

def metrics(trace,rate):
    threshold=ACCEPTANCE['self_oscillation']['startup_threshold_volts']
    crossings=[i for i,v in enumerate(trace) if abs(v)>=threshold]
    start=len(trace)//2;late=trace[start:]
    zero=[]
    for i in range(start+1,len(trace)):
        a,b=trace[i-1:i+1]
        if a<=0<b:zero.append(i-1-a/(b-a))
    return {'startup_seconds':crossings[0]/rate if crossings else None,'rms_volts':math.sqrt(sum(x*x for x in late)/len(late)),'peak_volts':max(map(abs,late)),'frequency_hz':(len(zero)-1)*rate/(zero[-1]-zero[0]) if len(zero)>=10 else None,'late_rising_crossings':len(zero)}


def resonance_counterfactuals(cpp,rust,rate,env,retain):
    declaration=RESONANCE_FIXTURE['counterfactual_predeclaration']
    frames=declaration['frames_per_fresh_trace'];seed=declaration['seed_words']
    baseline=None;baseline_states=None;audits=[]
    configurations=[('baseline',declaration['baseline'])]+[(f"sign_combination_{i+1}" if i<4 else f"zero_depth_{i-3}",segment) for i,segment in enumerate(RESONANCE_FIXTURE['segments'])]
    for label,configuration in configurations:
        row=next(vectors('resonance_cv_extremes',1,rate)).split()
        row[4]=format(configuration['resonance_cv_amount'],'.9g');row[25]=format(configuration['resonance_cv_volts'],'.9g')
        data=(' '.join(row)+'\n')*frames
        results=[subprocess.run([str(exe),str(rate),*seed],input=data,text=True,capture_output=True,check=True,env=env) for exe in [cpp,rust]]
        traces=[[list(map(f32,map(float,line.split()))) for line in result.stdout.splitlines()] for result in results]
        assert all(len(trace)==frames for trace in traces)
        states=[list(map(int,result.stderr.split())) for result in results]
        failed=[];max_error=0.;differences=[[0.,0.],[0.,0.]]
        for frame,(reference,actual) in enumerate(zip(*traces)):
            assert len(reference)==len(actual)==33 and reference[0]==actual[0]==RESONANCE_FIXTURE['audio_channels']
            assert all(math.isfinite(v) for v in reference+actual)
            for out in range(2):
                for lane in range(RESONANCE_FIXTURE['audio_channels']):
                    index=1+out*16+lane;error=abs(reference[index]-actual[index]);max_error=max(max_error,error)
                    if error>ACCEPTANCE['stable_driven']['absolute_volts']+ACCEPTANCE['stable_driven']['relative']*abs(reference[index]) and len(failed)<8:failed.append({'frame':frame,'output':out,'lane':lane,'reason':'SDK/Rust pointwise mismatch'})
            if baseline is not None:
                for implementation in range(2):
                    for out in range(2):
                        for lane in range(RESONANCE_FIXTURE['audio_channels']):
                            index=1+out*16+lane;differences[implementation][out]=max(differences[implementation][out],abs(traces[implementation][frame][index]-baseline[implementation][frame][index]))
                if configuration['resonance_cv_amount']==0.0 and (reference!=baseline[0][frame] or actual!=baseline[1][frame]) and len(failed)<8:failed.append({'frame':frame,'reason':'zero-depth rejection not exact'})
        if states[0]!=states[1]:failed.append({'reason':'SDK/Rust RNG mismatch'})
        if baseline is None:baseline=traces;baseline_states=states
        else:
            if states!=baseline_states:failed.append({'reason':'counterfactual RNG history not identical to baseline'})
            if configuration['resonance_cv_amount']!=0.0 and not all(v>declaration['nonzero_depth_minimum_difference_volts'] for implementation in differences for v in implementation):failed.append({'reason':'counterfactual sensitivity insufficient'})
        audit={'configuration':label,'sample_rate':rate,'frames':frames,'resonance_cv_amount':configuration['resonance_cv_amount'],'resonance_cv_volts':configuration['resonance_cv_volts'],'input_sha256':hashlib.sha256(data.encode()).hexdigest(),'maximum_sdk_rust_absolute_error_volts':max_error,'maximum_difference_from_fresh_zero_depth_baseline_volts':{'sdk':differences[0],'rust':differences[1]},'sdk_initial_rng_state':states[0][:2],'rust_initial_rng_state':states[1][:2],'sdk_final_rng_state':states[0][2:4],'rust_final_rng_state':states[1][2:4],'processed_group_draws':states[0][4],'passed':not failed,'first_failures':failed[:8],'raw_traces':{}}
        for implementation,result in zip(['sdk','rust'],results):
            path=retain/f'resonance-counterfactual-{rate}-{label}-{implementation}.txt';path.parent.mkdir(parents=True,exist_ok=True);path.write_text(result.stdout);audit['raw_traces'][implementation]={'path':str(path),'sha256':digest(path)}
        audits.append(audit)
        print(json.dumps({'audit':'resonance_counterfactual','rate':rate,'configuration':label,'passed':audit['passed'],'sdk_difference':differences[0]}),flush=True)
    return audits

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk',type=Path,default=Path.home()/'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--output',type=Path,default=ROOT/'docs/benchmarks/stock-vcf-reference.json')
    parser.add_argument('--serde-rlib',type=Path,help='Use existing dependency; avoids concurrent Cargo work')
    parser.add_argument('--case',action='append',choices=list(CASES))
    parser.add_argument('--rate',action='append',type=int,choices=[44100,48000,96000])
    parser.add_argument('--retain-traces',type=Path,default=Path(tempfile.gettempdir())/'stock-vcf-reference-traces',help='Save exact raw oracle and Rust self-oscillation traces outside the repository')
    args=parser.parse_args();sdk=args.sdk.resolve()
    source=subprocess.check_output(['curl','--fail','--silent','--show-error','--location',URL]);upstream=source.decode()
    assert upstream.count('struct VCFWidget')==1 and upstream.count('#include "plugin.hpp"')==1
    prefix=upstream.split('struct VCFWidget',1)[0].replace('#include "plugin.hpp"','#include <rack.hpp>\nusing namespace rack;')
    library=sdk/('libRack.dylib' if sys.platform=='darwin' else 'libRack.so')
    headers=['rack.hpp','dsp/approx.hpp','dsp/common.hpp','random.hpp','simd/functions.hpp','simd/Vector.hpp','engine/Port.hpp']
    report={'reference':{'fundamental_commit':PIN,'source_url':URL,'source_sha256':hashlib.sha256(source).hexdigest(),'compiled_prefix_sha256':hashlib.sha256(prefix.encode()).hexdigest(),'sdk_library_sha256':digest(library),'sdk_header_sha256':{h:digest(sdk/'include'/h) for h in headers}},'acceptance':ACCEPTANCE,'acceptance_sha256':digest(HERE/'acceptance.json'),'additional_fixture_predeclaration':RESONANCE_FIXTURE,'additional_fixture_predeclaration_sha256':digest(HERE/'resonance_fixture.json'),'method':'Compile actual pinned VCF class prefix in temporary storage, replacing plugin.hpp only and omitting widgets. Genuine SDK random::uniform() remains intact. Seed SDK random::local() after constructing VCF, with the exact Rust algorithm and seed warmup. Assert actual SDK connected-output channel counts before adapting disconnected host output frames. Rust source is unchanged except appended test-only private RNG accessors; parameter changes rebuild production module and adopt all state. Compare every driven visible sample from frame zero. Decimal traces are parsed back to f32 before comparison; both print formats retain enough significant digits for exact f32 round-trip. Separate fixed self-oscillation metrics; no trace alignment or warmup.','production_source_sha256':digest(ROOT/'crates/openpatch-core/src/stock_vcf.rs'),'driver_sha256':{p:digest(HERE/p) for p in ['run.py','sdk_probe.cpp','rust_probe.rs']},'toolchain':{'cpp':subprocess.check_output(['c++','--version'],text=True).splitlines()[0],'rust':subprocess.check_output(['rustc','--version'],text=True).strip(),'platform':sys.platform,'machine':os.uname().machine,'cpp_flags':['-std=c++17','-O2','-ffp-contract=off'],'rust_flags':['--edition=2021','-O']},'limitations':['No native app, GUI, audio device or speakers opened','Minimal adapters bypass Engine graph/cable delay, selected-output and tap consumers, public parameter validation, bypass and lifecycle; separate host tests qualify these','Nonfinite inputs and host finite-voltage clamp tested separately; oracle inputs are finite and host-bounded','Rust disconnected output frames are zeroed and retain audio-derived width; source disconnected storage is deliberately not exposed','Only the reported SDK/compiler/platform combination is numerically qualified'],'cases':[],'resonance_counterfactual_audits':[]}
    serde=args.serde_rlib
    if serde is None:
        built=subprocess.check_output(['cargo','build','--offline','--locked','-p','openpatch-core','--lib','--message-format=json'],cwd=ROOT,text=True)
        for line in built.splitlines():
            if line.startswith('{'):
                item=json.loads(line)
                if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name')=='serde_json':serde=next(Path(p) for p in item['filenames'] if p.endswith('.rlib'))
    assert serde is not None
    with tempfile.TemporaryDirectory(prefix='openpatch-stock-vcf-reference-') as directory:
        tmp=Path(directory);(tmp/'reference_module.hpp').write_text(prefix)
        cpp=tmp/'sdk_probe';rust=tmp/'rust_probe';flags=[]
        if sys.platform=='darwin':
            mac_sdk=Path(subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip());flags=['-isysroot',str(mac_sdk),'-isystem',str(mac_sdk/'usr/include/c++/v1')]
        subprocess.run(['c++','-std=c++17','-O2','-ffp-contract=off',*flags,'-I'+str(sdk/'include'),'-I'+str(sdk/'dep/include'),'-I'+str(tmp),str(HERE/'sdk_probe.cpp'),'-L'+str(sdk),'-lRack','-Wl,-rpath,'+str(sdk),'-o',str(cpp)],check=True)
        production=(ROOT/'crates/openpatch-core/src/stock_vcf.rs').read_text()
        accessors='\npub(crate) fn reference_seed(m:&mut StockVcf,a:u64,b:u64){m.rng=[a,b];m.draw();}\npub(crate) fn reference_state(m:&StockVcf)->[u64;2]{m.rng}\n'
        (tmp/'module.rs').write_text(production+accessors)
        (tmp/'probe.rs').write_text((HERE/'rust_probe.rs').read_text().replace('__STOCK_VCF_SOURCE__',json.dumps(str(tmp/'module.rs'))))
        subprocess.run(['rustc','--edition=2021','-O',str(tmp/'probe.rs'),'--extern','serde_json='+str(serde.resolve()),'-L','dependency='+str(serde.resolve().parent),'-o',str(rust)],check=True)
        env=os.environ.copy();env['DYLD_LIBRARY_PATH' if sys.platform=='darwin' else 'LD_LIBRARY_PATH']=str(sdk)
        for rate in args.rate or [44100,48000,96000]:
            for name in args.case or CASES:
                frames=CASES[name];data='\n'.join(vectors(name,frames,rate))+'\n'
                for seed in SEEDS if name=='self_oscillation' else SEEDS[:1]:
                    results=[subprocess.run([str(exe),str(rate),*seed],input=data,text=True,capture_output=True,check=True,env=env) for exe in [cpp,rust]]
                    lines=[r.stdout.splitlines() for r in results];assert len(lines[0])==len(lines[1])==frames
                    state=[list(map(int,r.stderr.split())) for r in results];failures=[];max_ratio=0.;worst={'absolute_error_volts':-1.};tracks=[[[] for _ in range(2)] for _ in range(2)]
                    for frame,(ref_line,got_line) in enumerate(zip(*lines)):
                        ref,got=[list(map(f32,map(float,line.split()))) for line in [ref_line,got_line]];assert len(ref)==len(got)==33
                        if not all(math.isfinite(v) for v in ref+got):raise AssertionError((name,rate,frame,'nonfinite actual source or Rust output'))
                        if ref[0]!=got[0]:failures.append({'frame':frame,'reason':'channel count'})
                        width=int(ref[0])
                        for out in range(2):
                            if name=='self_oscillation':tracks[0][out].append(ref[1+out*16]);tracks[1][out].append(got[1+out*16])
                            for lane in range(width):
                                index=1+out*16+lane;error=abs(ref[index]-got[index]);tolerance=0.002+1e-5*abs(ref[index]);max_ratio=max(max_ratio,error/tolerance)
                                if error>worst['absolute_error_volts']:worst={'frame_zero_based':frame,'output':out,'channel_zero_based':lane,'reference_volts':ref[index],'actual_volts':got[index],'absolute_error_volts':error,'tolerance_volts':tolerance}
                                if name!='self_oscillation' and error>tolerance and len(failures)<8:failures.append({'frame':frame,'output':out,'lane':lane,'reference':ref[index],'actual':got[index]})
                    case={'name':name,'sample_rate':rate,'frames':frames,'seed_words':seed,'input_sha256':hashlib.sha256(data.encode()).hexdigest(),'worst_sample':worst,'maximum_pointwise_tolerance_ratio':max_ratio,'reference_initial_rng_state':state[0][:2],'rust_initial_rng_state':state[1][:2],'reference_final_rng_state':state[0][2:4],'rust_final_rng_state':state[1][2:4],'processed_group_draws':state[0][4],'seed_warmup_draws':1}
                    if state[0]!=state[1]:failures.append({'reason':'RNG final state or group draw mismatch'})
                    if name=='self_oscillation':
                        case['metrics']=[];s=ACCEPTANCE['self_oscillation']
                        for out in range(2):
                            a,b=[metrics(t[out],rate) for t in tracks];ok=a['startup_seconds'] is not None and b['startup_seconds'] is not None and a['frequency_hz'] is not None and b['frequency_hz'] is not None
                            if ok:
                                ok &= abs(a['startup_seconds']-b['startup_seconds'])<=max(s['startup_absolute_seconds'],s['startup_relative']*a['startup_seconds'])
                                for key,tol in [('rms_volts',s['rms_relative']),('peak_volts',s['peak_relative']),('frequency_hz',s['frequency_relative'])]:ok &= abs(a[key]-b[key])<=tol*abs(a[key])
                            case['metrics'].append({'output':out,'reference':a,'actual':b,'passed':ok})
                            if not ok:failures.append({'reason':'self-oscillation metric failure','output':out})
                        if args.retain_traces:
                            args.retain_traces.mkdir(parents=True,exist_ok=True);case['raw_traces']={}
                            for label,result in zip(['sdk','rust'],results):
                                path=args.retain_traces/f'{name}-{rate}-{seed[0]}-{seed[1]}-{label}.txt';path.write_text(result.stdout);case['raw_traces'][label]={'path':str(path),'sha256':digest(path)}
                    case['passed']=not failures;case['first_failures']=failures[:8];report['cases'].append(case)
                    print(json.dumps({'case':name,'rate':rate,'seed':seed,'passed':case['passed'],'pointwise_ratio':max_ratio}),flush=True)
            if not args.case or 'resonance_cv_extremes' in args.case:
                report['resonance_counterfactual_audits'].extend(resonance_counterfactuals(cpp,rust,rate,env,args.retain_traces))
    report['passed']=all(c['passed'] for c in report['cases']) and all(a['passed'] for a in report['resonance_counterfactual_audits']);report['counterfactual_reference_frames']=sum(a['frames'] for a in report['resonance_counterfactual_audits']);report['total_frames']=sum(c['frames'] for c in report['cases']);args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps({'passed':report['passed'],'cases':len(report['cases']),'frames':report['total_frames'],'report':str(args.output)}));return 0 if report['passed'] else 1
if __name__=='__main__':raise SystemExit(main())
