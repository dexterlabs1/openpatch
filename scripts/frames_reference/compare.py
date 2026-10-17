#!/usr/bin/env python3
"""Compare the actual native kernel with immutable C++ integer records."""
import argparse
import hashlib
import itertools
import json
from pathlib import Path
import shutil
import subprocess
import sys
HERE=Path(__file__).resolve().parent
ROOT=HERE.parent.parent

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    parser=argparse.ArgumentParser(description=__doc__); parser.add_argument('--reference',type=Path,required=True); args=parser.parse_args()
    reference=args.reference.resolve(); assert json.loads((reference/'result.json').read_text())['passed']
    for name,expected in json.loads((reference/'artifact-sha256.json').read_text()).items(): assert sha(reference/name)==expected, f'changed reference: {name}'
    cases=json.loads((reference/'cases.json').read_text())['cases']
    base=ROOT/'native-test-output/frames-reference'; index=1
    while (base/f'rust-comparison-{index:03}').exists(): index+=1
    evidence=base/f'rust-comparison-{index:03}'; evidence.mkdir(parents=True)
    sources=[HERE/'rust_probe.rs',HERE/'compare.py',ROOT/'crates/openpatch-core/src/frame_scene/kernel.rs',ROOT/'crates/openpatch-core/src/frame_scene/tables.rs']
    hashes={}
    for source in sources:
        relative=source.relative_to(ROOT); target=evidence/'source'/relative; target.parent.mkdir(parents=True,exist_ok=True); shutil.copy2(source,target); hashes[str(relative)]=sha(source)
    (evidence/'source-sha256.json').write_text(json.dumps(hashes,indent=2)+'\n')
    (evidence/'rustc.txt').write_text(subprocess.check_output(['rustc','-Vv'],text=True,timeout=30))
    executable=evidence/'rust_probe'; command=['rustc','--edition=2021','-O',str(evidence/'source/scripts/frames_reference/rust_probe.rs'),'-o',str(executable)]
    (evidence/'compile-command.json').write_text(json.dumps(command,indent=2)+'\n')
    report={'passed':False,'reference':str(reference),'reference_manifest_sha256':sha(reference/'artifact-sha256.json'),'scope':'raw keyframer only; no undefined source boundary, native adapter, storage, hardware claim','cases':{}}
    try:
        result=subprocess.run(command,capture_output=True,text=True,timeout=60); (evidence/'compile.stdout').write_text(result.stdout); (evidence/'compile.stderr').write_text(result.stderr); result.check_returncode()
        for case in cases:
            name=case['id']; target=evidence/name; target.mkdir()
            with (target/'output.jsonl').open('w') as out,(target/'stderr.txt').open('w') as err:
                result=subprocess.run([str(executable)],input=(reference/name/'input.txt').read_text(),text=True,stdout=out,stderr=err,timeout=60)
            result.check_returncode(); mismatches=0; count=0
            with (reference/name/'output.jsonl').open() as expected,(target/'output.jsonl').open() as actual:
                for count,(a,b) in enumerate(itertools.zip_longest(expected,actual),1):
                    a=json.loads(a) if a else None; b=json.loads(b) if b else None
                    if a!=b:
                        mismatches+=1
                        if mismatches==1: (target/'first-mismatch.json').write_text(json.dumps({'record':count-1,'expected':a,'actual':b},indent=2)+'\n')
            report['cases'][name]={'passed':mismatches==0,'records':count,'mismatched_records':mismatches}
        report['passed']=all(case['passed'] for case in report['cases'].values())
    except Exception as error: report['error']=str(error)
    report['compared_records']=sum(case.get('records',0) for case in report['cases'].values())
    (evidence/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    (evidence/'artifact-sha256.json').write_text(json.dumps({str(p.relative_to(evidence)):sha(p) for p in sorted(evidence.rglob('*')) if p.is_file()},indent=2,sort_keys=True)+'\n')
    print(json.dumps({'passed':report['passed'],'cases_passed':sum(c['passed'] for c in report['cases'].values()),'records':report['compared_records'],'evidence':str(evidence),'error':report.get('error')}))
    return 0 if report['passed'] else 1
if __name__=='__main__': sys.exit(main())
