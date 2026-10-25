#!/usr/bin/env python3
"""Compare production VCA-2 and Unity with the genuine pinned Fundamental classes.

Original OpenPatch harness, MIT licensed. Fetches the pinned sources, checks
them against source_manifest.json, extracts the module classes unmodified
(only `plugin.hpp` replaced by the SDK include), compiles reference_probe.cpp
against the Rack SDK, builds the production Rust probe, runs every case of
cases.json through both and requires bit-identical output.
"""
import argparse
import hashlib
import json
import math
import os
import random
import shutil
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
OUT = ROOT / 'native-test-output/fundamental-legacy-reference'
VCA_PORTS = ['exp1', 'lin1', 'in1', 'exp2', 'lin2', 'in2']
UNITY_PORTS = [f'ch{s}_{j}' for s in (1, 2) for j in range(1, 7)]
CLASSES = {'VCA.cpp': ('reference_vca.hpp', 'struct VCAWidget'),
           'Unity.cpp': ('reference_unity.hpp', 'struct UnityWidget')}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    return sha(Path(path).read_bytes())


def f32_hex(value):
    return struct.pack('>f', value).hex()


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


class Signal:
    """Expands one declarative signal into per-frame, per-lane f32 bits."""

    def __init__(self, spec, frames, rate, start):
        self.spec, self.frames, self.rate, self.start = spec, frames, rate, start
        self.noise = {}

    def lane(self, t, lane):
        spec = self.spec
        if 'bits' in spec:
            return spec['bits'][lane % len(spec['bits'])]
        if 'const' in spec:
            return f32_hex(spec['const'][lane % len(spec['const'])])
        if 'ramp' in spec:
            lo, hi = spec['ramp']
            span = max(self.frames - 1, 1)
            return f32_hex(lo + (hi - lo) * t / span + spec.get('lane_step', 0.0) * lane)
        if 'sine' in spec:
            s = spec['sine']
            freq = s['freq'] + s.get('lane_freq_step', 0.0) * lane
            phase = 2 * math.pi * freq * (self.start + t) / self.rate + 0.3 * lane
            return f32_hex(s['offset'] + s['amp'] * math.sin(phase))
        if 'noise' in spec:
            s = spec['noise']
            generator = self.noise.setdefault(lane, random.Random(s['seed'] * 1000 + lane))
            return f32_hex(s['offset'] + s['amp'] * (2 * generator.random() - 1))
        raise ValueError(f'unknown signal {spec}')


def expand(case):
    """Token lines for one case; both probes read the same text."""
    lines, frame = [], 0
    rate = case['rate']
    for event in case['events']:
        if 'merge' in event:
            lines.append(f"M {int(bool(event['merge']))}")
            continue
        if event.get('reset'):
            lines.append('R')
            continue
        if 'data' in event:
            lines.append('D ' + json.dumps(event['data'], separators=(',', ':')))
            continue
        frames = event['frames']
        ports = VCA_PORTS if case['module'] == 'vca' else UNITY_PORTS
        signals = {}
        for name, port in event['inputs'].items():
            assert name in ports, name
            signals[name] = (port['channels'], Signal(port['signal'], frames, rate, frame))
        controls = event['levels'] if case['module'] == 'vca' else event['average']
        for t in range(frames):
            row = ['F', *(f32_hex(v) for v in controls)]
            for name in ports:
                channels, signal = signals.get(name, (0, None))
                assert 0 <= channels <= 16
                if case['module'] == 'vca':
                    row.append(str(channels))
                    row.extend(signal.lane(t, lane) if lane < channels else '00000000' for lane in range(16))
                else:
                    row.append(str(channels))
                    row.append(signal.lane(t, 0) if channels else '00000000')
            lines.append(' '.join(row))
        frame += frames
    return '\n'.join(lines) + '\n'


def next_attempt(prefix):
    OUT.mkdir(parents=True, exist_ok=True)
    n = 1
    while (OUT / f'{prefix}-{n:03d}').exists():
        n += 1
    path = OUT / f'{prefix}-{n:03d}'
    path.mkdir()
    return path


def fetch(manifest, evidence):
    source = evidence / 'source'
    source.mkdir()
    for entry in manifest['upstream']:
        data = subprocess.check_output(['curl', '--fail', '--silent', '--show-error', '--location', entry['url']],
                                       timeout=120)
        if sha(data) != entry['sha256']:
            raise SystemExit(f"{entry['path']}: sha256 {sha(data)} does not match the manifest")
        (source / entry['path']).write_bytes(data)
    return source


def check_sdk(manifest, sdk):
    for path, expected in manifest['sdk']['files'].items():
        actual = digest(sdk / path)
        if actual != expected:
            raise SystemExit(f'SDK {path}: sha256 {actual} does not match the manifest')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path,
                        default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--validate-only', action='store_true',
                        help='fetch and check sources and expand cases without compiling or running')
    parser.add_argument('--case', action='append', help='run only these case ids')
    args = parser.parse_args()
    manifest = json.loads((HERE / 'source_manifest.json').read_text())
    # Each declared case runs once per listed sample rate.
    cases = [dict(case, id=case['id'] if len(case['rates']) == 1 else f"{case['id']}_{rate}", rate=rate)
             for case in json.loads((HERE / 'cases.json').read_text())['cases'] for rate in case['rates']]
    if args.case:
        cases = [c for c in cases if c['id'] in args.case]
    evidence = next_attempt('validate' if args.validate_only else 'attempt')
    harness = evidence / 'harness'
    harness.mkdir()
    for name in ['run.py', 'make_cases.py', 'cases.json', 'source_manifest.json', 'reference_probe.cpp',
                 'NOTICE.md', 'README.md']:
        shutil.copy2(HERE / name, harness / name)
    probe_rs = ROOT / 'crates/openpatch-core/examples/fundamental_legacy_reference_probe.rs'
    shutil.copy2(probe_rs, harness / probe_rs.name)
    sdk = args.sdk.resolve()
    check_sdk(manifest, sdk)
    source = fetch(manifest, evidence)
    for name, (header, cut) in CLASSES.items():
        text = (source / name).read_text()
        assert text.count(cut) == 1 and text.count('#include "plugin.hpp"') == 1
        prefix = text.split(cut, 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
        (source / header).write_text('#pragma once\n' + prefix)
    inputs = {c['id']: expand(c) for c in cases}
    result = {'source': manifest['revision'], 'sdk_version': manifest['sdk']['version'],
              'harness_sha256': {p.name: digest(p) for p in sorted(harness.iterdir())},
              'cases': []}
    if args.validate_only:
        result['inputs_sha256'] = {k: sha(v.encode()) for k, v in inputs.items()}
        (evidence / 'result.json').write_text(json.dumps(result, indent=1) + '\n')
        print(f'validated {len(cases)} cases; evidence {evidence.relative_to(ROOT)}')
        return 0
    # Genuine classes: C++17, IEEE single precision, no contraction.
    flags = ['-std=c++17', '-O2', '-ffp-contract=off', '-fno-fast-math']
    if sys.platform == 'darwin':
        mac_sdk = subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip()
        flags += ['-isysroot', mac_sdk, '-isystem', mac_sdk + '/usr/include/c++/v1']
    cpp = evidence / 'reference_probe'
    command = ['c++', *flags, f'-I{sdk}/include', f'-I{sdk}/dep/include', f'-I{source}',
               str(harness / 'reference_probe.cpp'), f'-L{sdk}', '-lRack', f'-Wl,-rpath,{sdk}', '-o', str(cpp)]
    subprocess.run(command, check=True)
    cargo = ['cargo', 'build', '--release', '--locked', '-p', 'openpatch-core', '--example',
             'fundamental_legacy_reference_probe']
    subprocess.run(cargo, check=True, cwd=ROOT)
    rust = ROOT / 'target/release/examples/fundamental_legacy_reference_probe'
    (evidence / 'toolchain.json').write_text(json.dumps({
        'cxx': subprocess.check_output(['c++', '--version'], text=True),
        'cxx_command': command, 'rustc': subprocess.check_output(['rustc', '-Vv'], text=True),
        'cargo_command': cargo, 'platform': sys.platform, 'machine': os.uname().machine}, indent=1) + '\n')
    result['probe_sha256'] = {'reference': digest(cpp), 'rust': digest(rust)}
    result['production_sha256'] = {str(p.relative_to(ROOT)): digest(p) for p in sorted(
        list((ROOT / 'crates/openpatch-core/src').glob('stock_vca2*')) +
        list((ROOT / 'crates/openpatch-core/src/stock_vca2').glob('*.rs')) +
        list((ROOT / 'crates/openpatch-core/src').glob('stock_unity*')) +
        list((ROOT / 'crates/openpatch-core/src/stock_unity').glob('*.rs')) +
        [ROOT / 'crates/openpatch-core/src/native_float.rs']) if p.is_file()}
    env = os.environ.copy()
    env['DYLD_LIBRARY_PATH'] = str(sdk)
    totals = {'cases': 0, 'passed': 0, 'frames': 0, 'compared_values': 0, 'mismatched_values': 0}
    for case in cases:
        text = inputs[case['id']]
        runs = [subprocess.run([str(exe), case['module'], str(case['rate'])], input=text, text=True,
                               capture_output=True, env=env, timeout=600) for exe in (cpp, rust)]
        for label, run in zip(['reference', 'rust'], runs):
            if run.returncode != 0:
                raise SystemExit(f"{case['id']}: {label} probe failed ({run.returncode}): {run.stderr[-2000:]}")
        ref, got = (r.stdout.splitlines() for r in runs)
        frames = sum(1 for line in text.splitlines() if line.startswith('F'))
        compared, mismatched, first = 0, 0, []
        if len(ref) != len(got):
            mismatched += 1
            first.append({'line': None, 'reason': f'{len(ref)} reference lines, {len(got)} rust lines'})
        for index, (a, b) in enumerate(zip(ref, got)):
            ta, tb = a.split(), b.split()
            compared += max(len(ta), len(tb))
            bad = sum(1 for x, y in zip(ta, tb) if x != y) + abs(len(ta) - len(tb))
            if bad:
                mismatched += bad
                if len(first) < 8:
                    first.append({'line': index, 'reference': a[:400], 'rust': b[:400]})
        entry = {'id': case['id'], 'module': case['module'], 'rate': case['rate'], 'frames': frames,
                 'compared_values': compared, 'mismatched_values': mismatched, 'passed': mismatched == 0,
                 'input_sha256': sha(text.encode()),
                 'output_sha256': {'reference': sha(runs[0].stdout.encode()), 'rust': sha(runs[1].stdout.encode())},
                 'first_mismatches': first}
        result['cases'].append(entry)
        totals['cases'] += 1
        totals['passed'] += entry['passed']
        totals['frames'] += frames
        totals['compared_values'] += compared
        totals['mismatched_values'] += mismatched
        print(f"{'PASS' if entry['passed'] else 'FAIL'} {case['id']}: {frames} frames, {compared} values, "
              f"{mismatched} mismatched")
    result['totals'] = totals
    manifest_hashes = {str(p.relative_to(evidence)): digest(p) for p in sorted(evidence.rglob('*')) if p.is_file()}
    (evidence / 'artifacts.sha256.json').write_text(json.dumps(manifest_hashes, indent=1) + '\n')
    (evidence / 'result.json').write_text(json.dumps(result, indent=1) + '\n')
    print(json.dumps(totals))
    print(f'evidence {evidence.relative_to(ROOT)}')
    return 0 if totals['passed'] == totals['cases'] else 1


if __name__ == '__main__':
    sys.exit(main())
