#!/usr/bin/env python3
"""Compare the production Rust Rings kernel against an immutable C++ reference run.

Acceptance is exact: every int16 odd/even output sample and the per-block
strum flag must be identical inside the source's defined domain.

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
BLOCK_SAMPLES = 48


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path):
    return [list(map(int, line.split())) for line in path.read_text().splitlines()]


def diff(expected, actual, domain_end):
    stats = {'samples': 0, 'mismatched_samples': 0, 'max_abs_difference': 0, 'strum_mismatches': 0,
             'outside_samples': 0, 'outside_mismatched': 0, 'outside_max': 0, 'first_mismatch': None}
    for block, (a, b) in enumerate(zip(expected, actual)):
        assert len(a) == len(b) == BLOCK_SAMPLES + 1
        for position, (x, y) in enumerate(zip(a[:-1], b[:-1])):
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
                                               'output': 'even' if position % 2 else 'odd', 'expected': x, 'actual': y}
        if block < domain_end and a[-1] != b[-1]:
            stats['strum_mismatches'] += 1
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
    base = ROOT / 'native-test-output/rings-reference'
    index = 1
    while (base / f'rust-comparison-{index:03}').exists():
        index += 1
    evidence = base / f'rust-comparison-{index:03}'
    evidence.mkdir(parents=True)
    sources = [HERE / 'rust_probe.rs', HERE / 'compare.py']
    sources += [ROOT / 'crates/openpatch-core/src/string_field/kernel.rs']
    sources += sorted((ROOT / 'crates/openpatch-core/src/string_field/kernel').rglob('*.rs'))
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
    command = ['rustc', '--edition=2021', '-O', str(evidence / 'source/scripts/rings_reference/rust_probe.rs'), '-o', str(executable)]
    (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    report = {'passed': False, 'reference': str(reference), 'reference_manifest_sha256': sha(reference / 'artifact-sha256.json'),
              'scope': 'portable rings::Strummer + rings::Part kernel with the FillBuffer noise gate at 48 kHz / 24-frame blocks, int16 codec frames, source RNG seeded per case; not the native host adapter, resampler or hardware',
              'acceptance': 'bit-identical int16 odd and even samples and identical strum flags against the pinned oracle; zero guarded reads', 'cases': {}}
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
            expected = load(reference / name / 'output.txt')
            actual = load(target / 'output.txt')
            assert len(actual) == len(expected) == case['blocks']
            stats = diff(expected, actual, domain_end)
            item = {'passed': stats['mismatched_samples'] == 0 and stats['strum_mismatches'] == 0, 'samples': stats['samples'],
                    'mismatched_samples': stats['mismatched_samples'], 'max_abs_difference': stats['max_abs_difference'],
                    'strum_mismatches': stats['strum_mismatches'], 'guarded_reads': guarded}
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
    report['strum_mismatches'] = sum(item.get('strum_mismatches', 0) for item in items)
    report['excluded_undefined_samples'] = sum(item.get('source_undefined_domain', {}).get('excluded_samples', 0) for item in items)
    (evidence / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    artifacts = {str(path.relative_to(evidence)): sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases_passed': sum(item['passed'] for item in items), 'cases': len(cases),
                      'compared_samples': report['compared_samples'], 'mismatched_samples': report['mismatched_samples'],
                      'strum_mismatches': report['strum_mismatches'], 'excluded_undefined_samples': report['excluded_undefined_samples'],
                      'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
