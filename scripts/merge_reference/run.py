#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify bounded Merge rows using the actual upstream class and public Engine."""
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

from fixtures import fixture_text, rows

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
ACCEPTANCE_PATH = HERE / 'acceptance.json'
FROZEN = HERE / 'fixtures.json'
ACCEPTANCE = json.loads(ACCEPTANCE_PATH.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stable_json(value):
    return json.dumps(value, indent=2, sort_keys=True) + '\n'


def manifest(text):
    return {
        'declaration': 'Frozen before first oracle comparison; generator and acceptance are immutable',
        'acceptance_sha256': digest(ACCEPTANCE_PATH),
        'generator_sha256': digest(HERE / 'fixtures.py'),
        'rows_per_rate': len(text.splitlines()),
        'total_rows': len(text.splitlines()) * len(ACCEPTANCE['sample_rates']),
        'input_sha256': {str(rate): hashlib.sha256(text.encode()).hexdigest()
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
              ROOT / 'crates/openpatch-core/examples/merge_reference_probe.rs']
    return {str(path.relative_to(ROOT)): digest(path) for path in paths}


def check_schema(text):
    all_rows = list(rows(ACCEPTANCE))
    assert len(all_rows) == ACCEPTANCE['rows_per_rate'] == 5472
    assert len(all_rows) * len(ACCEPTANCE['sample_rates']) == ACCEPTANCE['total_rows'] == 16416
    assert len(text.splitlines()) == len(all_rows)
    for _, row in all_rows:
        assert len(row) == 273
        assert row[0] in ACCEPTANCE['wire_configurations']
        for slot in range(16):
            width = row[1 + slot * 17]
            assert isinstance(width, int) and 0 <= width <= 16
            active = row[2 + slot * 17:2 + slot * 17 + width]
            assert all(math.isfinite(v) and abs(v) <= ACCEPTANCE['maximum_absolute_active_volts']
                       for v in active)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--freeze-fixtures', action='store_true')
    parser.add_argument('--check-fixtures', action='store_true', help='Pure Python validation; no oracle/build/network')
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--rust-probe', type=Path, help='Existing root-built merge_reference_probe executable; never builds Cargo')
    parser.add_argument('--rust-build-command', default='cargo build -p openpatch-core --example merge_reference_probe --release',
                        help='Record the actual external command used to build --rust-probe')
    parser.add_argument('--evidence', type=Path, default=ROOT / 'native-test-output/merge/qualification')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/merge-reference.json')
    args = parser.parse_args()
    text = fixture_text(ACCEPTANCE)
    check_schema(text)
    declaration = manifest(text)
    if args.freeze_fixtures:
        # Exclusive create is intentional: no preexisting declaration can be replaced.
        with FROZEN.open('x') as out:
            out.write(stable_json(declaration))
        print('Fixtures frozen; no build or oracle comparison performed')
        return 0
    assert FROZEN.exists(), 'Run --freeze-fixtures before the first oracle comparison'
    assert json.loads(FROZEN.read_text()) == declaration, 'Frozen acceptance/generator/input hashes differ'
    if args.check_fixtures:
        print(json.dumps({'passed': True, 'rows': declaration['total_rows'], 'fixtures_sha256': digest(FROZEN)}))
        return 0
    if args.rust_probe is None:
        parser.error('--rust-probe must identify an existing externally built executable')
    sdk, rust = args.sdk.resolve(), args.rust_probe.resolve()
    assert rust.is_file() and os.access(rust, os.X_OK), 'Rust probe is not executable'
    evidence = args.evidence.resolve()
    assert ROOT / 'native-test-output' in evidence.parents, 'Evidence must remain in ignored native-test-output storage'
    evidence.mkdir(parents=True, exist_ok=True)
    source = ROOT / 'native-test-output/merge/reference/Merge.cpp'
    license_path = source.with_name('LICENSE.md')
    assert digest(source) == ACCEPTANCE['reference_source_sha256'], 'Pinned upstream source mismatch'
    assert license_path.is_file(), 'Retained upstream GPL license is required'
    upstream = source.read_text()
    assert upstream.count('struct MergeChannelDisplay') == 1
    prefix = upstream.split('struct MergeChannelDisplay', 1)[0]
    assert prefix.count('#include "plugin.hpp"') == 1
    prefix = prefix.replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    assert prefix.count('struct Merge : Module') == 1
    (evidence / 'reference_module.hpp').write_text(prefix)
    shutil.copy2(source, evidence / 'Merge.cpp')
    shutil.copy2(license_path, evidence / 'LICENSE.md')
    for name in ['acceptance.json', 'fixtures.json', 'fixtures.py', 'run.py', 'sdk_probe.cpp']:
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
                      'source_url': f"https://raw.githubusercontent.com/VCVRack/Fundamental/{ACCEPTANCE['reference_commit']}/src/Merge.cpp",
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
                      'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                      'python': sys.version,
                      'platform': sys.platform, 'machine': os.uname().machine},
        'method': 'Actual pinned class prefix with only plugin.hpp replaced by rack.hpp; genuine SDK linked. Original C++ input adapter maps saved sentinels and zeroes missing input lanes. Rust binary uses public Patch/Engine, 16 poly_constant sources and one Merge, fresh per row, two frames to settle one cable. Rust inactive source parameters are zeroed: poisoned protocol padding is ignored, not injected as internal PortFrame padding. Counts and active f32 bits exact; inactive Rust output must be zero; raw SDK inactive lanes retained but excluded.',
        'limitations': ['Bounded finite synthetic rows only; no full-parity or hardware claim',
                        'Fresh graph/class per row; no adoption, bypass or initialization qualification from this harness',
                        'Core tests separately cover callback partition, allocation and lifecycle policies',
                        'No GUI, audio device, physical cable latency or panel rendering qualification',
                        'SDK raw inactive output slots are intentionally outside voltage parity',
                        'Only recorded compiler/SDK/platform combination is qualified'],
        'rates': [], 'evidence_directory': str(evidence.relative_to(ROOT)),
    }
    fixtures = list(rows(ACCEPTANCE))
    total_values = 0
    for rate in ACCEPTANCE['sample_rates']:
        compressed(evidence / f'inputs-{rate}.txt.gz', text.encode())
        results = {}
        for label, exe in [('sdk', cpp), ('rust', saved_rust)]:
            result = subprocess.run([str(exe), str(rate)], input=text, text=True, capture_output=True, env=env)
            compressed(evidence / f'{label}-{rate}.txt.gz', result.stdout.encode())
            (evidence / f'{label}-{rate}.stderr').write_text(result.stderr)
            result.check_returncode()
            results[label] = result.stdout
        parsed = {label: [[float(v) for v in line.split()] for line in trace.splitlines()]
                  for label, trace in results.items()}
        assert all(len(trace) == len(fixtures) for trace in parsed.values()), 'Trace row count mismatch'
        failures, compared, failed_rows = [], 0, 0
        for index, (name, row) in enumerate(fixtures):
            sdk_row, rust_row = parsed['sdk'][index], parsed['rust'][index]
            assert len(sdk_row) == len(rust_row) == 18, 'Trace field count mismatch'
            assert all(math.isfinite(v) for v in sdk_row + rust_row), 'Nonfinite trace'
            automatic = max((slot + 1 for slot in range(16) if row[1 + slot * 17] > 0), default=0)
            expected_count = automatic if row[0] == 0 else 0 if row[0] == -1 else row[0]
            reasons = []
            for label, values in [('sdk', sdk_row), ('rust', rust_row)]:
                if values[:2] != [expected_count, automatic]:
                    reasons.append(f'{label}: counts {values[:2]} != {[expected_count, automatic]}')
            for lane in range(expected_count):
                expected = row[2 + lane * 17] if row[1 + lane * 17] > 0 else 0.0
                ref, actual = sdk_row[2 + lane], rust_row[2 + lane]
                if bits(ref) != bits(actual) or bits(ref) != bits(expected):
                    reasons.append(f'lane {lane}: SDK={bits(ref)} Rust={bits(actual)} fixture={bits(expected)}')
                compared += 1
            if any(value != 0.0 for value in rust_row[2 + expected_count:]):
                reasons.append('Rust inactive output padding is nonzero')
            if reasons:
                failed_rows += 1
                if len(failures) < 16:
                    failures.append({'row': index, 'case': name, 'reasons': reasons})
        total_values += compared
        report['rates'].append({'sample_rate': rate, 'rows': len(fixtures), 'compared_active_values': compared,
                                'input_sha256': hashlib.sha256(text.encode()).hexdigest(),
                                'trace_sha256': {label: hashlib.sha256(trace.encode()).hexdigest() for label, trace in results.items()},
                                'failed_rows': failed_rows, 'first_failures': failures, 'passed': failed_rows == 0})
        print(json.dumps({'rate': rate, 'rows': len(fixtures), 'failed_rows': failed_rows}), flush=True)
    assert source_closure() == closure, 'Production sources changed during qualification'
    report['passed'] = all(rate['passed'] for rate in report['rates'])
    report['total_rows'] = sum(rate['rows'] for rate in report['rates'])
    report['total_compared_active_values'] = total_values
    artifacts = {str(path.relative_to(evidence)): digest(path)
                 for path in sorted(evidence.rglob('*')) if path.is_file() and path.name != 'artifact-sha256.json'}
    (evidence / 'artifact-sha256.json').write_text(stable_json(artifacts))
    report['artifact_manifest_sha256'] = digest(evidence / 'artifact-sha256.json')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(stable_json(report))
    print(json.dumps({'passed': report['passed'], 'rows': report['total_rows'], 'report': str(args.output)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
