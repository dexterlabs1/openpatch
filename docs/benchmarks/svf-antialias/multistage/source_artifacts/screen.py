#!/usr/bin/env python3
"""Original Blackman-sinc design and complete cascade spectral screen (MIT).
All frequencies are cycles per host sample. No dependency beyond NumPy.
"""
from pathlib import Path
import json
import numpy as np
ROOT=Path(__file__).resolve().parent

def kernel(n, rate, cutoff, halfband=False):
    t=np.arange(n)-(n-1)/2
    h=2*cutoff/rate*np.sinc(2*cutoff/rate*t)*np.blackman(n)
    if halfband:
        h[(t!=0)&(t%2==0)]=0
    h/=h.sum()
    return h

def response(h, rate, f):
    # Bounded working storage; screen is under one second on normal laptops.
    return np.exp(-2j*np.pi*np.asarray(f)[:,None]*np.arange(len(h))[None,:]/rate)@h

def db(v): return 20*np.log10(np.maximum(np.abs(v),1e-300))

def screen(lengths):
    rates=[2,4,8,16]
    hs=[kernel(n,r,.45 if r==2 else r/4,r!=2) for n,r in zip(lengths,rates)]
    wanted=np.linspace(0,18000/44100,2049)
    stages=[]
    for h,r in zip(hs,rates):
        protect=np.linspace(.5918367346938775 if r==2 else r/2-.5,r/2,2049)
        passdb=db(response(h,r,wanted))
        stopdb=db(response(h,r,protect))
        stages.append(dict(rate=r,taps=len(h),pass_min_db=float(passdb.min()),pass_max_db=float(passdb.max()),upper_fold_max_db=float(stopdb.max())))
    def cascade(f):
        out=np.ones(len(f),dtype=complex)
        for h,r in zip(hs,rates):out*=response(h,r,f)
        return out
    out=db(cascade(wanted))
    # The equivalent interpolation FIR is product H_i(f), upscaled to 16Fs;
    # zero insertion adds a common gain16. Compare unwanted images to desired.
    images=[]
    for k in range(1,9):
        f=np.linspace(max(.5,k-18000/44100),min(8,k+18000/44100),2049)
        images.append(dict(center=k,max_db=float(db(cascade(f)).max())))
    # Every internal frequency which aliases into desired host passband after
    # complete decimation, not just the individual protected upper folds.
    alias_max=max(x['max_db'] for x in images)
    return dict(lengths=lengths,latency=sum((n-1)/r for n,r in zip(lengths,rates)),stages=stages,roundtrip_pass_min_db=float(2*out.min()),roundtrip_pass_max_db=float(2*out.max()),complete_input_image_and_decimator_alias_bands=images,complete_wanted_alias_max_db=alias_max),hs

if __name__=='__main__':
    results=[]
    for lengths in [[129,33,17,33],[129,33,33,33]]:
        result,hs=screen(lengths);results.append(result)
    (ROOT/'frequency-screen.json').write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps(results,indent=2))
