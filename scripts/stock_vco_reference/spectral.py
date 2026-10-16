#!/usr/bin/env python3
"""Non-gating spectral diagnostics from retained stock VCO reference traces.

Usage: python3 spectral.py EVIDENCE_DIR REPORT_PATH [--numerical-report PATH]
       python3 spectral.py --self-test
No dependencies, subprocesses, synthesis, or oracle execution.
"""
import argparse
import cmath
import gzip
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

WAVES = ('sine', 'triangle', 'saw', 'square')


def canonical_f32(token):
    """Protocol decimals denote binary32; analysis itself uses Python doubles."""
    return struct.unpack('<f',struct.pack('<f',float(token)))[0]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024*1024), b''):
            h.update(block)
    return h.hexdigest()


def fft(values):
    """Unnormalized forward radix-2 DFT, standard negative exponent."""
    n = len(values)
    if n < 2 or n & (n-1):
        raise ValueError('FFT length must be a power of two >=2')
    result = [complex(v) for v in values]
    j = 0
    for i in range(1,n):
        bit = n >> 1
        while j & bit:
            j ^= bit
            bit >>= 1
        j ^= bit
        if i < j:
            result[i],result[j] = result[j],result[i]
    size = 2
    while size <= n:
        step = cmath.exp(-2j*math.pi/size)
        for start in range(0,n,size):
            twiddle = 1.+0j
            half = size//2
            for offset in range(half):
                even = result[start+offset]
                odd = twiddle*result[start+offset+half]
                result[start+offset] = even+odd
                result[start+offset+half] = even-odd
                twiddle *= step
        size *= 2
    return result


def measure(samples,rate,requested_hz):
    n = len(samples)
    if not all(math.isfinite(v) for v in samples):
        raise ValueError('Nonfinite selected output voltage')
    # Symmetric Hann, not periodic Hann; retain DC (no mean subtraction).
    window = [.5-.5*math.cos(2*math.pi*i/(n-1)) for i in range(n)]
    transformed = fft([v*w for v,w in zip(samples,window)])
    norm = n*sum(w*w for w in window)
    power = [abs(transformed[k])**2*(1 if k in (0,n//2) else 2)/norm
             for k in range(n//2+1)]
    bin_hz = rate/n
    expected_bin = requested_hz/bin_hz
    search_radius = max(2.,.02*expected_bin)
    low = max(1,math.ceil(expected_bin-search_radius))
    high = min(n//2-1,math.floor(expected_bin+search_radius))
    peak = max(range(low,high+1),key=lambda k:power[k])
    fractional = 0.
    if all(power[k]>0 for k in (peak-1,peak,peak+1)):
        a,b,c = [math.log(power[k]) for k in (peak-1,peak,peak+1)]
        denominator = a-2*b+c
        if denominator:
            fractional = max(-.5,min(.5,.5*(a-c)/denominator))
    fundamental = (peak+fractional)*bin_hz if power[peak]>0 else None
    # Fractional harmonic centers: include integer bins within ±3 bins.
    # Centers use each measured fundamental, not arbitrary free fitted peaks.
    harmonic_bins = set()
    harmonic_count = 0
    if fundamental:
        harmonic_count = math.floor((rate/2)/fundamental)
        for harmonic in range(1,harmonic_count+1):
            center = harmonic*fundamental/bin_hz
            harmonic_bins.update(range(max(4,math.ceil(center-3)),min(n//2,math.floor(center+3))+1))
    ac_bins = range(4,n//2+1)
    total = sum(power[k] for k in ac_bins)
    off = sum(power[k] for k in ac_bins if k not in harmonic_bins)
    fraction = off/total if total else None
    return {
        'requested_frequency_hz':requested_hz,
        'fundamental_estimate_hz':fundamental,
        'fundamental_peak_bin':peak if fundamental else None,
        'fundamental_search_bins_inclusive':[low,high],
        'frequency_resolution_hz':bin_hz,
        'sample_mean_volts':sum(samples)/n,
        'sample_rms_volts':math.sqrt(sum(v*v for v in samples)/n),
        'window_normalized_total_power_v2':sum(power),
        'excluded_dc_guard_power_v2':sum(power[:4]),
        'analyzed_ac_power_v2':total,
        'off_harmonic_power_v2':off,
        'off_harmonic_fraction_of_analyzed_ac':fraction,
        'off_harmonic_to_analyzed_ac_db':10*math.log10(fraction) if fraction and fraction>0 else None,
        'zero_off_harmonic_energy':off==0,
        'harmonic_centers_through_nyquist':harmonic_count,
        'harmonic_guard_bins_in_ac_band':len(harmonic_bins),
        'analyzed_ac_bins':len(ac_bins),
        'off_harmonic_bins':len(ac_bins)-len(harmonic_bins),
    }


def selected_samples(path,items,start,length,expected_rows):
    windows = sorted((item['start_row']+start,item['start_row']+start+length,item) for item in items)
    if any(a[1]>b[0] for a,b in zip(windows,windows[1:])):
        raise ValueError('Overlapping spectral windows in frozen manifest')
    samples = {item['name']:[[] for _ in WAVES] for item in items}
    h = hashlib.sha256()
    cursor = total = 0
    opener = gzip.open if path.suffix=='.gz' else open
    with opener(path,'rb') as stream:
        for index,line in enumerate(stream):
            total += 1
            h.update(line)
            while cursor<len(windows) and index>=windows[cursor][1]:
                cursor += 1
            if cursor==len(windows) or index<windows[cursor][0]:
                continue
            item = windows[cursor][2]
            tokens = line.split()
            if len(tokens) not in (218,2266):
                raise ValueError(f'{path.name} row{index}: invalid compact/checkpoint width')
            counts = [float(t) for t in tokens[:4]]
            if counts != [1.,1.,1.,1.]:
                raise ValueError(f'{path.name} row{index}: expected connected mono spectral outputs')
            for wave in range(4):
                value = canonical_f32(tokens[4+16*wave])
                if not math.isfinite(value):
                    raise ValueError(f'{path.name} row{index}: nonfinite spectral voltage')
                samples[item['name']][wave].append(value)
    if total!=expected_rows:
        raise ValueError(f'{path.name}: {total} rows, expected {expected_rows}')
    if any(len(channel)!=length for waves in samples.values() for channel in waves):
        raise ValueError(f'{path.name}: incomplete selected windows')
    return samples,{'rows':total,'uncompressed_sha256':h.hexdigest(),
                    'file_sha256':digest(path),'filename':path.name}


def qualification_context(original,result_path,revised_path=None):
    context={'original_declaration_passed':original.get('passed',False),
             'original_numerical_report_sha256':digest(result_path),
             'numerical_qualification_passed':original.get('passed',False),
             'numerical_report_sha256':digest(result_path),
             'revised_qualification':None}
    if revised_path is None:
        return context
    revised=json.loads(revised_path.read_text())
    for key in ('artifact_manifest_sha256','fixtures_sha256','production_source_sha256'):
        if key not in original or revised.get(key)!=original[key]:
            raise ValueError(f'Revised numerical report {key} differs from original evidence')
    if revised.get('original_failed_result_sha256')!=digest(result_path):
        raise ValueError('Revised numerical report does not identify this original result hash')
    if not revised.get('qualification_basis') or not revised.get('phase_endpoint_erratum_sha256'):
        raise ValueError('Revised numerical report lacks explicit versioned erratum qualification basis')
    # Bind the contextual qualification to the very same inputs and trace data,
    # in addition to its source and compressed-artifact closure.
    for phase in ('rates','safety_rates'):
        old={int(r['sample_rate']):r for r in original[phase]}
        new={int(r['sample_rate']):r for r in revised[phase]}
        if set(old)!=set(new) or len(new)!=len(revised[phase]):
            raise ValueError(f'Revised report {phase} sample-rate coverage differs')
        for rate in old:
            for key in ('input_sha256','trace_sha256','frames'):
                if new[rate].get(key)!=old[rate][key]:
                    raise ValueError(f'Revised report {phase}/{rate}/{key} differs from original')
    if revised.get('kernel',{}).get('trace_sha256')!=original['kernel']['trace_sha256']:
        raise ValueError('Revised kernel trace hashes differ from original evidence')
    context.update({'numerical_qualification_passed':revised.get('passed',False),
                    'numerical_report_sha256':digest(revised_path),
                    'revised_qualification':{
                        'passed':revised.get('passed',False),
                        'report_path':str(revised_path.resolve()),
                        'report_sha256':digest(revised_path),
                        'basis':revised['qualification_basis'],
                        'phase_endpoint_erratum_sha256':revised['phase_endpoint_erratum_sha256'],
                        'revalidation_driver_sha256':revised.get('revalidation_driver_sha256',{}),
                        'same_artifact_fixture_production_input_trace_hashes_verified':True}})
    return context


def analyze(evidence,numerical_report=None):
    acceptance_path,manifest_path = evidence/'acceptance.json',evidence/'fixtures.json'
    a = json.loads(acceptance_path.read_text())
    manifest = json.loads(manifest_path.read_text())
    if digest(acceptance_path)!=manifest['acceptance_sha256']:
        raise ValueError('Retained acceptance hash differs from frozen manifest')
    if digest(evidence/'fixtures.py')!=manifest['generator_sha256']:
        raise ValueError('Retained generator hash differs from frozen manifest')
    if digest(evidence/'VCO.cpp')!=a['reference_source_sha256']:
        raise ValueError('Retained reference source hash differs from declaration')
    result_path = evidence/'result.json'
    numerical = json.loads(result_path.read_text())
    artifacts = json.loads((evidence/'artifact-sha256.json').read_text())
    if digest(evidence/'artifact-sha256.json')!=numerical['artifact_manifest_sha256']:
        raise ValueError('Artifact manifest hash differs from retained numerical report')
    start,length = a['fixture_plan']['spectral_window_start'],a['fixture_plan']['spectral_window_length']
    if (start,length)!=(4096,8192):
        raise ValueError('Expected frozen spectral window [4096,12288)')
    report = {
        'diagnostic_only':True,'numerical_gate_changed':False,
        **qualification_context(numerical,result_path,numerical_report),
        'evidence_directory':str(evidence),
        'spectral_script_sha256':digest(Path(__file__).resolve()),
        'python':sys.version,'reference_source_sha256':digest(evidence/'VCO.cpp'),
        'production_source_sha256':numerical.get('production_source_sha256',{}),
        'acceptance_sha256':digest(acceptance_path),'fixtures_sha256':digest(manifest_path),
        'artifact_manifest_sha256':digest(evidence/'artifact-sha256.json'),
        'window':{'first_case_frame':start,'end_case_frame_exclusive':start+length,
                  'samples':length,'function':'symmetric Hann: 0.5-0.5*cos(2*pi*n/(N-1))',
                  'mean_subtracted':False,'waveforms':list(WAVES),'lane':0},
        'metric_definition':{
            'input_decoding':'decimal trace tokens rounded to canonical IEEE754 binary32 before promotion to Python double for windowing and FFT; serializer decimal precision is not measured signal',
            'power':'one-sided |DFT(x*window)|^2/(N*sum(window^2)); double interior bins, retain DC/Nyquist once; units V^2',
            'fundamental':'largest bin near requested frequency within max(2 bins,2% requested); log-power parabolic interpolation using its immediate neighbors, limited ±0.5bin; null if local power is zero; this is a local frequency estimate, not general pitch detection',
            'dc_policy':'exclude bins0..3 inclusive from analyzed AC denominator and numerator; separately report their energy; no subtraction of sample mean',
            'harmonic_policy':'positive integer multiples of each trace waveform measured fundamental through Nyquist; union integer bins within ±3 bins of each fractional center; no folded harmonics',
            'off_harmonic':'analyzed AC power outside that union; fraction and10log10 ratio relative to all analyzed AC power; null ratio if denominator zero and null dB if numerator zero',
            'gate':'none; every valid measured value is reported, with no threshold or SDK-vs-Rust spectral pass/fail'},
        'limitations':[
            'Finite Hann-window leakage remains outside harmonic guards; this energy is not a direct alias-power measurement.',
            'Aliases coinciding with harmonic guards are counted as harmonic; low frequencies and overlapping guards can leave few off-harmonic bins.',
            'No universal aliasing, audibility, hardware, host, or superiority claim follows from these bounded cases.',
            'Only lane0 of the three stationary spectrum cases and declared later windows is analyzed; all startup frames remain part of the separate numerical qualification.',
            'An interpolated local peak estimates frequency; DC filtering, reconstruction, window leakage and nearby content can bias it.'
        ],'traces':[],'measurements':[]}
    numeric_rates = {int(r['sample_rate']):r for r in numerical['rates']}
    for rate in a['sample_rates']:
        phase = manifest['rates'][str(rate)]['reference']
        items = [item for item in phase['case_manifest'] if item.get('spectral_hz') is not None]
        if sorted(item['spectral_hz'] for item in items)!=sorted(a['fixture_plan']['spectral_frequencies_hz']):
            raise ValueError(f'{rate}: missing/duplicate frozen spectral cases')
        for item in items:
            if item['frames']<start+length:
                raise ValueError('Spectral window exceeds case length')
        for label in ('sdk','rust'):
            path = evidence/f'reference-{label}-{rate}.txt.gz'
            if not path.exists():
                path = path.with_suffix('')
            sample_map,trace = selected_samples(path,items,start,length,phase['observed_frames'])
            if artifacts.get(path.name)!=trace['file_sha256']:
                raise ValueError(f'{path.name}: file hash differs from retained artifact manifest')
            if numeric_rates[rate]['trace_sha256'][label]!=trace['uncompressed_sha256']:
                raise ValueError(f'{path.name}: uncompressed hash differs from numerical evidence')
            trace.update({'implementation':label,'sample_rate':rate})
            report['traces'].append(trace)
            for item in items:
                for wave,name in enumerate(WAVES):
                    measurement=measure(sample_map[item['name']][wave],rate,item['spectral_hz'])
                    measurement.update({'implementation':label,'sample_rate':rate,
                        'case':item['name'],'waveform':name,'lane':0,
                        'trace_first_row':item['start_row']+start,
                        'trace_end_row_exclusive':item['start_row']+start+length})
                    report['measurements'].append(measurement)
    report['completed']=True
    return report


def self_test():
    # Mathematical FFT tests only, not a VCO synthesis/reference implementation.
    n=32
    impulse=fft([1.]+[0.]*(n-1))
    assert max(abs(v-1) for v in impulse)<1e-12
    constant=fft([2.]*n)
    assert abs(constant[0]-2*n)<1e-12 and max(abs(v) for v in constant[1:])<1e-12
    values=[complex((i%7)-3,(i%3)-1) for i in range(n)]
    actual=fft(values)
    direct=[sum(v*cmath.exp(-2j*math.pi*k*i/n) for i,v in enumerate(values)) for k in range(n)]
    assert max(abs(x-y) for x,y in zip(actual,direct))<1e-10
    assert abs(sum(abs(v)**2 for v in actual)/n-sum(abs(v)**2 for v in values))<1e-10
    n=8192;rate=48000;hz=128*rate/n
    result=measure([math.sin(2*math.pi*128*i/n) for i in range(n)],rate,hz)
    assert abs(result['fundamental_estimate_hz']-hz)<.001
    assert abs(result['window_normalized_total_power_v2']-.5)<1e-8
    assert result['off_harmonic_fraction_of_analyzed_ac']<1e-8
    zero=measure([0.]*n,rate,hz)
    assert zero['fundamental_estimate_hz'] is None and zero['off_harmonic_fraction_of_analyzed_ac'] is None
    assert float('1.23456788')!=float('1.234567881')
    assert canonical_f32('1.23456788')==canonical_f32('1.234567881')
    assert struct.pack('<f',canonical_f32('-0.0'))==struct.pack('<f',-0.)
    return {'self_test_passed':True,'checks':['impulse','DC','direct DFT','Parseval','bin-centered sine','zero signal','canonical f32 decimal decoding'],
            'actual_traces_read':False,'oracle_executed':False}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('evidence',type=Path,nargs='?')
    parser.add_argument('report',type=Path,nargs='?')
    parser.add_argument('--self-test',action='store_true')
    parser.add_argument('--numerical-report',type=Path,
                        help='Optional corrected erratum qualification report for this same immutable evidence')
    args=parser.parse_args()
    if args.self_test:
        print(json.dumps(self_test(),indent=2));return 0
    if args.evidence is None or args.report is None:
        parser.error('Provide EVIDENCE_DIR and REPORT_PATH, or --self-test')
    evidence=args.evidence.resolve();target=args.report.resolve()
    if target==evidence or evidence in target.parents:
        parser.error('REPORT_PATH must be outside the immutable evidence directory')
    if args.numerical_report is not None and target==args.numerical_report.resolve():
        parser.error('REPORT_PATH must not overwrite the numerical qualification report')
    try:
        report=analyze(evidence,args.numerical_report)
    except Exception as error:
        report={'completed':False,'diagnostic_only':True,'numerical_gate_changed':False,
                'error':{'type':type(error).__name__,'message':str(error)},
                'spectral_script_sha256':digest(Path(__file__).resolve())}
    target.parent.mkdir(parents=True,exist_ok=True)
    target.write_text(json.dumps(report,indent=2,sort_keys=True,allow_nan=False)+'\n')
    print(json.dumps({'completed':report['completed'],'report':str(target),
                      'measurements':len(report.get('measurements',[]))}))
    return 0 if report['completed'] else 1


if __name__=='__main__':
    raise SystemExit(main())
