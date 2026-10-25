#!/usr/bin/env python3
"""Write cases.json for the VCA-2 and Unity reference runner.

Original OpenPatch harness, MIT licensed. The cases are declarative; run.py
expands each signal into exact f32 bit patterns that both probes read.
"""
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
VCA_PORTS = ['exp1', 'lin1', 'in1', 'exp2', 'lin2', 'in2']
UNITY_PORTS = [f'ch{s}_{j}' for s in (1, 2) for j in range(1, 7)]
WIDTHS = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16]


def ramp(lo, hi, lane_step=0.0):
    return {'ramp': [lo, hi], 'lane_step': lane_step}


def const(*values):
    return {'const': list(values)}


def vca_cases():
    cases = []

    def case(id, purpose, events, rate=48000):
        cases.append({'id': id, 'module': 'vca', 'rates': [rate], 'purpose': purpose, 'events': events})

    case('vca_unpatched_defaults', 'Nothing patched: each channel is one voice of 0 V at 100% level.',
         [{'frames': 4, 'levels': [1, 1], 'inputs': {}}])
    events = []
    for level in [0.0, 1e-40, 1e-30, 0.25, 0.333333, 0.5, 0.75, 0.999999, 1.0]:
        events.append({'frames': 3, 'levels': [level, 1 - level],
                       'inputs': {'in1': {'channels': 16, 'signal': ramp(-10, 10, 0.37)},
                                  'in2': {'channels': 3, 'signal': const(5, -5, 1e-39)}}})
    case('vca_level_knob', 'Level knob alone, including a subnormal level the engine flushes.', events)
    for cv in ['lin', 'exp']:
        events = []
        for in_w in range(0, 17):
            for cv_w in WIDTHS:
                events.append({'frames': 2, 'levels': [0.8, 0.6], 'inputs': {
                    'in1': {'channels': in_w, 'signal': ramp(-9, 9, 0.61)},
                    f'{cv}1': {'channels': cv_w, 'signal': ramp(-2, 12, 0.93)},
                    'in2': {'channels': cv_w, 'signal': ramp(3, -3, 0.5)},
                    f'{cv}2': {'channels': in_w, 'signal': ramp(11, -1, -0.71)}}})
        case(f'vca_{cv}_widths', f'{cv.upper()} CV widths against every IN width: mono CV applies to all voices, '
             'a polyphonic CV is read per voice and missing voices are muted; width follows IN only.', events)
    events = []
    for in_w in [1, 4, 16]:
        for lin_w in WIDTHS:
            for exp_w in WIDTHS:
                events.append({'frames': 1, 'levels': [1, 0.5], 'inputs': {
                    'in1': {'channels': in_w, 'signal': ramp(-10, 10, 1.1)},
                    'lin1': {'channels': lin_w, 'signal': ramp(0, 10, 0.4)},
                    'exp1': {'channels': exp_w, 'signal': ramp(10, 0, 0.3)},
                    'in2': {'channels': exp_w, 'signal': ramp(10, -10, 0.2)},
                    'lin2': {'channels': in_w, 'signal': ramp(5, 9, 0.1)},
                    'exp2': {'channels': lin_w, 'signal': ramp(2, 8, -0.2)}}})
    case('vca_lin_and_exp_together', 'Linear then exponential CV on the same channel, all width combinations.', events)
    case('vca_lin_mono_sweep', 'Mono linear CV swept from -5 V to 15 V, through both clamp limits.',
         [{'frames': 8000, 'levels': [1, 0.7], 'inputs': {
             'in1': {'channels': 2, 'signal': const(10, -3.3)}, 'lin1': {'channels': 1, 'signal': ramp(-5, 15)},
             'in2': {'channels': 1, 'signal': const(7.5)}, 'lin2': {'channels': 1, 'signal': ramp(15, -5)}}}])
    case('vca_exp_mono_sweep', 'Mono exponential CV (powf) swept from -5 V to 15 V.',
         [{'frames': 20000, 'levels': [1, 0.9], 'inputs': {
             'in1': {'channels': 3, 'signal': const(10, -10, 1)}, 'exp1': {'channels': 1, 'signal': ramp(-5, 15)},
             'in2': {'channels': 1, 'signal': const(-6)}, 'exp2': {'channels': 1, 'signal': ramp(10.5, -0.5)}}}])
    case('vca_exp_poly_sweep', 'Polyphonic exponential CV (SSE exp approximation) densely swept over 0-10 V on 16 voices.',
         [{'frames': 20000, 'levels': [1, 1], 'inputs': {
             'in1': {'channels': 16, 'signal': const(10)}, 'exp1': {'channels': 16, 'signal': ramp(-0.5, 10.5, 1 / 32768)},
             'in2': {'channels': 16, 'signal': const(-10)}, 'exp2': {'channels': 16, 'signal': ramp(10.5, -0.5, -1 / 65536)}}}])
    specials = ['00000000', '80000000', '00000001', '80000001', '007fffff', '00800000', '00800001', '3f800000',
                '41200000', '41200001', '411fffff', 'c1200000', '0d000000', '1e3ce508', '33d6bf95', '3dcccccd',
                '40a00000', '49742400', 'c9742400', '42c80000']
    events = []
    for cv_w in [1, 2]:
        for value in specials:
            events.append({'frames': 1, 'levels': [1, 1], 'inputs': {
                'in1': {'channels': 4, 'signal': {'bits': ['41200000', 'c1200000', '00000001', '80000000']}},
                'lin1': {'channels': cv_w, 'signal': {'bits': [value]}},
                'in2': {'channels': 4, 'signal': {'bits': ['41200000', 'c1200000', '3f800000', '80000000']}},
                'exp2': {'channels': cv_w, 'signal': {'bits': [value]}}}})
            events.append({'frames': 1, 'levels': [0.5, 0.5], 'inputs': {
                'in1': {'channels': cv_w, 'signal': {'bits': [value]}},
                'exp1': {'channels': 1, 'signal': const(7)},
                'in2': {'channels': cv_w, 'signal': {'bits': [value]}},
                'lin2': {'channels': 3, 'signal': const(-0.0, 3, 10)}}})
    case('vca_special_values', 'Signed zero, subnormal, tiny, boundary and large CV and signal bit patterns.', events)
    case('vca_audio_rate', 'Audio-rate signal and CV on all 16 voices of both channels.',
         [{'frames': 9600, 'levels': [0.9, 0.4], 'inputs': {
             'in1': {'channels': 16, 'signal': {'sine': {'freq': 440, 'amp': 5, 'offset': 0, 'lane_freq_step': 17}}},
             'lin1': {'channels': 16, 'signal': {'sine': {'freq': 3, 'amp': 6, 'offset': 5, 'lane_freq_step': 0.5}}},
             'exp1': {'channels': 1, 'signal': {'sine': {'freq': 7, 'amp': 6, 'offset': 4}}},
             'in2': {'channels': 9, 'signal': {'noise': {'seed': 7, 'amp': 10, 'offset': 0}}},
             'lin2': {'channels': 1, 'signal': {'sine': {'freq': 1000, 'amp': 5, 'offset': 5}}},
             'exp2': {'channels': 9, 'signal': {'noise': {'seed': 11, 'amp': 6, 'offset': 5}}}}}])
    events = []
    for width in [16, 1, 5, 16, 0, 3]:
        events.append({'frames': 50, 'levels': [1, 0.3], 'inputs': {
            'in1': {'channels': width, 'signal': {'noise': {'seed': width + 1, 'amp': 10, 'offset': 0}}},
            'exp1': {'channels': max(width - 2, 0), 'signal': {'noise': {'seed': 99, 'amp': 6, 'offset': 5}}},
            'in2': {'channels': 16 - width, 'signal': {'noise': {'seed': 5, 'amp': 10, 'offset': 0}}},
            'lin2': {'channels': width, 'signal': {'noise': {'seed': 6, 'amp': 7, 'offset': 4}}}}})
    case('vca_width_changes', 'Voice counts shrinking and growing between frames.', events)
    return cases


def unity_cases():
    cases = []
    rates = [44100, 48000, 96000]

    def case(id, purpose, events, rate_list=rates):
        cases.append({'id': id, 'module': 'unity', 'rates': rate_list, 'purpose': purpose, 'events': events})

    case('unity_unpatched', 'Nothing patched: +0 V mixes, -0 V inverses, lights stay dark across divider cycles.',
         [{'frames': 600, 'average': [0, 0], 'inputs': {}}])
    events = []
    for index, port in enumerate(UNITY_PORTS):
        events.append({'frames': 3, 'average': [index % 2, (index // 2) % 2],
                       'inputs': {port: {'channels': 1, 'signal': const(index + 0.5)}}})
    case('unity_each_input', 'Each input alone reaches only its own section.', events, [48000])
    events = []
    for mask in range(64):
        for average in ([0, 0], [1, 1]):
            inputs = {}
            for bit in range(6):
                if mask & (1 << bit):
                    inputs[UNITY_PORTS[bit]] = {'channels': 1, 'signal': const((bit + 1) * 1.25 - 4)}
                if (63 - mask) & (1 << bit):
                    inputs[UNITY_PORTS[6 + bit]] = {'channels': 1 + bit, 'signal': const(-(bit + 1) * 0.7)}
            events.append({'frames': 2, 'average': average, 'inputs': inputs})
    case('unity_masks', 'Every connection mask of six inputs in sum and average, polyphonic cables counted once.', events)
    events = []
    for merge in (1, 0, 1):
        events.append({'merge': merge})
        for average in ([0, 0], [1, 0], [0, 1], [1, 1]):
            events.append({'frames': 100, 'average': average, 'inputs': {
                'ch1_1': {'channels': 1, 'signal': {'sine': {'freq': 50, 'amp': 4, 'offset': 1}}},
                'ch1_4': {'channels': 1, 'signal': const(0)},
                'ch2_2': {'channels': 2, 'signal': {'sine': {'freq': 300, 'amp': 3, 'offset': -1}}},
                'ch2_6': {'channels': 1, 'signal': const(2.5)}}})
    case('unity_merge_and_average', 'Merge with every combination of the two mode switches.', events)
    events = [{'frames': 1000, 'average': [0, 0], 'inputs': {'ch1_1': {'channels': 1, 'signal': const(12)},
                                                             'ch2_1': {'channels': 1, 'signal': const(-3)}}},
              {'frames': 4 * 48000, 'average': [0, 0], 'inputs': {}}]
    case('unity_meter_decay', 'Loud burst then four seconds of silence: meter release into the flush-to-zero range.',
         events, [48000])
    case('unity_meter_decay_short', 'Burst then one second of silence.',
         [{'frames': 500, 'average': [0, 0], 'inputs': {'ch1_3': {'channels': 1, 'signal': const(-10)}}},
          {'frames': 48000, 'average': [0, 0], 'inputs': {'ch1_3': {'channels': 1, 'signal': const(0)}}}],
         [44100, 96000])
    case('unity_light_bands', 'Slow level ramps through every 6 dB light band and the clip light.',
         [{'frames': 60000, 'average': [0, 1], 'inputs': {
             'ch1_2': {'channels': 1, 'signal': ramp(0, 12)},
             'ch2_1': {'channels': 1, 'signal': ramp(-14, 0)}, 'ch2_5': {'channels': 1, 'signal': const(0)}}}])
    events = [{'merge': 1},
              {'frames': 700, 'average': [1, 1], 'inputs': {'ch1_1': {'channels': 1, 'signal': const(8)},
                                                            'ch2_2': {'channels': 1, 'signal': const(4)}}},
              {'reset': True},
              {'frames': 700, 'average': [0, 0], 'inputs': {'ch1_1': {'channels': 1, 'signal': const(8)},
                                                            'ch2_2': {'channels': 1, 'signal': const(4)}}}]
    case('unity_reset', 'Reset turns off merge and returns both switches to sum while meters keep running.', events)
    events = [{'data': value} for value in
              [{'merge': True}, {'merge': False}, {'merge': 1}, {'merge': 'yes'}, {}, {'merge': None}, [], None,
               {'merge': True, 'other': 3}]]
    case('unity_saved_data', 'Saved module data as a fresh module loads it, and what it saves back.', events, [48000])
    specials = ['00000000', '80000000', '00000001', '80000001', '007fffff', '00800000', '49742400', 'c9742400',
                '3f800000', '41200000']
    events = []
    for a in specials:
        for b in specials[::3]:
            events.append({'frames': 1, 'average': [1, 0], 'inputs': {
                'ch1_1': {'channels': 1, 'signal': {'bits': [a]}}, 'ch1_2': {'channels': 1, 'signal': {'bits': [b]}},
                'ch2_3': {'channels': 1, 'signal': {'bits': [a]}}, 'ch2_4': {'channels': 1, 'signal': {'bits': [b]}},
                'ch2_5': {'channels': 3, 'signal': {'bits': [a]}}}})
    case('unity_special_values', 'Signed zero, subnormal, boundary and large voltages.', events, [48000])
    events = []
    for step in range(12):
        inputs = {port: {'channels': 1 + (step + i) % 3,
                         'signal': {'noise': {'seed': step * 13 + i, 'amp': 5, 'offset': 0}}}
                  for i, port in enumerate(UNITY_PORTS) if (step * 7 + i) % 4}
        events.append({'merge': step % 3 == 0})
        events.append({'frames': 1500, 'average': [step % 2, (step // 2) % 2], 'inputs': inputs})
    case('unity_noise', 'Noise on changing input sets, modes and merge.', events)
    return cases


def main():
    cases = vca_cases() + unity_cases()
    ids = [c['id'] for c in cases]
    assert len(ids) == len(set(ids))
    document = {'schema': 1, 'source': 'VCVRack/Fundamental@10dd0160c664770910e5584b7b00498cc48d9ddd',
                'generator': 'make_cases.py', 'cases': cases}
    # One case per line keeps the frozen file reviewable and compact.
    body = ',\n'.join(json.dumps(c, separators=(',', ':')) for c in cases)
    header = json.dumps({k: v for k, v in document.items() if k != 'cases'})[:-1]
    (HERE / 'cases.json').write_text(header + ', "cases": [\n' + body + '\n]}\n')
    print(f'{len(cases)} cases')


if __name__ == '__main__':
    main()
