#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify production plug-light detector/clock against actual SDK doProcess."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
ACCEPTANCE = json.loads((HERE / 'acceptance.json').read_text())
SOURCE = ROOT / 'crates/openpatch-core/src/port_lights.rs'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def f32(value):
    return struct.unpack('f', struct.pack('f', value))[0]


def fixture(name, frame, rate):
    def port(values):
        return [len(values), *values, *[991.0 - i for i in range(len(values), 16)]]
    values, other, bypass = [5.0], [-2.5], 0
    if name == 'mono_positive': values = [5.0]
    elif name == 'mono_negative': values = [-10.0]
    elif name == 'mono_zero': values = [0.0]
    elif name == 'mono_overrange': values = [25.0 if frame < 35 else -40.0]
    elif name == 'sign_reversal': values = [[10.0, -10.0, 3.0, -7.0][(frame // 127) % 4]]
    elif name == 'fall_to_zero': values = [10.0 if frame < 29 else 0.0]
    elif name == 'stepped_unipolar': values = [[0.0, 2.0, 8.0, 1.0][(frame // 43) % 4]]
    elif name == 'audio_rate_bipolar': values = [13 * math.sin(2 * math.pi * 1733 * frame / rate)]
    elif name == 'poly_3_4': values = [3.0, 4.0]
    elif name == 'poly_16_sparse': values = [0.0] * 15 + [-12.0]
    elif name == 'poly_16_full': values = [(-1 if i % 2 else 1) * (i + 1) * 1.125 for i in range(16)]
    elif name == 'poly_16_silent': values = [0.0] * 16
    elif name == 'poly_16_varying': values = [17 * math.sin(2 * math.pi * (311 + i * 97) * frame / rate) for i in range(16)]
    elif name == 'width_transitions':
        width = [0, 1, 2, 16, 1, 0, 16][(frame // 31) % 7]
        values = [(-1 if i % 2 else 1) * (i + 1) for i in range(width)]
    elif name == 'inactive_padding': values = [0.0] if frame % 37 < 18 else [3.0, 4.0]
    elif name == 'cadence_impulses':
        # All seven phase positions occur, both isolated and on acquisition ticks.
        values = [10.0 if frame % 29 < 2 else -3.0 if frame % 31 == 0 else 0.0]
    elif name == 'bypass_paths':
        bypass = (frame // 47) % 2
        values = [7 * math.sin(frame * 0.07), 4 * math.cos(frame * 0.03)]
    elif name == 'independent_endpoints':
        values = [8.0 if frame < 77 else 0.0]
        other = [0.0] * 15 + [6.0 if frame % 113 < 51 else 0.0]
    else: raise ValueError(name)
    if name != 'independent_endpoints':
        # Distinct input/output traces exercise both SDK loops in every case.
        other = [-v * 0.375 for v in values]
    return ' '.join(format(f32(v), '.9g') for v in [bypass, *port(values), *port(other)])


def inputs():
    return {(rate, name): '\n'.join(fixture(name, frame, rate) for frame in range(ACCEPTANCE['frames_per_case'])) + '\n'
            for rate in ACCEPTANCE['sample_rates'] for name in ACCEPTANCE['cases']}


def fetch(path, commit):
    url = f'https://raw.githubusercontent.com/VCVRack/Rack/{commit}/{path}'
    data = subprocess.check_output(['/usr/bin/curl', '--fail', '--silent', '--show-error', '--location', url])
    return url, data


def detector_block(data):
    source = data.decode()
    assert re.search(r'PORT_DIVIDER\s*=\s*7\s*;', source)
    start = source.index('static void Port_step(')
    end = source.index('\n\nvoid Module::jsonStripIds', start)
    return source[start:end].encode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/port-lights-reference.json')
    parser.add_argument('--freeze-fixtures', action='store_true')
    args = parser.parse_args()
    data = inputs()
    manifest = {'declaration': 'Fixtures and tolerances frozen before first comparison, 2026-10-15',
                'acceptance_sha256': digest(HERE / 'acceptance.json'),
                'generator_sha256': digest(Path(__file__)),
                'input_sha256': {f'{rate}:{name}': sha(text.encode()) for (rate, name), text in data.items()}}
    frozen = HERE / 'fixtures.json'
    if args.freeze_fixtures:
        if frozen.exists(): raise SystemExit('Refusing to overwrite frozen fixtures')
        frozen.write_text(json.dumps(manifest, indent=2) + '\n')
        print('Frozen fixtures/tolerances; no compilation or comparison performed')
        return 0
    assert json.loads(frozen.read_text()) == manifest, 'Frozen fixture/generator/acceptance mismatch'
    sdk = args.sdk.resolve()
    library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
    reference = {'reference_commit': ACCEPTANCE['reference_commit'], 'sdk_source_commit': ACCEPTANCE['sdk_source_commit'],
                 'sdk_library_sha256': digest(library), 'sources': {}, 'sdk_header_sha256': {}}
    for commit in [ACCEPTANCE['reference_commit'], ACCEPTANCE['sdk_source_commit']]:
        url, source = fetch('src/engine/Module.cpp', commit)
        block_hash = sha(detector_block(source))
        assert block_hash == ACCEPTANCE['detector_block_sha256'], 'Detector source differs from frozen pin'
        reference['sources'][commit] = {'url': url, 'source_sha256': sha(source), 'detector_block_sha256': block_hash}
    for name in ['Port.hpp', 'Light.hpp']:
        path = 'include/engine/' + name
        url, source = fetch(path, ACCEPTANCE['reference_commit'])
        assert digest(sdk / path) == sha(source), f'SDK {name} differs from source pin'
        reference['sdk_header_sha256'][path] = {'sha256': sha(source), 'source_url': url, 'matches_pin': True}
    production_hash = digest(SOURCE)
    report = {'reference': reference, 'acceptance': ACCEPTANCE,
              'acceptance_sha256': digest(HERE / 'acceptance.json'), 'fixtures_sha256': digest(frozen),
              'production_source_sha256': production_hash,
              'driver_sha256': {name: digest(HERE / name) for name in ['run.py', 'sdk_probe.cpp', 'rust_probe.rs']},
              'method': 'Actual linked SDK Module::doProcess with no-op processing callbacks; actual production Rust detector and Observer included unchanged. Both ports compared every sample before display clamping. No translated C++ detector and no Cargo invocation.',
              'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0],
                            'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                            'platform': sys.platform, 'machine': os.uname().machine,
                            'cpp_flags': ['-std=c++17', '-O2', '-ffp-contract=off'], 'rust_flags': ['--edition=2021', '-O']},
              'limitations': ['Finite detector and seven-sample Observer cadence qualification only; engine routing, block APIs, adoption, host publication and UI require integration tests',
                              'SDK zero channels represents disconnected; OpenPatch connected zero-channel semantics and invalid/overflow protections require separate core tests',
                              'No GUI, audio devices, MIDI devices, speakers or application launched',
                              'SDK 2.6.6 is a different commit from audit pin; relevant detector source block and Port/Light headers are verified identical',
                              'Only the recorded SDK binary/compiler/platform combination qualified'], 'cases': []}
    with tempfile.TemporaryDirectory(prefix='openpatch-port-light-reference-') as directory:
        tmp = Path(directory)
        cpp, rust = tmp / 'sdk_probe', tmp / 'rust_probe'
        flags = []
        if sys.platform == 'darwin':
            mac_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
            flags = ['-isysroot', str(mac_sdk), '-isystem', str(mac_sdk / 'usr/include/c++/v1')]
        subprocess.run(['c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags,
                        '-I' + str(sdk / 'include'), '-I' + str(sdk / 'dep/include'),
                        str(HERE / 'sdk_probe.cpp'), '-L' + str(sdk), '-lRack', '-Wl,-rpath,' + str(sdk), '-o', str(cpp)], check=True)
        rendered = (HERE / 'rust_probe.rs').read_text().replace('__PORT_LIGHT_SOURCE__', json.dumps(str(SOURCE)))
        (tmp / 'probe.rs').write_text(rendered)
        subprocess.run(['rustc', '--edition=2021', '-O', str(tmp / 'probe.rs'), '-o', str(rust)], check=True)
        assert digest(SOURCE) == production_hash, 'Production source changed while compiling'
        report['probe_binary_sha256'] = {'sdk': digest(cpp), 'rust': digest(rust)}
        report['rendered_rust_probe_sha256'] = sha(rendered.encode())
        env = os.environ.copy()
        env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(sdk)
        for (rate, name), text in data.items():
            results = [subprocess.run([str(exe), str(rate)], input=text, text=True, capture_output=True, check=True, env=env) for exe in [cpp, rust]]
            sdk_meta = results[0].stderr.split()
            assert len(sdk_meta) == 3 and sdk_meta[0] == ACCEPTANCE['sdk_version'], f'Unexpected SDK metadata {sdk_meta}'
            expected_bypass = sum(int(row.split()[0]) for row in text.splitlines())
            assert [int(v) for v in sdk_meta[1:]] == [ACCEPTANCE['frames_per_case'] - expected_bypass, expected_bypass]
            reference['runtime_version'] = sdk_meta[0]
            parsed = [[[f32(float(v)) for v in line.split()] for line in result.stdout.splitlines()] for result in results]
            assert len(parsed[0]) == len(parsed[1]) == ACCEPTANCE['frames_per_case']
            failures, max_error, max_ratio = [], 0.0, 0.0
            for frame, (ref, got) in enumerate(zip(*parsed)):
                assert len(ref) == len(got) == 9 and all(math.isfinite(v) for v in ref + got)
                assert ref[0] == got[0] == frame and ref[1] == got[1] and ref[5] == got[5], 'Frame/channel mismatch'
                for index in [2, 3, 4, 6, 7, 8]:
                    error = abs(ref[index] - got[index])
                    tolerance = ACCEPTANCE['absolute'] + ACCEPTANCE['relative'] * abs(ref[index])
                    max_error, max_ratio = max(max_error, error), max(max_ratio, error / tolerance)
                    if error > tolerance and len(failures) < 8:
                        failures.append({'frame': frame, 'column': index, 'sdk': ref[index], 'rust': got[index], 'absolute_error': error})
            case = {'name': name, 'sample_rate': rate, 'frames': len(parsed[0]), 'compared_values': len(parsed[0]) * 6,
                    'input_sha256': sha(text.encode()), 'trace_sha256': {key: sha(result.stdout.encode()) for key, result in zip(['sdk', 'rust'], results)},
                    'sdk_process_calls': int(sdk_meta[1]), 'sdk_bypass_calls': int(sdk_meta[2]),
                    'maximum_absolute_error': max_error, 'maximum_tolerance_ratio': max_ratio,
                    'passed': not failures, 'first_failures': failures}
            report['cases'].append(case)
            print(json.dumps({'case': name, 'rate': rate, 'passed': case['passed'], 'maximum_error': max_error}), flush=True)
    assert digest(SOURCE) == production_hash, 'Production source changed during qualification; rerun'
    report['passed'] = all(case['passed'] for case in report['cases'])
    report['total_frames'] = sum(case['frames'] for case in report['cases'])
    report['total_compared_values'] = sum(case['compared_values'] for case in report['cases'])
    report['maximum_absolute_error'] = max(case['maximum_absolute_error'] for case in report['cases'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases': len(report['cases']), 'frames': report['total_frames'], 'compared_values': report['total_compared_values'], 'maximum_error': report['maximum_absolute_error'], 'report': str(args.output)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
