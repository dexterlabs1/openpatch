#!/usr/bin/env python3
"""Freeze and run genuine unmodified Frames C++; root runs serially."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
from fetch import DEFAULT_CACHE, HERE, ROOT, fetch


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def protocol(case):
    for op in case['operations']:
        if op[0] == 'D_RANGE':
            for gain in range(op[1], op[2] + 1, op[3]):
                yield f'D {gain} {op[4]}\n'
        else:
            yield ' '.join(map(str, op)) + '\n'


def records(case):
    return sum((op[2] - op[1]) // op[3] + 1 if op[0] == 'D_RANGE' else op[0] in ('A', 'R', 'E', 'D', 'V') for op in case['operations'])


def validate(document):
    assert document['schema'] == 1
    ids = set()
    for case in document['cases'] + document['negative_cases']:
        assert case['id'] not in ids and case['id'].replace('_', '').isalnum()
        ids.add(case['id'])
        for op in case['operations']:
            assert op[0] in ('S', 'I', 'C', 'A', 'R', 'E', 'D', 'V', 'D_RANGE')
            assert len(op) == {'S':4, 'I':3, 'C':1, 'A':6, 'R':2, 'E':3, 'D':3, 'V':4, 'D_RANGE':5}[op[0]]
            assert all(type(value) is int and 0 <= value <= 65535 for value in op[1:])
            if op[0] == 'S': assert op[1] < 4 and op[2] < 6 and op[3] < 256
            if op[0] == 'I': assert op[1] < 4
            if op[0] == 'D': assert op[2] < 256
            if op[0] == 'V': assert op[1] < 4 and op[3] < 2
            if op[0] == 'D_RANGE': assert op[3] > 0 and op[1] <= op[2] and op[4] < 256
    return document['cases'], document['negative_cases']


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache',type=Path,default=DEFAULT_CACHE)
    parser.add_argument('--cases',type=Path,default=HERE/'cases.json')
    parser.add_argument('--validate-only',action='store_true')
    parser.add_argument('--compiler',default='clang++')
    args=parser.parse_args()
    cases, negatives=validate(json.loads(args.cases.read_text()))
    if args.validate_only:
        print(json.dumps({'input_validation_passed':True,'cases':len(cases),'negative_cases':len(negatives),'records':sum(map(records,cases)),'dsp_executed':False})); return 0
    manifest=fetch(args.cache)
    base=ROOT/'native-test-output/frames-reference'; index=1
    while (base/f'attempt-{index:03}').exists(): index+=1
    evidence=base/f'attempt-{index:03}'; evidence.mkdir(parents=True)
    report={'passed':False,'scope':'raw original Frames keyframer TEST branch; no hardware, storage or native adapter claim','stage':'prepare','cases':len(cases),'negative_cases':len(negatives)}
    try:
        for name in ('source_manifest.json','reference_probe.cpp','fetch.py','run.py','README.md','NOTICE.md'):
            shutil.copy2(HERE/name,evidence/name)
        shutil.copy2(args.cases,evidence/'cases.json')
        retained=evidence/'upstream'
        for name in manifest['files']:
            target=retained/name; target.parent.mkdir(parents=True,exist_ok=True); shutil.copy2(args.cache/name,target)
        (evidence/'compiler.txt').write_text(subprocess.check_output([args.compiler,'--version'],text=True,timeout=30))
        executable=evidence/'reference_probe'
        command=[args.compiler,'-std=c++11','-O2','-DTEST','-ffp-contract=off','-fno-fast-math','-I',str(retained),str(evidence/'reference_probe.cpp')]
        if sys.platform=='darwin':
            sdk=Path(subprocess.check_output(['xcrun','--show-sdk-path'],text=True,timeout=30).strip())
            command+=['-isysroot',str(sdk),'-isystem',str(sdk/'usr/include/c++/v1')]
        command += [str(retained/name) for name in manifest['compile_sources']]+['-o',str(executable)]
        (evidence/'compile-command.json').write_text(json.dumps(command,indent=2)+'\n')
        report['stage']='compile'
        build=subprocess.run(command,capture_output=True,text=True,timeout=60)
        (evidence/'compile.stdout').write_text(build.stdout); (evidence/'compile.stderr').write_text(build.stderr); build.check_returncode()
        report['stage']='sanitizer audit compile'
        sanitized=evidence/'reference_probe_sanitized'
        sanitizer_command=command[:-2]+['-fsanitize=undefined,bounds','-fno-sanitize-recover=all','-o',str(sanitized)]
        (evidence/'sanitizer-compile-command.json').write_text(json.dumps(sanitizer_command,indent=2)+'\n')
        audit_build=subprocess.run(sanitizer_command,capture_output=True,text=True,timeout=60)
        (evidence/'sanitizer-compile.stdout').write_text(audit_build.stdout); (evidence/'sanitizer-compile.stderr').write_text(audit_build.stderr); audit_build.check_returncode()
        report['stage']='reference execution and UBSan/bounds audit'; counts={}; rejected={}; audited={}

        for case in negatives+cases:
            target=evidence/case['id']; target.mkdir(); encoded=''.join(protocol(case)); (target/'input.txt').write_text(encoded)
            with (target/'output.jsonl').open('w') as out,(target/'stderr.txt').open('w') as err:
                result=subprocess.run([str(executable)],input=encoded,text=True,stdout=out,stderr=err,timeout=60)
            if case in negatives:
                assert result.returncode==3 and (target/'stderr.txt').read_text()==case['expected_error']+'\n'
                rejected[case['id']]={'guard_rejected':True,'source_undefined_access_executed':False}; continue
            result.check_returncode(); count=0
            for line in (target/'output.jsonl').open():
                item=json.loads(line); assert item['record']==count
                if item['op']=='E': assert len(item['levels'])==len(item['dac_code'])==4 and len(item['color'])==3
                count+=1
            assert count==records(case); counts[case['id']]=count
            with (target/'sanitizer-output.jsonl').open('w') as out,(target/'sanitizer-stderr.txt').open('w') as err:
                audit_result=subprocess.run([str(sanitized)],input=encoded,text=True,stdout=out,stderr=err,timeout=60)
            audit_result.check_returncode()
            assert (target/'sanitizer-stderr.txt').stat().st_size==0, 'sanitizer diagnostics: '+case['id']
            assert sha(target/'sanitizer-output.jsonl')==sha(target/'output.jsonl'), 'sanitizer changed records: '+case['id']
            audited[case['id']]={'passed':True,'records':count,'integer_records_identical':True}

        report.update(passed=True,stage='source traces frozen and source-boundary guards verified',records_by_case=counts,records=sum(counts.values()),rejected_boundaries=rejected,sanitizer_audit=audited,sanitizer_positive_cases_defined=True,production_equivalence=False)
    except Exception as error: report['error']=str(error)
    artifacts={str(p.relative_to(evidence)):sha(p) for p in sorted(evidence.rglob('*')) if p.is_file()}
    (evidence/'artifact-sha256.json').write_text(json.dumps(artifacts,indent=2,sort_keys=True)+'\n')
    (evidence/'result.json').write_text(json.dumps(report,indent=2,sort_keys=True)+'\n')
    print(json.dumps({'passed':report['passed'],'stage':report['stage'],'evidence':str(evidence),'error':report.get('error')}))
    return 0 if report['passed'] else 1
if __name__=='__main__': sys.exit(main())
