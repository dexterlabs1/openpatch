#!/usr/bin/env python3
"""Root-only actual-production probe comparison; no compiler or DSP mirror.

Accepts a separately built probe and explicit build attestation. Retains exact
inputs, both entire traces, actual binary, source closure and every failure.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
from run_sdk import FILES,HERE,ROOT,attempt,digest,finish,stable,verify_lock


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe',type=Path,required=True)
    parser.add_argument('--build-manifest',type=Path,required=True)
    parser.add_argument('--sdk-evidence',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True)
    parser.add_argument('--probe-source',default='crates/openpatch-core/examples/stock_scope_host_reference_probe.rs')
    args=parser.parse_args();evidence=attempt(ROOT/'native-test-output/stock-scope/paired-reference')
    report={'passed':False,'stage':'verify frozen evidence','scope':'whole actual Core/genuine Rack Engine snapshots',
            'evidence_directory':str(evidence.relative_to(ROOT)),'discarded_snapshots':0}
    try:
        sdk=args.sdk_evidence.resolve();prior=json.loads((sdk/'result.json').read_text())
        if not prior['passed'] or prior['paired_production_equivalence']:raise ValueError('successful genuine SDK-only evidence required')
        artifacts=json.loads((sdk/'artifact-sha256.json').read_text())
        if digest(sdk/'artifact-sha256.json')!=prior['artifact_manifest_sha256']:raise ValueError('SDK manifest changed')
        for name,expected in artifacts.items():
            if digest(sdk/name)!=expected:raise ValueError('SDK artifact changed: '+name)
        lock=verify_lock(args.lock,sdk/'inputs.jsonl',sdk/'safety-inputs.jsonl')
        if lock!=prior['frozen_before_execution']:raise ValueError('declaration differs from genuine execution')
        for name in FILES:shutil.copy2(HERE/name,evidence/name)
        shutil.copy2(args.lock,evidence/'declaration-lock.json')
        for name in ('inputs.jsonl','safety-inputs.jsonl','sdk.jsonl','result.json','artifact-sha256.json'):
            target=evidence/'sdk-evidence'/name;target.parent.mkdir(exist_ok=True);shutil.copy2(sdk/name,target)
        report.update(sdk_report_sha256=digest(sdk/'result.json'),sdk_artifacts_verified=len(artifacts),frozen_before_execution=lock)
        build=json.loads(args.build_manifest.read_text())
        if build.get('profile')!='release' or not build.get('build_command') or not build.get('toolchain'):raise ValueError('explicit release build provenance required')
        if digest(args.probe)!=build['binary_sha256']:raise ValueError('probe differs from recorded build')
        expected_files={str(p.relative_to(ROOT)) for p in (ROOT/'crates/openpatch-core/src').rglob('*.rs')}
        expected_files.update(('Cargo.toml','Cargo.lock','crates/openpatch-core/Cargo.toml',args.probe_source))
        if not expected_files.issubset(build['source_sha256']):raise ValueError('incomplete production source closure')
        for name,expected in build['source_sha256'].items():
            path=ROOT/name
            if path.resolve().is_relative_to(ROOT) is False:raise ValueError('source path escapes repository')
            if digest(path)!=expected:raise ValueError('recorded build source changed: '+name)
            target=evidence/'production-source'/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(path,target)
        shutil.copy2(args.build_manifest,evidence/'production-build-manifest.json')
        binary=evidence/'production-probe';shutil.copy2(args.probe,binary)
        report.update(production_probe_sha256=digest(binary),production_build_manifest_sha256=digest(args.build_manifest),
                      source_provenance='Caller-recorded separate build; source closure and actual binary verified against its manifest')
        report['stage']='execute actual Core host graph'
        with (evidence/'sdk-evidence/inputs.jsonl').open('rb') as source,(evidence/'production.jsonl').open('wb') as sink,(evidence/'production.stderr').open('wb') as errors:
            subprocess.run([str(binary)],stdin=source,stdout=sink,stderr=errors,check=True)
        report['stage']='unchanged full-field gate'
        command=[sys.executable,str(evidence/'verify.py'),'--inputs',str(evidence/'sdk-evidence/inputs.jsonl'),
                 '--safety-inputs',str(evidence/'sdk-evidence/safety-inputs.jsonl'),
                 '--sdk-trace',str(evidence/'sdk-evidence/sdk.jsonl'),'--paired',str(evidence/'production.jsonl')]
        checked=subprocess.run(command,text=True,capture_output=True)
        (evidence/'validator.stdout').write_text(checked.stdout);(evidence/'validator.stderr').write_text(checked.stderr);checked.check_returncode()
        report['validation']=json.loads(checked.stdout)
        verify_lock(args.lock,sdk/'inputs.jsonl',sdk/'safety-inputs.jsonl')
        for name,expected in build['source_sha256'].items():
            if digest(ROOT/name)!=expected:raise ValueError('production source changed during comparison: '+name)
        report.update(passed=True,stage='complete')
    except Exception as error:report['error']={'type':type(error).__name__,'message':str(error)}
    return finish(evidence,report)


if __name__=='__main__':sys.exit(main())
