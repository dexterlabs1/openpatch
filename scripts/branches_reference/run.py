#!/usr/bin/env python3
"""Execute original Branches firmware and compare exact raw production Rust scans."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
from fetch import DEFAULT_CACHE, HERE, ROOT, fetch, sha


def validate(document):
    assert document['schema'] == 1
    ids = set()
    for case in document['cases']:
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        assert 0 <= case['seed'] <= 0xffffffff
        assert case['scans']
        for row in case['scans']:
            assert len(row) == 9 and all(type(v) is int for v in row)
            assert 0 < row[0] <= 100000
            assert all(row[i] in (0, 1) for i in (1, 2, 5, 6, 7, 8))
            assert all(0 <= row[i] <= 65535 for i in (3, 4))
    return document['cases']


def invoke(command, directory, name, **kwargs):
    (directory / (name+'-command.json')).write_text(json.dumps(command, indent=2)+'\n')
    result = subprocess.run(command, capture_output=True, text=True, timeout=90, **kwargs)
    (directory / (name+'.stdout')).write_text(result.stdout)
    (directory / (name+'.stderr')).write_text(result.stderr)
    result.check_returncode()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=DEFAULT_CACHE)
    parser.add_argument('--cases', type=Path, default=HERE/'cases.json')
    parser.add_argument('--compiler', default='clang++')
    parser.add_argument('--rustc', default='rustc')
    parser.add_argument('--validate-only', action='store_true')
    args = parser.parse_args()
    cases = validate(json.loads(args.cases.read_text()))
    scans = sum(row[0] for case in cases for row in case['scans'])
    if args.validate_only:
        print(json.dumps({'validated': True, 'cases': len(cases), 'scans': scans, 'dsp_executed': False}))
        return 0
    manifest = fetch(args.cache)
    base = ROOT/'native-test-output/branches-reference'
    index = 1
    while (base/f'attempt-{index:03}').exists():
        index += 1
    evidence = base/f'attempt-{index:03}'
    evidence.mkdir(parents=True)
    report = {'passed': False, 'cases': len(cases), 'scans': scans,
              'scope': 'raw firmware scan; excludes ADC/button/EEPROM scheduling, circuit and CPU timing',
              'stage': 'prepare', 'production_equivalence': False}
    try:
        for name in ('reference_probe.cpp', 'fetch.py', 'run.py', 'rust_probe.rs',
                     'source_manifest.json', 'NOTICE.md', 'README.md'):
            shutil.copy2(HERE/name, evidence/name)
        shutil.copy2(args.cases, evidence/'cases.json')
        shutil.copytree(HERE/'stubs', evidence/'stubs')
        retained = evidence/'upstream'
        for name in manifest['files']:
            destination = retained/name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(args.cache/name, destination)
        kernel = ROOT/'crates/openpatch-core/src/event_fork/kernel.rs'
        shutil.copy2(kernel, evidence/'production-kernel.rs')
        (evidence/'compiler.txt').write_text(subprocess.check_output([args.compiler, '--version'], text=True))
        (evidence/'rustc.txt').write_text(subprocess.check_output([args.rustc, '--version', '--verbose'], text=True))
        command = [args.compiler, '-std=c++17', '-O2', '-Wno-return-type',
                   '-I', str(evidence/'stubs'), '-I', str(retained), str(evidence/'reference_probe.cpp')]
        if sys.platform == 'darwin':
            sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
            command += ['-isysroot', str(sdk), '-isystem', str(sdk/'usr/include/c++/v1')]
        source = evidence/'reference_probe'
        sanitized = evidence/'reference_probe_sanitized'
        report['stage'] = 'compile and retain preprocessed source'
        invoke(command+['-o', str(source)], evidence, 'compile')
        invoke(command+['-fsanitize=undefined,bounds', '-fno-sanitize-recover=all',
                        '-o', str(sanitized)], evidence, 'sanitizer-compile')
        invoke(command+['-E', '-o', str(evidence/'reference_probe.ii')], evidence, 'preprocess')
        rust = evidence/'rust_probe'
        # Compile the committed direct-path adapter against the production file;
        # retain both exact inputs in evidence and verify it was not mutated.
        invoke([args.rustc, '--edition=2021', '-O', str(HERE/'rust_probe.rs'), '-o', str(rust)],
               evidence, 'rust-compile')
        counts = {}
        report['stage'] = 'genuine source, sanitizer, Rust exact comparison'
        for case in cases:
            target = evidence/case['id']
            target.mkdir()
            encoded = str(case['seed'])+'\n'+''.join(' '.join(map(str,row))+'\n' for row in case['scans'])
            (target/'input.txt').write_text(encoded)
            for name, executable in (('source', source), ('sanitizer', sanitized), ('rust', rust)):
                invoke([str(executable)], target, name, input=encoded)
                assert (target/(name+'.stderr')).stat().st_size == 0
            actual = (target/'source.stdout').read_bytes()
            assert actual == (target/'sanitizer.stdout').read_bytes(), 'sanitizer trace mismatch: '+case['id']
            other = (target/'rust.stdout').read_bytes()
            if actual != other:
                for number, (left, right) in enumerate(zip(actual.splitlines(), other.splitlines())):
                    if left != right:
                        raise AssertionError(f'{case["id"]} record {number}: source {left!r}; Rust {right!r}')
                raise AssertionError('trace length mismatch: '+case['id'])
            rows = [json.loads(line) for line in actual.splitlines()]
            assert len(rows) == 256 + sum(row[0] for row in case['scans'])
            assert all(row[:2] == ['adc', i] and 0 <= row[2] <= 65535 for i, row in enumerate(rows[:256]))
            assert all(len(row) == 14 and row[0] == i for i,row in enumerate(rows[256:]))
            counts[case['id']] = len(rows)-256
        assert sha(kernel) == sha(evidence/'production-kernel.rs'), 'production file changed during qualification'
        report.update(passed=True, stage='exact source scan comparison passed',
                      records_by_case=counts, adc_table_records=256*len(cases),
                      source_sanitizer_passed=True, raw_kernel_equivalence=True)
    except Exception as error:
        report['error'] = str(error)
    (evidence/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    artifacts = {str(path.relative_to(evidence)):sha(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence/'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True)+'\n')
    print(json.dumps({'passed': report['passed'], 'stage': report['stage'], 'scans': scans,
                      'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
