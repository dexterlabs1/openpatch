#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Compare production StockVca against the genuine pinned VCA_1 SDK class."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
ACCEPTANCE = json.loads((HERE / 'acceptance.json').read_text())
PIN = ACCEPTANCE['reference_commit']
URL = f'https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/VCA-1.cpp'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def f32(x):
    return struct.unpack('f', struct.pack('f', x))[0]


def vectors(name, rate):
    for frame in range(ACCEPTANCE['frames_per_case']):
        connected, level, response = 1, 1.0, 1
        ports = [[0, *([991.0] * 16)] for _ in range(2)]
        def port(i, width, values):
            values = values if isinstance(values, list) else [values] * width
            ports[i] = [width, *values[:width], *[991.0 - lane for lane in range(width, 16)]]
        def audio(width):
            port(1, width, [8 * math.sin(2 * math.pi * (173 + lane * 71) * frame / rate) for lane in range(width)])
        audio(1)
        if name == 'defaults': pass
        elif name.startswith('level_'):
            level = {'level_zero': 0.0, 'level_one': 1.0, 'level_intermediate': 0.371}[name]
            port(0, 1, 5.0)
        elif name in ['linear', 'exponential']:
            response = int(name == 'linear'); port(0, 1, 5.0)
        elif name == 'absent_cv': port(0, 0, 0.0); audio(5)
        elif name == 'connected_zero_cv': port(0, 1, 0.0); audio(5)
        elif name == 'cv_extremes':
            response = frame % 2
            values = [-1e6, -10.0, -0.001, 0.0, 0.001, 2.5, 5.0, 10.0, 20.0, 1e6]
            port(0, 16, [values[(frame // 3 + lane) % len(values)] for lane in range(16)]); port(1, 16, [(-1.0 if lane % 2 else 1.0) * 1e6 for lane in range(16)])
        elif name == 'mono_cv_broadcast': port(0, 1, 5.0); audio(16)
        elif name == 'mono_audio_broadcast': port(0, 16, [lane * 10.0 / 15 for lane in range(16)]); port(1, 1, 8.0)
        elif name in ['width_grid_0_1_3_5_16', 'poisoned_padding']:
            widths = [0, 1, 3, 5, 16]; cv_width = widths[(frame // 5) % 5]; audio_width = widths[frame % 5]
            port(0, cv_width, [2.0 + lane * 0.5 for lane in range(cv_width)]); port(1, audio_width, [lane - 5.0 for lane in range(audio_width)])
            response = (frame // 25) % 2
        elif name == 'shrink_regrow_16_1_5_16':
            width = [16, 1, 5, 16][frame // 64 % 4]; port(0, width, [lane * 0.5 + 1.0 for lane in range(width)]); audio(width)
        elif name == 'audio_rate_audio_cv':
            audio(16); port(0, 5, [7.0 + 9.0 * math.sin(2 * math.pi * (1093 + lane * 677) * frame / rate) for lane in range(5)]); response = frame // 128 % 2
        elif name == 'parameter_edits':
            level = [0.0, 1.0, 0.371, 0.9][frame // 7 % 4]; response = frame // 11 % 2; audio(5); port(0, 3, [2.0, 5.0, 9.0])
        elif name == 'silent_audio_gain': port(0, 16, [lane * 10.0 / 15 for lane in range(16)]); port(1, 0, 0.0)
        elif name == 'unused_output_gain':
            connected = frame // 16 % 2; port(0, 5, [(frame % 13) * 0.8] * 5); audio(1)
        else: raise ValueError(name)
        yield ' '.join(format(f32(v), '.9g') for v in [connected, level, response, *[v for p in ports for v in p]])


def fixture_inputs():
    data = {(rate, name): '\n'.join(vectors(name, rate)) + '\n' for rate in ACCEPTANCE['sample_rates'] for name in ACCEPTANCE['cases']}
    c = ACCEPTANCE['counterfactual']
    for rate in ACCEPTANCE['sample_rates']:
        for cv in c['cv_volts']:
            for mode in [0, 1]:
                row = [1, c['level'], mode, 1, cv, *([991.0] * 15), 1, c['audio_volts'], *([991.0] * 15)]
                data[(rate, f'fresh_cv_{cv}_response_{mode}')] = (' '.join(format(f32(v), '.9g') for v in row) + '\n') * c['frames_per_fresh_trace']
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--serde-rlib', type=Path, help='Required existing serde_json rlib; this runner never invokes Cargo')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/stock-vca-reference.json')
    parser.add_argument('--freeze-fixtures', action='store_true', help='Freeze all input hashes and exit before compiling/comparing')
    args = parser.parse_args(); data = fixture_inputs()
    manifest = {'declaration': 'Fixed before the first oracle comparison, 2026-10-15', 'acceptance_sha256': digest(HERE / 'acceptance.json'), 'generator_sha256': digest(Path(__file__)), 'input_sha256': {f'{rate}:{name}': hashlib.sha256(text.encode()).hexdigest() for (rate, name), text in data.items()}}
    frozen = HERE / 'fixtures.json'
    if args.freeze_fixtures:
        if frozen.exists(): raise SystemExit('Fixtures already frozen; refusing overwrite')
        frozen.write_text(json.dumps(manifest, indent=2) + '\n'); print('Frozen all fixture hashes; no comparison performed'); return 0
    assert json.loads(frozen.read_text()) == manifest, 'Frozen fixtures or generator changed'
    assert args.serde_rlib is not None, '--serde-rlib must identify an existing dependency'
    sdk = args.sdk.resolve(); serde = args.serde_rlib.resolve()
    source = subprocess.check_output(['curl', '--fail', '--silent', '--show-error', '--location', URL])
    upstream = source.decode(); assert upstream.count('struct VCA_1VUKnob') == 1
    prefix = upstream.split('struct VCA_1VUKnob', 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    assert prefix.count('struct VCA_1 : Module') == 1 and prefix.count('std::pow(cv, 4.f)') == 1
    library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
    # Hash all SDK headers as well as the central headers and linked library.
    sdk_headers = sorted((sdk / 'include').rglob('*.hpp')) + sorted((sdk / 'include').rglob('*.h')) + sorted((sdk / 'dep/include').rglob('*.h')) + sorted((sdk / 'dep/include').rglob('*.hpp'))
    header_hashes = {str(p.relative_to(sdk)): digest(p) for p in sdk_headers}
    aggregate = hashlib.sha256(json.dumps(header_hashes, sort_keys=True).encode()).hexdigest()
    report = {'reference': {'fundamental_commit': PIN, 'source_url': URL, 'source_sha256': hashlib.sha256(source).hexdigest(), 'compiled_prefix_sha256': hashlib.sha256(prefix.encode()).hexdigest(), 'sdk_library_sha256': digest(library), 'sdk_headers_aggregate_sha256': aggregate, 'sdk_header_sha256': header_hashes}, 'acceptance': ACCEPTANCE, 'acceptance_sha256': digest(HERE / 'acceptance.json'), 'fixtures_sha256': digest(frozen), 'production_source_sha256': digest(ROOT / 'crates/openpatch-core/src/stock_vca.rs'), 'driver_sha256': {p: digest(HERE / p) for p in ['run.py', 'sdk_probe.cpp', 'rust_probe.rs']}, 'serde_rlib_sha256': digest(serde), 'method': 'Compile the actual pinned VCA_1 class prefix with only plugin.hpp replaced and all widget code omitted, in a temporary directory against genuine Rack SDK. std::pow(cv,4.f) is unchanged. Production Rust is included unchanged. Compare actual SDK output channel count when connected and actual lastChannels/lastGains every frame, including unused output. Rust always publishes max-width output; SDK disconnected output metadata remains zero. SDK inactive input padding is zero as provided by Rack cable engine; Rust gets poisoned padding to verify host missing-channel semantics. All fixtures were hashed before the first comparison. Decimal traces round-trip f32 before fixed pointwise checks.', 'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0], 'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(), 'platform': sys.platform, 'machine': os.uname().machine, 'cpp_flags': ['-std=c++17', '-O2', '-ffp-contract=off'], 'rust_flags': ['--edition=2021', '-O']}, 'limitations': ['Finite synthetic coverage only; no analog hardware, acoustic, device-latency or superiority claim', 'No native app, GUI, audio device or speakers opened', 'Oracle finite inputs are within the host voltage bound; nonfinite and finite-bound handling is separately unit tested', 'Minimal adapters bypass public Engine cable delay, output selection, taps, bypass, callback partition and lifecycle; those require host tests', 'SDK stale gains outside lastChannels are intentionally not exposed; host inactive gain/output slots are zero', 'Only reported SDK/compiler/platform combination qualified'], 'cases': [], 'counterfactual_audits': []}
    with tempfile.TemporaryDirectory(prefix='openpatch-stock-vca-reference-') as directory:
        tmp = Path(directory); (tmp / 'reference_module.hpp').write_text(prefix)
        cpp, rust = tmp / 'sdk_probe', tmp / 'rust_probe'
        flags = []
        if sys.platform == 'darwin':
            mac_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip()); flags = ['-isysroot', str(mac_sdk), '-isystem', str(mac_sdk / 'usr/include/c++/v1')]
        subprocess.run(['c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags, '-I' + str(sdk / 'include'), '-I' + str(sdk / 'dep/include'), '-I' + str(tmp), str(HERE / 'sdk_probe.cpp'), '-L' + str(sdk), '-lRack', '-Wl,-rpath,' + str(sdk), '-o', str(cpp)], check=True)
        (tmp / 'probe.rs').write_text((HERE / 'rust_probe.rs').read_text().replace('__STOCK_VCA_SOURCE__', json.dumps(str(ROOT / 'crates/openpatch-core/src/stock_vca.rs'))))
        subprocess.run(['rustc', '--edition=2021', '-O', str(tmp / 'probe.rs'), '--extern', 'serde_json=' + str(serde), '-L', 'dependency=' + str(serde.parent), '-o', str(rust)], check=True)
        report['probe_binary_sha256'] = {'sdk': digest(cpp), 'rust': digest(rust)}
        report['rendered_rust_probe_sha256'] = digest(tmp / 'probe.rs')
        env = os.environ.copy(); env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(sdk)
        traces = {}
        for (rate, name), text in data.items():
            results = [subprocess.run([str(exe), str(rate)], input=text, text=True, capture_output=True, check=True, env=env) for exe in [cpp, rust]]
            parsed = [[[f32(float(v)) for v in line.split()] for line in r.stdout.splitlines()] for r in results]
            assert len(parsed[0]) == len(parsed[1]) == len(text.splitlines())
            failures, maximum_error, maximum_ratio, compared = [], 0.0, 0.0, 0
            for frame, (ref, got, row) in enumerate(zip(*parsed, text.splitlines())):
                assert len(ref) == len(got) == 34 and all(math.isfinite(v) for v in ref + got)
                connected = int(row.split()[0]); width = int(ref[1])
                assert 1 <= width <= 16
                if got[1] != ref[1] or got[0] != ref[1] or ref[0] != (ref[1] if connected else 0): failures.append({'frame': frame, 'reason': 'actual output/lastChannels width mismatch'})
                for kind, start in [('output', 2), ('gain', 18)]:
                    if kind == 'output' and not connected: continue
                    for lane in range(width):
                        a, b = ref[start + lane], got[start + lane]
                        error = abs(a - b); tolerance = ACCEPTANCE['absolute'] + ACCEPTANCE['relative'] * abs(a)
                        maximum_error = max(maximum_error, error); maximum_ratio = max(maximum_ratio, error / tolerance); compared += 1
                        if error > tolerance and len(failures) < 8: failures.append({'frame': frame, 'kind': kind, 'lane': lane, 'reference': a, 'actual': b, 'absolute_error': error})
            case = {'name': name, 'sample_rate': rate, 'frames': len(parsed[0]), 'input_sha256': hashlib.sha256(text.encode()).hexdigest(), 'trace_sha256': {label: hashlib.sha256(result.stdout.encode()).hexdigest() for label, result in zip(['sdk', 'rust'], results)}, 'compared_values': compared, 'maximum_absolute_error': maximum_error, 'maximum_tolerance_ratio': maximum_ratio, 'passed': not failures, 'first_failures': failures[:8]}
            report['cases'].append(case)
            if name.startswith('fresh_'): traces[(rate, name)] = parsed
            print(json.dumps({'case': name, 'rate': rate, 'passed': case['passed'], 'ratio': maximum_ratio}), flush=True)
        c = ACCEPTANCE['counterfactual']
        for rate in ACCEPTANCE['sample_rates']:
            for index, cv in enumerate(c['cv_volts']):
                exp, lin = [traces[(rate, f'fresh_cv_{cv}_response_{mode}')] for mode in [0, 1]]
                audit = {'rate': rate, 'cv_volts': cv, 'fresh_frames_per_mode': c['frames_per_fresh_trace'], 'implementations': {}}
                for impl, label in enumerate(['sdk', 'rust']):
                    output_delta = max(abs(a[2] - b[2]) for a, b in zip(exp[impl], lin[impl])); gain_delta = max(abs(a[18] - b[18]) for a, b in zip(exp[impl], lin[impl]))
                    exact = all(row[18] == expected for trace, expected in [(exp[impl], c['exponential_expected_gain'][index]), (lin[impl], c['linear_expected_gain'][index])] for row in trace)
                    passed = exact and ((output_delta > c['partial_minimum_output_difference'] and gain_delta > c['partial_minimum_gain_difference']) if cv == 5.0 else output_delta == gain_delta == 0.0)
                    audit['implementations'][label] = {'linear_gain': lin[impl][0][18], 'exponential_gain': exp[impl][0][18], 'maximum_output_difference': output_delta, 'maximum_gain_difference': gain_delta, 'expected_gains_exact_every_frame': exact, 'passed': passed}
                audit['passed'] = all(v['passed'] for v in audit['implementations'].values()); report['counterfactual_audits'].append(audit)
    report['passed'] = all(c['passed'] for c in report['cases']) and all(a['passed'] for a in report['counterfactual_audits'])
    report['total_frames'] = sum(c['frames'] for c in report['cases']); report['total_compared_values'] = sum(c['compared_values'] for c in report['cases'])
    args.output.parent.mkdir(parents=True, exist_ok=True); args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases': len(report['cases']), 'frames': report['total_frames'], 'report': str(args.output)})); return 0 if report['passed'] else 1

if __name__ == '__main__': raise SystemExit(main())
