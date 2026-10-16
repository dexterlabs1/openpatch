#!/usr/bin/env python3
"""Compare frozen producer histories with genuine Rack Engine API execution."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from fixtures import check_declaration, write_fixture
from verify import validate

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DECLARATION = ['acceptance.json', 'fixtures.py', 'fixtures.json', 'verify.py', 'README.md']


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def stable(value):
    return json.dumps(value, sort_keys=True, indent=2) + '\n'


def closure():
    paths = sorted((ROOT / 'crates/openpatch-core/src').rglob('*.rs'))
    paths += [ROOT / name for name in ['Cargo.toml', 'Cargo.lock',
        'crates/openpatch-core/Cargo.toml', 'crates/openpatch-core/LICENSE-THIRD-PARTY',
        'crates/openpatch-core/examples/output_lifecycle_reference_probe.rs']]
    return {str(p.relative_to(ROOT)): digest(p) for p in paths}


def compress(path):
    with path.open('rb') as source, path.with_suffix(path.suffix + '.gz').open('wb') as sink:
        with gzip.GzipFile(filename='', mode='wb', fileobj=sink, mtime=0, compresslevel=6) as compressed:
            shutil.copyfileobj(source, compressed)
    path.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check-fixtures', action='store_true')
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--rust-probe', type=Path)
    parser.add_argument('--rust-toolchain')
    parser.add_argument('--evidence', type=Path, default=ROOT / 'native-test-output/stock-mid-side/output-lifecycle')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/output-lifecycle-reference.json')
    args = parser.parse_args()
    acceptance = json.loads((HERE / 'acceptance.json').read_text())
    frozen = check_declaration()
    five = {name: digest(HERE / name) for name in DECLARATION}
    if args.check_fixtures:
        print(stable({'passed': True, 'cases': frozen['cases'], 'frozen_five': five}))
        return 0
    if not args.rust_probe or not args.rust_toolchain:
        parser.error('Provide an externally built --rust-probe and exact --rust-toolchain')
    rust, sdk = args.rust_probe.resolve(), args.sdk.resolve()
    assert rust.is_file() and os.access(rust, os.X_OK)
    production = closure()
    assert rust.stat().st_mtime_ns >= max((ROOT / name).stat().st_mtime_ns for name in production)
    evidence_root = args.evidence.resolve()
    assert ROOT / 'native-test-output' in evidence_root.parents
    evidence_root.mkdir(parents=True, exist_ok=True)
    attempt = 1
    while (evidence_root / f'attempt-{attempt:03}').exists():
        attempt += 1
    evidence = evidence_root / f'attempt-{attempt:03}'
    evidence.mkdir()
    report = {'passed': False, 'stage': 'prepare', 'evidence_directory': str(evidence.relative_to(ROOT))}
    try:
        reference = ROOT / 'native-test-output/stock-mid-side/reference'
        files = acceptance['source']['files']
        for name, upstream in [('Engine.cpp', 'Rack/src/engine/Engine.cpp'),
                ('Module.cpp', 'Rack/src/engine/Module.cpp'),
                ('MidSide.cpp', 'Fundamental/src/MidSide.cpp')]:
            assert digest(reference / name) == files[upstream], name
            shutil.copy2(reference / name, evidence / name)
        assert digest(sdk / 'include/engine/Port.hpp') == files['Rack-SDK/include/engine/Port.hpp']
        for name in ['Rack-LICENSE.md', 'Fundamental-LICENSE.md']:
            shutil.copy2(reference / name, evidence / name)
        upstream = (reference / 'MidSide.cpp').read_text()
        assert upstream.count('struct MidSideWidget') == 1
        prefix = upstream.split('struct MidSideWidget', 1)[0]
        assert prefix.count('#include "plugin.hpp"') == 1
        (evidence / 'reference_module.hpp').write_text(prefix.replace(
            '#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;'))
        for name in DECLARATION + ['run.py', 'sdk_probe.cpp']:
            shutil.copy2(HERE / name, evidence / name)
        headers = {}
        for directory in ['include', 'dep/include']:
            shutil.copytree(sdk / directory, evidence / 'sdk' / directory)
            for path in sorted((sdk / directory).rglob('*')):
                if path.is_file(): headers[str(path.relative_to(sdk))] = digest(path)
        library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
        shutil.copy2(library, evidence / library.name)
        for name in production:
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
        commands = {'cpp_argv': command, 'external_rust_command':
            'cargo build --locked --release -p openpatch-core --example output_lifecycle_reference_probe',
            'external_rust_executable': str(rust), 'runner_invokes_cargo_or_rustc': False}
        (evidence / 'build-command.json').write_text(stable(commands))
        built = subprocess.run(command, text=True, capture_output=True)
        (evidence / 'build.stdout').write_text(built.stdout)
        (evidence / 'build.stderr').write_text(built.stderr)
        built.check_returncode()
        report.update({'stage': 'execute', 'acceptance': acceptance, 'frozen_five': five,
            'production_source_sha256': production, 'sdk_header_sha256': headers,
            'sdk_library_sha256': digest(library), 'compiled_prefix_sha256': digest(evidence / 'reference_module.hpp'),
            'build_commands': commands, 'probe_binary_sha256': {'sdk': digest(cpp), 'rust': digest(saved_rust)},
            'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0],
                'rust': args.rust_toolchain, 'python': sys.version, 'platform': sys.platform, 'machine': os.uname().machine},
            'limitations': acceptance['limitations'] + [
                'Recorded SDK/library/headers/compiler/platform environment only; system-library bytes are not archived',
                'Engine.cpp/Module.cpp are retained primary source references, not recompiled engine implementations',
                'Production transform is qualified; complete application producer/consumer integration remains separate']})
        inputs = evidence / 'inputs.txt'
        generated = write_fixture(inputs, acceptance)
        assert generated['cases'] == frozen['cases'] and generated['input_sha256'] == frozen['input_sha256']
        env = os.environ.copy()
        env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(evidence)
        for label, executable in [('sdk', cpp), ('rust', saved_rust)]:
            with inputs.open('rb') as source, (evidence / f'{label}.txt').open('wb') as sink, (evidence / f'{label}.stderr').open('wb') as errors:
                executed = subprocess.run([str(executable)], stdin=source, stdout=sink, stderr=errors, env=env)
            executed.check_returncode()
        metadata = (evidence / 'sdk.stderr').read_text()
        assert 'engine_frames_final=0\n' in metadata and 'engine_blocks_final=0\n' in metadata
        assert f'cases={frozen["cases"]}\n' in metadata
        result = validate(acceptance, inputs, evidence / 'sdk.txt', evidence / 'rust.txt')
        report.update({'validation': result, 'input_sha256': digest(inputs), 'actual_engine_dsp_frames': 0,
            'trace_sha256': {label: digest(evidence / f'{label}.txt') for label in ['sdk', 'rust']},
            'sdk_metadata': metadata, 'cases': frozen['cases']})
        assert closure() == production, 'Production source changed during execution'
        assert {name: digest(HERE / name) for name in DECLARATION} == five
        report['passed'] = bool(result['passed'])
        for path in [inputs, evidence / 'sdk.txt', evidence / 'rust.txt']: compress(path)
    except Exception as error:
        report['passed'] = False
        report['error'] = {'type': type(error).__name__, 'message': str(error)}
        print(json.dumps({'passed': False, 'error': report['error']}), flush=True)
    artifacts = {str(p.relative_to(evidence)): digest(p) for p in sorted(evidence.rglob('*'))
        if p.is_file() and p.name not in ['artifact-sha256.json', 'result.json']}
    (evidence / 'artifact-sha256.json').write_text(stable(artifacts))
    report['artifact_manifest_sha256'] = digest(evidence / 'artifact-sha256.json')
    (evidence / 'result.json').write_text(stable(report))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(stable(report))
    print(json.dumps({'passed': report['passed'], 'cases': report.get('cases'), 'report': str(args.output)}), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
