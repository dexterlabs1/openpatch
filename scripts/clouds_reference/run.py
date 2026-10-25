#!/usr/bin/env python3
"""Root-run Clouds reference builder. --validate-only checks inputs without DSP execution."""
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

DISCRETE = ('mode', 'quality', 'freeze', 'trigger', 'gate')
CONTINUOUS = ('position', 'size', 'pitch', 'density', 'texture', 'dry_wet',
              'stereo_spread', 'feedback', 'reverb')
CONTROLS = DISCRETE + CONTINUOUS
SIGNALS = ('zero', 'sine', 'saw', 'square', 'noise', 'pulse_train', 'decay', 'chirp')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def f32(value):
    """Round to binary32 so both probes parse exactly the same value."""
    return struct.unpack('<f', struct.pack('<f', value))[0]


def validate(document):
    assert document['schema'] == 1
    assert document['sample_rate'] == 32000 and document['block_size'] == 32
    ids = set()
    for case in document['cases']:
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        assert type(case['blocks']) is int and 1 <= case['blocks'] <= 40000
        assert 1 <= case.get('prepare_calls', 8) <= 64
        assert 0 <= case.get('seed', 0x21) < 2 ** 32
        assert set(case['start']) == set(CONTROLS) - {'trigger'}, case['id']
        assert set(case.get('end', {})) <= set(CONTINUOUS)
        for key, value in list(case['start'].items()) + list(case.get('end', {}).items()):
            assert math.isfinite(value)
            if key in ('mode', 'quality'):
                assert value in (0, 1, 2, 3), key
            elif key in ('freeze', 'gate'):
                assert value in (0, 1), key
            elif key == 'pitch':
                assert -48.0 <= value <= 48.0
            else:
                assert 0.0 <= value <= 1.0, key
        blocks = set()
        for event in case.get('events', []):
            assert 0 <= event['block'] < case['blocks'] and event['block'] not in blocks
            blocks.add(event['block'])
            assert set(event) - {'block'} <= set(CONTROLS) - {'trigger'}
            for key in set(event) & set(case.get('end', {})):
                raise ValueError('event overrides a ramped control: ' + key)
        for block in case.get('triggers', []):
            assert 0 <= block < case['blocks']
        period = case.get('trigger_period')
        assert period is None or (type(period) is int and period > 0)
        for side in ('left', 'right'):
            signal = case[side]
            assert signal['type'] in SIGNALS
            if signal['type'] != 'zero':
                assert 0.0 < signal['amplitude'] <= 1.5
            if signal['type'] in ('sine', 'saw', 'square', 'pulse_train', 'decay', 'chirp'):
                assert 0.0 < signal['frequency'] < 16000.0
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
        else:
            phase = (signal['frequency'] * t) % 1.0
            if kind == 'sine':
                x = math.sin(2.0 * math.pi * phase)
            elif kind == 'saw':
                x = 2.0 * phase - 1.0
            elif kind == 'square':
                x = 1.0 if phase < 0.5 else -1.0
            elif kind == 'pulse_train':
                x = (1.0 if phase < 0.18 else -0.22) + 0.15 * noise.next()
            else:  # decay: repeating plucks
                period = int(signal['decay_blocks'] * 32 * 4)
                x = math.sin(2.0 * math.pi * phase) * math.exp(-(n % period) / (signal['decay_blocks'] * 32.0))
        values.append(max(-32768, min(32767, int(round(x * amplitude * 32767.0)))))
    return values


def protocol(case, rate, size):
    frames = case['blocks'] * size
    left = render_signal(case['left'], frames, rate)
    right = render_signal(case['right'], frames, rate)
    lines = [f"{case['blocks']} {size} {case.get('prepare_calls', 8)} {case.get('seed', 0x21)}"]
    state = dict(case['start'])
    events = {event['block']: event for event in case.get('events', [])}
    triggers = set(case.get('triggers', []))
    period = case.get('trigger_period')
    start, end = case['start'], case.get('end', {})
    for block in range(case['blocks']):
        position = block / max(1, case['blocks'] - 1)
        for key, target in end.items():
            state[key] = start[key] + (target - start[key]) * position
        for key, value in events.get(block, {}).items():
            if key != 'block':
                state[key] = value
        trigger = int(block in triggers or (period is not None and block % period == period - 1))
        values = [str(int(state[key])) for key in ('mode', 'quality', 'freeze')]
        values += [str(trigger), str(int(state['gate']))]
        values += [repr(f32(state[key])) for key in CONTINUOUS]
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
    base = ROOT / 'native-test-output/clouds-reference'
    index = 1
    while (base / f'attempt-{index:03}').exists():
        index += 1
    evidence = base / f'attempt-{index:03}'
    evidence.mkdir(parents=True)
    report = {'passed': False, 'variants': 'window_2019 rebuilds the pinned source with clouds/dsp/window.h from eurorack 2a46873 (see source_manifest.json)', 'scope': 'original portable clouds::GranularProcessor::Process/Prepare at 32 kHz / 32-frame blocks with int16 codec frames; no OpenPatch host adapter or hardware qualification', 'stage': 'prepare', 'cases': len(cases), 'subprocess_timeout_seconds': 120}
    try:
        for name in ('source_manifest.json', 'reference_probe.cpp', 'fetch.py', 'run.py', 'README.md', 'NOTICE.md'):
            shutil.copy2(HERE / name, evidence / name)
        shutil.copytree(HERE / 'stubs', evidence / 'stubs')
        shutil.copy2(args.cases, evidence / 'cases.json')
        retained = evidence / 'upstream'
        variant_files = {name: record for variant in manifest.get('variants', {}).values() for name, record in variant['files'].items()}
        for name in list(manifest['files']) + list(variant_files):
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
            command += ['-I', str(retained), '-I', str(evidence / 'stubs'), str(evidence / 'reference_probe.cpp')]
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
        variants = {}
        for variant in manifest.get('variants', {}):
            variants[variant] = evidence / f'reference_probe_{variant}'
            compile_probe(variants[variant], [retained / variant])
        report['stage'] = 'source execution'
        counts = {}
        for case in cases:
            case_dir = evidence / case['id']
            case_dir.mkdir()
            encoded = protocol(case, rate, size)
            (case_dir / 'input.txt').write_text(encoded)
            with (case_dir / 'output.txt').open('w') as output, (case_dir / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=encoded, text=True, stdout=output, stderr=errors, timeout=120)
            result.check_returncode()
            for variant, variant_executable in variants.items():
                with (case_dir / f'output-{variant}.txt').open('w') as output, (case_dir / f'stderr-{variant}.txt').open('w') as errors:
                    result = subprocess.run([str(variant_executable)], input=encoded, text=True, stdout=output, stderr=errors, timeout=120)
                result.check_returncode()
            lines = (case_dir / 'output.txt').read_text().splitlines()
            assert len(lines) == case['blocks']
            assert all(len(line.split()) == 2 * size for line in lines)
            nonzero = sum(1 for line in lines for value in line.split() if value != '0')
            counts[case['id']] = {'frames': case['blocks'] * size, 'nonzero_samples': nonzero,
                                  'variants_identical': {variant: (case_dir / f'output-{variant}.txt').read_text() == (case_dir / 'output.txt').read_text() for variant in variants}}
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
