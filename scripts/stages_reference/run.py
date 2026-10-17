#!/usr/bin/env python3
"""Freeze genuine unmodified Stages DSP and audit its defined input domain."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import subprocess
import sys
from fetch import DEFAULT_CACHE, HERE, ROOT, fetch


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def protocol(case):
    return f"{case['quantized']} {case['seed']}\n" + ''.join(
        ' '.join(map(str, op)) + '\n' for op in case['operations'])


def records(case):
    return sum(op[2] * op[3] * (1 + op[7]) for op in case['operations'] if op[0] == 'B')


def validate(document):
    assert document['schema'] == 1
    ids = set()
    for case in document['cases'] + document['negative_cases'] + document.get('audit_negative_cases', []):
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        assert case['quantized'] in (0, 1) and 0 <= case['seed'] <= 0xffffffff
        for op in case['operations']:
            assert op[0] in ('C', 'I', 'P', 'S', 'B') and 0 <= op[1] < 6
            if op[0] in ('C', 'I'):
                assert 1 <= op[3] <= 36 and len(op) == 4 + 2 * op[3]
                assert op[2] in (0, 1)
                assert all(op[i] in (0, 1, 2) and op[i+1] in (0, 1) for i in range(4, len(op), 2))
                assert op[0] != 'I' or op[3] == 1
            elif op[0] == 'P':
                assert len(op) == 5 and 0 <= op[2] < 36
            elif op[0] == 'S':
                assert len(op) == 3 and 0 <= op[2] < 36
            else:
                assert len(op) == 8 + op[7] and 1 <= op[2] <= 64 and 0 <= op[3] <= 50000
                assert 0 <= op[5] <= op[4] and 0 <= op[6] and 0 <= op[7] < 6
                assert len(set(op[8:])) == op[7] and all(0 <= x < 6 and x != op[1] for x in op[8:])
    return document['cases'], document['negative_cases'], document.get('audit_negative_cases', [])


def execute(executable, encoded, target, prefix=''):
    with (target/(prefix+'output.jsonl')).open('w') as out, (target/(prefix+'stderr.txt')).open('w') as err:
        return subprocess.run([str(executable)], input=encoded, text=True,
                              stdout=out, stderr=err, timeout=90)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=DEFAULT_CACHE)
    parser.add_argument('--cases', type=Path, default=HERE/'cases.json')
    parser.add_argument('--validate-only', action='store_true')
    parser.add_argument('--compiler', default='clang++')
    args = parser.parse_args()
    cases, negatives, audit_negatives = validate(json.loads(args.cases.read_text()))
    if args.validate_only:
        print(json.dumps({'validated': True, 'cases': len(cases), 'negative_cases': len(negatives), 'audit_negative_cases': len(audit_negatives),
                          'records': sum(map(records, cases)), 'dsp_executed': False}))
        return 0
    manifest = fetch(args.cache)
    base = ROOT/'native-test-output/stages-reference'
    index = 1
    while (base/f'attempt-{index:03}').exists():
        index += 1
    evidence = base/f'attempt-{index:03}'
    evidence.mkdir(parents=True)
    report = {'passed': False, 'cases': len(cases), 'negative_cases': len(negatives), 'audit_negative_cases': len(audit_negatives),
              'scope': 'raw pinned STANDARD3 Stages DSP, not native adapter/firmware/hardware', 'stage': 'prepare'}
    try:
        for name in ('source_manifest.json', 'reference_probe.cpp', 'fetch.py', 'run.py', 'NOTICE.md', 'README.md'):
            shutil.copy2(HERE/name, evidence/name)
        shutil.copy2(args.cases, evidence/'cases.json')
        retained = evidence/'upstream'
        for name in manifest['files']:
            target = retained/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(args.cache/name, target)
        (evidence/'compiler.txt').write_text(subprocess.check_output([args.compiler, '--version'], text=True))
        executable = evidence/'reference_probe'
        command = [args.compiler, '-std=c++11', '-O2', '-DTEST', '-ffp-contract=off', '-fno-fast-math',
                   '-I', str(retained), str(evidence/'reference_probe.cpp')]
        if sys.platform == 'darwin':
            sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
            command += ['-isysroot', str(sdk), '-isystem', str(sdk/'usr/include/c++/v1')]
        command += [str(retained/name) for name in manifest['compile_sources']] + ['-o', str(executable)]
        report['stage'] = 'compile'
        (evidence/'compile-command.json').write_text(json.dumps(command, indent=2)+'\n')
        build = subprocess.run(command, capture_output=True, text=True, timeout=90)
        (evidence/'compile.stdout').write_text(build.stdout)
        (evidence/'compile.stderr').write_text(build.stderr)
        build.check_returncode()
        sanitized = evidence/'reference_probe_sanitized'
        sanitizer = command[:-2] + ['-fsanitize=undefined,bounds,float-cast-overflow,float-divide-by-zero',
                                   '-fno-sanitize-recover=all', '-o', str(sanitized)]
        report['stage'] = 'sanitizer compile'
        (evidence/'sanitizer-compile-command.json').write_text(json.dumps(sanitizer, indent=2)+'\n')
        build = subprocess.run(sanitizer, capture_output=True, text=True, timeout=90)
        (evidence/'sanitizer-compile.stdout').write_text(build.stdout)
        (evidence/'sanitizer-compile.stderr').write_text(build.stderr)
        build.check_returncode()
        counts, audits, bounds, activity = {}, {}, {}, {}
        report['stage'] = 'source execution and sanitizer audit'
        for case in cases + negatives + audit_negatives:
            target = evidence/case['id']
            target.mkdir()
            encoded = protocol(case)
            (target/'input.txt').write_text(encoded)
            result = execute(executable, encoded, target)
            if case in negatives:
                assert result.returncode != 0, 'expected rejection/nonfinite source: '+case['id']
                assert (target/'stderr.txt').read_text() == case['expected_error']+'\n'
                bounds[case['id']] = {'passed': True, 'scope': case['scope']}
                continue
            result.check_returncode()
            count, nonzero, segments = 0, 0, set()
            for line in (target/'output.jsonl').open():
                item = json.loads(line)
                assert len(item) == 7 and 0 <= item[1] < 6 and 0 <= item[2] <= 7
                assert all(0 <= item[i] <= 0xffffffff for i in (3, 4, 6))
                segments.add(item[5])
                nonzero += struct.unpack('<f', struct.pack('<I', item[3]))[0] != 0.0
                count += 1
            assert count == records(case)
            if case in audit_negatives:
                result = execute(sanitized, encoded, target, 'sanitizer-')
                assert result.returncode != 0
                assert case['expected_diagnostic'] in (target/'sanitizer-stderr.txt').read_text()
                bounds[case['id']] = {'passed': True, 'scope': case['scope'], 'optimized_output_finite': True, 'records': count, 'sanitizer_rejected': True}
                continue
            counts[case['id']] = count
            activity[case['id']] = {'nonzero_records': nonzero, 'segments': sorted(segments)}
            result = execute(sanitized, encoded, target, 'sanitizer-')
            result.check_returncode()
            assert (target/'sanitizer-stderr.txt').stat().st_size == 0
            assert sha(target/'output.jsonl') == sha(target/'sanitizer-output.jsonl')
            audits[case['id']] = {'passed': True, 'records': count, 'bit_records_identical': True}
        report.update(passed=True, stage='frozen source traces and sanitizer audit passed',
                      records=sum(counts.values()), records_by_case=counts, source_activity=activity,
                      negative_domains=bounds, sanitizer_audit=audits, production_equivalence=False)
    except Exception as error:
        report['error'] = str(error)
    (evidence/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    artifacts = {str(p.relative_to(evidence)): sha(p) for p in sorted(evidence.rglob('*')) if p.is_file()}
    (evidence/'artifact-sha256.json').write_text(json.dumps(artifacts, indent=2, sort_keys=True)+'\n')
    print(json.dumps({'passed': report['passed'], 'stage': report['stage'], 'evidence': str(evidence), 'error': report.get('error')}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
