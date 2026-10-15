#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualify native Free Delay against the actual pinned Fundamental Delay class.

No Cargo, Rack plugin/runtime, GUI or audio-device invocation. The caller must
provide an already-built serde_json rlib. Compiles only temporary standalone
probes against the SDK headers/library and the production Rust source.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
PIN = '10dd0160c664770910e5584b7b00498cc48d9ddd'
SRC_PIN = 'f6730d03c3e7660bb6ecad8816f1b09c5825142a'
URL = f'https://raw.githubusercontent.com/VCVRack/Fundamental/{PIN}/src/Delay.cpp'
ACCEPTANCE_PATH = HERE / 'acceptance.json'
ACCEPTANCE_SHA256 = '4e274862e9e8effa4a177eeb4d737005fd25a07e66d272dd4d5833cc52e36e50'
ACCEPTANCE = json.loads(ACCEPTANCE_PATH.read_text())
CASES = {case['name']: case for case in ACCEPTANCE['cases']}
PRODUCTION = ROOT / 'crates/openpatch-core/src/stock_delay.rs'
SINC = ROOT / 'crates/openpatch-core/src/stock_delay/sinc.rs'
U32 = struct.Struct('<I')
F32 = struct.Struct('<f')
DELAY_OUTPUT = struct.Struct('<IIff')

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def fetch(url):
    return subprocess.check_output(['curl', '--fail', '--silent', '--show-error', '--location', url])

def run(command, **kwargs):
    return subprocess.run([str(value) for value in command], check=True, **kwargs)

def raw_time(seconds):
    return math.log10(seconds * 1000) / 4

def vectors(name, frames, rate):
    """Deterministic finite, host-bounded inputs in actual source port order."""
    for frame in range(frames):
        t = frame / rate
        p = [raw_time(.02), .35, .5, .65, 0., 0., 0., 0.]
        ports = [[], [], [], [], [2 * math.sin(2 * math.pi * 233 * t)], []]
        control = 0
        if name == 'startup_impulse':
            p[0] = raw_time(.5)
            ports[4] = [5. if frame == 0 else 0.]
        elif name == 'startup_dc':
            p[0] = raw_time(.5)
            ports[4] = [2.]
        elif name in ('minimum_delay', 'maximum_delay'):
            p = [0. if name == 'minimum_delay' else 1., 0., .5, 1., 0., 0., 0., 0.]
            ports[4] = [5. if frame == 0 else 0.]
        elif name == 'tone_feedback_extremes':
            segment = int(t * 3) % 12
            p[1] = [0., .5, 1.][segment % 3]
            p[2] = [0., .5, 1.][segment // 3 % 3]
            p[3] = [0., .5, 1.][segment // 2 % 3]
            ports[4] = [500. if frame % 997 == 0 else 2 * math.sin(2 * math.pi * 5771 * t)]
        elif name in ('signed_cv', 'cv_zero_depth'):
            segment = int(t * 8)
            p[4:] = ([1., -1., 1., -1.] if segment % 2 == 0 else [-1., 1., -1., 1.]) if name == 'signed_cv' else [0.] * 4
            levels = [-20., -10., -2., 0., 2., 10., 20.]
            for port in range(4):
                ports[port] = [levels[(segment + port) % len(levels)]]
        elif name == 'time_jumps':
            p[0] = raw_time([.001, .02, .8, .005, 10., .04][int(t * 2) % 6])
            p[1] = .7
        elif name.startswith('clock_'):
            p[0] = raw_time(.125)
            p[4] = -.25
            ports[0] = [math.sin(2 * math.pi * .3 * t)]
            ports[5] = [0.]
            if name == 'clock_connected_startup':
                pass  # A patched clock with no edges retains constructor 1 Hz.
            elif name == 'clock_tempo_change':
                period = .5 if t < 2. else .25
                ports[5] = [10. if t % period < .002 else 0.]
            elif name == 'clock_unpatch_reconnect':
                ports[5] = [] if 1.5 <= t < 3. else [10. if t % .4 < .002 else 0.]
            elif name == 'clock_thresholds':
                levels = [2., .1, 1.9, 2., 0., .11, 1.99, 2.01, -.1, 10.]
                ports[5] = [levels[frame // max(1, rate // 100) % len(levels)]]
        elif name == 'poly_sum_aux_first_lane':
            width = [16, 1, 5, 0, 4, 16][int(t * 6) % 6]
            ports[4] = [(.25 + lane / 32) * math.sin(2 * math.pi * (137 + lane * 29) * t) for lane in range(width)]
            p[4:] = [.5, -.75, .8, -1.]
            for port in range(4):
                ports[port] = [.8 * math.sin(t * (11 + port)), *[(-1 if lane % 2 else 1) * (1000 + lane * 23) for lane in range(1, 16)]]
            ports[5] = [10. if t % .3 < .002 else 0., *[1000.] * 15]
        elif name == 'parameter_adoption':
            segment = int(t * 10)
            p = [raw_time([.002, .1, .03][segment % 3]), [0., .8, 1.][segment % 3], (segment % 5) / 4, (segment % 3) / 2, -.5, .5, -.5, .5]
            for port in range(4):
                ports[port] = [2 * math.sin(t * (13 + port))]
            if frame in [127, 4093, 8191, int(rate * 1.1), int(rate * 2.3)]:
                control = 1
        elif name == 'history_wrap':
            p[0] = raw_time(.001)
            p[1] = .4
            ports[4] = [2 * math.sin(2 * math.pi * 113 * t) + (3. if frame % 50021 == 0 else 0.)]
        else:
            raise ValueError(name)
        yield control, p, ports

def write_delay_input(path, name, frames, rate):
    with path.open('wb') as out:
        for control, params, ports in vectors(name, frames, rate):
            out.write(struct.pack('<I8f', control, *params))
            for values in ports:
                out.write(U32.pack(len(values)))
                if values:
                    out.write(struct.pack('<' + 'f' * len(values), *values))

def src_calls(name):
    accepted = ACCEPTANCE['resampler']
    pending = []
    cursor = 0
    for call in range(accepted['calls_per_scenario']):
        capacity = 16
        wanted = 16
        if name == 'constant_min': ratio = .25
        elif name == 'constant_unity': ratio = 1.
        elif name == 'constant_max': ratio = 4.
        elif name == 'steps': ratio = accepted['ratios'][call // 37 % len(accepted['ratios'])]
        elif name == 'smooth': ratio = .25 * 16 ** ((math.sin(call / 91) + 1) / 2)
        elif name == 'capacity_and_short_input':
            capacity = [0, 1, 3, 16, 7, 2, 15][call % 7]
            wanted = [0, 1, 2, 15, 16, 3, 9][call // 7 % 7]
            ratio = accepted['ratios'][call // 13 % len(accepted['ratios'])]
        elif name == 'analytic_1_to_quarter': ratio = 1. if call == 0 else .25
        else: raise ValueError(name)
        # The runner replays the entire identical call stream to each probe.
        # Reoffering unconsumed data is not necessary to qualify the call API:
        # this scenario deliberately supplies fresh bounded arbitrary buffers.
        values = [(.5 * math.sin((cursor + i) * .173) + (.75 if (cursor + i) % 127 == 0 else 0.)) for i in range(wanted)]
        cursor += wanted
        yield wanted, capacity, ratio, values

def compare_delay(reference, actual, frames):
    failures = []
    worst = {'absolute_error_volts': -1.}
    maximum_ratio = 0.
    bound = ACCEPTANCE['delay']
    with reference.open('rb') as ref, actual.open('rb') as got:
        for frame in range(frames):
            a, b = ref.read(16), got.read(16)
            if len(a) != 16 or len(b) != 16:
                raise AssertionError(f'truncated delay output at frame {frame}')
            a, b = DELAY_OUTPUT.unpack(a), DELAY_OUTPUT.unpack(b)
            if a[:2] != (1, 1) or b[:2] != (1, 1):
                if len(failures) < 8: failures.append({'frame': frame, 'reason': 'not exactly mono'})
            for output in range(2):
                x, y = a[2 + output], b[2 + output]
                if not math.isfinite(x) or not math.isfinite(y):
                    raise AssertionError(f'nonfinite output {frame}/{output}')
                error = abs(x - y)
                tolerance = bound['absolute_volts'] + bound['relative'] * abs(x)
                maximum_ratio = max(maximum_ratio, error / tolerance)
                if error > worst['absolute_error_volts']:
                    worst = {'frame_zero_based': frame, 'output': output, 'reference_volts': x, 'actual_volts': y, 'absolute_error_volts': error, 'tolerance_volts': tolerance}
                if error > tolerance and len(failures) < 8:
                    failures.append({'frame': frame, 'output': output, 'reference': x, 'actual': y})
        if ref.read(1) or got.read(1): raise AssertionError('extra delay output frames')
    return {'passed': not failures, 'worst_sample': worst, 'maximum_pointwise_tolerance_ratio': maximum_ratio, 'first_failures': failures}

def compare_src(reference, actual):
    failures = []
    maximum_error = 0.
    maximum_ratio = 0.
    bound = ACCEPTANCE['resampler']
    with reference.open('rb') as ref, actual.open('rb') as got:
        for call in range(bound['calls_per_scenario']):
            a, b = ref.read(8), got.read(8)
            if len(a) != 8 or len(b) != 8: raise AssertionError(f'truncated SRC result at call {call}')
            used_a, count_a = struct.unpack('<II', a)
            used_b, count_b = struct.unpack('<II', b)
            if max(used_a, count_a, used_b, count_b) > 16: raise AssertionError('invalid SRC count')
            a, b = ref.read(count_a * 4), got.read(count_b * 4)
            if len(a) != count_a * 4 or len(b) != count_b * 4: raise AssertionError('truncated SRC samples')
            if (used_a, count_a) != (used_b, count_b) and len(failures) < 8:
                failures.append({'call': call, 'reference_counts': [used_a, count_a], 'actual_counts': [used_b, count_b]})
            for offset, (x, y) in enumerate(zip(struct.unpack('<' + 'f' * count_a, a), struct.unpack('<' + 'f' * count_b, b))):
                if not math.isfinite(x) or not math.isfinite(y): raise AssertionError('nonfinite SRC sample')
                error = abs(x - y)
                tolerance = bound['absolute'] + bound['relative'] * abs(x)
                maximum_error = max(maximum_error, error)
                maximum_ratio = max(maximum_ratio, error / tolerance)
                if error > tolerance and len(failures) < 8: failures.append({'call': call, 'offset': offset, 'reference': x, 'actual': y})
        if ref.read(1) or got.read(1): raise AssertionError('extra SRC result calls')
    return {'passed': not failures, 'maximum_absolute_error': maximum_error, 'maximum_tolerance_ratio': maximum_ratio, 'first_failures': failures}

def validate_src_safety(path, name):
    """Check the complete native extension trace without inventing C samples."""
    samples = 0
    with path.open('rb') as source:
        for call, (n, capacity, _, _) in enumerate(src_calls(name)):
            header = source.read(8)
            if len(header) != 8: raise AssertionError(f'truncated native safety trace at {call}')
            used, generated = struct.unpack('<II', header)
            if used > n or generated > capacity: raise AssertionError(f'invalid native safety counts at {call}')
            data = source.read(generated * 4)
            if len(data) != generated * 4: raise AssertionError('truncated native safety samples')
            if not all(math.isfinite(v) for v in struct.unpack('<' + 'f' * generated, data)): raise AssertionError('nonfinite native safety sample')
            samples += generated
        if source.read(1): raise AssertionError('extra native safety calls')
    return {'passed': True, 'calls': ACCEPTANCE['resampler']['calls_per_scenario'], 'finite_samples': samples, 'counts_within_supplied_slices': True}

def validate_delay_safety(path, frames):
    """No invented SDK voltages on a proven source-undefined whole trace."""
    with path.open('rb') as source:
        for frame in range(frames):
            data = source.read(DELAY_OUTPUT.size)
            if len(data) != DELAY_OUTPUT.size: raise AssertionError(f'truncated native Delay safety trace at frame {frame}')
            width0, width1, mix, wet = DELAY_OUTPUT.unpack(data)
            if (width0, width1) != (1, 1): raise AssertionError('native Delay safety trace is not mono')
            if not math.isfinite(mix) or not math.isfinite(wet): raise AssertionError('nonfinite native Delay safety trace')
        if source.read(1): raise AssertionError('extra native Delay safety frames')
    return {'passed': True, 'frames': frames, 'finite_visible_samples': frames * 2, 'mono_outputs': True, 'padding_zero_asserted_by_probe': True}

def guarded_sinc_source(source):
    """Inject guards before the actual two mono-kernel buffer reads only.

    This derives no state or indexing from Rust. All source C arithmetic,
    control flow and reads remain unchanged when the guarded index is valid.
    """
    start = source.index('static inline double\ncalc_output_single ')
    end = source.index('static int\nsinc_mono_vari_process ', start)
    kernel = source[start:end]
    expression = 'filter->buffer [data_index]'
    assert kernel.count(expression) == 2
    helper = r'''extern long reference_call_index ;
extern long reference_frame_index ;
static inline float
reference_history_sample (SINC_FILTER *filter, int data_index, int line)
{
    if (data_index < 0 || data_index >= filter->b_len + filter->channels)
    {
        fprintf (stderr, "{\"kind\":\"source_undefined\",\"frame_zero_based\":%ld,\"call_zero_based\":%ld,\"data_index\":%d,\"allocated_samples\":%d,\"current\":%d,\"output_offset\":%ld,\"line\":%d}\n",
            reference_frame_index, reference_call_index, data_index, filter->b_len + filter->channels,
            filter->b_current, filter->out_gen, line) ;
        fflush (stderr) ;
        exit (87) ;
    }
    return filter->buffer [data_index] ;
}

'''
    kernel = kernel.replace(expression, 'reference_history_sample (filter, data_index, __LINE__)')
    return source[:start] + helper + kernel + source[end:]

def execute_pair(executables, arguments, input_path, prefix, env, retain):
    results = []
    for label, executable in zip(['sdk', 'rust'], executables):
        path = prefix.with_name(prefix.name + '-' + label + '.bin')
        with input_path.open('rb') as source, path.open('wb') as output:
            result = run([executable, *arguments], stdin=source, stdout=output, stderr=subprocess.PIPE, env=env)
        if result.stderr:
            raise AssertionError(f'{label} probe unexpected stderr: {result.stderr.decode(errors="replace")}')
        results.append(path)
    return results

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, default=Path.home() / 'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--serde-rlib', type=Path, required=True, help='Existing serde_json rlib; this script never runs Cargo')
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/benchmarks/stock-delay-reference.json')
    parser.add_argument('--case', action='append', choices=list(CASES))
    parser.add_argument('--rate', action='append', type=int, choices=ACCEPTANCE['sample_rates'])
    parser.add_argument('--resampler-only', action='store_true')
    parser.add_argument('--retain-traces', type=Path, help='Optional destination for all exact binary input/output traces')
    args = parser.parse_args()
    if sys.byteorder != 'little': raise AssertionError('SDK binary driver requires little-endian platform')
    if digest(ACCEPTANCE_PATH) != ACCEPTANCE_SHA256: raise AssertionError('frozen acceptance declaration changed')
    sdk = args.sdk.resolve()
    library = sdk / ('libRack.dylib' if sys.platform == 'darwin' else 'libRack.so')
    upstream = fetch(URL)
    text = upstream.decode()
    assert text.count('struct DelayWidget') == 1 and text.count('#include "plugin.hpp"') == 1
    prefix = text.split('struct DelayWidget', 1)[0].replace('#include "plugin.hpp"', '#include <rack.hpp>\nusing namespace rack;')
    src_files = ['src/samplerate.c', 'src/src_sinc.c', 'src/src_linear.c', 'src/src_zoh.c', 'src/common.h', 'src/float_cast.h', 'src/samplerate.h', 'src/fastest_coeffs.h', 'src/mid_qual_coeffs.h', 'src/high_qual_coeffs.h', 'COPYING']
    src_sources = {name: fetch(f'https://raw.githubusercontent.com/libsndfile/libsamplerate/{SRC_PIN}/{name}') for name in src_files}
    src_hashes = {name: hashlib.sha256(source).hexdigest() for name, source in src_sources.items()}
    header_paths = sorted((sdk / 'include').rglob('*.hpp')) + [sdk / 'dep/include/samplerate.h']
    headers = {str(path.relative_to(sdk)): digest(path) for path in header_paths}
    report = {
        'reference': {'fundamental_commit': PIN, 'source_url': URL, 'source_sha256': hashlib.sha256(upstream).hexdigest(), 'compiled_prefix_sha256': hashlib.sha256(prefix.encode()).hexdigest(), 'libsamplerate_commit': SRC_PIN, 'libsamplerate_source_sha256': src_hashes, 'sdk_library_path': str(library), 'sdk_library_sha256': digest(library), 'sdk_header_sha256': headers},
        'acceptance': ACCEPTANCE, 'acceptance_sha256': digest(ACCEPTANCE_PATH),
        'production_source_sha256': {str(path.relative_to(ROOT)): digest(path) for path in [PRODUCTION, SINC, SINC.with_name('fastest_coeffs.rs')]},
        'driver_sha256': {path.name: digest(path) for path in sorted(HERE.iterdir()) if path.is_file()},
        'dependency': {'serde_json_rlib_path': str(args.serde_rlib.resolve()), 'serde_json_rlib_sha256': digest(args.serde_rlib)},
        'toolchain': {'cpp': subprocess.check_output(['c++', '--version'], text=True).splitlines()[0], 'cc': subprocess.check_output(['cc', '--version'], text=True).splitlines()[0], 'rust': subprocess.check_output(['rustc', '--version', '--verbose'], text=True).strip(), 'python': sys.version, 'platform': platform.platform(), 'machine': platform.machine(), 'cpp_flags': ['-std=c++17', '-O2', '-ffp-contract=off'], 'preflight_c_flags': ['-std=c99', '-O2', '-ffp-contract=off'], 'rust_flags': ['--edition=2021', '-O']},
        'method': 'Actual pinned Delay class prefix compiled against actual SDK. Only plugin.hpp include replaced; widgets omitted. Actual pinned C0.1.9 mono history-read guard preflights every artificial SRC stream and every whole-Delay trace before its SDK call. Source-undefined scenarios receive no SDK fidelity pass and still require a complete finite/count-bounded native safety trace. Every source-defined scenario requires genuine SDK SRC_SINC_FASTEST exact counts and frozen sample comparison. No Cargo/plugin/runtime/device/GUI execution. In-place validated parameter updates; fresh-build/adopt at explicit lifecycle markers; source remains continuous. Exact binary f32 traces, identical sample indices from startup, mono outputs, all native padding asserted zero. No alignment/warmup exclusions.',
        'authorized_adapter_amendment': 'After acceptance freeze but before any execution, the owning agent requested parameter-only updates use production set_parameters to match actual SDK parameter updates and avoid allocations. Explicit control==1 fresh-build/adopt markers remain unchanged. The frozen acceptance lifecycle sentence describes the earlier adapter; every input, numerical bound and declared explicit adoption marker is unchanged.',
        'limitations': ['Only recorded SDK/toolchain/platform qualified', 'DSP adapter does not exercise host graph/cable delay, public validation, bypass, initialize/reset or sample-rate changes; qualify separately with host tests', 'Clock lamp visual smoothing is not observed by this audio-only adapter; source clock behavior is exercised through delay audio'],
        'resampler_cases': [], 'cases': [],
    }
    env = os.environ.copy()
    env['DYLD_LIBRARY_PATH' if sys.platform == 'darwin' else 'LD_LIBRARY_PATH'] = str(sdk)
    with tempfile.TemporaryDirectory(prefix='openpatch-stock-delay-reference-') as directory:
        tmp = Path(directory)
        (tmp / 'reference_module.hpp').write_text(prefix)
        flags = []
        if sys.platform == 'darwin':
            mac_sdk = Path(subprocess.check_output(['xcrun', '--show-sdk-path'], text=True).strip())
            flags = ['-isysroot', str(mac_sdk), '-isystem', str(mac_sdk / 'usr/include/c++/v1')]
            report['toolchain']['macos_sdk'] = str(mac_sdk)
        cpp = tmp / 'sdk_probe'
        cpp_src = tmp / 'src_probe'
        rust = tmp / 'rust_probe'
        rust_src = tmp / 'rust_src_probe'
        guarded_src = tmp / 'guarded_src_probe'
        guarded_delay = tmp / 'guarded_delay_probe'
        guard_dir = tmp / 'guarded-src'
        guard_dir.mkdir()
        for name, source in src_sources.items():
            if name.startswith('src/'):
                (guard_dir / Path(name).name).write_bytes(source)
        guarded_source = guarded_sinc_source(src_sources['src/src_sinc.c'].decode())
        (guard_dir / 'src_sinc.c').write_text(guarded_source)
        config = (HERE / 'preflight_config.h').read_text()
        (guard_dir / 'config.h').write_text(config)
        report['reference']['source_definedness_preflight'] = {
            'scope': 'Only actual mono calc_output_single history reads are bounds-guarded; not a general C undefined-behavior proof or a claim that arbitrary-call source UB is reachable from Delay.',
            'guard_exit_status': 87,
            'guarded_source_sha256': digest(guard_dir / 'src_sinc.c'),
            'guarded_source_exact': guarded_source,
            'config_sha256': digest(guard_dir / 'config.h'),
            'config_exact': config,
            'analytic_extension_predeclaration': {'name': 'analytic_1_to_quarter', 'calls': ACCEPTANCE['resampler']['calls_per_scenario'], 'input_frames': 16, 'output_frames': 16, 'first_target_ratio': 1.0, 'subsequent_target_ratio': .25, 'predicted_call_zero_based': 5, 'predicted_output_offset': 10, 'predicted_data_index': -1, 'predicted_current': 35},
        }
        api_names = sorted(set(re.findall(r'\b(src_[a-zA-Z0-9_]+)\s*\(', src_sources['src/samplerate.h'].decode())) | {f'{kind}_{operation}' for kind in ['sinc', 'linear', 'zoh'] for operation in ['get_name', 'get_description', 'set_converter']})
        api_defines = ['-D' + name + '=reference_guarded_' + name for name in api_names]
        report['reference']['source_definedness_preflight']['private_api_renames'] = dict(zip(api_names, ['reference_guarded_' + name for name in api_names]))
        report['reference']['source_definedness_preflight']['whole_delay_method'] = 'Compile the same actual Delay class prefix and SDK adapter against privately renamed guarded C0.1.9 SRC symbols. The executable owns its private converter; libRack supplies Module and engine support. No dylib interposition assumption. Every whole-Delay fixture preflights before genuine SDK numerical execution. Actual outBuffer.empty() is read only to label SRC calls in diagnostics.'
        c_flags = ['-isysroot', str(mac_sdk)] if sys.platform == 'darwin' else []
        guarded_objects = []
        for name in ['samplerate.c', 'src_sinc.c', 'src_linear.c', 'src_zoh.c']:
            object_path = guard_dir / (Path(name).stem + '.o')
            run(['cc', '-std=c99', '-O2', '-ffp-contract=off', *c_flags, *api_defines, '-I' + str(guard_dir), '-c', guard_dir / name, '-o', object_path])
            guarded_objects.append(object_path)
        report['reference']['source_definedness_preflight']['object_sha256'] = {path.name: digest(path) for path in guarded_objects}
        run(['cc', '-std=c99', '-O2', '-ffp-contract=off', *c_flags, *api_defines, '-I' + str(guard_dir), HERE / 'src_guard_probe.c', *guarded_objects, '-lm', '-o', guarded_src])
        run(['c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags, *api_defines, '-DREFERENCE_GUARD=1', '-I' + str(sdk / 'include'), '-I' + str(guard_dir), '-I' + str(sdk / 'dep/include'), '-I' + str(tmp), HERE / 'sdk_probe.cpp', *guarded_objects, '-L' + str(sdk), '-lRack', '-Wl,-rpath,' + str(sdk), '-o', guarded_delay])
        for source, executable in [('sdk_probe.cpp', cpp), ('src_probe.cpp', cpp_src)]:
            run(['c++', '-std=c++17', '-O2', '-ffp-contract=off', *flags, '-I' + str(sdk / 'include'), '-I' + str(sdk / 'dep/include'), '-I' + str(tmp), HERE / source, '-L' + str(sdk), '-lRack', '-Wl,-rpath,' + str(sdk), '-o', executable])
        report['reference']['sdk_src_identity'] = subprocess.check_output([str(cpp_src), '--identity'], text=True, env=env).strip()
        report['reference']['sdk_actual_bypass_width_observation'] = json.loads(subprocess.check_output([str(cpp), '--bypass-widths'], text=True, env=env))
        if '0.1.9' not in report['reference']['sdk_src_identity']: raise AssertionError('SDK SRC version is not pinned 0.1.9')
        for template, replacement, executable in [('rust_probe.rs', ('__STOCK_DELAY_SOURCE__', PRODUCTION), rust), ('rust_src_probe.rs', ('__SINC_SOURCE__', SINC), rust_src)]:
            path = tmp / template
            path.write_text((HERE / template).read_text().replace(replacement[0], json.dumps(str(replacement[1]))))
            run(['rustc', '--edition=2021', '-O', path, '--extern', 'serde_json=' + str(args.serde_rlib.resolve()), '-L', 'dependency=' + str(args.serde_rlib.resolve().parent), '-o', executable])
        trace_dir = args.retain_traces.resolve() if args.retain_traces else tmp / 'traces'
        trace_dir.mkdir(parents=True, exist_ok=True)
        for name in [*ACCEPTANCE['resampler']['scenarios'], 'analytic_1_to_quarter']:
            input_path = trace_dir / f'src-{name}-input.bin'
            with input_path.open('wb') as output:
                for n, capacity, ratio, values in src_calls(name):
                    output.write(struct.pack('<IId', n, capacity, ratio))
                    if values: output.write(struct.pack('<' + 'f' * n, *values))
            preflight_path = trace_dir / f'src-{name}-preflight.bin'
            with input_path.open('rb') as source, preflight_path.open('wb') as output:
                preflight = subprocess.run([str(guarded_src)], stdin=source, stdout=output, stderr=subprocess.PIPE, env=env)
            case = {'name': name, 'calls': ACCEPTANCE['resampler']['calls_per_scenario'], 'input_sha256': digest(input_path), 'preflight_output_sha256': digest(preflight_path), 'output_sha256': {}, 'preflight_exit_status': preflight.returncode}
            if preflight.returncode == 87:
                diagnostic = json.loads(preflight.stderr.decode())
                if diagnostic.get('kind') != 'source_undefined': raise AssertionError('invalid source-definedness diagnostic')
                native_path = trace_dir / f'src-{name}-rust.bin'
                with input_path.open('rb') as source, native_path.open('wb') as output:
                    native = run([rust_src], stdin=source, stdout=output, stderr=subprocess.PIPE, env=env)
                if native.stderr: raise AssertionError('native safety probe unexpected stderr')
                paths = [native_path]
                case.update({'status': 'source_undefined', 'passed': None, 'fidelity_passed': None, 'source_undefined': diagnostic, 'sdk_execution_skipped': True, 'native_safety': validate_src_safety(native_path, name)})
                case['output_sha256']['rust'] = digest(native_path)
            elif preflight.returncode == 0 and not preflight.stderr:
                paths = execute_pair([cpp_src, rust_src], [], input_path, trace_dir / f'src-{name}', env, args.retain_traces)
                comparison = compare_src(*paths)
                case.update({'status': 'source_defined', **comparison, 'fidelity_passed': comparison['passed'], 'sdk_execution_skipped': False, 'native_safety': validate_src_safety(paths[1], name)})
                case['output_sha256'] = dict(zip(['sdk', 'rust'], map(digest, paths)))
            else:
                raise AssertionError(f'preflight failed with {preflight.returncode}: {preflight.stderr.decode(errors="replace")}')
            if name == 'analytic_1_to_quarter':
                expected = report['reference']['source_definedness_preflight']['analytic_extension_predeclaration']
                event = case.get('source_undefined', {})
                if any(event.get(key) != expected['predicted_' + key] for key in ['call_zero_based', 'output_offset', 'data_index', 'current']): raise AssertionError('pinned C source did not reproduce the analytically predicted extension violation')
            paths.append(preflight_path)
            if args.retain_traces: case['raw_traces'] = [str(input_path), *map(str, paths)]
            report['resampler_cases'].append(case)
            print(json.dumps({'resampler': name, 'status': case['status'], 'fidelity_passed': case['fidelity_passed'], 'native_safety_passed': case['native_safety']['passed'], 'tolerance_ratio': case.get('maximum_tolerance_ratio')}), flush=True)
            if not args.retain_traces:
                for path in [input_path, *paths]: path.unlink()
        src_passed = all(case['native_safety']['passed'] and (case['status'] == 'source_undefined' or case['fidelity_passed']) for case in report['resampler_cases'])
        if src_passed and not args.resampler_only:
            for rate in args.rate or ACCEPTANCE['sample_rates']:
                for name in args.case or CASES:
                    declaration = CASES[name]
                    if rate not in declaration.get('rates', ACCEPTANCE['sample_rates']): continue
                    frames = declaration.get('frames', round(declaration.get('seconds', 0) * rate))
                    input_path = trace_dir / f'{name}-{rate}-input.bin'
                    write_delay_input(input_path, name, frames, rate)
                    preflight_path = trace_dir / f'{name}-{rate}-preflight.bin'
                    with input_path.open('rb') as source, preflight_path.open('wb') as output:
                        preflight = subprocess.run([str(guarded_delay), str(rate)], stdin=source, stdout=output, stderr=subprocess.PIPE, env=env)
                    case = {'name': name, 'sample_rate': rate, 'frames': frames, 'input_sha256': digest(input_path), 'output_sha256': {}, 'preflight_output_sha256': digest(preflight_path), 'preflight_exit_status': preflight.returncode}
                    if preflight.returncode == 87:
                        diagnostic = json.loads(preflight.stderr.decode())
                        if diagnostic.get('kind') != 'source_undefined' or not 0 <= diagnostic.get('frame_zero_based', -1) < frames: raise AssertionError('invalid whole-Delay source-definedness diagnostic')
                        native_path = trace_dir / f'{name}-{rate}-rust.bin'
                        with input_path.open('rb') as source, native_path.open('wb') as output:
                            native = run([rust, rate], stdin=source, stdout=output, stderr=subprocess.PIPE, env=env)
                        if native.stderr: raise AssertionError('native whole-Delay safety probe unexpected stderr')
                        paths = [native_path]
                        case.update({'status': 'source_undefined_whole_delay', 'passed': None, 'fidelity_passed': None, 'source_undefined': diagnostic, 'sdk_execution_skipped': True, 'native_safety': validate_delay_safety(native_path, frames)})
                        case['output_sha256']['rust'] = digest(native_path)
                    elif preflight.returncode == 0 and not preflight.stderr:
                        paths = execute_pair([cpp, rust], [rate], input_path, trace_dir / f'{name}-{rate}', env, args.retain_traces)
                        comparison = compare_delay(*paths, frames)
                        case.update({'status': 'source_defined', **comparison, 'fidelity_passed': comparison['passed'], 'sdk_execution_skipped': False, 'native_safety': validate_delay_safety(paths[1], frames)})
                        case['output_sha256'] = dict(zip(['sdk', 'rust'], map(digest, paths)))
                    else:
                        raise AssertionError(f'whole-Delay preflight failed with {preflight.returncode}: {preflight.stderr.decode(errors="replace")}')
                    paths.append(preflight_path)
                    if args.retain_traces: case['raw_traces'] = [str(input_path), *map(str, paths)]
                    report['cases'].append(case)
                    print(json.dumps({'case': name, 'rate': rate, 'status': case['status'], 'fidelity_passed': case['fidelity_passed'], 'native_safety_passed': case['native_safety']['passed'], 'tolerance_ratio': case.get('maximum_pointwise_tolerance_ratio')}), flush=True)
                    if not args.retain_traces:
                        for path in [input_path, *paths]: path.unlink()
    report['production_source_end_sha256'] = {name: digest(ROOT / name) for name in report['production_source_sha256']}
    report['production_sources_stable_during_run'] = report['production_source_end_sha256'] == report['production_source_sha256']
    report['final_rerun_required_due_to_production_changes'] = not report['production_sources_stable_during_run']
    delay_defined_passed = all(case['native_safety']['passed'] and (case['status'] == 'source_undefined_whole_delay' or case['fidelity_passed']) for case in report['cases'])
    report['passed'] = src_passed and delay_defined_passed and report['production_sources_stable_during_run']
    report['passed_scope'] = 'Every source-defined direct SRC and whole-Delay comparison, every native safety trace, and unchanged production source hashes throughout the run; source-undefined scenarios do not receive a fidelity pass.'
    report['source_undefined_resampler_cases'] = [case['name'] for case in report['resampler_cases'] if case['status'] == 'source_undefined']
    report['all_resampler_scenarios_fidelity_passed'] = all(case['fidelity_passed'] is True for case in report['resampler_cases'])
    report['source_undefined_whole_delay_cases'] = [{'name': case['name'], 'sample_rate': case['sample_rate'], 'first_invalid_source_read': case['source_undefined']} for case in report['cases'] if case['status'] == 'source_undefined_whole_delay']
    report['all_whole_delay_cases_fidelity_passed'] = bool(report['cases']) and all(case['fidelity_passed'] is True for case in report['cases'])
    report['whole_delay_skipped_due_to_src_failure'] = not src_passed and not args.resampler_only
    report['selection'] = {'rates': args.rate, 'cases': args.case, 'resampler_only': args.resampler_only}
    report['full_matrix_exercised'] = src_passed and not args.rate and not args.case and not args.resampler_only
    report['full_matrix'] = report['passed'] and report['all_whole_delay_cases_fidelity_passed'] and report['full_matrix_exercised']
    report['total_delay_frames'] = sum(case['frames'] for case in report['cases'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'passed': report['passed'], 'full_matrix': report['full_matrix'], 'all_whole_delay_cases_fidelity_passed': report['all_whole_delay_cases_fidelity_passed'], 'source_undefined_whole_delay_cases': len(report['source_undefined_whole_delay_cases']), 'final_rerun_required_due_to_production_changes': report['final_rerun_required_due_to_production_changes'], 'cases': len(report['cases']), 'frames': report['total_delay_frames'], 'report': str(args.output)}))
    return 0 if report['passed'] else 1

if __name__ == '__main__':
    raise SystemExit(main())
