#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Run frozen stock-SEQ3 fixtures against the actual SDK class and production Rust."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from fixtures import check_declaration, rows, safety_rows, row_text
from verify import validate

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
A = json.loads((HERE / 'acceptance.json').read_text())


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def stable(value):
    return json.dumps(value, indent=2, sort_keys=True) + '\n'


def closure():
    paths = sorted((ROOT / 'crates/openpatch-core/src').rglob('*.rs'))
    paths += [ROOT / name for name in ['Cargo.toml', 'Cargo.lock',
        'crates/openpatch-core/Cargo.toml', 'crates/openpatch-core/LICENSE-THIRD-PARTY',
        'crates/openpatch-core/examples/stock_seq3_reference_probe.rs']]
    return {str(p.relative_to(ROOT)): digest(p) for p in paths}


def compress(path):
    target = path.with_suffix(path.suffix + '.gz')
    with path.open('rb') as source, target.open('wb') as sink:
        with gzip.GzipFile(filename='', mode='wb', fileobj=sink, mtime=0, compresslevel=6) as out:
            shutil.copyfileobj(source, out)
    path.unlink()
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-fixtures', action='store_true')
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--rust-probe', type=Path)
    parser.add_argument('--rust-toolchain')
    parser.add_argument('--rust-build-command', default='cargo build --locked --release -p openpatch-core --example stock_seq3_reference_probe')
    parser.add_argument('--evidence', type=Path, default=ROOT / 'native-test-output/stock-seq3/qualification')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/stock-seq3-reference.json')
    args = parser.parse_args()
    frozen = check_declaration()
    if args.check_fixtures:
        print(stable({'passed': True, 'fixtures_sha256': digest(HERE / 'fixtures.json')}))
        return 0
    if not args.rust_probe or not args.rust_toolchain:
        parser.error('Provide an existing externally built --rust-probe and recorded --rust-toolchain')
    rust, sdk = args.rust_probe.resolve(), args.sdk.resolve()
    assert rust.is_file() and os.access(rust, os.X_OK)
    source_closure = closure()
    assert rust.stat().st_mtime_ns >= max((ROOT / name).stat().st_mtime_ns for name in source_closure), 'Production probe older than source closure'
    evidence_root = args.evidence.resolve()
    assert ROOT / 'native-test-output' in evidence_root.parents
    evidence_root.mkdir(parents=True, exist_ok=True)
    attempt = 1
    while (evidence_root / f'attempt-{attempt:03}').exists():
        attempt += 1
    evidence = evidence_root / f'attempt-{attempt:03}'
    evidence.mkdir()
    report = {'passed': False, 'evidence_directory': str(evidence.relative_to(ROOT)), 'stage': 'prepare'}
    try:
        reference = ROOT / 'native-test-output/stock-seq3/reference'
        source, license_path = reference / 'SEQ3.cpp', reference / 'LICENSE.md'
        assert digest(source) == A['reference_source_sha256'] and license_path.is_file()
        upstream = source.read_text()
        assert upstream.count('struct SEQ3Widget') == 1
        prefix = upstream.split('struct SEQ3Widget', 1)[0]
        assert prefix.count('#include "plugin.hpp"') == 1
        prefix = prefix.replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
        (evidence / 'reference_module.hpp').write_text(prefix)
        for path in [source, license_path]: shutil.copy2(path, evidence / path.name)
        for name in ['acceptance.json', 'fixtures.json', 'fixtures.py', 'README.md', 'run.py', 'verify.py', 'sdk_probe.cpp']:
            shutil.copy2(HERE / name, evidence / name)
        headers = {}
        for directory in ['include', 'dep/include']:
            shutil.copytree(sdk / directory, evidence / 'sdk' / directory)
            for path in sorted((sdk / directory).rglob('*')):
                if path.is_file(): headers[str(path.relative_to(sdk))] = digest(path)
        library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
        shutil.copy2(library, evidence / library.name)
        for name in source_closure:
            target = evidence / 'production' / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
        saved_rust, cpp = evidence / 'rust_probe', evidence / 'sdk_probe'
        shutil.copy2(rust, saved_rust)
        flags = []
        if sys.platform == 'darwin':
            system_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
            flags = ['-isysroot', str(system_sdk), '-isystem', str(system_sdk / 'usr/include/c++/v1')]
        command = ['c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags,
            '-I' + str(sdk / 'include'), '-I' + str(sdk / 'dep/include'), '-I' + str(evidence),
            str(HERE / 'sdk_probe.cpp'), '-L' + str(sdk), '-lRack', '-Wl,-rpath,' + str(sdk), '-o', str(cpp)]
        commands = {'cpp_argv': command, 'external_rust_command': args.rust_build_command,
            'external_rust_executable': str(rust), 'runner_invokes_cargo_or_rustc': False}
        (evidence / 'build-command.json').write_text(stable(commands))
        built = subprocess.run(command, text=True, capture_output=True)
        (evidence / 'build.stdout').write_text(built.stdout)
        (evidence / 'build.stderr').write_text(built.stderr)
        built.check_returncode()
        env = os.environ.copy()
        env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(evidence)
        report = {'reference': {'fundamental_commit': A['reference_commit'],
            'source_url': f"https://raw.githubusercontent.com/VCVRack/Fundamental/{A['reference_commit']}/src/SEQ3.cpp",
            'source_sha256': digest(source), 'license_sha256': digest(license_path),
            'compiled_prefix_sha256': digest(evidence / 'reference_module.hpp'),
            'sdk_version_required': A['sdk_version_required'], 'sdk_library_sha256': digest(library),
            'sdk_header_sha256': headers}, 'acceptance': A,
            'fixtures_sha256': digest(HERE / 'fixtures.json'), 'production_source_sha256': source_closure,
            'driver_sha256': {name: digest(HERE / name) for name in ['run.py', 'verify.py', 'fixtures.py', 'sdk_probe.cpp']},
            'probe_binary_sha256': {'sdk': digest(cpp), 'rust': digest(saved_rust)},
            'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0],
                'rust': args.rust_toolchain, 'python': sys.version, 'platform': sys.platform, 'machine': os.uname().machine},
            'build_commands': commands, 'evidence_directory': str(evidence.relative_to(ROOT)),
            'rates': [], 'safety_rates': [],
            'limitations': ['Bounded direct-class qualification; not complete host, native-device, hardware or import parity',
                'All sixteen connected mono outputs, complete detector/pulse/gate/phase state and all twenty-seven source lamps are compared every reference frame',
                'Safety phase executes Rust only and is not an SDK equivalence result',
                'Only recorded compiler, SDK, FPU environment and platform combination qualified',
                'Initialize compares the runtime onReset hook; control-default reset, public host continuity, persistence and UI require separate tests',
                'Bypass is an explicit OpenPatch host-policy shim, not Rack Engine bypass equivalence',
                'Physical held controls are compared; coalesced clock/run/reset convenience actions are separate host extensions']}
        for rate in A['sample_rates']:
            for phase in ['reference', 'safety']:
                safety = phase == 'safety'
                inputs = evidence / f'{phase}-inputs-{rate}.txt'
                input_hash = hashlib.sha256()
                with inputs.open('wb') as sink:
                    for _, _, row in (safety_rows if safety else rows)(rate, A):
                        encoded = row_text(row).encode()
                        input_hash.update(encoded)
                        sink.write(encoded)
                assert input_hash.hexdigest() == frozen['rates'][str(rate)][phase]['input_sha256']
                traces, trace_hashes = {}, {}
                for label, exe in [('sdk', cpp), ('rust', saved_rust)]:
                    if safety and label == 'sdk': continue
                    out, err = evidence / f'{phase}-{label}-{rate}.txt', evidence / f'{phase}-{label}-{rate}.stderr'
                    argv = [str(exe), str(rate)] + (['--safety'] if safety else [])
                    with inputs.open('rb') as source_stream, out.open('wb') as sink, err.open('wb') as error_stream:
                        run = subprocess.run(argv, stdin=source_stream, stdout=sink, stderr=error_stream, env=env)
                    run.check_returncode()
                    traces[label], trace_hashes[label] = out, digest(out)
                result = validate(rate, A, phase, inputs, traces.get('sdk'), traces['rust'])
                result.update({'sample_rate': rate, 'input_sha256': digest(inputs), 'trace_sha256': trace_hashes})
                report['safety_rates' if safety else 'rates'].append(result)
                (evidence / f'{phase}-{rate}-result.json').write_text(stable(result))
                print(json.dumps({'phase': phase, 'rate': rate, 'passed': result['passed'], 'frames': result['frames'], 'failed_frames': result['failed_frames']}), flush=True)
                for path in [inputs, *traces.values()]: compress(path)
        assert closure() == source_closure, 'Production sources changed during qualification'
        report['passed'] = all(r['passed'] for r in report['rates'] + report['safety_rates'])
        report['reference_frames'] = sum(r['frames'] for r in report['rates'])
        report['safety_frames'] = sum(r['frames'] for r in report['safety_rates'])
    except Exception as error:
        report['passed'] = False
        report['error'] = {'type': type(error).__name__, 'message': str(error)}
        print(json.dumps({'passed': False, 'error': report['error']}), flush=True)
    artifacts = {str(path.relative_to(evidence)): digest(path) for path in sorted(evidence.rglob('*'))
        if path.is_file() and path.name not in ['artifact-sha256.json', 'result.json']}
    (evidence / 'artifact-sha256.json').write_text(stable(artifacts))
    report['artifact_manifest_sha256'] = digest(evidence / 'artifact-sha256.json')
    (evidence / 'result.json').write_text(stable(report))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(stable(report))
    print(json.dumps({'passed': report['passed'], 'reference_frames': report.get('reference_frames', 0), 'safety_frames': report.get('safety_frames', 0), 'report': str(args.output)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
