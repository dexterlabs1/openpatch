#!/usr/bin/env python3
"""Build the declared ideal VCA inverse from an immutable source DAC sweep.

This is a native modeling convention, not measured hardware calibration.
"""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    reference = args.reference.resolve()
    report = json.loads((reference / 'result.json').read_text())
    assert report['passed'] and report['sanitizer_audit']
    trace = reference / 'dac_full_gain_response_0/output.jsonl'
    digest = hashlib.sha256(trace.read_bytes()).hexdigest()
    assert json.loads((reference / 'artifact-sha256.json').read_text())['dac_full_gain_response_0/output.jsonl'] == digest
    records = [json.loads(line) for line in trace.open()]
    assert len(records) == 65536
    assert all(r['op'] == 'D' and r['gain'] == i and r['response'] == 0 for i, r in enumerate(records))
    codes = [r['dac_code'] for r in records]
    assert all(0 <= c <= 4095 for c in codes)
    assert all(a >= b for a, b in zip(codes, codes[1:]))
    gains = []
    for code in range(4096):
        if code >= codes[0]:
            gains.append(0)
        elif code <= codes[-1]:
            gains.append(65535)
        else:
            lo, hi = 0, 65535
            while lo < hi:
                mid = (lo + hi) // 2
                if codes[mid] <= code:
                    hi = mid
                else:
                    lo = mid + 1
            gains.append(lo)
    assert all(a >= b for a, b in zip(gains, gains[1:]))
    output = ROOT / 'crates/openpatch-core/src/frame_scene/native_gain.rs'
    header = f'''// Derived from the MIT Frames compensation table, Copyright 2013 Emilie Gillet.
// Original native inverse convention Copyright 2026 OpenPatch contributors.
// Complete permission notice: scripts/frames_reference/NOTICE.md.
// Source DAC sweep SHA256: {digest}
// H(d): first normalized gain g with sourceDAC(g,0)<=d; saturation endpoints
// H(d>=sourceDAC(0,0))=0 and H(d<=sourceDAC(65535,0))=1. Quantized to u16.
// This ideal, monotone transfer is NOT measured/calibrated analog hardware.
pub(super) const IDEAL_GAIN: [u16; 4096] = [
'''
    rows = ['    ' + ', '.join(map(str, gains[i:i+16])) + ',' for i in range(0, 4096, 16)]
    output.write_text(header + '\n'.join(rows) + '\n];\n')
    print(json.dumps({'entries': len(gains), 'trace_sha256': digest, 'output': str(output)}))


if __name__ == '__main__':
    main()
