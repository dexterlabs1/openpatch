#!/usr/bin/env python3
"""Fetch only hash-pinned, unmodified primary files; never compile or run DSP."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_CACHE = ROOT / 'native-test-output/tides2-reference/upstream'


def fetch(cache):
    manifest = json.loads((HERE / 'source_manifest.json').read_text())
    for target, record in manifest['files'].items():
        destination = cache / target
        if destination.exists() and hashlib.sha256(destination.read_bytes()).hexdigest() == record['sha256']:
            continue
        repository = manifest['repositories'][record['repository']]
        url = repository['url'].replace('https://github.com/', 'https://raw.githubusercontent.com/')
        url += '/' + repository['commit'] + '/' + record['path']
        data = subprocess.check_output(['curl', '--fail', '--silent', '--show-error', '--location', url])
        if hashlib.sha256(data).hexdigest() != record['sha256']:
            raise ValueError('primary source hash mismatch: ' + target)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=DEFAULT_CACHE)
    args = parser.parse_args()
    manifest = fetch(args.cache)
    print(json.dumps({'fetched_verified_files': len(manifest['files']), 'cache': str(args.cache), 'compiled': False}))
