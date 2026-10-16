#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify production Random against the actual pinned Fundamental class."""
import argparse
import contextlib
import gzip
import hashlib
import io
import itertools
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile

from fixtures import (ACCEPTANCE, HERE, INPUT_FIELDS, OUTPUT_FIELDS, f32,
                      frame_count, inputs, manifest)

ROOT = HERE.parents[1]
PIN = ACCEPTANCE['reference_commit']
URL = f'https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/Random.cpp'
TOL = ACCEPTANCE['pointwise']


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def bits(value):
    return struct.pack('<f', value)


def production_closure():
    crate = ROOT / 'crates/openpatch-core'
    files = sorted((crate / 'src').rglob('*.rs'))
    files += sorted((crate / 'examples').rglob('*.rs'))
    files += [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', crate / 'Cargo.toml']
    return {str(path.relative_to(ROOT)): digest(path) for path in files}


def summary_hash(mapping):
    return hashlib.sha256(json.dumps(mapping, sort_keys=True).encode()).hexdigest()


def parse_trace(path, frames):
    count = 0
    with path.open() as source:
        for index, line in enumerate(source):
            row = [f32(float(value)) for value in line.split()]
            assert len(row) == OUTPUT_FIELDS, f'Output frame {index}: wrong field count'
            assert all(math.isfinite(value) for value in row), f'Output frame {index}: nonfinite'
            count += 1
            yield row
    assert count == frames, f'Expected {frames} frames, received {count}'


def raw_event(row, previous, input_row):
    if input_row[45] > 0:  # source TRIG_INPUT width: 11 + 2*17
        return row[120] == 0.
    return row[119] < (previous[119] if previous is not None else 0.)


def expected_event_and_draws(row, previous, input_row):
    raw = raw_event(row, previous, input_row)
    prob = input_row[5]
    # Source reads just input lane zero; disconnected raw voltage is zero.
    if input_row[79] > 0:
        prob = f32(prob + f32(f32(input_row[80] / f32(10.)) * input_row[9]))
    prob = max(0., min(1., prob))
    probability_draw = raw and prob < 1.
    accepted = raw and (not probability_draw or input_row[113] <= prob)
    draws = int(probability_draw) + int(accepted and input_row[62] == 0)
    return raw, accepted, draws


def compare(reference, actual, input_text, name, rate):
    failures, compared = [], 0
    maxima = {}
    events = {'sdk_raw': 0, 'rust_raw': 0, 'sdk_accepted': 0, 'rust_accepted': 0,
              'sdk_draws': 0, 'rust_draws': 0}
    previous = [None, None]

    def failure(frame, reason, **details):
        if len(failures) < 12:
            failures.append({'frame': frame, 'reason': reason, **details})

    def exact(frame, label, a, b):
        nonlocal compared
        compared += 1
        if a != b:
            failure(frame, label, reference=a, actual=b)

    def close(frame, label, a, b, absolute, relative=0., lane=None):
        nonlocal compared
        error = abs(a - b)
        tolerance = absolute + relative * abs(a)
        record = maxima.setdefault(label, {'absolute': 0., 'tolerance_ratio': 0.})
        record['absolute'] = max(record['absolute'], error)
        record['tolerance_ratio'] = max(record['tolerance_ratio'], error / tolerance)
        compared += 1
        if error > tolerance:
            failure(frame, label, lane=lane, reference=a, actual=b, absolute_error=error)

    frames = 0
    for frame, (ref, got, text_row) in enumerate(itertools.zip_longest(reference, actual, io.StringIO(input_text))):
        assert ref is not None and got is not None and text_row is not None, 'Trace/input frame counts differ'
        frames += 1
        source = [f32(float(value)) for value in text_row.split()]
        assert len(source) == INPUT_FIELDS
        if source[0] == 2:
            previous = [None, None]
        width = max(1, int(source[62]))
        for index in range(5):
            expected_width = width if index < 4 else 1
            exact(frame, f'sdk output {index} width contract', ref[index], expected_width)
            exact(frame, f'output {index} width', ref[index], got[index])
            for lane in range(16):
                start = 5 + index * 16 + lane
                if index == 4:
                    exact(frame, 'trigger voltage', ref[start], got[start])
                else:
                    close(frame, 'output voltage', ref[start], got[start],
                          TOL['voltage_absolute'], TOL['voltage_relative'], lane)
                if lane >= expected_width:
                    exact(frame, 'sdk output inactive padding', ref[start], 0.)
                    exact(frame, 'rust output inactive padding', got[start], 0.)
        for label, start in [('last held voltage', 85), ('next held voltage', 101)]:
            for lane in range(16):
                close(frame, label, ref[start + lane], got[start + lane],
                      TOL['voltage_absolute'], TOL['voltage_relative'], lane)
                if lane >= width:
                    for impl, row in enumerate([ref, got]):
                        old = previous[impl][start + lane] if previous[impl] is not None else 0.
                        exact(frame, f'{impl} inactive held storage bits',
                              bits(row[start + lane]).hex(), bits(old).hex())
        close(frame, 'phase', ref[117], got[117], TOL['phase_absolute'])
        close(frame, 'clock frequency', ref[118], got[118],
              TOL['clock_frequency_absolute'], TOL['clock_frequency_relative'])
        close(frame, 'clock phase', ref[119], got[119], TOL['phase_absolute'])
        close(frame, 'timer', ref[120], got[120], TOL['timer_absolute_seconds'])
        close(frame, 'pulse remaining', ref[121], got[121], TOL['pulse_absolute_seconds'])
        exact(frame, 'Schmitt state', ref[122], got[122])
        exact(frame, 'uniform draw count', ref[128], got[128])
        raw_flags, accepted_flags = [], []
        for impl, (label, row) in enumerate([('sdk', ref), ('rust', got)]):
            raw, accepted, draws = expected_event_and_draws(row, previous[impl], source)
            raw_flags.append(raw)
            accepted_flags.append(accepted)
            events[f'{label}_raw'] += int(raw)
            events[f'{label}_accepted'] += int(accepted)
            events[f'{label}_draws'] += int(row[128])
            exact(frame, f'{label} uniform call-site contract', row[128], draws)
            exact(frame, f'{label} trigger legal level', row[69] in [0., 10.], True)
            old_lights = previous[impl][123:128] if previous[impl] is not None else [0.] * 5
            dt = f32(1. / rate)
            for light, lit in [(0, raw), (2, accepted)]:
                bright = f32(3.) if lit else old_lights[light]
                expected = f32(bright + f32(f32(f32(-bright) * f32(30.)) * dt))
                close(frame, f'{label} source light recurrence', expected, row[123 + light],
                      TOL['light_absolute'], TOL['light_relative'], light)
        exact(frame, 'raw clock event', *raw_flags)
        exact(frame, 'accepted trigger event', *accepted_flags)
        for light in range(5):
            close(frame, 'light', ref[123 + light], got[123 + light],
                  TOL['light_absolute'], TOL['light_relative'], light)
        if name in ['startup_positive', 'startup_between_thresholds'] and frame == 0:
            expected_state = 1. if name == 'startup_positive' else 2.
            for label, row in [('sdk', ref), ('rust', got)]:
                exact(frame, f'{label} initial-high suppression', row[69], 0.)
                exact(frame, f'{label} startup Schmitt state', row[122], expected_state)
                exact(frame, f'{label} no startup draws', row[128], 0.)
        previous = [ref, got]
    assert frames == frame_count(name, rate)
    return {'name': name, 'sample_rate': rate, 'frames': frames,
            'compared_values': compared, 'maximum_errors': maxima,
            'events': events, 'passed': not failures, 'first_failures': failures}


def source_prefix(source):
    assert hashlib.sha256(source).hexdigest() == ACCEPTANCE['reference_source_sha256'], 'Pinned source hash mismatch'
    upstream = source.decode()
    assert upstream.count('#include "plugin.hpp"') == 1
    assert upstream.count('struct RandomWidget : ModuleWidget') == 1
    prefix = upstream.split('struct RandomWidget : ModuleWidget', 1)[0]
    assert prefix.count('struct Random : Module') == 1
    assert prefix.count('random::uniform()') == 2
    prefix = prefix.replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    return prefix.replace('random::uniform()', 'referenceUniform()')


def evidence_directory(requested):
    if requested is None:
        return tempfile.TemporaryDirectory(prefix='openpatch-random-reference-')
    assert not requested.is_symlink(), 'Build directory must not be a symlink'
    directory = requested.resolve()
    if directory.exists():
        assert directory.is_dir() and not any(directory.iterdir()), 'Build directory must be new or empty; refusing overwrite'
    else:
        directory.mkdir(parents=True)
    return contextlib.nullcontext(str(directory))


def save_gzip(path, directory, expected_sha256):
    """Compress a completed artifact without replacing any existing file."""
    saved = path.with_suffix(path.suffix + '.gz')
    raw_hash, raw_bytes = hashlib.sha256(), 0
    with path.open('rb') as source, saved.open('xb') as destination:
        with gzip.GzipFile(filename='', mode='wb', fileobj=destination,
                           compresslevel=6, mtime=0) as compressed:
            for chunk in iter(lambda: source.read(1024 * 1024), b''):
                raw_hash.update(chunk)
                raw_bytes += len(chunk)
                compressed.write(chunk)
    assert raw_hash.hexdigest() == expected_sha256, 'Raw artifact changed before retention'
    record = {'relative_path': str(saved.relative_to(directory)),
              'sha256': digest(saved), 'raw_sha256': raw_hash.hexdigest(),
              'raw_bytes': raw_bytes, 'gzip_bytes': saved.stat().st_size}
    path.unlink()
    return record


def retained_file(path, directory):
    return {'relative_path': str(path.relative_to(directory)),
            'sha256': digest(path), 'bytes': path.stat().st_size}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=ROOT / 'native-test-output/random/reference/Random.cpp')
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--rust-probe', type=Path, help='Existing root-built production example executable')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/random-reference.json')
    parser.add_argument('--build-dir', type=Path, help='New or empty owned directory retaining probes, build command, and all compressed inputs/traces')
    parser.add_argument('--freeze-fixtures', action='store_true')
    parser.add_argument('--check-fixtures', action='store_true', help='Pure Python hash/protocol check; no builds or probes')
    args = parser.parse_args()
    frozen = HERE / 'fixtures.json'
    declaration = manifest()
    if args.freeze_fixtures:
        if frozen.exists():
            raise SystemExit('Fixtures already frozen; refusing overwrite')
        frozen.write_text(json.dumps(declaration, indent=2) + '\n')
        print('Frozen acceptance, generator and all input hashes; no comparison performed')
        return 0
    assert json.loads(frozen.read_text()) == declaration, 'Frozen acceptance/fixtures/generator changed'
    if args.check_fixtures:
        print(json.dumps({'passed': True, 'cases': len(declaration['input_sha256']),
                          'input_fields': INPUT_FIELDS, 'output_fields': OUTPUT_FIELDS,
                          'total_frames': sum(declaration['frame_counts'].values())}))
        return 0
    assert args.rust_probe, '--rust-probe must identify the separately built production example'
    rust = args.rust_probe.resolve()
    closure = production_closure()
    assert rust.is_file() and os.access(rust, os.X_OK), 'Missing executable production probe'
    assert rust.stat().st_mtime_ns >= max((ROOT / name).stat().st_mtime_ns for name in closure), 'Rebuild production probe after source changes'
    sdk = args.sdk.resolve()
    source = args.source.read_bytes()
    prefix = source_prefix(source)
    library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
    headers = sorted({*sdk.joinpath('include').rglob('*.h'), *sdk.joinpath('include').rglob('*.hpp'),
                      *sdk.joinpath('dep/include').rglob('*.h'), *sdk.joinpath('dep/include').rglob('*.hpp')})
    header_hashes = {str(path.relative_to(sdk)): digest(path) for path in headers}
    assert header_hashes, 'No installed SDK headers'
    drivers = {path.name: digest(path) for path in HERE.iterdir() if path.suffix in ['.py', '.cpp', '.json', '.md']}
    report = {
        'reference': {'fundamental_commit': PIN, 'source_url': URL,
                      'source_sha256': hashlib.sha256(source).hexdigest(),
                      'compiled_prefix_sha256': hashlib.sha256(prefix.encode()).hexdigest(),
                      'sdk_version_required': ACCEPTANCE['sdk_version'],
                      'sdk_library_sha256': digest(library),
                      'sdk_headers_aggregate_sha256': summary_hash(header_hashes),
                      'sdk_header_sha256': header_hashes},
        'acceptance': ACCEPTANCE, 'acceptance_sha256': digest(HERE / 'acceptance.json'),
        'fixtures_sha256': digest(frozen), 'driver_sha256': drivers,
        'production_source_closure_sha256': summary_hash(closure),
        'production_source_sha256': closure,
        'rust_probe_binary_sha256': digest(rust),
        'method': 'Actual pinned Random class compiled in an owned build directory against installed Rack SDK. Only plugin include and widget/registration suffix removed, and the two uniform call sites redirected to an explicitly supplied identical f32 draw stream. The Rust example calls production RandomProbe step_with_uniform and production parameter adoption. Full suite, frame zero, all five outputs and all sixteen storage lanes, all held lanes and shared clock/timer/Schmitt/pulse/light state. No native global PRNG sequence claim.',
        'limitations': ['Finite synthetic digital coverage only; no GUI, hardware, audio device, acoustic, latency or performance claim',
                        'The production example executable is built separately; its exact binary and complete local source closure are hashed',
                        'Source host-input bookkeeping zeroes missing lanes; poisoned padding is delivered only to production host PortFrame',
                        'Public Engine cables, callback partitioning, bypass, persistence and graph replacement require separate host integration tests',
                        ACCEPTANCE['excluded_source_paths']],
        'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0],
                      'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                      'platform': sys.platform, 'machine': os.uname().machine,
                      'cpp_flags': ['-std=c++17', '-O2', '-ffp-contract=off']},
        'cases': []}
    with evidence_directory(args.build_dir) as directory:
        tmp = Path(directory)
        retained = args.build_dir is not None
        report['trace_retention'] = {
            'mode': 'persistent' if retained else 'temporary_removed_after_run',
            'retained_after_exit': retained, 'directory': str(tmp),
            'format': 'One deterministic gzip per input/SDK/Rust trace; raw SHA256 and gzip SHA256 recorded per case',
            'case_artifacts': {'input': 0, 'sdk': 0, 'rust': 0},
            'files': {}}
        with (tmp / '.openpatch-random-reference-owned.json').open('x') as marker:
            json.dump({'purpose': 'Random actual-class full-suite evidence',
                       'reference_commit': PIN, 'acceptance_sha256': report['acceptance_sha256'],
                       'fixtures_sha256': report['fixtures_sha256']}, marker, indent=2)
            marker.write('\n')
        with (tmp / 'reference_module.hpp').open('x') as output:
            output.write(prefix)
        with (tmp / 'upstream-Random.cpp').open('xb') as output:
            output.write(source)
        with (tmp / 'sdk_probe.cpp').open('xb') as output:
            output.write((HERE / 'sdk_probe.cpp').read_bytes())
        saved_rust = tmp / 'rust_probe'
        with rust.open('rb') as input_stream, saved_rust.open('xb') as output_stream:
            shutil.copyfileobj(input_stream, output_stream)
        saved_rust.chmod(rust.stat().st_mode)
        assert digest(saved_rust) == report['rust_probe_binary_sha256']
        traces_dir = tmp / 'traces'
        traces_dir.mkdir()
        cpp = tmp / 'sdk_probe'
        flags = []
        if sys.platform == 'darwin':
            mac_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
            flags = ['-isysroot', str(mac_sdk), '-isystem', str(mac_sdk / 'usr/include/c++/v1')]
        compile_argv = [shutil.which('c++') or 'c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags,
                        '-I' + str(sdk / 'include'), '-I' + str(sdk / 'dep/include'), '-I' + str(tmp),
                        str(tmp / 'sdk_probe.cpp'), '-L' + str(sdk), '-lRack',
                        '-Wl,-rpath,' + str(sdk), '-o', str(cpp)]
        report['toolchain']['cpp_compile_argv'] = compile_argv
        report['toolchain']['cpp_flags'] = compile_argv[1:]
        with (tmp / 'compile-argv.json').open('x') as output:
            json.dump(compile_argv, output, indent=2)
            output.write('\n')
        subprocess.run(compile_argv, check=True)
        report['sdk_probe_binary_sha256'] = digest(cpp)
        for path in [tmp / 'reference_module.hpp', tmp / 'upstream-Random.cpp',
                     tmp / 'sdk_probe.cpp', cpp, saved_rust, tmp / 'compile-argv.json']:
            report['trace_retention']['files'][path.name] = retained_file(path, tmp)
        env = os.environ.copy()
        env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(sdk)
        case_keys = set()
        for rate, name, text in inputs():
            assert re.fullmatch(r'[a-z0-9_]+', name), 'Unsafe fixture name'
            key = f'{rate}-{name}'
            assert key not in case_keys, 'Duplicate case artifact name'
            case_keys.add(key)
            input_file = traces_dir / f'{key}.input.txt'
            with input_file.open('x') as input_stream:
                input_stream.write(text)
            trace_files = [traces_dir / f'{key}.{label}.txt' for label in ['sdk', 'rust']]
            for exe, output_file in zip([cpp, saved_rust], trace_files):
                with input_file.open('rb') as input_stream, output_file.open('xb') as output_stream:
                    subprocess.run([str(exe), str(rate)], stdin=input_stream, stdout=output_stream,
                                   stderr=subprocess.PIPE, check=True, env=env)
            traces = [parse_trace(path, frame_count(name, rate)) for path in trace_files]
            case = compare(*traces, text, name, rate)
            case['input_sha256'] = hashlib.sha256(text.encode()).hexdigest()
            case['trace_sha256'] = {label: digest(path) for label, path in zip(['sdk', 'rust'], trace_files)}
            case['saved_artifacts'] = {
                'input': save_gzip(input_file, tmp, case['input_sha256']),
                **{label: save_gzip(path, tmp, case['trace_sha256'][label])
                   for label, path in zip(['sdk', 'rust'], trace_files)}}
            for label in ['input', 'sdk', 'rust']:
                report['trace_retention']['case_artifacts'][label] += 1
            report['cases'].append(case)
            print(json.dumps({'name': name, 'rate': rate, 'passed': case['passed'],
                              'events': case['events'], 'first_failures': case['first_failures'][:2]}), flush=True)
        expected_cases = len(ACCEPTANCE['sample_rates']) * len(ACCEPTANCE['cases'])
        assert len(case_keys) == expected_cases
        assert report['trace_retention']['case_artifacts'] == {label: expected_cases for label in ['input', 'sdk', 'rust']}
        assert len(list(traces_dir.iterdir())) == 3 * expected_cases, 'Missing, duplicate, or uncompressed case artifacts'
        for case in report['cases']:
            for artifact in case['saved_artifacts'].values():
                assert digest(tmp / artifact['relative_path']) == artifact['sha256'], 'Saved artifact hash changed'
        index = {'retained_after_exit': retained,
                 'acceptance_sha256': report['acceptance_sha256'],
                 'fixtures_sha256': report['fixtures_sha256'],
                 'files': report['trace_retention']['files'],
                 'case_artifact_counts': report['trace_retention']['case_artifacts'],
                 'cases': [{'name': case['name'], 'sample_rate': case['sample_rate'],
                            'artifacts': case['saved_artifacts']} for case in report['cases']]}
        with (tmp / 'evidence-index.json').open('x') as output:
            json.dump(index, output, indent=2)
            output.write('\n')
        report['trace_retention']['evidence_index'] = retained_file(tmp / 'evidence-index.json', tmp)
    assert closure == production_closure(), 'Production source changed during qualification'
    assert report['rust_probe_binary_sha256'] == digest(rust), 'Production binary changed during qualification'
    assert drivers == {path.name: digest(path) for path in HERE.iterdir() if path.suffix in ['.py', '.cpp', '.json', '.md']}, 'Oracle scaffold changed during qualification'
    report['passed'] = (len(report['cases']) == len(ACCEPTANCE['sample_rates']) * len(ACCEPTANCE['cases'])
                        and all(case['passed'] for case in report['cases']))
    report['qualification'] = 'full frozen suite' if report['passed'] else 'failed; no qualification'
    report['total_frames'] = sum(case['frames'] for case in report['cases'])
    report['total_compared_values'] = sum(case['compared_values'] for case in report['cases'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'cases': len(report['cases']),
                      'frames': report['total_frames'], 'report': str(args.output)}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
