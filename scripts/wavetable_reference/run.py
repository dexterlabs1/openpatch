#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Generate and qualify actual pinned WTVCO/WTLFO classes using the Rack SDK.

--prepare-only never invokes a compiler, SDK process, plugin, GUI or audio device.
--sdk-only builds and executes the actual-class oracle. Optional --rust-probe
must be an already-built adapter implementing the documented binary protocol.
"""
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

from fixtures import MANIFEST, OUTPUT, generate, rows

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PIN = '10dd0160c664770910e5584b7b00498cc48d9ddd'
BASE = f'https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/'
PINS = {
    'WTVCO.cpp': '37723b2d01b950090400e65982681e8d8abded59191090b7aaa4d5ffbb37d337',
    'WTLFO.cpp': '20e9634eedcab363122217d9b09cb0dbafa7a9967e923fc1fbb4038004064077',
    'Wavetable.hpp': 'bc45992469e938a229aad35d78d44f16ae6d0664addbc556dfcd46d9263bbbb5',
    'dr_wav.h': 'd435cea2b6828840ba40d61e23247c41a335be1cefc2e5734a05e62fec1accb0',
    'dr_wav.c': hashlib.sha256(b'#define DR_WAV_IMPLEMENTATION\n#include "dr_wav.h"\n').hexdigest(),
}
ACCEPTANCE = json.loads((HERE/'acceptance.json').read_text())
# This hash records the numerical contract before any Rust oracle comparison.
FROZEN_ACCEPTANCE_SHA256 = '4d3668cfe2e84f1b4ecf75945685d49474d3db70c4bbb12a40b0c4c046f605b0'

def sha(data): return hashlib.sha256(data).hexdigest()
def digest(path): return sha(path.read_bytes())

def obtain(name, reference_dir):
    path = reference_dir/name
    data = path.read_bytes() if path.exists() else subprocess.check_output([
        'curl', '--fail', '--silent', '--show-error', '--location', BASE+name])
    if sha(data) != PINS[name]:
        raise ValueError(f'pinned source hash mismatch: {name}')
    return data

def prepare(build_dir, reference_dir, sdk, mac_sdk=None):
    build_dir.mkdir(parents=True, exist_ok=True)
    source_hashes, trimmed_hashes = {}, {}
    for name in PINS:
        data = obtain(name, reference_dir)
        source_hashes[name] = sha(data)
        if name in ['WTVCO.cpp', 'WTLFO.cpp']:
            text = data.decode()
            marker = 'struct WTVCOWidget' if name == 'WTVCO.cpp' else 'struct WTLFOWidget'
            assert text.count(marker) == 1 and text.count('#include "plugin.hpp"') == 1
            text = text.split(marker, 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
            dest = build_dir/('reference_vco.hpp' if name == 'WTVCO.cpp' else 'reference_lfo.hpp')
            dest.write_text(text)
            trimmed_hashes[dest.name] = digest(dest)
        elif name == 'Wavetable.hpp':
            text = data.decode()
            assert text.count('static Wavetable defaultWavetable;') == 1
            # Preparation, JSON, loading/saving and menu methods remain untouched.
            # Only unused GUI display template and its global preview bank are omitted.
            text = text.split('static Wavetable defaultWavetable;', 1)[0]
            # Guard the actual source read without changing in-bounds arithmetic.
            # Rounded phase=1 may legitimately read the next packed row, but a
            # final-buffer overrun cannot be used as numerical reference data.
            original = 'return interpolatedSamples[samples.size() * quality * octave + waveLen * quality * waveIndex + sampleIndex];'
            checked = 'return interpolatedSamples.at(samples.size() * quality * octave + waveLen * quality * waveIndex + sampleIndex);'
            assert text.count(original) == 1
            text = text.replace(original, checked)
            dest = build_dir/name
            dest.write_text(text)
            trimmed_hashes[name] = digest(dest)
        else:
            (build_dir/name).write_bytes(data)
    c_flags, cpp_flags = [], []
    if mac_sdk:
        c_flags = ['-isysroot', str(mac_sdk)]
        cpp_flags = c_flags + ['-isystem', str(mac_sdk/'usr/include/c++/v1')]
    commands = [
        ['cc', '-std=c99', '-O2', '-ffp-contract=off', *c_flags, '-I'+str(build_dir), '-c', str(build_dir/'dr_wav.c'), '-o', str(build_dir/'dr_wav.o')],
        ['c++', '-std=c++17', '-O2', '-ffp-contract=off', '-fno-fast-math', *cpp_flags,
         '-I'+str(sdk/'include'), '-I'+str(sdk/'dep/include'), '-I'+str(build_dir),
         str(HERE/'sdk_probe.cpp'), str(build_dir/'dr_wav.o'), '-L'+str(sdk), '-lRack',
         '-Wl,-rpath,'+str(sdk), '-o', str(build_dir/'sdk_probe')],
    ]
    metadata = {'fundamental_commit': PIN, 'source_urls': {n: BASE+n for n in PINS},
                'source_sha256': source_hashes, 'compiled_prefix_sha256': trimmed_hashes,
                'commands': commands, 'sdk': str(sdk), 'reference_bounds_checked': True, 'mac_sdk': str(mac_sdk) if mac_sdk else None}
    (build_dir/'compile-commands.json').write_text(json.dumps(metadata, indent=2)+'\n')
    return metadata

def unpack(data, frames):
    if len(data) != frames*OUTPUT.size:
        raise AssertionError(f'expected {frames*OUTPUT.size} output bytes, got {len(data)}')
    records = list(OUTPUT.iter_unpack(data))
    for index, record in enumerate(records):
        if record[0] not in range(1, 17) or not all(math.isfinite(v) for v in record[1:68]):
            raise AssertionError(f'invalid or nonfinite output/state at frame {index}')
        if record[73] not in [0, 1]:
            raise AssertionError(f'invalid loading state at frame {index}')
    return records

def freeze_audit(records, kind, name, frames, rate):
    failures = []
    for frame, (event, _, _, ports) in enumerate(rows(kind, name, frames, rate)):
        record = records[frame]
        width = max(1, ports[0][0], ports[3][0]) if kind == 0 else max(1, ports[0][0])
        if record[0] != width:
            failures.append({'frame': frame, 'reason': 'source width violates declared routing', 'expected': width, 'actual': record[0]})
        if frame == 0 or event == 1:
            continue
        old = records[frame-1]
        valid = not record[73] and record[74] >= 2 and record[75]//record[74] >= 1
        for group in range(4):
            if valid and 4*group < width:
                continue
            indices = list(range(17+group*4, 21+group*4))
            if kind == 0:
                indices += list(range(33+group*4, 37+group*4))
                indices += list(range(49+group*4, 53+group*4))
            else:
                indices += [68+group]
            if any(record[i] != old[i] for i in indices):
                failures.append({'frame': frame, 'group': group, 'reason': 'inactive complete group state advanced'})
        if len(failures) >= 8:
            break
    return failures[:8]

def compare(reference, actual):
    failures, worst = [], {'absolute_error_volts': -1.}
    max_ratio = 0.
    exact_indices = list(range(33, 65)) + list(range(68, 76))
    for frame, (ref, got) in enumerate(zip(reference, actual)):
        if ref[0] != got[0]:
            failures.append({'frame': frame, 'reason': 'channel count'})
        for lane in range(ref[0]):
            error = abs(ref[1+lane]-got[1+lane])
            tolerance = ACCEPTANCE['pointwise']['absolute_volts'] + ACCEPTANCE['pointwise']['relative']*abs(ref[1+lane])
            max_ratio = max(max_ratio, error/tolerance)
            if error > worst['absolute_error_volts']:
                worst = {'frame': frame, 'channel': lane, 'reference_volts': ref[1+lane], 'actual_volts': got[1+lane], 'absolute_error_volts': error, 'tolerance_volts': tolerance}
            if error > tolerance and len(failures) < 8:
                failures.append({'frame': frame, 'lane': lane, 'reason': 'voltage'})
        if any(ref[i] != got[i] for i in exact_indices) and len(failures) < 8:
            failures.append({'frame': frame, 'reason': 'exact event/state metadata'})
        for i in list(range(17, 33))+[65, 66, 67]:
            obs = ACCEPTANCE['observations']
            if 17 <= i < 33: tolerance = obs['phase_absolute_cycles']
            elif i == 67: tolerance = obs['table_position_absolute_waves']
            else: tolerance = obs['clock_absolute']+obs['clock_relative']*abs(ref[i])
            if abs(ref[i]-got[i]) > tolerance and len(failures) < 8:
                failures.append({'frame': frame, 'observation_index': i, 'reason': 'continuous state', 'reference': ref[i], 'actual': got[i]})
    return {'passed': not failures, 'worst_sample': worst, 'maximum_pointwise_tolerance_ratio': max_ratio, 'first_failures': failures[:8]}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--prepare-only', action='store_true')
    mode.add_argument('--sdk-only', action='store_true')
    mode.add_argument('--self-check', action='store_true', help='Python fixture and protocol checks only')
    parser.add_argument('--sdk', type=Path, default=Path.home()/'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--reference-dir', type=Path, default=ROOT/'native-test-output/wavetable/reference')
    parser.add_argument('--build-dir', type=Path)
    parser.add_argument('--mac-sdk', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--rust-probe', type=Path, help='Already built; same stdin/stdout protocol, never built here')
    parser.add_argument('--kind', choices=['vco', 'lfo'])
    parser.add_argument('--case', action='append')
    parser.add_argument('--rate', type=int, action='append', choices=MANIFEST['sample_rates_hz'])
    parser.add_argument('--frames', type=int, help='Developer smoke only; marks report non-qualifying if changed')
    args = parser.parse_args()
    assert digest(HERE/'acceptance.json') == FROZEN_ACCEPTANCE_SHA256, 'frozen acceptance changed'
    if args.self_check:
        count = 0
        for kind, key in enumerate(['vco_cases', 'lfo_cases']):
            for name in MANIFEST[key]:
                for rate in MANIFEST['sample_rates_hz']:
                    data = generate(kind, name, rate)
                    assert data[:4] == b'OWT1' and data == generate(kind, name, rate)
                    assert len(list(rows(kind, name, MANIFEST['frames_default'], rate))) == MANIFEST['frames_default']
                    count += 1
        print(json.dumps({'python_fixture_checks': count, 'record_bytes': OUTPUT.size, 'acceptance_sha256': FROZEN_ACCEPTANCE_SHA256}))
        return 0
    build_dir = (args.build_dir or Path(tempfile.mkdtemp(prefix='openpatch-wavetable-reference-'))).resolve()
    if build_dir == ROOT or ROOT in build_dir.parents and ROOT/'native-test-output' not in build_dir.parents:
        raise ValueError('upstream sources/build products must remain outside tracked repository paths')
    mac_sdk = args.mac_sdk
    if args.sdk_only and sys.platform == 'darwin' and mac_sdk is None:
        mac_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
    reference = prepare(build_dir, args.reference_dir, args.sdk.resolve(), mac_sdk)
    if args.prepare_only:
        print(json.dumps({'prepared': str(build_dir), 'commands': str(build_dir/'compile-commands.json'), 'acceptance_sha256': FROZEN_ACCEPTANCE_SHA256}))
        return 0
    for command in reference['commands']:
        subprocess.run(command, check=True)
    sdk = args.sdk.resolve()
    library = sdk/('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
    reference['sdk_library_sha256'] = digest(library)
    headers = ['rack.hpp', 'dsp/fft.hpp', 'dsp/minblep.hpp', 'dsp/digital.hpp', 'dsp/approx.hpp', 'simd/functions.hpp', 'simd/Vector.hpp', 'engine/Port.hpp']
    reference['sdk_header_sha256'] = {name: digest(sdk/'include'/name) for name in headers}
    env = os.environ.copy()
    env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(sdk)
    report = {'reference': reference, 'acceptance': ACCEPTANCE, 'acceptance_sha256': FROZEN_ACCEPTANCE_SHA256,
              'fixture_manifest_sha256': digest(HERE/'fixtures.json'),
              'driver_sha256': {name: digest(HERE/name) for name in ['fixtures.py', 'run.py', 'sdk_probe.cpp']},
              'comparison': 'sdk-and-rust' if args.rust_probe else 'sdk-only-no-rust-fidelity-claim',
              'full_fixture_lengths': args.frames is None, 'cases': []}
    if args.rust_probe:
        proof_paths = sorted((ROOT/'crates/openpatch-core/src').glob('*.rs')) + [ROOT/'crates/openpatch-core/examples/wavetable_probe.rs',ROOT/'crates/openpatch-core/Cargo.toml',ROOT/'Cargo.lock']
        report['rust_probe_sha256'] = digest(args.rust_probe)
        report['rust_source_sha256'] = {str(path.relative_to(ROOT)):digest(path) for path in proof_paths}
    for kind, key in enumerate(['vco_cases', 'lfo_cases']):
        label = ['vco', 'lfo'][kind]
        if args.kind and args.kind != label: continue
        selected = args.case or MANIFEST[key]
        for name in selected:
            if name not in MANIFEST[key]:
                if args.kind: raise ValueError(f'{name} not a {label} case')
                continue
            for rate in args.rate or MANIFEST['sample_rates_hz']:
                frames = args.frames or MANIFEST['frames_default']
                data = generate(kind, name, rate, frames)
                paths = [build_dir/'sdk_probe', args.rust_probe or build_dir/'sdk_probe']
                results = [subprocess.run([str(exe)], input=data, capture_output=True, env=env, check=True) for exe in paths]
                traces = [unpack(result.stdout, frames) for result in results]
                audit = freeze_audit(traces[0], kind, name, frames, rate)
                result = compare(*traces)
                if not args.rust_probe and results[0].stdout != results[1].stdout:
                    audit.append({'reason': 'fresh actual-class instances are not bit-identical'})
                result['passed'] &= not audit
                case = {'kind': label, 'name': name, 'rate': rate, 'frames': frames, 'input_sha256': sha(data),
                        'sdk_output_sha256': sha(results[0].stdout), 'second_output_sha256': sha(results[1].stdout),
                        'source_invariants': audit, **result}
                report['cases'].append(case)
                (build_dir/f'{label}-{name}-{rate}.input').write_bytes(data)
                (build_dir/f'{label}-{name}-{rate}.sdk-output').write_bytes(results[0].stdout)
                if args.rust_probe: (build_dir/f'{label}-{name}-{rate}.rust-output').write_bytes(results[1].stdout)
                print(json.dumps({'kind': label, 'case': name, 'rate': rate, 'passed': case['passed'], 'maximum_pointwise_tolerance_ratio': result['maximum_pointwise_tolerance_ratio']}), flush=True)
    if args.rust_probe:
        if any(digest(ROOT/name) != expected for name,expected in report['rust_source_sha256'].items()):
            raise ValueError('Rust source changed during qualification')
        if digest(args.rust_probe) != report['rust_probe_sha256']:
            raise ValueError('Rust probe changed during qualification')
    if not report['cases']: raise ValueError('no matching cases')
    report['passed'] = all(case['passed'] for case in report['cases'])
    report['total_frames'] = sum(case['frames'] for case in report['cases'])
    report['trace_directory'] = str(build_dir)
    report['qualifies_complete_suite'] = bool(args.rust_probe) and args.frames is None and args.case is None and args.kind is None and args.rate is None and report['passed']
    output = args.output or build_dir/'report.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'report': str(output), 'passed': report['passed'], 'frames': report['total_frames'], 'qualifies_complete_suite': report['qualifies_complete_suite']}))
    return 0 if report['passed'] else 1

if __name__ == '__main__':
    raise SystemExit(main())
