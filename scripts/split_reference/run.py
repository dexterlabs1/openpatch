#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify bounded Split rows using the actual upstream class and public Engine."""
import argparse
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys

from fixtures import fixture_text, rows, raw_fixture_text, raw_rows

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ACCEPTANCE_PATH = HERE / 'acceptance.json'
FROZEN = HERE / 'fixtures.json'
ACCEPTANCE = json.loads(ACCEPTANCE_PATH.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stable_json(value):
    return json.dumps(value, indent=2, sort_keys=True) + '\n'


def manifest(text, raw_text):
    return {
        'declaration': 'Frozen before first oracle comparison; generator and acceptance are immutable',
        'acceptance_sha256': digest(ACCEPTANCE_PATH),
        'generator_sha256': digest(HERE / 'fixtures.py'),
        'rows_per_rate': len(text.splitlines()),
        'total_rows': len(text.splitlines()) * len(ACCEPTANCE['sample_rates']),
        'input_sha256': {str(rate): hashlib.sha256(text.encode()).hexdigest()
                         for rate in ACCEPTANCE['sample_rates']},
        'raw_rows_per_rate': len(raw_text.splitlines()),
        'raw_total_rows': len(raw_text.splitlines()) * len(ACCEPTANCE['sample_rates']),
        'raw_input_sha256': {str(rate): hashlib.sha256(raw_text.encode()).hexdigest()
                             for rate in ACCEPTANCE['sample_rates']},
    }


def bits(value):
    return struct.pack('<f', value).hex()


def compressed(path, data):
    path.write_bytes(gzip.compress(data, mtime=0))


def source_closure():
    paths = sorted((ROOT / 'crates/openpatch-core/src').rglob('*.rs'))
    paths += [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock',
              ROOT / 'crates/openpatch-core/Cargo.toml',
              ROOT / 'crates/openpatch-core/LICENSE-THIRD-PARTY',
              ROOT / 'crates/openpatch-core/examples/split_reference_probe.rs']
    return {str(path.relative_to(ROOT)): digest(path) for path in paths}


def check_schema(text, raw_text):
    all_rows, diagnostics = list(rows(ACCEPTANCE)), list(raw_rows(ACCEPTANCE))
    assert len(all_rows) == ACCEPTANCE['rows_per_rate'] == 355
    assert len(all_rows) * len(ACCEPTANCE['sample_rates']) == ACCEPTANCE['total_rows'] == 1065
    assert len(all_rows) * len(ACCEPTANCE['sample_rates']) * 16 == ACCEPTANCE['total_compared_output_values']
    assert len(text.splitlines()) == len(all_rows)
    assert len(diagnostics) == ACCEPTANCE['raw_rows_per_rate'] == 68
    assert len(diagnostics) * len(ACCEPTANCE['sample_rates']) == ACCEPTANCE['raw_total_rows'] == 204
    assert len(diagnostics) * len(ACCEPTANCE['sample_rates']) * 16 == ACCEPTANCE['raw_total_compared_output_values']
    assert len(raw_text.splitlines()) == len(diagnostics)
    assert len({name for name, _ in all_rows}) == len(all_rows)
    assert len({name for name, _ in diagnostics}) == len(diagnostics)
    for _, row in all_rows:
        assert len(row) == ACCEPTANCE['fields_per_input_row'] == 17
        width = row[0]
        assert isinstance(width, int) and width in ACCEPTANCE['input_widths']
        assert all(math.isfinite(v) and abs(v) <= ACCEPTANCE['maximum_absolute_active_volts']
                   for v in row[1:])
        assert all(bits(v) == bits(0.0) for v in row[1 + width:])
    for _, row in diagnostics:
        assert len(row) == 17 and row[0] in ACCEPTANCE['input_widths']
        assert all(len(token) == 8 and token in ACCEPTANCE['raw_value_bits'] for token in row[1:])
        assert all(0 <= int(token, 16) <= 0xffffffff for token in row[1:])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze-fixtures', action='store_true')
    parser.add_argument('--check-fixtures', action='store_true', help='Pure Python validation; no oracle/build/network')
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--rust-probe', type=Path, help='Existing root-built split_reference_probe executable; never builds Cargo')
    parser.add_argument('--rust-build-command', default='cargo build -p openpatch-core --example split_reference_probe --release',
                        help='Record the actual external command used to build --rust-probe')
    parser.add_argument('--rust-toolchain', help='Actual externally recorded Rust toolchain; runner never invokes rustc')
    parser.add_argument('--evidence', type=Path, default=ROOT / 'native-test-output/split/qualification')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/split-reference.json')
    args = parser.parse_args()
    text, raw_text = fixture_text(ACCEPTANCE), raw_fixture_text(ACCEPTANCE)
    check_schema(text, raw_text)
    declaration = manifest(text, raw_text)
    if args.freeze_fixtures:
        # Exclusive create is intentional: no preexisting declaration can be replaced.
        with FROZEN.open('x') as out:
            out.write(stable_json(declaration))
        print('Fixtures frozen; no build or oracle comparison performed')
        return 0
    assert FROZEN.exists(), 'Run --freeze-fixtures before the first oracle comparison'
    assert json.loads(FROZEN.read_text()) == declaration, 'Frozen acceptance/generator/input hashes differ'
    if args.check_fixtures:
        print(json.dumps({'passed': True, 'rows': declaration['total_rows'], 'raw_rows': declaration['raw_total_rows'], 'fixtures_sha256': digest(FROZEN)}))
        return 0
    if args.rust_probe is None:
        parser.error('--rust-probe must identify an existing externally built executable')
    if not args.rust_toolchain:
        parser.error('--rust-toolchain must record the toolchain used by the external Rust build')
    sdk, rust = args.sdk.resolve(), args.rust_probe.resolve()
    assert rust.is_file() and os.access(rust, os.X_OK), 'Rust probe is not executable'
    evidence = args.evidence.resolve()
    assert ROOT / 'native-test-output' in evidence.parents, 'Evidence must remain in ignored native-test-output storage'
    evidence.mkdir(parents=True, exist_ok=True)
    source = ROOT / 'native-test-output/split/reference/Split.cpp'
    license_path = source.with_name('LICENSE.md')
    assert digest(source) == ACCEPTANCE['reference_source_sha256'], 'Pinned upstream source mismatch'
    assert license_path.is_file(), 'Retained upstream GPL license is required'
    upstream = source.read_text()
    assert upstream.count('struct SplitChannelDisplay') == 1
    prefix = upstream.split('struct SplitChannelDisplay', 1)[0]
    assert prefix.count('#include "plugin.hpp"') == 1
    prefix = prefix.replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    assert prefix.count('struct Split : Module') == 1
    (evidence / 'reference_module.hpp').write_text(prefix)
    shutil.copy2(source, evidence / 'Split.cpp')
    shutil.copy2(license_path, evidence / 'LICENSE.md')
    for name in ['acceptance.json', 'fixtures.json', 'fixtures.py', 'run.py', 'sdk_probe.cpp', 'README.md']:
        shutil.copy2(HERE / name, evidence / name)
    library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
    shutil.copy2(library, evidence / library.name)
    # Retain the actual complete include trees used, rather than merely their hashes.
    header_hashes = {}
    for directory in ['include', 'dep/include']:
        tree = sdk / directory
        shutil.copytree(tree, evidence / 'sdk' / directory, dirs_exist_ok=True)
        for path in sorted(tree.rglob('*')):
            if path.is_file():
                header_hashes[str(path.relative_to(sdk))] = digest(path)
    closure = source_closure()
    assert rust.stat().st_mtime_ns >= max((ROOT / name).stat().st_mtime_ns for name in closure), 'Rebuild production probe after source changes'
    for relative in closure:
        target = evidence / 'production' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / relative, target)
    saved_rust = evidence / 'rust_probe'
    shutil.copy2(rust, saved_rust)
    cpp = evidence / 'sdk_probe'
    flags = []
    if sys.platform == 'darwin':
        system_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
        flags = ['-isysroot', str(system_sdk), '-isystem', str(system_sdk / 'usr/include/c++/v1')]
    command = ['c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags,
               '-I' + str(sdk / 'include'), '-I' + str(sdk / 'dep/include'),
               '-I' + str(evidence), str(HERE / 'sdk_probe.cpp'), '-L' + str(sdk),
               '-lRack', '-Wl,-rpath,' + str(sdk), '-o', str(cpp)]
    (evidence / 'build-command.json').write_text(stable_json({
        'cpp_argv': command, 'external_rust_command': args.rust_build_command,
        'external_rust_executable': str(rust), 'runner_invokes_cargo': False,
    }))
    built = subprocess.run(command, text=True, capture_output=True)
    (evidence / 'build.stdout').write_text(built.stdout)
    (evidence / 'build.stderr').write_text(built.stderr)
    built.check_returncode()
    env = os.environ.copy()
    env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(evidence)
    report = {
        'reference': {'fundamental_commit': ACCEPTANCE['reference_commit'],
                      'sdk_version_required': '2.6.6',
                      'source_url': f"https://raw.githubusercontent.com/VCVRack/Fundamental/{ACCEPTANCE['reference_commit']}/src/Split.cpp",
                      'source_sha256': digest(source), 'license_sha256': digest(license_path),
                      'compiled_prefix_sha256': digest(evidence / 'reference_module.hpp'),
                      'sdk_library_sha256': digest(library), 'sdk_header_sha256': header_hashes,
                      'sdk_headers_aggregate_sha256': hashlib.sha256(stable_json(header_hashes).encode()).hexdigest()},
        'acceptance': ACCEPTANCE, 'acceptance_sha256': digest(ACCEPTANCE_PATH),
        'fixtures_sha256': digest(FROZEN), 'production_source_sha256': closure,
        'production_source_aggregate_sha256': hashlib.sha256(stable_json(closure).encode()).hexdigest(),
        'probe_binary_sha256': {'sdk': digest(cpp), 'rust': digest(saved_rust)},
        'driver_sha256': {name: digest(HERE / name) for name in ['run.py', 'fixtures.py', 'sdk_probe.cpp']},
        'build_command': command, 'external_rust_build_command': args.rust_build_command,
        'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0],
                      'rust': args.rust_toolchain,
                      'python': sys.version,
                      'platform': sys.platform, 'machine': os.uname().machine},
        'method': 'Normal phase: actual pinned SDK Split class with connected mono outputs versus public production Engine poly_constant -> Split and sixteen channel-zero resolved taps. Both fresh per row; two Engine frames settle one cable. Exact input count and all sixteen output values/active flags compared to frozen legal zero-padded fixtures. Initial input count asserted zero by both adapters. Raw phase: same actual class versus exact private production split.rs included by path in the Rust example, using raw u32 bit storage, no DSP mirror and no public test API. All raw slots compared independently of declared width; this deliberately bypasses host cable/producer bookkeeping.',
        'limitations': ['Normal phase covers bounded finite legal Engine frames only; no full-parity or hardware claim',
                        'Fresh graph/class per row; no adoption, bypass or initialization qualification from this harness',
                        'Core tests separately cover callback partition, allocation and lifecycle policies',
                        'No GUI, audio device, physical cable latency or panel rendering qualification',
                        'Raw diagnostic phase bypasses host producers and cables; poisoned internal-frame behavior is not a Rack host parity claim',
                        'Rust normal-phase output count column is the real channel-zero tap-active flag; Core tests independently assert mono output frame shape',
                        'Only recorded compiler/SDK/platform combination is qualified'],
        'initial_count_verified_by_adapters': ACCEPTANCE['initial_count'],
        'raw_rates': [],
        'rates': [], 'evidence_directory': str(evidence.relative_to(ROOT)),
    }
    total_values = total_raw_values = 0
    for raw, data, fixtures in [(False, text, list(rows(ACCEPTANCE))),
                                (True, raw_text, list(raw_rows(ACCEPTANCE)))]:
        phase = 'raw' if raw else 'normal'
        for rate in ACCEPTANCE['sample_rates']:
            compressed(evidence / f'{phase}-inputs-{rate}.txt.gz', data.encode())
            results = {}
            for label, exe in [('sdk', cpp), ('rust', saved_rust)]:
                argv = [str(exe), str(rate), *(['--raw'] if raw else [])]
                result = subprocess.run(argv, input=data, text=True, capture_output=True, env=env)
                compressed(evidence / f'{phase}-{label}-{rate}.txt.gz', result.stdout.encode())
                (evidence / f'{phase}-{label}-{rate}.stderr').write_text(result.stderr)
                result.check_returncode()
                results[label] = result.stdout
            parsed = {label: [line.split() for line in trace.splitlines()]
                      for label, trace in results.items()}
            assert all(len(trace) == len(fixtures) for trace in parsed.values()), 'Trace row count mismatch'
            failures, compared, failed_rows = [], 0, 0
            for index, (name, row) in enumerate(fixtures):
                sdk_row, rust_row = parsed['sdk'][index], parsed['rust'][index]
                assert len(sdk_row) == len(rust_row) == ACCEPTANCE['fields_per_output_row'] == 33
                reasons = []
                for label, values in [('sdk', sdk_row), ('rust', rust_row)]:
                    if int(values[0]) != row[0]:
                        reasons.append(f'{label}: count {values[0]} != {row[0]}')
                    for lane in range(16):
                        if int(values[1 + lane * 2]) != 1:
                            reasons.append(f'{label}: output {lane + 1} count/active flag is not one')
                for lane in range(16):
                    ref, actual = sdk_row[2 + lane * 2], rust_row[2 + lane * 2]
                    if raw:
                        assert len(ref) == len(actual) == 8, 'Raw bit field width mismatch'
                        ref_bits, actual_bits, expected_bits = int(ref, 16), int(actual, 16), int(row[lane + 1], 16)
                    else:
                        assert math.isfinite(float(ref)) and math.isfinite(float(actual)), 'Nonfinite normal trace'
                        ref_bits, actual_bits, expected_bits = bits(float(ref)), bits(float(actual)), bits(row[lane + 1])
                    if ref_bits != actual_bits or ref_bits != expected_bits:
                        reasons.append(f'lane {lane + 1}: SDK={ref_bits} Rust={actual_bits} fixture={expected_bits}')
                    compared += 1
                if reasons:
                    failed_rows += 1
                    if len(failures) < 16:
                        failures.append({'row': index, 'case': name, 'reasons': reasons})
            if raw:
                total_raw_values += compared
            else:
                total_values += compared
            report['raw_rates' if raw else 'rates'].append({
                'sample_rate': rate, 'rows': len(fixtures), 'compared_output_values': compared,
                'input_sha256': hashlib.sha256(data.encode()).hexdigest(),
                'trace_sha256': {label: hashlib.sha256(trace.encode()).hexdigest() for label, trace in results.items()},
                'failed_rows': failed_rows, 'first_failures': failures, 'passed': failed_rows == 0,
            })
            print(json.dumps({'phase': phase, 'rate': rate, 'rows': len(fixtures), 'failed_rows': failed_rows}), flush=True)
    assert source_closure() == closure, 'Production sources changed during qualification'
    report['passed'] = all(rate['passed'] for rate in report['rates'] + report['raw_rates'])
    report['total_rows'] = sum(rate['rows'] for rate in report['rates'])
    report['total_compared_output_values'] = total_values
    report['raw_total_rows'] = sum(rate['rows'] for rate in report['raw_rates'])
    report['raw_total_compared_output_values'] = total_raw_values
    assert report['total_rows'] == ACCEPTANCE['total_rows']
    assert total_values == ACCEPTANCE['total_compared_output_values']
    assert report['raw_total_rows'] == ACCEPTANCE['raw_total_rows']
    assert total_raw_values == ACCEPTANCE['raw_total_compared_output_values']
    artifacts = {str(path.relative_to(evidence)): digest(path)
                 for path in sorted(evidence.rglob('*')) if path.is_file() and path.name != 'artifact-sha256.json'}
    (evidence / 'artifact-sha256.json').write_text(stable_json(artifacts))
    report['artifact_manifest_sha256'] = digest(evidence / 'artifact-sha256.json')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(stable_json(report))
    print(json.dumps({'passed': report['passed'], 'rows': report['total_rows'], 'raw_rows': report['raw_total_rows'], 'report': str(args.output)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
