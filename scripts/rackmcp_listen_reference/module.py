#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Frozen exact raw ListenModule/Core comparison using genuine Rack Engine."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PIN = '285e84c5ff2d3ecef25890da07f578af6a0c5ca6'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def write(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')

def run(argv, log=None, timeout=120, **kwargs):
    result = subprocess.run([str(a) for a in argv], cwd=ROOT, capture_output=True,
                            timeout=timeout, check=False, **kwargs)
    if log:
        log.write_bytes(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f'command failed ({result.returncode}): see {log}')
    return result.stdout.decode()

def bits(value):
    return struct.unpack('<I', struct.pack('<f', value))[0]

def fixtures():
    lines = []
    def emit(*args): lines.append(' '.join(map(str, args)))
    def source(port, values): emit('source', port, len(values), *map(bits, values))
    for rate in (44100, 48000, 96000):
        for width in range(17):
            for input_mask in range(4):
                for output_mask in range(4):
                    emit('new', rate, input_mask, output_mask)
                    source(0, [(-1 if c % 2 else 1) * (c + 0.25) for c in range(width)])
                    source(1, [2, -3, 4][:min(width, 3)])
                    emit('step', 2)
                    emit('arm', 1)
                    emit('step', 253)  # framesSeen=255: old published hints/lamp
                    emit('arm', 0)
                    emit('step', 1)    # frame 256 sees actual disarmed state
                    emit('arm', 1)
                    emit('step', 256)
                    emit('drain', 1000)
    # Source-order cancellation, tiny values, sign-bit passthrough, retirement,
    # regrowth, output raw writes while disconnected and bypass transitions.
    for output_mask in range(4):
        emit('new', 48000, 3, output_mask)
        source(0, [1e20, 1, -1e20, 3, -0.0, 1e-40, -1e-40, -4] * 2)
        source(1, [-0.0, 2, -7, 0.5])
        emit('arm', 1)
        emit('step', 257)
        source(0, [1.0])
        source(1, [])
        emit('step', 2)
        emit('bypass', 1)
        emit('step', 300)
        source(0, list(range(16)))
        emit('step', 2)
        emit('bypass', 0)
        emit('step', 253)
        emit('arm', 0)
        emit('step', 2)
        emit('drain', 1000)
    # The source ring capacity and rejection of new frames are exercised by the
    # original SpscRing. Native transport in this probe is a fixed bounded sink.
    emit('new', 192000, 1, 3)
    source(0, [3, -2, 0.5])
    emit('arm', 1)
    emit('step', (1 << 18) + 11)
    emit('drain', 500)
    emit('step', 250)
    emit('drain', 1 << 18)
    return '\n'.join(lines) + '\n'

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, default=ROOT/'native-test-output/rack-control-import/reference/rackmcp')
    parser.add_argument('--sdk', type=Path, default=Path.home()/'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    if out.exists(): parser.error('output exists; retain evidence and choose a new directory')
    out.mkdir(parents=True)
    reference, sdk = args.reference.resolve(), args.sdk.resolve()
    frozen = out/'frozen'
    frozen.mkdir()
    (out/'inputs.txt').write_text(fixtures())
    source_files = list((ROOT/'crates/openpatch-core/src').rglob('*.rs'))
    source_files += [ROOT/'Cargo.toml', ROOT/'Cargo.lock', ROOT/'crates/openpatch-core/Cargo.toml', ROOT/'crates/openpatch-core/LICENSE-THIRD-PARTY']
    source_files += [HERE/'module.cpp', HERE/'module.rs', HERE/'module.py', HERE/'MODULE.md']
    native_hashes = {}
    for path in source_files:
        relative = path.relative_to(ROOT)
        dest = frozen/'native'/relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest)
        native_hashes[str(relative)] = sha(path)
    upstream_hashes = {}
    for name in ('src/rackside/ListenModule.hpp', 'src/rackside/ListenModule.cpp', 'src/core/spsc.hpp', 'LICENSE', 'plugin.json'):
        path = reference/name
        dest = frozen/'reference'/name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest)
        upstream_hashes[name] = sha(path)
    # The SDK must match the separately qualified genuine 2.6.6 attestation.
    attestation = ROOT/'docs/benchmarks/output-lifecycle-reference.json'
    prior = json.loads(attestation.read_text())
    if not prior['passed'] or 'SDK=2.6.6\n' not in prior['sdk_metadata']:
        raise ValueError('missing successful genuine SDK 2.6.6 attestation')
    headers = {}
    for directory in ('include', 'dep/include'):
        shutil.copytree(sdk/directory, frozen/'sdk'/directory)
        for path in sorted((sdk/directory).rglob('*')):
            if path.is_file(): headers[str(path.relative_to(sdk))] = sha(path)
    if headers != prior['sdk_header_sha256']: raise ValueError('SDK headers differ from attestation')
    if sha(sdk/'libRack.dylib') != prior['sdk_library_sha256']: raise ValueError('SDK library differs from attestation')
    shutil.copy2(sdk/'libRack.dylib', out/'libRack.dylib')
    shutil.copy2(attestation, out/'sdk-attestation.json')
    source = (frozen/'reference/src/rackside/ListenModule.cpp').read_text()
    boundary = 'ListenModule* findListenModule('
    if source.count(boundary) != 1 or source.count('#include "rackmcp_plugin.hpp"') != 1:
        raise ValueError('unexpected source boundary')
    # Preserve the constructor/process verbatim; omit plugin/UI integration.
    prefix = source.split(boundary)[0].replace('#include "rackmcp_plugin.hpp"', '')
    (frozen/'reference_listen.hpp').write_text(prefix + '\n} // namespace rackmcp\n')
    sysroot = run(['xcrun', '--show-sdk-path']).strip()
    cpp = out/'module-cpp'
    rust = out/'module-rust'
    cpp_command = ['c++', '-std=c++17', '-O2', '-ffp-contract=off', '-isysroot', sysroot,
                   '-isystem', str(Path(sysroot)/'usr/include/c++/v1'),
                   '-I'+str(frozen/'sdk/include'), '-I'+str(frozen/'sdk/dep/include'),
                   '-I'+str(frozen), '-I'+str(frozen/'reference/src'),
                   str(frozen/'native/scripts/rackmcp_listen_reference/module.cpp'),
                   '-L'+str(out), '-lRack', '-Wl,-rpath,'+str(out), '-o', str(cpp)]
    manifest = dict(referencePin=PIN, upstream=upstream_hashes, nativeAndDrivers=native_hashes,
                    fixturesSha256=sha(out/'inputs.txt'), sdkHeaders=headers,
                    sdkLibrarySha256=sha(out/'libRack.dylib'), sdkAttestationSha256=sha(attestation),
                    compiledPrefixSha256=sha(frozen/'reference_listen.hpp'),
                    toolchain=dict(cpp=run(['c++', '--version']), rustc=run(['rustc', '-Vv']),
                    cargo=run(['cargo', '-V']), platform=platform.platform(),
                    RUSTFLAGS=os.environ.get('RUSTFLAGS', ''), CARGO_ENCODED_RUSTFLAGS=os.environ.get('CARGO_ENCODED_RUSTFLAGS', '')),
                    cppCommand=cpp_command)
    write(out/'inputs-before-build.json', manifest)
    run(cpp_command, out/'cpp-build.log')
    cargo_output = run(['cargo', 'build', '--release', '-p', 'openpatch-core', '--message-format=json'], out/'cargo-build.log', timeout=600)
    artifacts = [json.loads(line) for line in cargo_output.splitlines() if line.startswith('{')]
    library = next(Path(name) for item in artifacts if item.get('reason') == 'compiler-artifact'
                   and item.get('target', {}).get('name') == 'openpatch_core'
                   for name in item.get('filenames', []) if name.endswith('.rlib'))
    rust_command = ['rustc', '--edition=2021', '-O', '-C', 'lto=thin', frozen/'native/scripts/rackmcp_listen_reference/module.rs',
                    '--extern', f'openpatch_core={library}', '-L', f'dependency={library.parent / "deps" if (library.parent / "deps").is_dir() else library.parent}', '-o', rust]
    run(rust_command, out/'rust-build.log')
    changed = [name for name, digest in native_hashes.items() if sha(ROOT/name) != digest]
    if changed:
        write(out/'source-changed.json', changed)
        raise RuntimeError('native sources changed during build')
    write(out/'executables-before-samples.json', dict(cpp=sha(cpp), rust=sha(rust), library=sha(library),
                                                    rustCommand=list(map(str, rust_command))))
    env = os.environ.copy()
    env['DYLD_LIBRARY_PATH'] = str(out)
    for name, binary in [('cpp', cpp), ('rust', rust)]:
        with (out/'inputs.txt').open('rb') as source, (out/f'{name}.txt').open('wb') as sink, (out/f'{name}.stderr').open('wb') as errors:
            subprocess.run([str(binary)], stdin=source, stdout=sink, stderr=errors,
                           env=env, timeout=180, check=True)
    cpp_lines = (out/'cpp.txt').read_text().splitlines()
    rust_lines = (out/'rust.txt').read_text().splitlines()
    differences = []
    for index in range(max(len(cpp_lines), len(rust_lines))):
        a = cpp_lines[index] if index < len(cpp_lines) else None
        b = rust_lines[index] if index < len(rust_lines) else None
        if a != b:
            # Preserve full raw trace files, with compact first mismatches here.
            differences.append(dict(line=index, cpp=a[:200] if a else a, rust=b[:200] if b else b))
    write(out/'exact-differences.json', differences[:50])
    result = dict(passed=not differences, snapshots=len(cpp_lines), differingLines=len(differences),
                  cppTraceSha256=sha(out/'cpp.txt'), rustTraceSha256=sha(out/'rust.txt'),
                  discardedSnapshots=0, tolerance=0)
    write(out/'result.json', result)
    print(json.dumps(result, sort_keys=True))
    return 0 if result['passed'] else 1

if __name__ == '__main__':
    try: sys.exit(main())
    except Exception as error:
        print(f'{type(error).__name__}: {error}', file=sys.stderr)
        sys.exit(2)
