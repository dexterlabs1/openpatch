#!/usr/bin/env python3
"""Compare original Grids C++ firmware, sanitizer and production Rust exactly."""
import argparse
import difflib
import hashlib
import json
import re
from pathlib import Path
import shutil
import subprocess
import sys
from fetch import DEFAULT_CACHE, HERE, ROOT, fetch, sha

def validate(document):
    assert document['schema']==1
    ids=set()
    domains={0:(7,[255]*7),1:(1,[6]),2:(0,[]),3:(0,[]),4:(0,[]),5:(0,[]),
             10:(2,[511,2]),11:(0,[]),12:(1,[255]),13:(0,[]),14:(0,[]),15:(0,[]),20:(2,[255,255]),30:(0,[]),40:(2,[1,1])}
    for c in document['cases']:
        assert c['id'] not in ids and c['id'].replace('_','').isalnum();ids.add(c['id'])
        assert 0<=c['seed']<=0xffffffff and c['operations']
        for row in c['operations']:
            assert all(type(v) is int for v in row)
            assert row[0] in domains and 1<=row[1]<=100000
            size,maximum=domains[row[0]]
            assert len(row)==size+2
            assert all(0<=v<=bound for v,bound in zip(row[2:],maximum))
            if row[0]==0: assert (row[2]&7)<=3, 'source enum-undefined EEPROM byte'
            if row[0]==1: assert row[2] in (1,3,6)
    assert document['negative_source_cases']==['--undefined-clock','--undefined-map','--undefined-options-4','--undefined-options-7']
    return document['cases']

def invoke(command,directory,name,**kwargs):
    (directory/(name+'-command.json')).write_text(json.dumps(command,indent=2)+'\n')
    result=subprocess.run(command,capture_output=True,text=True,timeout=180,**kwargs)
    (directory/(name+'.stdout')).write_text(result.stdout)
    (directory/(name+'.stderr')).write_text(result.stderr)
    result.check_returncode();return result

def compare(left,right,label):
    if left==right:return
    for number,(a,b) in enumerate(zip(left.splitlines(),right.splitlines())):
        if a!=b:raise AssertionError(f'{label} record {number}: {a!r} != {b!r}')
    raise AssertionError(label+' trace length mismatch')

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache',type=Path,default=DEFAULT_CACHE)
    parser.add_argument('--cases',type=Path,default=HERE/'cases.json')
    parser.add_argument('--compiler',default='clang++');parser.add_argument('--rustc',default='rustc')
    parser.add_argument('--validate-only',action='store_true');args=parser.parse_args()
    document=json.loads(args.cases.read_text());cases=validate(document)
    operations=sum(row[1] for c in cases for row in c['operations'])
    if args.validate_only:
        print(json.dumps({'validated':True,'cases':len(cases),'operations':operations,'dsp_executed':False}));return 0
    manifest=fetch(args.cache)
    base=ROOT/'native-test-output/grids-reference';index=1
    while (base/f'attempt-{index:03}').exists():index+=1
    evidence=base/f'attempt-{index:03}';evidence.mkdir(parents=True)
    report={'passed':False,'cases':len(cases),'operations':operations,'stage':'prepare',
            'scope':'raw Grids firmware/clock and extracted clock-input ISR helper; excludes ADC/button/EEPROM scheduling, hardware and audio host',
            'production_equivalence':False}
    try:
        for name in ('reference_probe.cpp','rust_probe.rs','run.py','fetch.py','generate_resources.py','make_cases.py','source_manifest.json','NOTICE.md','README.md'):
            shutil.copy2(HERE/name,evidence/name)
        shutil.copy2(args.cases,evidence/'cases.json');shutil.copytree(HERE/'stubs',evidence/'stubs')
        retained=evidence/'upstream'
        for name in manifest['files']:
            destination=retained/name;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(args.cache/name,destination)
        # Select original portable arithmetic. Its unrelated U24Sub fallback is
        # broken upstream (uses undeclared `sum` after declaring `difference`).
        # Exclude exactly that unused function: any later call fails compilation.
        overlay=evidence/'host-overlay/avrlib';overlay.mkdir(parents=True)
        original=(retained/'avrlib/op.h').read_text()
        assert original.count('#define USE_OPTIMIZED_OP')==1
        modified=original.replace('#define USE_OPTIMIZED_OP','// Host qualification: select original portable fallback.')
        marker='#else\n\nstatic inline uint24c_t U24AddC'
        assert modified.count(marker)==1
        fallback_at=modified.index(marker)
        unused=re.search(r'^static inline uint24_t U24Sub\(uint24_t a, uint24_t b\) \{.*?^\}',modified[fallback_at:],re.S|re.M)
        assert unused
        unused_text=unused.group(0)
        assert 'uint32_t difference = av - bv;' in unused_text
        assert 'result.integral = sum >> 8;' in unused_text and 'result.fractional = sum & 0xff;' in unused_text
        begin=fallback_at+unused.start();end=fallback_at+unused.end()
        modified=modified[:begin]+'#if 0  // Host-only: unused upstream U24Sub has an undeclared sum.\n'+unused_text+'\n#endif'+modified[end:]
        # Every compiled production input (and full ISR source) is audited for
        # calls. The sole U24Sub matches in retained op.h are its two definitions.
        audited=['grids/pattern_generator.h','grids/pattern_generator.cc','grids/clock.h','grids/clock.cc',
                 'grids/resources.h','grids/resources.cc','grids/grids.cc','avrlib/random.h','avrlib/random.cc',
                 'avrlib/base.h','avrlib/resources_manager.h']
        for name in audited:assert not re.search(r'\bU24Sub\s*\(', (retained/name).read_text()), 'excluded helper used by '+name
        assert not re.search(r'\bU24Sub\s*\(', (evidence/'reference_probe.cpp').read_text())
        arithmetic_hashes={}
        for name,count in [('U8Mix',2),('U8U8MulShift8',1),('U8U8Mul',1)]:
            expression=r'^static inline \w+ '+name+r'\([^}]*?^\}'
            before=re.findall(expression,original[original.index(marker):],re.M)
            after=re.findall(expression,modified[modified.index(marker):],re.M)
            assert len(before)==count and before==after, 'used arithmetic changed: '+name
            arithmetic_hashes[name]=[hashlib.sha256(block.encode()).hexdigest() for block in before]
        (overlay/'op.h').write_text(modified)
        (evidence/'host-overlay.patch').write_text(''.join(difflib.unified_diff(original.splitlines(keepends=True),modified.splitlines(keepends=True),fromfile='upstream/avrlib/op.h',tofile='host-overlay/avrlib/op.h')))
        (evidence/'host-overlay-audit.json').write_text(json.dumps({
            'original_sha256':sha(retained/'avrlib/op.h'),'overlay_sha256':sha(overlay/'op.h'),
            'changes':['disable USE_OPTIMIZED_OP to select original portable arithmetic',
                       'exclude only unused broken portable U24Sub definition'],
            'excluded_helper_call_matches':0,'audited_upstream_files':audited,
            'used_arithmetic_unchanged':arithmetic_hashes,
            'excluded_helper_reason':'original portable U24Sub declares difference but reads undeclared sum; never used by Grids or probe'
        },indent=2)+'\n')
        firmware=(retained/'grids/grids.cc').read_text()
        function=re.search(r'inline void HandleClockResetInputs\(\) \{.*?^\}',firmware,re.S|re.M)
        granularity=re.search(r'^uint8_t ticks_granularity\[\].*?;',firmware,re.M)
        assert function and granularity
        extracted=granularity.group(0)+'\n\n'+function.group(0)+'\n'
        (evidence/'host-overlay/original_clock_inputs.h').write_text(extracted)
        production=ROOT/'crates/openpatch-core/src/rhythm_terrain'
        for name in ('kernel.rs','resources.rs'):shutil.copy2(production/name,evidence/('production-'+name))
        (evidence/'compiler.txt').write_text(subprocess.check_output([args.compiler,'--version'],text=True))
        (evidence/'rustc.txt').write_text(subprocess.check_output([args.rustc,'--version','--verbose'],text=True))
        command=[args.compiler,'-std=c++17','-O2','-Wno-shift-op-parentheses','-Wno-int-to-pointer-cast',
                 '-I',str(evidence/'host-overlay'),'-I',str(evidence/'stubs'),'-I',str(retained),str(evidence/'reference_probe.cpp')]
        if sys.platform=='darwin':
            sdk=Path(subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip())
            command+=['-isysroot',str(sdk),'-isystem',str(sdk/'usr/include/c++/v1')]
        source=evidence/'reference_probe';sanitized=evidence/'reference_probe_sanitized';rust=evidence/'rust_probe'
        report['stage']='compile and retain preprocessed original source'
        invoke(command+['-o',str(source)],evidence,'compile')
        invoke(command+['-fsanitize=address,undefined,bounds','-fno-sanitize-recover=all','-o',str(sanitized)],evidence,'sanitizer-compile')
        invoke(command+['-E','-o',str(evidence/'reference_probe.ii')],evidence,'preprocess')
        invoke([args.rustc,'--edition=2021','-O',str(HERE/'rust_probe.rs'),'-o',str(rust)],evidence,'rust-compile')
        report['stage']='exact source/sanitizer/Rust comparison';counts={};table_hashes={}
        for case in cases:
            target=evidence/case['id'];target.mkdir()
            encoded=str(case['seed'])+'\n'+''.join(' '.join(map(str,row))+'\n' for row in case['operations'])
            (target/'input.txt').write_text(encoded)
            for name,executable in [('source',source),('sanitizer',sanitized),('rust',rust)]:
                invoke([str(executable)],target,name,input=encoded)
                assert not (target/(name+'.stderr')).stat().st_size
            actual=(target/'source.stdout').read_bytes()
            compare(actual,(target/'sanitizer.stdout').read_bytes(),case['id']+' sanitizer')
            compare(actual,(target/'rust.stdout').read_bytes(),case['id']+' Rust')
            rows=[json.loads(line) for line in actual.splitlines()]
            scans=[row for row in rows if isinstance(row[0],int)]
            assert len(scans)==sum(row[1] for row in case['operations'])
            assert all(len(row)==23 and row[0]==i for i,row in enumerate(scans))
            counts[case['id']]={'operations':len(scans),'total_records':len(rows)}
            if case['id']=='resources_and_map':
                for tag,size,width in [('e',1024,4),('t',512,4),('n',2400,1)]:
                    values=[row[-1] for row in rows if row[0]==tag];assert len(values)==size
                    table_hashes[tag]=hashlib.sha256(b''.join(v.to_bytes(width,'little') for v in values)).hexdigest()
        assert table_hashes == manifest['table_little_endian_sha256'], 'pinned resource hash mismatch'
        # Explicit source-undefined boundaries MUST fail the original sanitizer.
        negative={}
        for name in document['negative_source_cases']:
            target=evidence/name.removeprefix('--');target.mkdir()
            command=[str(sanitized),name]
            (target/'command.json').write_text(json.dumps(command)+'\n')
            result=subprocess.run(command,capture_output=True,text=True,timeout=30)
            (target/'stdout').write_text(result.stdout);(target/'stderr').write_text(result.stderr)
            assert result.returncode!=0 and ('Sanitizer' in result.stderr or 'runtime error' in result.stderr),name+' did not expose original undefined boundary'
            negative[name]={'returncode':result.returncode,'sanitizer_rejected':True}
        for name in ('kernel.rs','resources.rs'):
            assert sha(production/name)==sha(evidence/('production-'+name)),'production changed during qualification'
        report.update(passed=True,stage='exact original-source qualification passed',records_by_case=counts,
                      source_sanitizer_passed=True,raw_kernel_equivalence=True,table_little_endian_sha256=table_hashes,negative_source_cases=negative)
    except Exception as error:report['error']=str(error)
    (evidence/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    artifacts={str(p.relative_to(evidence)):sha(p) for p in sorted(evidence.rglob('*')) if p.is_file()}
    (evidence/'artifact-sha256.json').write_text(json.dumps(artifacts,indent=2,sort_keys=True)+'\n')
    print(json.dumps({'passed':report['passed'],'stage':report['stage'],'operations':operations,'evidence':str(evidence),'error':report.get('error')}))
    return 0 if report['passed'] else 1
if __name__=='__main__':sys.exit(main())
