#!/usr/bin/env python3
"""Root-run Rings reference builder. --validate-only checks inputs without DSP execution."""
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

DISCRETE = ('polyphony', 'model', 'internal_exciter', 'internal_strum', 'internal_note', 'chord')
CONTINUOUS = ('tonic', 'note', 'fm', 'structure', 'brightness', 'damping', 'position')
CONTROLS = DISCRETE + CONTINUOUS
SIGNALS = ('zero', 'sine', 'saw', 'noise', 'decay', 'chirp', 'clicks', 'bursts')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def f32(value):
    """Round to binary32 so both probes parse exactly the same value."""
    return struct.unpack('<f', struct.pack('<f', value))[0]


def check_control(key, value):
    assert math.isfinite(value), key
    if key == 'polyphony':
        assert value in (1, 2, 3, 4)
    elif key == 'model':
        assert value in range(6)
    elif key == 'chord':
        assert value in range(11)
    elif key.startswith('internal_'):
        assert value in (0, 1), key
    elif key == 'structure':
        # The firmware's CV scaler constrains structure to 0..0.9995; the
        # stiffness table read is out of bounds at 1.0.
        assert 0.0 <= value <= 0.9995
    elif key in ('brightness', 'damping', 'position'):
        assert 0.0 <= value <= 1.0, key
    elif key == 'tonic':
        assert 0.0 <= value <= 84.0
    elif key == 'note':
        assert -60.0 <= value <= 60.0
    elif key == 'fm':
        assert -48.0 <= value <= 48.0


def validate(document):
    assert document['schema'] == 1
    assert document['sample_rate'] == 48000 and document['block_size'] == 24
    ids = set()
    for case in document['cases']:
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        assert case.get('purpose'), case['id']
        assert type(case['blocks']) is int and 1 <= case['blocks'] <= 40000
        assert 0 <= case.get('seed', 0x21) < 2 ** 32
        assert set(case['start']) == set(CONTROLS), case['id']
        assert set(case.get('end', {})) <= set(CONTINUOUS)
        for key, value in list(case['start'].items()) + list(case.get('end', {}).items()):
            check_control(key, value)
        blocks = set()
        for event in case.get('events', []):
            assert 0 <= event['block'] < case['blocks'] and event['block'] not in blocks
            blocks.add(event['block'])
            assert set(event) - {'block'} <= set(CONTROLS)
            for key in set(event) - {'block'}:
                check_control(key, event[key])
            for key in set(event) & set(case.get('end', {})):
                raise ValueError('event overrides a ramped control: ' + key)
        for block in case.get('strums', []):
            assert 0 <= block < case['blocks']
        period = case.get('strum_period')
        assert period is None or (type(period) is int and period > 0)
        signal = case['input']
        assert signal['type'] in SIGNALS
        if signal['type'] != 'zero':
            assert 0.0 < signal['amplitude'] <= 1.5
        if signal['type'] in ('sine', 'saw', 'decay', 'chirp', 'bursts'):
            assert 0.0 < signal['frequency'] < 24000.0
        if signal['type'] in ('decay', 'clicks', 'bursts'):
            assert type(signal['period_blocks']) is int and signal['period_blocks'] > 0
    return document['cases']


class Noise:
    """Independent 32-bit LCG for test signals; unrelated to the source PRNG."""

    def __init__(self, seed):
        self.state = seed & 0xffffffff

    def next(self):
        self.state = (self.state * 22695477 + 1) & 0xffffffff
        return ((self.state >> 8) / float(1 << 24)) * 2.0 - 1.0


def render_signal(signal, frames, rate, size):
    kind = signal['type']
    if kind == 'zero':
        return [0] * frames
    amplitude = signal['amplitude']
    noise = Noise(signal.get('seed', 1))
    values = []
    phase = 0.0
    for n in range(frames):
        t = n / rate
        if kind == 'noise':
            x = noise.next()
        elif kind == 'chirp':
            # Exponential sweep from frequency to frequency * ratio over the case.
            f = signal['frequency'] * signal['ratio'] ** (n / frames)
            phase = (phase + f / rate) % 1.0
            x = math.sin(2.0 * math.pi * phase)
        elif kind in ('clicks', 'bursts', 'decay'):
            period = signal['period_blocks'] * size
            age = n % period
            if kind == 'clicks':
                x = 1.0 if age < 2 else 0.0
            elif kind == 'bursts':
                # Short noisy plucks with a tonal body.
                envelope = math.exp(-age / (0.004 * rate))
                x = envelope * (0.6 * noise.next() + 0.4 * math.sin(2.0 * math.pi * signal['frequency'] * age / rate))
            else:
                x = math.sin(2.0 * math.pi * signal['frequency'] * age / rate) * math.exp(-age / (0.05 * rate))
        else:
            phase = (signal['frequency'] * t) % 1.0
            x = math.sin(2.0 * math.pi * phase) if kind == 'sine' else 2.0 * phase - 1.0
        values.append(max(-32768, min(32767, int(round(x * amplitude * 32767.0)))))
    return values


def protocol(case, rate, size):
    frames = case['blocks'] * size
    samples = render_signal(case['input'], frames, rate, size)
    lines = [f"{case['blocks']} {size} {case.get('seed', 0x21)}"]
    state = dict(case['start'])
    events = {event['block']: event for event in case.get('events', [])}
    strums = set(case.get('strums', []))
    period = case.get('strum_period')
    start, end = case['start'], case.get('end', {})
    for block in range(case['blocks']):
        position = block / max(1, case['blocks'] - 1)
        for key, target in end.items():
            state[key] = start[key] + (target - start[key]) * position
        for key, value in events.get(block, {}).items():
            if key != 'block':
                state[key] = value
        strum = int(block in strums or (period is not None and block % period == period - 1))
        values = [str(int(state['polyphony'])), str(int(state['model'])), str(strum)]
        values += [str(int(state[key])) for key in ('internal_exciter', 'internal_strum', 'internal_note', 'chord')]
        values += [repr(f32(state[key])) for key in CONTINUOUS]
        offset = block * size
        values += [str(samples[offset + i]) for i in range(size)]
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
    base = ROOT / 'native-test-output/rings-reference'
    index = 1
    while (base / f'attempt-{index:03}').exists():
        index += 1
    evidence = base / f'attempt-{index:03}'
    evidence.mkdir(parents=True)
    report = {'passed': False, 'scope': 'original portable rings::Strummer and rings::Part with the FillBuffer noise gate and Clip16 output at 48 kHz / 24-frame blocks with int16 codec frames; no OpenPatch host adapter, CV scaler or hardware qualification', 'stage': 'prepare', 'cases': len(cases), 'subprocess_timeout_seconds': 300}
    try:
        for name in ('source_manifest.json', 'reference_probe.cpp', 'fetch.py', 'run.py', 'README.md', 'NOTICE.md'):
            shutil.copy2(HERE / name, evidence / name)
        shutil.copy2(args.cases, evidence / 'cases.json')
        retained = evidence / 'upstream'
        for name in list(manifest['files']) + list(manifest.get('reference_only', {})):
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
        def compile_probe(executable, prefix_includes):
            command = [args.compiler, '-std=c++11', '-O2', '-DTEST', '-ffp-contract=off', '-fno-fast-math']
            for include in prefix_includes:
                command += ['-I', str(include)]
            command += ['-I', str(retained), str(evidence / 'reference_probe.cpp')]
            command += platform_flags
            command += [str(retained / name) for name in manifest['compile_sources']]
            command += ['-o', str(executable)]
            (evidence / f'compile-command-{executable.name}.json').write_text(json.dumps(command, indent=2) + '\n')
            build = subprocess.run(command, capture_output=True, text=True, timeout=180)
            (evidence / f'compile-{executable.name}.stdout').write_text(build.stdout)
            (evidence / f'compile-{executable.name}.stderr').write_text(build.stderr)
            build.check_returncode()

        report['stage'] = 'compile'
        compile_probe(executable, [])
        report['stage'] = 'source execution'
        counts = {}
        for case in cases:
            case_dir = evidence / case['id']
            case_dir.mkdir()
            encoded = protocol(case, rate, size)
            (case_dir / 'input.txt').write_text(encoded)
            with (case_dir / 'output.txt').open('w') as output, (case_dir / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=encoded, text=True, stdout=output, stderr=errors, timeout=300)
            result.check_returncode()
            lines = (case_dir / 'output.txt').read_text().splitlines()
            assert len(lines) == case['blocks']
            assert all(len(line.split()) == 2 * size + 1 for line in lines)
            nonzero = sum(1 for line in lines for value in line.split()[:-1] if value != '0')
            strums = sum(int(line.split()[-1]) for line in lines)
            counts[case['id']] = {'frames': case['blocks'] * size, 'nonzero_samples': nonzero, 'strums': strums}
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
