#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Revalidate immutable actual traces under the documented float32 endpoint erratum.

Does not compile or execute DSP. The original declaration, validator, failed
attempt and every raw trace remain unchanged. All gates except the erroneous
open phase interval remain identical; exact cross-phase equality is retained.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import tempfile

from fixtures import check_declaration
from verify_phase_endpoint import validate, validate_kernel

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def stable(value):
    return json.dumps(value, indent=2, sort_keys=True) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', required=True, type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/stock-vco-reference.json')
    args = parser.parse_args()
    source = args.evidence.resolve()
    assert ROOT / 'native-test-output' in source.parents
    original = json.loads((source / 'result.json').read_text())
    assert original['passed'] is False and original['kernel']['passed'] is True
    assert digest(source / 'artifact-sha256.json') == original['artifact_manifest_sha256']
    artifacts = json.loads((source / 'artifact-sha256.json').read_text())
    for name, sha in artifacts.items():
        assert digest(source / name) == sha, ('retained artifact changed', name)
    production = original['production_source_sha256']
    for name, sha in production.items():
        assert digest(ROOT / name) == sha, ('production changed', name)
    for name in ['acceptance.json', 'fixtures.json', 'fixtures.py', 'README.md', 'verify.py']:
        assert digest(HERE / name) == digest(source / name), ('frozen contract changed', name)
    erratum_path = HERE / 'phase_endpoint_erratum.json'
    erratum = json.loads(erratum_path.read_text())
    # The revised validator must remain a minimal, reviewable correction.
    text = (HERE / 'verify.py').read_text()
    revised = (HERE / 'verify_phase_endpoint.py').read_text()
    expected = text.replace('if not 0 <= v[68+lane] < 1:', 'if not 0 <= v[68+lane] <= 1:').replace('outside [0,1):', 'outside [0,1]:')
    shebang, body = expected.split('\n', 1)
    assert revised.endswith(body), 'Revised validator body differs beyond the documented endpoint bound'
    prefix = revised[:-len(body)].splitlines()
    assert prefix[0] == shebang and all(line.startswith('#') for line in prefix), 'Only explanatory comments may precede the unchanged body'
    assert digest(HERE / 'verify_phase_endpoint.py') == erratum['new_validator']['sha256']
    for name, sha in erratum['original_sha256'].items():
        assert digest(HERE / name) == sha, ('erratum original hash mismatch', name)
    check_declaration()
    base = ROOT / 'native-test-output/stock-vco/phase-endpoint-revalidation'
    base.mkdir(parents=True, exist_ok=True)
    index = 1
    while (base / f'attempt-{index:03}').exists():
        index += 1
    retained = base / f'attempt-{index:03}'
    retained.mkdir()
    for name in ['phase_endpoint_erratum.json', 'verify_phase_endpoint.py', 'revalidate_phase_endpoint.py']:
        shutil.copy2(HERE / name, retained / name)
    shutil.copy2(source / 'result.json', retained / 'original-failed-result.json')
    shutil.copy2(source / 'artifact-sha256.json', retained / 'original-artifact-sha256.json')
    report = dict(original)
    report.update({'passed': False, 'rates': [], 'safety_rates': [],
        'qualification_basis': 'Original immutable declaration plus versioned float32 phase-endpoint erratum; not a pass against the original open-interval invariant',
        'phase_endpoint_erratum': erratum,
        'phase_endpoint_erratum_sha256': digest(erratum_path),
        'revalidation_evidence_directory': str(retained.relative_to(ROOT)),
        'original_failed_result_sha256': digest(source / 'result.json'),
        'revalidation_driver_sha256': {name: digest(HERE / name) for name in ['phase_endpoint_erratum.json', 'verify_phase_endpoint.py', 'revalidate_phase_endpoint.py']},
        'revalidation_invokes_compiler_or_dsp': False,
        'retained_artifact_files_verified': len(artifacts),
        'unchanged_production_files_verified': len(production)})
    report['kernel'] = validate_kernel(original['acceptance'], source / 'kernel-sdk.txt', source / 'kernel-rust.txt')
    report['kernel']['trace_sha256'] = original['kernel']['trace_sha256']
    print(json.dumps({'phase': 'kernel', 'passed': report['kernel']['passed'], 'unchanged_production_files': len(production)}), flush=True)
    for rate in original['acceptance']['sample_rates']:
        for phase in ['reference', 'safety']:
            previous = next(item for item in original['safety_rates' if phase == 'safety' else 'rates'] if item['sample_rate'] == rate)
            with tempfile.TemporaryDirectory(prefix='openpatch-vco-phase-revalidation-') as temporary:
                paths = {}
                labels = ['inputs', 'rust'] + ([] if phase == 'safety' else ['sdk'])
                for label in labels:
                    name = f'{phase}-{label}-{rate}.txt'
                    target = Path(temporary) / name
                    hasher = hashlib.sha256()
                    with gzip.open(source / (name + '.gz'), 'rb') as incoming, target.open('wb') as sink:
                        for chunk in iter(lambda: incoming.read(1024 * 1024), b''):
                            hasher.update(chunk); sink.write(chunk)
                    expected_hash = previous['input_sha256'] if label == 'inputs' else previous['trace_sha256'][label]
                    assert hasher.hexdigest() == expected_hash, ('decompressed trace changed', name)
                    paths[label] = target
                result = validate(rate, original['acceptance'], phase, paths['inputs'], paths.get('sdk'), paths['rust'])
            result.update({'sample_rate': rate, 'input_sha256': previous['input_sha256'], 'trace_sha256': previous['trace_sha256']})
            report['safety_rates' if phase == 'safety' else 'rates'].append(result)
            (retained / f'{phase}-{rate}-result.json').write_text(stable(result))
            print(json.dumps({'phase': phase, 'rate': rate, 'passed': result['passed'], 'frames': result['frames'], 'failed_frames': result['failed_frames']}), flush=True)
    for name, sha in production.items():
        assert digest(ROOT / name) == sha, ('production changed during revalidation', name)
    report['passed'] = report['kernel']['passed'] and all(item['passed'] for item in report['rates'] + report['safety_rates'])
    report['reference_frames'] = sum(item['frames'] for item in report['rates'])
    report['safety_frames'] = sum(item['frames'] for item in report['safety_rates'])
    report['revalidation_artifact_manifest'] = {str(path.relative_to(retained)): digest(path) for path in sorted(retained.rglob('*')) if path.is_file()}
    (retained / 'result.json').write_text(stable(report))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(stable(report))
    print(json.dumps({'passed': report['passed'], 'reference_frames': report['reference_frames'], 'safety_frames': report['safety_frames'], 'report': str(args.output)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
