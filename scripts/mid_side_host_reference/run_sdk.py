#!/usr/bin/env python3
"""Record genuine Engine snapshots and check frozen SDK-only hand anchors.

This phase deliberately makes no OpenPatch host-equivalence claim. The frozen
paired gate remains required after the production host path is implemented.
"""
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FROZEN = {
    'acceptance.json': 'f00be21fdfed433902bbad3ee7d8a5869a022fdf42501221159f67766acec02c',
    'cases.json': 'd4c7d48e3439a70952e55c912f71efd4bb7b77acae468535ef88ab13d159d81e',
    'verify.py': '51858dd978dfd573b4f5381a7be4265a15ddf2460b4863955b41813d16745dd3',
    'sdk_probe.cpp': '915b37b2899d4621ac956eb46d94f0752e439bfc46b1bcdccc50801c2091bd9d',
    'README.md': 'b1a78479af60ffd29d28b9d60cbb3e65a9ca7fcc2cf6220481e39aca15aa6d3c',
}


def digest(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def stable(v):
    return json.dumps(v, indent=2, sort_keys=True) + '\n'


def main():
    sdk = Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK'
    reference = ROOT / 'native-test-output/stock-mid-side/reference'
    base = ROOT / 'native-test-output/stock-mid-side/host-reference'
    base.mkdir(parents=True, exist_ok=True)
    for name, sha in FROZEN.items():
        assert digest(HERE / name) == sha, name
    index = 1
    while (base / f'attempt-{index:03}').exists(): index += 1
    evidence = base / f'attempt-{index:03}'
    evidence.mkdir()
    report = {'passed': False, 'scope': 'SDK hand-anchor validation only; production paired gate pending',
        'stage': 'prepare', 'frozen_before_compiler_execution': FROZEN,
        'evidence_directory': str(evidence.relative_to(ROOT))}
    try:
        # Bind to the already-executed, version-checked genuine SDK library.
        attestation = ROOT / 'docs/benchmarks/output-lifecycle-reference.json'
        prior = json.loads(attestation.read_text())
        assert prior['passed'] and 'SDK=2.6.6\n' in prior['sdk_metadata']
        library = sdk / 'libRack.dylib'
        assert digest(library) == prior['sdk_library_sha256']
        for name, sha in FROZEN.items(): shutil.copy2(HERE / name, evidence / name)
        shutil.copy2(HERE / 'run_sdk.py', evidence / 'run_sdk.py')
        source_hashes = {}
        for name in ['Engine.cpp', 'Module.cpp', 'MidSide.cpp', 'Rack-LICENSE.md', 'Fundamental-LICENSE.md']:
            shutil.copy2(reference / name, evidence / name)
            source_hashes[name] = digest(reference / name)
        assert source_hashes['MidSide.cpp'] == 'a5ce5f5703c7e4fb389b2590f1cffbb63a1260024ad18252ce23687b40d7b9be'
        headers = {}
        for directory in ['include', 'dep/include']:
            shutil.copytree(sdk / directory, evidence / 'sdk' / directory)
            for p in sorted((sdk / directory).rglob('*')):
                if p.is_file(): headers[str(p.relative_to(sdk))] = digest(p)
        assert headers == prior['sdk_header_sha256']
        shutil.copy2(library, evidence / library.name)
        upstream = (reference / 'MidSide.cpp').read_text()
        assert upstream.count('struct MidSideWidget') == 1
        prefix = upstream.split('struct MidSideWidget', 1)[0]
        assert prefix.count('#include "plugin.hpp"') == 1
        (evidence / 'reference_module.hpp').write_text(prefix.replace('#include "plugin.hpp"',
            '#include <rack.hpp>\nusing namespace rack;'))
        system_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
        cpp = evidence / 'sdk_probe'
        command = ['c++', '-std=c++17', '-O2', '-ffp-contract=off', '-isysroot', str(system_sdk),
            '-isystem', str(system_sdk / 'usr/include/c++/v1'), '-I' + str(sdk / 'include'),
            '-I' + str(sdk / 'dep/include'), '-I' + str(evidence), str(HERE / 'sdk_probe.cpp'),
            '-L' + str(sdk), '-lRack', '-Wl,-rpath,' + str(sdk), '-o', str(cpp)]
        (evidence / 'compiler-command.json').write_text(stable(command))
        built = subprocess.run(command, text=True, capture_output=True)
        (evidence / 'build.stdout').write_text(built.stdout)
        (evidence / 'build.stderr').write_text(built.stderr)
        built.check_returncode()
        report.update({'stage': 'execute', 'source_sha256': source_hashes,
            'sdk_library_sha256': digest(library), 'sdk_header_sha256': headers,
            'sdk_version_attestation_sha256': digest(attestation), 'sdk_version': '2.6.6',
            'compiled_prefix_sha256': digest(evidence / 'reference_module.hpp'),
            'compiler_command': command, 'cpp_toolchain': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0],
            'probe_binary_sha256': digest(cpp), 'python': sys.version})
        cases = json.loads((HERE / 'cases.json').read_text())['cases']
        inputs, trace = evidence / 'inputs.jsonl', evidence / 'sdk.jsonl'
        inputs.write_text(''.join(json.dumps(case, separators=(',', ':'), ensure_ascii=False) + '\n' for case in cases))
        env = os.environ.copy(); env['DYLD_LIBRARY_PATH'] = str(evidence)
        with inputs.open('rb') as source, trace.open('wb') as sink, (evidence / 'sdk.stderr').open('wb') as errors:
            executed = subprocess.run([str(cpp)], stdin=source, stdout=sink, stderr=errors, env=env)
        executed.check_returncode()
        checked = subprocess.run([sys.executable, str(HERE / 'verify.py'), str(trace)], text=True, capture_output=True)
        (evidence / 'validator.stdout').write_text(checked.stdout)
        (evidence / 'validator.stderr').write_text(checked.stderr)
        checked.check_returncode()
        result = json.loads(checked.stdout)
        report.update({'validation': result, 'input_sha256': digest(inputs), 'trace_sha256': digest(trace),
            'paired_production_equivalence': False, 'passed': bool(result['passed']), 'stage': 'complete'})
        assert {n: digest(HERE / n) for n in FROZEN} == FROZEN
        assert digest(library) == report['sdk_library_sha256']
        for path in [inputs, trace]:
            with path.open('rb') as source, path.with_suffix(path.suffix + '.gz').open('wb') as sink:
                with gzip.GzipFile(filename='', mode='wb', fileobj=sink, mtime=0) as zipped:
                    shutil.copyfileobj(source, zipped)
            path.unlink()
    except Exception as e:
        report['passed'] = False
        report['error'] = {'type': type(e).__name__, 'message': str(e)}
    artifacts = {str(p.relative_to(evidence)): digest(p) for p in sorted(evidence.rglob('*'))
        if p.is_file() and p.name not in ['artifact-sha256.json', 'result.json']}
    (evidence / 'artifact-sha256.json').write_text(stable(artifacts))
    report['artifact_manifest_sha256'] = digest(evidence / 'artifact-sha256.json')
    (evidence / 'result.json').write_text(stable(report))
    target = ROOT / 'docs/benchmarks/mid-side-sdk-host-reference.json'
    target.write_text(stable(report))
    print(json.dumps({key: report.get(key) for key in ['passed', 'scope', 'validation', 'error']}), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
