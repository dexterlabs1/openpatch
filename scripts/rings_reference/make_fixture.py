#!/usr/bin/env python3
"""Freeze the host-golden C++ traces into the native engine test fixture.

Copies, without modification, the int16 exciter inputs and odd/even outputs
of the host_golden_* cases from an immutable reference run.
"""
import argparse
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TARGET = ROOT / 'crates/openpatch-core/tests/fixtures/string-field-host-golden.json'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    args = parser.parse_args()
    reference = args.reference.resolve()
    assert json.loads((reference / 'result.json').read_text())['passed']
    manifest = json.loads((reference / 'artifact-sha256.json').read_text())
    for name, expected in manifest.items():
        assert sha(reference / name) == expected, f'changed reference: {name}'
    fixture = {'source': 'scripts/rings_reference C++ oracle', 'reference': reference.name,
               'reference_manifest_sha256': sha(reference / 'artifact-sha256.json'), 'cases': {}}
    for name in ('host_golden_modal', 'host_golden_string'):
        words = (reference / name / 'input.txt').read_text().split()
        blocks, size = int(words[0]), int(words[1])
        position = 3  # blocks, size, seed
        inputs = []
        for _ in range(blocks):
            position += 14  # seven discrete and seven continuous controls
            inputs += [int(value) for value in words[position:position + size]]
            position += size
        outputs = []
        for line in (reference / name / 'output.txt').read_text().splitlines():
            outputs += [int(value) for value in line.split()[:-1]]  # drop the strum flag
        assert 2 * len(inputs) == len(outputs) == 2 * blocks * size
        fixture['cases'][name] = {'input_sha256': manifest[f'{name}/input.txt'], 'output_sha256': manifest[f'{name}/output.txt'],
                                  'inputs': inputs, 'outputs': outputs}
    TARGET.write_text(json.dumps(fixture, separators=(',', ':')) + '\n')
    print(json.dumps({'fixture': str(TARGET.relative_to(ROOT)), 'cases': len(fixture['cases']), 'bytes': TARGET.stat().st_size}))


if __name__ == '__main__':
    main()
