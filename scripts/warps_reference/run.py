#!/usr/bin/env python3
"""Root-run Warps reference builder. --validate-only checks inputs without DSP execution."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import sys
from fetch import DEFAULT_CACHE, HERE, ROOT, fetch

CONTROLS = ('carrier_shape', 'drive_1', 'drive_2', 'algorithm', 'parameter',
            'frequency_shift_pot', 'frequency_shift_cv', 'phase_shift', 'note')
SIGNALS = ('zero', 'sine', 'saw', 'square', 'noise', 'pulse_train', 'decay')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def f32(value):
    """Round to binary32 so both probes parse exactly the same value."""
    return struct.unpack('<f', struct.pack('<f', value))[0]


def validate(document):
    assert document['schema'] == 1
    assert document['sample_rate'] == 96000 and document['block_size'] == 60
    ids = set()
    for case in document['cases']:
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        assert type(case['blocks']) is int and 1 <= case['blocks'] <= 20000
        assert case['easter_egg'] in (0, 1)
        assert set(case['start']) == set(CONTROLS)
        assert set(case.get('end', {})) <= set(CONTROLS) - {'carrier_shape'}
        assert case['start']['carrier_shape'] in (0, 1, 2, 3)
        for key, value in list(case['start'].items()) + list(case.get('end', {}).items()):
            assert math.isfinite(value)
            if key in ('drive_1', 'drive_2', 'algorithm', 'parameter', 'frequency_shift_pot', 'phase_shift'):
                assert 0.0 <= value <= 1.0, key
            elif key == 'frequency_shift_cv':
                assert -1.0 <= value <= 1.0
        blocks = set()
        for event in case.get('events', []):
            assert 0 <= event['block'] < case['blocks'] and event['block'] not in blocks
            blocks.add(event['block'])
            assert set(event) - {'block'} <= set(CONTROLS) | {'easter_egg'}
            if 'carrier_shape' in event:
                assert event['carrier_shape'] in (0, 1, 2, 3)
            if 'easter_egg' in event:
                assert event['easter_egg'] in (0, 1)
        for side in ('left', 'right'):
            signal = case[side]
            assert signal['type'] in SIGNALS
            if signal['type'] != 'zero':
                assert 0.0 < signal['amplitude'] <= 1.5
            if signal['type'] in ('sine', 'saw', 'square', 'pulse_train', 'decay'):
                assert 0.0 < signal['frequency'] < 48000.0
    return document['cases']


class Noise:
    """Independent 32-bit LCG for test signals; unrelated to the source PRNG."""

    def __init__(self, seed):
        self.state = seed & 0xffffffff

    def next(self):
        self.state = (self.state * 22695477 + 1) & 0xffffffff
        return ((self.state >> 8) / float(1 << 24)) * 2.0 - 1.0


def render_signal(signal, frames, rate):
    kind = signal['type']
    if kind == 'zero':
        return [0] * frames
    amplitude = signal['amplitude']
    noise = Noise(signal.get('seed', 1))
    values = []
    for n in range(frames):
        t = n / rate
        if kind == 'noise':
            x = noise.next()
        else:
            phase = (signal['frequency'] * t) % 1.0
            if kind == 'sine':
                x = math.sin(2.0 * math.pi * phase)
            elif kind == 'saw':
                x = 2.0 * phase - 1.0
            elif kind == 'square':
                x = 1.0 if phase < 0.5 else -1.0
            elif kind == 'pulse_train':
                # Vowel-like buzz: band-rich pulse with slow seeded jitter.
                x = (1.0 if phase < 0.18 else -0.22) + 0.15 * noise.next()
            else:  # decay
                x = math.sin(2.0 * math.pi * phase) * math.exp(-n / (signal['decay_blocks'] * 60.0))
        values.append(max(-32768, min(32767, int(round(x * amplitude * 32767.0)))))
    return values


def protocol(case, rate, size):
    frames = case['blocks'] * size
    left = render_signal(case['left'], frames, rate)
    right = render_signal(case['right'], frames, rate)
    lines = [f"{case['blocks']} {size}"]
    state = dict(case['start'])
    state['easter_egg'] = case['easter_egg']
    events = {event['block']: event for event in case.get('events', [])}
    start, end = case['start'], case.get('end', {})
    for block in range(case['blocks']):
        position = block / max(1, case['blocks'] - 1)
        for key, target in end.items():
            state[key] = start[key] + (target - start[key]) * position
        for key, value in events.get(block, {}).items():
            if key != 'block':
                state[key] = value
                if key in end:
                    raise ValueError('event overrides a ramped control')
        values = [str(int(state['easter_egg'])), str(int(state['carrier_shape']))]
        values += [repr(f32(state[key])) for key in CONTROLS[1:]]
        offset = block * size
        values += [f'{left[offset + i]} {right[offset + i]}' for i in range(size)]
        lines.append(' '.join(values))
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=DEFAULT_CACHE)
    parser.add_argument('--cases', type=Path, default=HERE / 'cases.json')
    parser.add_argument('--validate-only', action='store_true')
    parser.add_argument('--compiler', default='clang++')
    args = parser.parse_args()
    document = json.loads(args.cases.read_text())
    cases = validate(document)
    rate, size = document['sample_rate'], document['block_size']
    if args.validate_only:
        print(json.dumps({'input_validation_passed': True, 'cases': len(cases), 'frames': sum(c['blocks'] * size for c in cases), 'dsp_executed': False}))
        return 0
    manifest = fetch(args.cache)
    base = ROOT / 'native-test-output/warps-reference'
    index = 1
    while (base / f'attempt-{index:03}').exists():
        index += 1
    evidence = base / f'attempt-{index:03}'
    evidence.mkdir(parents=True)
    report = {'passed': False, 'scope': 'original portable warps::Modulator::Process at 96 kHz / 60-frame blocks with int16 codec frames; no OpenPatch host adapter or hardware qualification', 'stage': 'prepare', 'cases': len(cases), 'subprocess_timeout_seconds': 60}
    try:
        for name in ('source_manifest.json', 'reference_probe.cpp', 'fetch.py', 'run.py', 'README.md', 'NOTICE.md'):
            shutil.copy2(HERE / name, evidence / name)
        shutil.copytree(HERE / 'stubs', evidence / 'stubs')
        shutil.copy2(args.cases, evidence / 'cases.json')
        retained = evidence / 'upstream'
        for name in manifest['files']:
            target = retained / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(args.cache / name, target)
        compiler_version = subprocess.check_output([args.compiler, '--version'], text=True, timeout=30)
        (evidence / 'compiler.txt').write_text(compiler_version)
        executable = evidence / 'reference_probe'
        platform_flags = []
        if sys.platform == 'darwin':
            sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True, timeout=30).strip())
            platform_flags = ['-isysroot', str(sdk), '-isystem', str(sdk / 'usr/include/c++/v1')]
        command = [args.compiler, '-std=c++11', '-O2', '-DTEST', '-ffp-contract=off', '-fno-fast-math',
                   '-I', str(retained), '-I', str(evidence / 'stubs'), str(evidence / 'reference_probe.cpp')]
        command += platform_flags
        command += [str(retained / name) for name in manifest['compile_sources']]
        command += ['-o', str(executable)]
        (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
        report['stage'] = 'compile'
        build = subprocess.run(command, capture_output=True, text=True, timeout=120)
        (evidence / 'compile.stdout').write_text(build.stdout)
        (evidence / 'compile.stderr').write_text(build.stderr)
        build.check_returncode()
        report['stage'] = 'source execution'
        counts = {}
        for case in cases:
            case_dir = evidence / case['id']
            case_dir.mkdir()
            encoded = protocol(case, rate, size)
            (case_dir / 'input.txt').write_text(encoded)
            with (case_dir / 'output.txt').open('w') as output, (case_dir / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=encoded, text=True, stdout=output, stderr=errors, timeout=60)
            result.check_returncode()
            lines = (case_dir / 'output.txt').read_text().splitlines()
            assert len(lines) == case['blocks']
            assert all(len(line.split()) == 2 * size for line in lines)
            nonzero = sum(1 for line in lines for value in line.split() if value != '0')
            counts[case['id']] = {'frames': case['blocks'] * size, 'nonzero_samples': nonzero}
        report.update(passed=True, stage='source traces produced', frames_by_case=counts, production_equivalence=False)
    except Exception as error:
        report['error'] = str(error)
    artifacts = {str(path.relative_to(evidence)): sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True) + '\n')
    (evidence / 'result.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'passed': report['passed'], 'stage': report['stage'], 'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
