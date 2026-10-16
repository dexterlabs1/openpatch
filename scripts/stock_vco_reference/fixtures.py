#!/usr/bin/env python3
"""Frozen stock VCO input schedules, metadata and schema validation. No DSP/oracle."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

HERE = Path(__file__).resolve().parent
DEFAULT = [0., 0., .5, 0., 0., 0.]


def f32(v):
    return struct.unpack('<f', struct.pack('<f', v))[0]


def bits(v):
    return struct.unpack('<I', struct.pack('<f', v))[0]


def from_bits(v):
    return struct.unpack('<f', struct.pack('<I', v))[0]


def port(width=0, value=0.):
    values = list(value) if isinstance(value, (list, tuple)) else [value] * width
    assert 0 <= width <= 16 and len(values) >= width
    return [width, *map(f32, values[:width]), *([0.] * (16-width))]


def frame(params=None, pitch=None, fm=None, sync=None, pwm=None, mask=15, event=0, bypass=0):
    return [event, *map(f32, DEFAULT if params is None else params),
            *(port() if pitch is None else pitch), *(port() if fm is None else fm),
            *(port() if sync is None else sync), *(port() if pwm is None else pwm), mask, bypass, 0]


def segment(n, **kwargs):
    assert n > 0
    return {'repeat': n, 'fields': frame(**kwargs)}


def case(name, segments, tags, anchors=None, spectral_hz=None):
    return {'name': name, 'segments': segments, 'tags': tags, 'anchors': anchors or [],
            'spectral_hz': spectral_hz}


def modes():
    for linear in [0, 1]:
        for soft in [0, 1]:
            yield linear, soft


def cases(rate, a):
    plan = a['fixture_plan']
    for linear, soft in modes():
        for frequency in plan['static_frequencies']:
            for width in plan['widths']:
                yield case(f'static_l{linear}s{soft}_f{frequency}_w{width}',
                    [segment(plan['static_frames'], params=[frequency, 0, .5, 0, linear, soft],
                             pitch=port(width, [i / 16 for i in range(16)]))],
                    ['full_startup', 'frequency_range', 'four_modes', 'widths', 'filter_tails', 'lamps'])
    for linear, soft in modes():
        cycle_frames=math.ceil(rate/(f32(261.6256)*2**(-76/12)))+65
        yield case(f'low_frequency_full_cycle_l{linear}s{soft}',
            [segment(cycle_frames,params=[-76,0,.5,0,linear,soft],pitch=port(16)),
             segment(65,params=[0,1,.5,0,1,soft],pitch=port(16),fm=port(1,-1.))],
            ['minimum_knob_full_cycle','four_modes','all_voices','full_reconstruction_tail'],
            [{'frame':cycle_frames+64,'tails_zero':True}])
    for width in range(17):
        for linear in [0, 1]:
            yield case(f'fm_width{width}_linear{linear}',
                [segment(65, params=[0, amount, .5, 0, linear, 0], pitch=port(width),
                         fm=port(aux, [(-1 if i % 2 else 1) * (i+1) / 2 for i in range(16)]))
                 for aux, amount in [(0, 1), (1, -1), (3, .375), (16, 1)]],
                ['all_widths', 'signed_fm', 'aux_mono_short_poly', 'zero_frequency'])
    for linear, soft in modes():
        for width in [.01, .1, .5, .9, .99]:
            yield case(f'moving_pwm_l{linear}s{soft}_pw{width}',
                [segment(1, params=[54, .5, width, 1, linear, soft], pitch=port(5),
                    fm=port(3, [0, -.5, .5]),
                    pwm=port(1 if tick % 17 < 8 else 3,
                             [16. if (tick+i) % 4 < 2 else -16. for i in range(3)]),
                    sync=port(1, -3. if tick % 13 < 9 else 7.)) for tick in range(257)] +
                [segment(65, params=[0, 1, width, 0, 1, soft], pitch=port(5), fm=port(1,-1.))],
                ['audio_rate_pwm', 'narrow_pulses', 'moving_threshold', 'clamps', 'sync_same_frame', 'tail_drain'],
                [{'frame': 321, 'tails_zero': True}])
    sync_levels = [3., 0., 3., -3., 0., 3., -1., 3., -3., 1., -1., 1.,
                   -1., from_bits(1), 0., -3., 3., 3., -3., -3.]
    for soft in [0, 1]:
        for width in [1, 3, 5, 16]:
            for sync_width in [1, 2, 16]:
                parts = [segment(1, params=[54, .25, .01 if tick % 2 else .99, .5, 0, soft],
                    pitch=port(width, [i/16 for i in range(16)]), fm=port(1,.5),
                    sync=port(sync_width, [sync_levels[(tick+i*3) % len(sync_levels)] for i in range(16)]),
                    pwm=port(1, 2. if tick % 3 else -2.)) for tick in range(160)]
                parts += [segment(17, params=[54, 0, .5, 0, 0, soft], pitch=port(width)),
                          segment(17, params=[54, 0, .5, 0, 0, soft], pitch=port(width),sync=port(sync_width,3.))]
                yield case(f'sync_s{soft}_w{width}_aux{sync_width}',parts,
                    ['fractional_sync', 'endpoint_masks', 'startup_high', 'falling_zero', 'sync_disconnect', 'short_poly'])
    # Endpoint reset is analytical and independent of waveform reconstruction.
    for soft in [0,1]:
        yield case(f'endpoint_anchor_soft{soft}',
            [segment(1, params=[0,1,.5,0,1,soft],fm=port(1,-1.),sync=port(1,-1.)),
             segment(1, params=[0,1,.5,0,1,soft],fm=port(1,-1.),sync=port(1,0.)),
             segment(33, params=[0,1,.5,0,1,soft],fm=port(1,-1.),sync=port(1,0.))],
            ['zero_frequency', 'sync_endpoint', 'analytical_phase'],
            [{'frame':0,'phase_zero':list(range(16))}, {'frame':1,'phase_zero':list(range(16)),
                'direction_lanes':{'0':-1. if soft else 1.}}])
    exponents = [-127.,-126.99999237060547,-126.,-32.,-1.,0.,.5,1.,8.,16.,30.,100.,127.,127.99998474121094,128.,128.99998474121094]
    for linear, soft in modes():
        yield case(f'exponent_endpoints_l{linear}s{soft}',
            [segment(129,params=[0,0,.5,0,linear,soft],pitch=port(16,exponents))],
            ['defined_exp2_boundaries', 'zero_and_inf', 'sample_rate_clamp', 'delta_049_cap'])
    for widths in [(3,1,3),(5,4,5),(16,1,16)]:
        for soft in [0,1]:
            parts=[]
            for stage,width in enumerate(widths):
                for tick in range(65):
                    parts.append(segment(1,params=[48,.25,.4,.5,0,soft],pitch=port(width,[i/8 for i in range(16)]),
                        fm=port(16,[(i-8)/4 for i in range(16)]),
                        sync=port(16,[-1. if (tick+stage*7+i)%11<7 else 3. for i in range(16)]),
                        pwm=port(16,[8. if (tick+i)%3 else -8. for i in range(16)])))
            yield case(f'quartet_{widths[0]}_{widths[1]}_{widths[2]}_soft{soft}',parts,
                ['retained_hidden_lanes', 'visible_only_correction_insertion', 'group_freeze', 'regrowth'])
    for width in [1,3,5,16]:
        for soft in [0,1]:
            parts=[]
            for mask in [15,0,1,2,4,8,5,10,0,15]:
                parts.append(segment(37,params=[54,.3,.2,.8,0,soft],pitch=port(width),
                    fm=port(1,.5),pwm=port(1,2.),sync=port(1,-1.),mask=mask))
                parts.append(segment(1,params=[54,.3,.8,-.8,0,soft],pitch=port(width),
                    fm=port(1,.5),pwm=port(1,2.),sync=port(1,3.),mask=mask))
            yield case(f'consumer_history_w{width}_s{soft}',parts,
                ['per_wave_cursor_freeze', 'reconnect_first_sample', 'pending_tail_retention', 'all_masks'])
    for width in [1,5,16]:
        parts=[]
        for tick in range(257):
            parts.append(segment(1,params=[48 if tick<128 else 54,.5,.1 if tick%2 else .9,.5,
                    int(tick>=160),int(tick>=64)],pitch=port(width),fm=port(1,.25),
                sync=port(1,-2. if tick%7<4 else 3.),pwm=port(1,4. if tick%3 else -4.),
                event=2 if tick in [15,31,63,95,128,159,191] else 3 if tick in [16,32,64,96,129,160,192] else 0,
                bypass=int(100<=tick<117 or 200<=tick<233)))
        yield case(f'lifecycle_w{width}',parts,['initialize_noop','explicit_adoption','receiving_params','bypass_host_shim'])
    # Zero frequency from exact C4 + (-1)*C4 cancellation; no oscillator mirror.
    yield case('fresh_zero_frequency', [segment(97,params=[0,1,.5,0,1,0],pitch=port(16),fm=port(1,-1.))],
        ['zero_frequency','analytical_phase','no_discontinuities'],
        [{'frame':i,'phase_zero':list(range(16)),'tails_zero':True} for i in [0,1,31,32,64,96]])
    for width in [1,5,16]:
        yield case(f'drain_and_reconnect_w{width}',
            [segment(97,params=[54,0,.01,0,0,0],pitch=port(width)),
             segment(65,params=[0,1,.01,0,1,0],pitch=port(width),fm=port(1,-1.)),
             segment(33,params=[0,1,.01,0,1,0],pitch=port(width),fm=port(1,-1.),mask=0),
             segment(33,params=[0,1,.01,0,1,0],pitch=port(width),fm=port(1,-1.))],
            ['full_reconstruction_tail','zero_stop','disconnect_after_drain'],
            [{'frame':161,'tails_zero':True},{'frame':227,'tails_zero':True}])
    for hz in plan['spectral_frequencies_hz']:
        frequency=f32(12*math.log2(hz/f32(261.6256)))
        yield case(f'spectrum_{hz}Hz', [segment(plan['spectral_frames'],params=[frequency,0,.5,0,0,0])],
            ['separate_spectrum','full_startup_compared'],spectral_hz=hz)


def safety_cases(rate,a):
    extreme=[-1e6,-256.,-128.,-127.00001525878906,129.,255.,256.,1e6,
             from_bits(0x7f7fffff),from_bits(0xff7fffff),float('nan'),float('inf'),-float('inf'),0.,1.,-1.]
    for linear,soft in modes():
        yield case(f'safety_l{linear}s{soft}',
            [segment(65,params=[0,1,.5,1,linear,soft],pitch=port(16,extreme),fm=port(16,extreme),
                     sync=port(16,extreme),pwm=port(16,extreme)),
             segment(65,params=[0,0,.5,0,linear,soft],pitch=port(16),event=3),
             segment(33,params=[0,0,.5,0,linear,soft],pitch=port(1),event=2)],
            ['rust_only','outside_exp_domain','nonfinite_inputs','defined_finite_policy','recovery'])


def expand(schedules):
    for item in schedules:
        total=sum(part['repeat'] for part in item['segments'])
        checkpoint={0,1,15,16,31,32,63,64,total-1}
        checkpoint.update(anchor['frame'] for anchor in item['anchors'])
        index=0
        previous=None
        for part in item['segments']:
            for repeat in range(part['repeat']):
                row=list(part['fields'])
                row[0]=1 if index==0 else (row[0] if repeat==0 else 0)
                transition=previous is not None and any(row[k]!=previous[k] for k in [7,75,76])
                row[77]=int(index in checkpoint or row[0] in [2,3] or transition)
                yield item['name'],index,row
                previous=row
                index+=1


def rows(rate,a):
    return expand(cases(rate,a))


def safety_rows(rate,a):
    return expand(safety_cases(rate,a))


def row_text(row):
    def token(v):
        if isinstance(v,int): return str(v)
        if math.isnan(v): return 'NaN'
        if math.isinf(v): return '-inf' if v<0 else 'inf'
        return format(v,'.9g')
    return ' '.join(map(token,row))+'\n'


def fixture_text(rate,a,safety=False):
    return ''.join(row_text(row) for _,_,row in (safety_rows if safety else rows)(rate,a))


def safety_fixture_text(rate,a):
    return fixture_text(rate,a,True)


def validate_row(row,safety=False):
    assert len(row)==78 and row[0] in [0,1,2,3] and 0<=row[75]<=15 and row[76] in [0,1] and row[77] in [0,1]
    assert -76<=row[1]<=76 and -1<=row[2]<=1 and f32(.01)<=row[3]<=f32(.99) and -1<=row[4]<=1
    assert row[5] in [0,1] and row[6] in [0,1]
    for start in [7,24,41,58]:
        width=row[start]
        assert isinstance(width,int) and 0<=width<=16
        assert all(bits(v)==0 for v in row[start+1+width:start+17])
    if safety: return
    assert all(math.isfinite(v) for v in row)
    def poly(start,lane):
        width=row[start]
        return row[start+1] if width==1 else row[start+1+lane]
    for lane in range(((max(1,row[7])+3)//4)*4):
        exponent=f32(f32(row[1]/12)+poly(7,lane))
        if not row[5]: exponent=f32(exponent+f32(poly(24,lane)*row[2]))
        assert 0<=f32(exponent+127)<256,(exponent,row)


def digest_file(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest(a):
    result={'declaration':'Frozen before first numerical oracle comparison; immutable generator and acceptance',
        'acceptance_sha256':digest_file(HERE/'acceptance.json'),'generator_sha256':digest_file(HERE/'fixtures.py'),'rates':{}}
    for rate in a['sample_rates']:
        result['rates'][str(rate)]={}
        for phase,factory,iterator in [('reference',cases,rows),('safety',safety_cases,safety_rows)]:
            digest=hashlib.sha256();count=checkpoints=0
            for _,_,row in iterator(rate,a):
                validate_row(row,phase=='safety');digest.update(row_text(row).encode());count+=1;checkpoints+=row[77]
            items=[];cursor=0
            for item in factory(rate,a):
                length=sum(p['repeat'] for p in item['segments'])
                items.append({'name':item['name'],'frames':length,'start_row':cursor,'end_row_exclusive':cursor+length,
                    'tags':item['tags'],'anchors':item['anchors'],'spectral_hz':item['spectral_hz']});cursor+=length
            assert cursor==count and len({i['name'] for i in items})==len(items)
            if phase=='reference': assert count<=a['fixture_plan']['max_reference_frames_per_rate']
            result['rates'][str(rate)][phase]={'cases':len(items),'processed_frames':count,'observed_frames':count,
                'checkpoints':checkpoints,'input_sha256':digest.hexdigest(),'case_manifest':items}
    return result


def check_declaration():
    a=json.loads((HERE/'acceptance.json').read_text());expected=manifest(a)
    assert json.loads((HERE/'fixtures.json').read_text())==expected,'Frozen declaration changed'
    return expected


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--freeze',action='store_true');parser.add_argument('--check',action='store_true');args=parser.parse_args()
    a=json.loads((HERE/'acceptance.json').read_text());expected=manifest(a);target=HERE/'fixtures.json'
    if args.freeze:
        with target.open('x') as out: out.write(json.dumps(expected,indent=2,sort_keys=True)+'\n')
    else:
        assert args.check,'Specify --freeze or --check'
        assert json.loads(target.read_text())==expected,'Frozen declaration changed'
    print(json.dumps({'fixtures_sha256':digest_file(target),'generator_sha256':expected['generator_sha256'],
        'rates':{r:{p:{k:v for k,v in d.items() if k!='case_manifest'} for p,d in phases.items()} for r,phases in expected['rates'].items()}},indent=2))


if __name__=='__main__': main()
