#!/usr/bin/env python3
"""Fetch and hash-check the original firmware. Never compile or execute it."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_CACHE = ROOT / 'native-test-output/branches-reference/upstream'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fetch(cache):
    manifest = json.loads((HERE / 'source_manifest.json').read_text())
    for name, record in manifest['files'].items():
        destination = cache / name
        if destination.exists() and sha(destination) == record['sha256']:
            continue
        repository = manifest['repositories'][record['repository']]
        url = repository['url'].replace('https://github.com/', 'https://raw.githubusercontent.com/')
        url += '/' + repository['commit'] + '/' + record['path']
        data = subprocess.check_output(['curl', '--fail', '--silent', '--show-error', '--location', url])
        if hashlib.sha256(data).hexdigest() != record['sha256']:
            raise ValueError('pinned firmware hash mismatch: ' + name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    return manifest


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=DEFAULT_CACHE)
    args = parser.parse_args()
    manifest = fetch(args.cache)
    print(json.dumps({'verified_source_files': len(manifest['files']), 'cache': str(args.cache), 'executed': False}))
