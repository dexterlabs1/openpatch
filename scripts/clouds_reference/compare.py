#!/usr/bin/env python3
"""Compare the production Rust Clouds kernel against an immutable C++ reference run.

Acceptance is exact: every int16 left/right output sample must be identical
inside the source's defined domain.

Two oracles are built from unmodified upstream files. `pinned` is eurorack
f4cfa76 exactly. `window_2019` is the same tree with clouds/dsp/window.h from
eurorack 2a46873, the last revision before two 2023 edits together removed
both `done_ = false` assignments from Window::Start and left the stretch mode
silent. The Rust kernel keeps the 2a46873 behaviour. Each case is compared
against `pinned`; where the two oracles differ (stretch-mode cases) exact
acceptance uses `window_2019`, and the divergence from `pinned` is reported
separately and never hidden.

If the Rust kernel reports a guarded read (an access outside an object,
undefined behaviour in the C++ source), that block and every later block of
the case are outside the defined domain: they are reported with their own
mismatch counts and never counted as matched.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BLOCK_SAMPLES = 64


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    return [list(map(int, line.split())) for line in path.read_text().splitlines()]


def diff(expected, actual, domain_end):
    stats = {'samples': 0, 'mismatched_samples': 0, 'max_abs_difference': 0,
             'outside_samples': 0, 'outside_mismatched': 0, 'outside_max': 0, 'first_mismatch': None}
    for block, (a, b) in enumerate(zip(expected, actual)):
        assert len(a) == len(b) == BLOCK_SAMPLES
        for position, (x, y) in enumerate(zip(a, b)):
            if block >= domain_end:
                stats['outside_samples'] += 1
                if x != y:
                    stats['outside_mismatched'] += 1
                    stats['outside_max'] = max(stats['outside_max'], abs(x - y))
                continue
            stats['samples'] += 1
            if x != y:
                stats['mismatched_samples'] += 1
                stats['max_abs_difference'] = max(stats['max_abs_difference'], abs(x - y))
                if stats['first_mismatch'] is None:
                    stats['first_mismatch'] = {'block': block, 'frame': position // 2,
                                               'output': 'right' if position % 2 else 'left', 'expected': x, 'actual': y}
    return stats


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    reference = args.reference.resolve()
    assert json.loads((reference / 'result.json').read_text())['passed']
    for name, expected in json.loads((reference / 'artifact-sha256.json').read_text()).items():
        assert sha(reference / name) == expected, f'changed reference: {name}'
    cases = json.loads((reference / 'cases.json').read_text())['cases']
    base = ROOT / 'native-test-output/clouds-reference'
    index = 1
    while (base / f'rust-comparison-{index:03}').exists():
        index += 1
    evidence = base / f'rust-comparison-{index:03}'
    evidence.mkdir(parents=True)
    sources = [HERE / 'rust_probe.rs', HERE / 'compare.py']
    sources += sorted((ROOT / 'crates/openpatch-core/src/grain_weather').rglob('*.rs'))
    source_hashes = {}
    for source in sources:
        relative = source.relative_to(ROOT)
        target = evidence / 'source' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        source_hashes[str(relative)] = sha(source)
    (evidence / 'source-sha256.json').write_text(json.dumps(source_hashes, indent=2) + '\n')
    (evidence / 'rustc.txt').write_text(subprocess.check_output(['rustc', '-Vv'], text=True, timeout=30))
    executable = evidence / 'rust_probe'
    command = ['rustc', '--edition=2021', '-O', str(evidence / 'source/scripts/clouds_reference/rust_probe.rs'), '-o', str(executable)]
    (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    report = {'passed': False, 'reference': str(reference), 'reference_manifest_sha256': sha(reference / 'artifact-sha256.json'),
              'scope': 'portable clouds::GranularProcessor kernel at 32 kHz / 32-frame blocks, int16 codec frames, source RNG seeded per case; not the native host adapter, resampler or hardware',
              'acceptance': 'bit-identical int16 left and right samples against the pinned oracle, or against window_2019 where the oracles differ; zero guarded reads', 'cases': {}}
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=600)
        (evidence / 'compile.stderr').write_text(result.stderr)
        result.check_returncode()
        report['executable_sha256'] = sha(executable)
        for case in cases:
            name = case['id']
            target = evidence / name
            target.mkdir()
            with (target / 'output.txt').open('w') as output, (target / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=(reference / name / 'input.txt').read_text(), text=True, stdout=output, stderr=errors, timeout=300)
            result.check_returncode()
            diagnostics = (target / 'stderr.txt').read_text().splitlines()
            guarded = int(diagnostics[-1].split()[1])
            guarded_blocks = [int(value) for value in diagnostics[-2].split()[1:]]
            domain_end = guarded_blocks[0] if guarded_blocks else case['blocks']
            pinned = load(reference / name / 'output.txt')
            variant = load(reference / name / 'output-window_2019.txt')
            actual = load(target / 'output.txt')
            assert len(actual) == len(pinned) == len(variant) == case['blocks']
            oracle = 'pinned' if pinned == variant else 'window_2019'
            stats = diff(pinned if oracle == 'pinned' else variant, actual, domain_end)
            item = {'passed': stats['mismatched_samples'] == 0, 'oracle': oracle, 'samples': stats['samples'],
                    'mismatched_samples': stats['mismatched_samples'], 'max_abs_difference': stats['max_abs_difference'],
                    'guarded_reads': guarded}
            if oracle == 'window_2019':
                against_pinned = diff(pinned, actual, case['blocks'])
                item['versus_pinned'] = {'mismatched_samples': against_pinned['mismatched_samples'],
                                         'max_abs_difference': against_pinned['max_abs_difference'],
                                         'pinned_nonzero_samples': sum(1 for line in pinned for v in line if v != 0)}
            if guarded_blocks:
                item['source_undefined_domain'] = {'first_guarded_block': domain_end, 'guarded_blocks': guarded_blocks[:50],
                                                   'excluded_samples': stats['outside_samples'], 'mismatched_samples': stats['outside_mismatched'],
                                                   'max_abs_difference': stats['outside_max']}
            if stats['first_mismatch']:
                item['first_mismatch'] = stats['first_mismatch']
            report['cases'][name] = item
        report['passed'] = all(item['passed'] for item in report['cases'].values())
    except Exception as error:
        report['error'] = str(error)
    items = report['cases'].values()
    report['compared_samples'] = sum(item.get('samples', 0) for item in items)
    report['mismatched_samples'] = sum(item.get('mismatched_samples', 0) for item in items)
    report['compared_samples_by_oracle'] = {oracle: sum(item['samples'] for item in items if item['oracle'] == oracle) for oracle in ('pinned', 'window_2019')}
    report['excluded_undefined_samples'] = sum(item.get('source_undefined_domain', {}).get('excluded_samples', 0) for item in items)
    (evidence / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    artifacts = {str(path.relative_to(evidence)): sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases_passed': sum(item['passed'] for item in items), 'cases': len(cases),
                      'compared_samples': report['compared_samples'], 'by_oracle': report['compared_samples_by_oracle'],
                      'mismatched_samples': report['mismatched_samples'], 'excluded_undefined_samples': report['excluded_undefined_samples'],
                      'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
