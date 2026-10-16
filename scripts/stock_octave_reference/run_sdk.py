#!/usr/bin/env python3
"""Root-only execution runner; refuses an absent/unreviewed external freeze.

Writing this source does not authorize execution. Every attempt is append-only,
including compiler/oracle/validator failures. No production DSP is imported.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
FILES=('acceptance.json','fixtures.py','verify.py','sdk_probe.cpp','README.md','run_sdk.py','run_paired.py')
PINNED={'Octave.cpp':'d0ee41f787180db1c223dc15018619798797e875d86a73e62e53748b0ea835a6',
        'LICENSE-GPLv3.txt':'3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986',
        'system.cpp':'cc10706acb8029b2769618a09a44233be449d73c59b6f1d2d2e4ed248db85c18'}


def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda:source.read(1024*1024),b''):h.update(chunk)
    return h.hexdigest()


def stable(value):return json.dumps(value,indent=2,sort_keys=True)+'\n'


def verify_lock(path,inputs,safety_inputs):
    lock=json.loads(Path(path).read_text())
    if lock.get('reviewed_before_first_execution') is not True:raise ValueError('independent Root review/freeze required')
    if set(lock['files'])!=set(FILES):raise ValueError('lock must include every acceptance/adapter/runner file')
    for name in FILES:
        if digest(HERE/name)!=lock['files'][name]:raise ValueError('frozen file changed: '+name)
    if digest(inputs)!=lock['inputs_sha256']:raise ValueError('frozen inputs changed')
    if digest(safety_inputs)!=lock['safety_inputs_sha256']:raise ValueError('frozen separate safety records changed')
    if not lock.get('fixture_validation',{}).get('passed'):raise ValueError('successful retained input-only preflight required')
    return lock


def attempt(base):
    base.mkdir(parents=True,exist_ok=True);index=1
    while (base/f'attempt-{index:03}').exists():index+=1
    target=base/f'attempt-{index:03}';target.mkdir();return target


def finish(evidence,report):
    artifacts={str(p.relative_to(evidence)):digest(p) for p in sorted(evidence.rglob('*'))
               if p.is_file() and p not in (evidence/'result.json',evidence/'artifact-sha256.json')}
    (evidence/'artifact-sha256.json').write_text(stable(artifacts))
    report['artifact_manifest_sha256']=digest(evidence/'artifact-sha256.json')
    (evidence/'result.json').write_text(stable(report))
    print(json.dumps(report,sort_keys=True),flush=True)
    return 0 if report['passed'] else 1


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inputs',type=Path,required=True)
    parser.add_argument('--safety-inputs',type=Path,required=True)
    parser.add_argument('--lock',type=Path,required=True)
    parser.add_argument('--sdk',type=Path,default=Path.home()/'Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK')
    args=parser.parse_args();evidence=attempt(ROOT/'native-test-output/stock-octave/sdk-reference')
    report={'passed':False,'stage':'verify freeze','scope':'genuine SDK hand anchors only; no production equivalence claim',
            'evidence_directory':str(evidence.relative_to(ROOT)),'paired_production_equivalence':False}
    try:
        lock=verify_lock(args.lock,args.inputs,args.safety_inputs);report['frozen_before_execution']=lock
        for name in FILES:shutil.copy2(HERE/name,evidence/name)
        shutil.copy2(args.lock,evidence/'declaration-lock.json');shutil.copy2(args.inputs,evidence/'inputs.jsonl')
        shutil.copy2(args.safety_inputs,evidence/'safety-inputs.jsonl')
        fixture_check=subprocess.run([sys.executable,str(evidence/'verify.py'),'--inputs',str(evidence/'inputs.jsonl'),'--safety-inputs',str(evidence/'safety-inputs.jsonl'),'--fixtures-only'],text=True,capture_output=True)
        (evidence/'fixture-check.stdout').write_text(fixture_check.stdout);(evidence/'fixture-check.stderr').write_text(fixture_check.stderr);fixture_check.check_returncode()
        report['fixture_validation']=json.loads(fixture_check.stdout)
        if report['fixture_validation']!=lock['fixture_validation']:raise ValueError('input-only preflight differs from reviewed lock')
        reference=ROOT/'native-test-output/stock-octave/reference-research';source_hashes={}
        for name in ('Octave.cpp','Engine.cpp','Module.cpp','system.cpp','Rack-LICENSE.md','LICENSE-GPLv3.txt','plugin.json'):
            source_hashes[name]=digest(reference/name);shutil.copy2(reference/name,evidence/name)
        for name,expected in PINNED.items():
            if source_hashes[name]!=expected:raise ValueError('primary source hash: '+name)
        report['source_sha256']=source_hashes
        attestation=ROOT/'docs/benchmarks/output-lifecycle-reference.json';prior=json.loads(attestation.read_text())
        if not prior['passed'] or 'SDK=2.6.6\n' not in prior['sdk_metadata']:raise ValueError('genuine SDK 2.6.6 attestation required')
        sdk=args.sdk.resolve();library=sdk/'libRack.dylib'
        if digest(library)!=prior['sdk_library_sha256']:raise ValueError('SDK library differs from attestation')
        headers={}
        for directory in ('include','dep/include'):
            shutil.copytree(sdk/directory,evidence/'sdk'/directory)
            for path in sorted((sdk/directory).rglob('*')):
                if path.is_file():headers[str(path.relative_to(sdk))]=digest(path)
        if headers!=prior['sdk_header_sha256']:raise ValueError('SDK headers differ from attestation')
        shutil.copy2(library,evidence/library.name);shutil.copy2(attestation,evidence/'sdk-attestation.json')
        upstream=(evidence/'Octave.cpp').read_text()
        if upstream.count('struct OctaveButton :')!=1 or upstream.count('#include "plugin.hpp"')!=1:raise ValueError('unexpected source boundary')
        prefix=upstream.split('struct OctaveButton :',1)[0]
        (evidence/'reference_module.hpp').write_text(prefix.replace('#include "plugin.hpp"','#include <rack.hpp>\nusing namespace rack;'))
        report.update(sdk_version='2.6.6',sdk_library_sha256=digest(library),sdk_header_sha256=headers,
                      sdk_version_attestation_sha256=digest(attestation),compiled_prefix_sha256=digest(evidence/'reference_module.hpp'))
        report['stage']='compile genuine SDK adapter'
        sysroot=Path(subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip())
        binary=evidence/'sdk-probe'
        command=['c++','-std=c++17','-O2','-ffp-contract=off','-isysroot',str(sysroot),
                 '-isystem',str(sysroot/'usr/include/c++/v1'),'-I'+str(evidence/'sdk/include'),
                 '-I'+str(evidence/'sdk/dep/include'),'-I'+str(evidence),str(evidence/'sdk_probe.cpp'),
                 '-L'+str(evidence),'-lRack','-Wl,-rpath,'+str(evidence),'-o',str(binary)]
        (evidence/'compiler-command.json').write_text(stable(command));report['compiler_command']=command
        built=subprocess.run(command,text=True,capture_output=True)
        (evidence/'build.stdout').write_text(built.stdout);(evidence/'build.stderr').write_text(built.stderr);built.check_returncode()
        report['cpp_toolchain']=subprocess.check_output(['c++','--version'],text=True)
        report['probe_binary_sha256']=digest(binary);report['stage']='execute genuine Engine'
        env=os.environ.copy();env['DYLD_LIBRARY_PATH']=str(evidence)
        with (evidence/'inputs.jsonl').open('rb') as source,(evidence/'sdk.jsonl').open('wb') as sink,(evidence/'sdk.stderr').open('wb') as errors:
            subprocess.run([str(binary)],stdin=source,stdout=sink,stderr=errors,env=env,check=True)
        report['stage']='unchanged hand-anchor/whole-shape gate'
        command=[sys.executable,str(evidence/'verify.py'),'--inputs',str(evidence/'inputs.jsonl'),'--safety-inputs',str(evidence/'safety-inputs.jsonl'),'--sdk-trace',str(evidence/'sdk.jsonl')]
        checked=subprocess.run(command,text=True,capture_output=True)
        (evidence/'validator.stdout').write_text(checked.stdout);(evidence/'validator.stderr').write_text(checked.stderr);checked.check_returncode()
        report['validation']=json.loads(checked.stdout);verify_lock(args.lock,args.inputs,args.safety_inputs)
        for name,expected in lock['files'].items():
            if digest(evidence/name)!=expected:raise ValueError('retained frozen file changed: '+name)
        if digest(evidence/'inputs.jsonl')!=lock['inputs_sha256']:raise ValueError('retained inputs changed')
        if digest(evidence/'safety-inputs.jsonl')!=lock['safety_inputs_sha256']:raise ValueError('retained safety records changed')
        if digest(library)!=report['sdk_library_sha256']:raise ValueError('SDK library changed during run')
        report.update(passed=True,stage='complete',input_sha256=digest(evidence/'inputs.jsonl'),trace_sha256=digest(evidence/'sdk.jsonl'))
    except Exception as error:report['error']={'type':type(error).__name__,'message':str(error)}
    return finish(evidence,report)


if __name__=='__main__':sys.exit(main())
