#!/usr/bin/env python3
"""Compare actual Rust kernels against an existing immutable C++ reference run."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    reference = args.reference.resolve()
    assert json.loads((reference / 'result.json').read_text())['passed']
    for name, expected in json.loads((reference / 'artifact-sha256.json').read_text()).items():
        assert sha(reference / name) == expected, f'changed reference: {name}'
    cases = json.loads((reference / 'cases.json').read_text())
    base = ROOT / 'native-test-output/tides2-reference'
    index = 1
    while (base / f'rust-comparison-{index:03}').exists():
        index += 1
    evidence = base / f'rust-comparison-{index:03}'
    evidence.mkdir(parents=True)
    sources = [HERE / 'rust_probe.rs', HERE / 'compare.py']
    sources += sorted((ROOT / 'crates/openpatch-core/src/tidal_functions').rglob('*.rs'))
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
    command = ['rustc', '--edition=2021', '-O', str(evidence / 'source/scripts/tides2_reference/rust_probe.rs'), '-o', str(executable)]
    (evidence / 'compile-command.json').write_text(json.dumps(command, indent=2) + '\n')
    report = {'passed': False, 'reference': str(reference), 'reference_manifest_sha256': sha(reference / 'artifact-sha256.json'), 'scope': 'portable raw kernels; not native adapter or hardware qualification', 'cases': {}}
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=30)
        (evidence / 'compile.stderr').write_text(result.stderr)
        result.check_returncode()
        for case in cases['negative_cases'] + cases['cases']:
            name = case['id']
            target = evidence / name
            target.mkdir()
            with (target / 'output.jsonl').open('w') as output, (target / 'stderr.txt').open('w') as errors:
                result = subprocess.run([str(executable)], input=(reference / name / 'input.txt').read_text(), text=True, stdout=output, stderr=errors, timeout=30)
            if case in cases['negative_cases']:
                assert result.returncode != 0 and (target / 'stderr.txt').read_text() == case['expected_error'] + '\n'
                assert (target / 'output.jsonl').stat().st_size == 0
                report['cases'][name] = {'passed': True, 'negative': True}
                continue
            result.check_returncode()
            expected = [json.loads(line) for line in (reference / name / 'output.jsonl').open()]
            actual = [json.loads(line) for line in (target / 'output.jsonl').open()]
            assert len(actual) == len(expected) == case['blocks'] * case['block_size']
            if name.startswith('amplitude_scan_'):
                assert all(any(record['output_bits'][i] & 0x7fffffff for record in expected) for i in range(4)), 'amplitude scan failed to exercise every output'
            mismatches = [(i, a, b) for i, (a, b) in enumerate(zip(expected, actual)) if a != b]
            report['cases'][name] = {'passed': not mismatches, 'frames': len(actual), 'mismatched_frames': len(mismatches)}
            if mismatches:
                (target / 'first-mismatch.json').write_text(json.dumps(mismatches[0], indent=2) + '\n')
        report['passed'] = all(item['passed'] for item in report['cases'].values())
    except Exception as error:
        report['error'] = str(error)
    report['compared_frames'] = sum(item.get('frames', 0) for item in report['cases'].values())
    (evidence / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    artifacts = {str(path.relative_to(evidence)): sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases_passed': sum(item['passed'] for item in report['cases'].values()), 'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
