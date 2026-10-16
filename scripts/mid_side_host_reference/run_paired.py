#!/usr/bin/env python3
"""Compare a separately built production Core probe with retained genuine SDK data.

Never builds Rust or executes a device/GUI. Every attempt, including a failed
comparison, retains its actual binary, Core sources, inputs and complete traces.
"""
import argparse
import gzip
import json
from pathlib import Path
import shutil
import subprocess
import sys

from run_sdk import FROZEN, HERE, ROOT, digest, stable


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('probe', type=Path)
    parser.add_argument('--sdk-evidence', type=Path, default=ROOT / 'native-test-output/stock-mid-side/host-reference/attempt-001')
    args = parser.parse_args()
    base = ROOT / 'native-test-output/stock-mid-side/paired-host-reference'
    base.mkdir(parents=True, exist_ok=True)
    index = 1
    while (base / f'attempt-{index:03}').exists():
        index += 1
    evidence = base / f'attempt-{index:03}'
    evidence.mkdir()
    report = {'passed': False, 'scope': 'paired production Core and genuine Rack Engine host snapshots',
              'evidence_directory': str(evidence.relative_to(ROOT)), 'discarded_snapshots': 0,
              'limitations': ['Qualifies the frozen 74 host histories, not all possible patches, hardware accuracy, or performance.',
                              'SDK trace is the byte-verified retained first genuine execution; this runner does not re-execute Rack.']}
    try:
        report['stage'] = 'verify frozen declarations and genuine SDK evidence'
        for name, sha in FROZEN.items():
            assert digest(HERE / name) == sha, name
            shutil.copy2(HERE / name, evidence / name)
        sdk = args.sdk_evidence.resolve()
        sdk_report = json.loads((sdk / 'result.json').read_text())
        assert sdk_report['passed'] and not sdk_report['paired_production_equivalence']
        assert sdk_report['frozen_before_compiler_execution'] == FROZEN
        manifest = json.loads((sdk / 'artifact-sha256.json').read_text())
        assert digest(sdk / 'artifact-sha256.json') == sdk_report['artifact_manifest_sha256']
        for name, sha in manifest.items():
            assert digest(sdk / name) == sha, name
        for name in ('inputs.jsonl.gz', 'sdk.jsonl.gz', 'result.json', 'artifact-sha256.json'):
            destination = evidence / 'sdk-evidence' / name
            destination.parent.mkdir(exist_ok=True)
            shutil.copy2(sdk / name, destination)
        report['sdk_evidence_directory'] = str(sdk)
        report['sdk_evidence_report_sha256'] = digest(sdk / 'result.json')
        report['sdk_artifacts_verified'] = len(manifest)
        production = sorted((ROOT / 'crates/openpatch-core/src').rglob('*.rs'))
        production += [ROOT / name for name in ('Cargo.toml', 'Cargo.lock', 'crates/openpatch-core/Cargo.toml',
                                               'crates/openpatch-core/LICENSE-THIRD-PARTY',
                                               'crates/openpatch-core/examples/mid_side_host_reference_probe.rs')]
        sources = {str(path.relative_to(ROOT)): digest(path) for path in production}
        for path in production:
            destination = evidence / 'production-source' / path.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, destination)
        (evidence / 'production-source-sha256.json').write_text(stable(sources))
        probe = evidence / 'production-probe'
        shutil.copy2(args.probe.resolve(), probe)
        report['production_probe_sha256'] = digest(probe)
        report['production_source_files'] = len(sources)
        report['rustc'] = subprocess.check_output([str(Path.home() / '.cargo/bin/rustc'), '--version'], text=True).strip()
        inputs, sdk_trace, actual = [evidence / name for name in ('inputs.jsonl', 'sdk.jsonl', 'production.jsonl')]
        for name, destination in [('inputs.jsonl.gz', inputs), ('sdk.jsonl.gz', sdk_trace)]:
            with gzip.open(evidence / 'sdk-evidence' / name, 'rb') as source, destination.open('wb') as sink:
                shutil.copyfileobj(source, sink)
        assert digest(inputs) == sdk_report['input_sha256']
        assert digest(sdk_trace) == sdk_report['trace_sha256']
        report['stage'] = 'execute actual production Core'
        with inputs.open('rb') as source, actual.open('wb') as sink, (evidence / 'production.stderr').open('wb') as errors:
            subprocess.run([str(probe)], stdin=source, stdout=sink, stderr=errors, check=True)
        report['stage'] = 'unchanged full paired gate'
        command = [sys.executable, str(evidence / 'verify.py'), str(sdk_trace), '--cases', str(evidence / 'cases.json'), '--paired', str(actual)]
        checked = subprocess.run(command, text=True, capture_output=True)
        (evidence / 'validator.stdout').write_text(checked.stdout)
        (evidence / 'validator.stderr').write_text(checked.stderr)
        checked.check_returncode()
        report['validation'] = json.loads(checked.stdout)
        assert all(digest(ROOT / path) == sha for path, sha in sources.items()), 'production sources changed during execution'
        assert all(digest(HERE / name) == sha for name, sha in FROZEN.items()), 'frozen declarations changed'
        report['passed'] = True
        report['stage'] = 'complete'
    except Exception as error:
        report['error'] = {'type': type(error).__name__, 'message': str(error)}
    manifest = {str(path.relative_to(evidence)): digest(path) for path in sorted(evidence.rglob('*')) if path.is_file()}
    (evidence / 'artifact-sha256.json').write_text(stable(manifest))
    report['artifact_manifest_sha256'] = digest(evidence / 'artifact-sha256.json')
    (evidence / 'result.json').write_text(stable(report))
    (ROOT / 'docs/benchmarks/mid-side-paired-host-reference.json').write_text(stable(report))
    print(json.dumps(report, sort_keys=True), flush=True)
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
