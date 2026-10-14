#!/usr/bin/env python3
"""Offline whole-engine modal workloads. Never opens an audio device or window."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='openpatch-modal-benchmark-') as directory:
        tmp = Path(directory)
        (tmp / 'src').mkdir()
        (tmp / 'src/main.rs').write_bytes((HERE / 'measure.rs').read_bytes())
        (tmp / 'Cargo.toml').write_text('[package]\nname="modal-benchmark"\nversion="0.0.0"\nedition="2021"\n[dependencies]\nserde_json="1"\nopenpatch-core={path=' + json.dumps(str(ROOT / 'crates/openpatch-core')) + '}\n[profile.release]\nlto="thin"\ncodegen-units=1\n')
        subprocess.run([os.environ.get('CARGO', 'cargo'), 'build', '--release', '--offline', '--manifest-path', str(tmp / 'Cargo.toml')], check=True)
        result = json.loads(subprocess.check_output([str(tmp / 'target/release/modal-benchmark')], text=True))
    result.update(platform=platform.platform(), machine=platform.machine(),
                  production_sha256=hashlib.sha256((ROOT / 'crates/openpatch-core/src/modal_resonator.rs').read_bytes()).hexdigest(),
                  method='Whole-engine sequential offline throughput; 5 trials per case; compile and first 128 frames excluded; no realtime scheduling, devices, graphics, concurrent clients, graph swaps or callback-tail guarantee.',
                  limitations=['Coherent pitches across voices; not worst-case independent polyphonic modulation.', 'Measures this original model only; no Rack comparison or hardware equivalence.'])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))

if __name__ == '__main__':
    main()
