#!/usr/bin/env python3
"""Root-run reference builder. --validate-only checks input structure without DSP execution."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess
import sys
from fetch import DEFAULT_CACHE, HERE, ROOT, fetch


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate(document):
    assert document['schema'] == 2
    cases = document['cases']
    negative_cases = document['negative_cases']
    ids = set()
    for case in cases + negative_cases:
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        assert case['sample_rate'] in (31250, 48000, 62500)
        assert case['block_size'] in (4, 6, 8)
        assert type(case['blocks']) is int and 1 <= case['blocks'] <= 20000
        assert case['ramp_mode'] in (0, 1, 2) and case['output_mode'] in (0, 1, 2, 3) and case['range'] in (0, 1)
        assert type(case['trigger_patched']) is bool and type(case['clock_patched']) is bool
        for signal in ('gate', 'clock'):
            assert type(case[signal + '_period']) is int and case[signal + '_period'] >= 1
            assert type(case[signal + '_high']) is int and 0 < case[signal + '_high'] < case[signal + '_period']
        assert case['ratio'] in (0.5, 1.0, 2.0)
        assert type(case['ratio_denominator']) is int and case['ratio_denominator'] > 0
        assert type(case['clock_start']) is int and case['clock_start'] >= 0
        assert type(case['initial_clock_high']) is bool
        for control in ('slope', 'shape', 'smoothness', 'shift'):
            assert math.isfinite(case[control]) and 0 <= case[control] <= 1
        assert 0 < case['frequency_hz'] <= case['sample_rate'] * 0.25
        event_blocks = set()
        for event in case['events']:
            assert type(event['block']) is int and 0 <= event['block'] < case['blocks']
            assert event['block'] not in event_blocks
            event_blocks.add(event['block'])
            assert set(event) <= {'block', 'frequency_hz', 'slope', 'shape', 'smoothness', 'shift', 'reset_filter', 'reset_extractor'}
            for key, value in event.items():
                if key in ('reset_filter', 'reset_extractor'):
                    assert type(value) is bool
                elif key in ('slope', 'shape', 'smoothness', 'shift'):
                    assert math.isfinite(value) and 0 <= value <= 1
                elif key == 'frequency_hz':
                    assert math.isfinite(value) and 0 < value <= case['sample_rate'] * 0.25
        if case in negative_cases:
            assert case['expected_error'] == 'nonfinite frequency'
            assert case['range'] == 0 and case['clock_patched']
            assert case['clock_start'] == 0 and not case['initial_clock_high']
        elif case['clock_patched']:
            # At construction and Reset, a rising edge needs a nonzero elapsed
            # pulse duration. Validate this independently of DSP execution.
            reset_frames = {0} | {event['block'] * case['block_size'] for event in case['events'] if event.get('reset_extractor')}
            for frame in reset_frames:
                high = frame >= case['clock_start'] and (frame - case['clock_start']) % case['clock_period'] < case['clock_high']
                previous_high = case['initial_clock_high'] if frame == 0 else frame - 1 >= case['clock_start'] and (frame - 1 - case['clock_start']) % case['clock_period'] < case['clock_high']
                assert not (high and not previous_high), 'zero-duration rising edge in positive domain'
    return cases, negative_cases


def protocol(case):
    header = [case[key] for key in ('sample_rate', 'block_size', 'blocks', 'ramp_mode', 'output_mode', 'range')]
    header += [int(case[key]) for key in ('trigger_patched', 'clock_patched')]
    header += [case[key] for key in ('gate_period', 'gate_high', 'clock_period', 'clock_high', 'ratio', 'ratio_denominator')]
    header += [case['clock_start'], int(case['initial_clock_high'])]
    lines = [' '.join(map(str, header))]
    state = dict(case)
    events = {event['block']: event for event in case['events']}
    for block in range(case['blocks']):
        event = events.get(block, {})
        state.update(event)
        controls = [state['frequency_hz'] / case['sample_rate']]
        controls += [state[key] for key in ('slope', 'shape', 'smoothness', 'shift')]
        controls += [int(event.get(key, False)) for key in ('reset_filter', 'reset_extractor')]
        lines.append(' '.join(map(str, controls)))
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=DEFAULT_CACHE)
    parser.add_argument('--cases', type=Path, default=HERE / 'cases.json')
    parser.add_argument('--validate-only', action='store_true')
    parser.add_argument('--compiler', default='clang++')
    args = parser.parse_args()
    cases, negative_cases = validate(json.loads(args.cases.read_text()))
    if args.validate_only:
        print(json.dumps({'input_validation_passed': True, 'cases': len(cases), 'negative_cases': len(negative_cases), 'frames': sum(c['blocks'] * c['block_size'] for c in cases), 'dsp_executed': False}))
        return 0
    manifest = fetch(args.cache)
    base = ROOT / 'native-test-output/tides2-reference'
    index = 1
    while (base / f'attempt-{index:03}').exists():
        index += 1
    evidence = base / f'attempt-{index:03}'
    evidence.mkdir(parents=True)
    report = {'passed': False, 'scope': 'original portable PolySlopeGenerator and RampExtractor only; no OpenPatch/backend comparison or hardware qualification', 'stage': 'prepare', 'cases': len(cases), 'negative_cases': len(negative_cases), 'subprocess_timeout_seconds': 30}
    try:
        for name in ('source_manifest.json', 'reference_probe.cpp', 'fetch.py', 'run.py', 'README.md', 'NOTICE.md'):
            shutil.copy2(HERE / name, evidence / name)
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
        command = [args.compiler, '-std=c++11', '-O2', '-DTEST', '-ffp-contract=off', '-fno-fast-math', '-I', str(retained), str(evidence / 'reference_probe.cpp')]
        command += platform_flags
        command += [str(retained / name) for name in manifest['compile_sources']]
        command += ['-o', str(executable)]
        (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
        report['stage'] = 'compile'
        build = subprocess.run(command, capture_output=True, text=True, timeout=30)
        (evidence / 'compile.stdout').write_text(build.stdout)
        (evidence / 'compile.stderr').write_text(build.stderr)
        build.check_returncode()
        report['stage'] = 'source execution'
        counts = {}
        rejected_boundaries = {}
        for case in negative_cases + cases:
            case_dir = evidence / case['id']
            case_dir.mkdir()
            encoded = protocol(case)
            (case_dir / 'input.txt').write_text(encoded)
            with (case_dir / 'output.jsonl').open('w') as output, (case_dir / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=encoded, text=True, stdout=output, stderr=errors, timeout=30)
            if case in negative_cases:
                assert result.returncode != 0, 'negative boundary unexpectedly accepted'
                assert (case_dir / 'stderr.txt').read_text() == case['expected_error'] + '\n', 'negative boundary failed for a different reason'
                assert (case_dir / 'output.jsonl').stat().st_size == 0, 'negative boundary emitted accepted records'
                rejected_boundaries[case['id']] = {'returncode': result.returncode, 'error': case['expected_error']}
                continue
            result.check_returncode()
            count = 0
            for line in (case_dir / 'output.jsonl').open():
                record = json.loads(line)
                assert record['frame'] == count
                assert len(record['output_bits']) == 4
                count += 1
            assert count == case['blocks'] * case['block_size']
            counts[case['id']] = count
        report.update(passed=True, stage='source traces produced and negative boundary rejected', frames_by_case=counts, rejected_boundaries=rejected_boundaries, production_equivalence=False)
    except Exception as error:
        report['error'] = str(error)
    artifacts = {str(path.relative_to(evidence)): sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True) + '\n')
    (evidence / 'result.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'passed': report['passed'], 'stage': report['stage'], 'evidence': str(evidence)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
