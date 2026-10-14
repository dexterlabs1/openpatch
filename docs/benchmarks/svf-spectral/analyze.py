from pathlib import Path
import argparse,hashlib,json,subprocess
import numpy as np
parser=argparse.ArgumentParser(description="Analyze coherent actual-engine SVF traces; requires NumPy.")
parser.add_argument("--traces",type=Path,required=True)
parser.add_argument("--output",type=Path,required=True)
args=parser.parse_args()
assert not args.output.exists(), "Refusing to overwrite a report"
root=args.traces.resolve()
cases=json.loads((root/'cases.json').read_text()); N=8192
results=[]
for case in cases:
 raw=(root/case['trace']).read_bytes()
 assert len(raw)==2*N*3*4
 data=np.frombuffer(raw,dtype='<f4').reshape(2,N,3).astype(float)
 assert np.isfinite(data).all()
 wave=data[-1]
 fft=np.fft.rfft(wave,axis=0)/N
 energy=abs(fft)**2; energy[1:-1]*=2
 mask=np.zeros(N//2+1,dtype=bool);mask[0]=True;mask[np.arange(case['bin'],N//2+1,case['bin'])]=True
 rms=np.sqrt(energy[~mask].sum(axis=0))
 spectral_rms=np.sqrt(energy.sum(axis=0));time_rms=np.sqrt((wave**2).mean(axis=0))
 assert np.allclose(spectral_rms,time_rms,atol=1e-12,rtol=1e-12), 'Parseval calibration failed'
 results.append({**case,'trace_sha256':hashlib.sha256(raw).hexdigest(),'unwanted_rms_v':rms.tolist(),'unwanted_db_relative_5v_peak':(20*np.log10(np.maximum(rms,1e-15)/5)).tolist(),'period_difference_peak_v':abs(data[1]-data[0]).max(axis=0).tolist(),'signal_rms_v':time_rms.tolist()})
linear=[r for r in results if r['drive']==0]
assert max(max(r['unwanted_db_relative_5v_peak']) for r in linear)<-100, 'Linear calibration has excessive off-harmonic energy'
assert max(max(r['period_difference_peak_v']) for r in results)<1e-5, 'Unsettled/nonperiodic trace'
# Known FFT calibration: one legal tone plus an injected off-harmonic tone.
x=np.arange(N);probe=np.sin(2*np.pi*43*x/N)+0.01*np.sin(2*np.pi*157*x/N)
p=np.abs(np.fft.rfft(probe)/N)**2;p[1:-1]*=2
mask=np.ones(N//2+1,dtype=bool);mask[0]=False;mask[np.arange(43,N//2+1,43)]=False
assert abs(np.sqrt(p[mask].sum())-0.01/np.sqrt(2))<1e-12
report={'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'numpy':np.__version__,'method':'Actual v2 Engine oscillator -> gain -> SVF, no device or WAV quantization. Exactly representable coherent sine frequencies; last two of eight 8192-sample periods, FFT on last period. Sum energy outside DC and every in-band harmonic bin; Parseval weighting. Lower bound on alias energy: aliases coinciding with legitimate harmonics are excluded. Not a total harmonic distortion, hardware fidelity, modulation, or Rack comparison.','ports':['low','band','high'],'calibration':{'parseval':'passed all 144 cases','known_off_harmonic_injection':'passed','linear_off_harmonic_max_db_relative_5v_peak':max(max(r['unwanted_db_relative_5v_peak']) for r in linear),'max_period_difference_v':max(max(r['period_difference_peak_v']) for r in results)},'results':results}
args.output.write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report['calibration'],indent=2))
for rate in [44100,48000,96000]:
 rows=[r for r in results if r['rate']==rate and r['amplitude_volts_peak']==5 and r['drive']==1]
 worst=max(rows,key=lambda r:max(r['unwanted_db_relative_5v_peak']))
 print(rate,worst['frequency_hz'],worst['unwanted_db_relative_5v_peak'])
