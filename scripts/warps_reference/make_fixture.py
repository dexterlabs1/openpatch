#!/usr/bin/env python3
"""Freeze the host-golden C++ traces into the native engine test fixture.

Copies, without modification, the int16 inputs and outputs of the
host_golden_* cases from an immutable reference run.
"""
import argparse
import hashlib
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
TARGET = ROOT / 'crates/openpatch-core/tests/fixtures/cross-matter-host-golden.json'


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
    fixture = {'source': 'scripts/warps_reference C++ oracle', 'reference': reference.name,
               'reference_manifest_sha256': sha(reference / 'artifact-sha256.json'), 'cases': {}}
    for name in ('host_golden_external', 'host_golden_internal'):
        words = (reference / name / 'input.txt').read_text().split()
        blocks, size = int(words[0]), int(words[1])
        position = 2
        inputs = []
        for _ in range(blocks):
            position += 10  # mode, carrier and eight controls
            inputs += [int(value) for value in words[position:position + 2 * size]]
            position += 2 * size
        outputs = [int(value) for value in (reference / name / 'output.txt').read_text().split()]
        assert len(inputs) == len(outputs) == 2 * blocks * size
        fixture['cases'][name] = {'input_sha256': manifest[f'{name}/input.txt'], 'output_sha256': manifest[f'{name}/output.txt'],
                                  'inputs': inputs, 'outputs': outputs}
    TARGET.write_text(json.dumps(fixture, separators=(',', ':')) + '\n')
    print(json.dumps({'fixture': str(TARGET.relative_to(ROOT)), 'cases': len(fixture['cases'])}))


if __name__ == '__main__':
    main()
