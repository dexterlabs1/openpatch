#!/usr/bin/env python3
"""Bounded exact comparison of untouched RackMCP SequencerModule and public native Engine.
No network, window, audio device, copied DSP, fake engine registry or tolerance.
"""
import argparse
import copy
import hashlib
import itertools
import json
import os
from pathlib import Path
import shutil
import subprocess

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[1]
PIN="285e84c5ff2d3ecef25890da07f578af6a0c5ca6"
CASES={"poly_internal":7000,"selected_nonzero_first_advance":7000,
       "schmitt_startup_disconnect_reset":800,"stopped_divider_reset_button":512,
       "raw_clamp":256,"manual_cache_equal_reload":512,"changed_data":8000,
       "missing_initial_return":512,"written_target_disappear_return":512}
FILES=("run.py","source.cpp","native.rs","README.md")

def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def save(p,value):p.write_text(json.dumps(value,indent=2)+"\n")
def execute(command,output,label,timeout=180,**kw):
    save(output/(label+"-command.json"),list(map(str,command)))
    try:
        result=subprocess.run(command,text=True,capture_output=True,timeout=timeout,**kw)
    except subprocess.TimeoutExpired as error:
        for suffix,value in (("stdout",error.stdout),("stderr",error.stderr)):
            if isinstance(value,bytes):value=value.decode(errors="replace")
            (output/(label+"."+suffix+".log")).write_text(value or "")
        save(output/(label+".failure.json"),dict(reason="timeout",seconds=timeout))
        raise RuntimeError(f"{label} timed out after {timeout}s; partial logs retained in {output}") from error
    (output/(label+".stdout.log")).write_text(result.stdout)
    (output/(label+".stderr.log")).write_text(result.stderr)
    if result.returncode:raise RuntimeError(f"{label} failed ({result.returncode}); see {output}/{label}.stderr.log\n{result.stderr[-1800:]}")
    return result

def versions():
    tracks=[dict(voices=t+1,glideMs=37.5,steps=[dict(step=i,notes=[-12+t*12+i*3,12+t*12],vel=(t+1)/4,gate=.65,accent=i%2==0,slide=i%3==0,ratchet=1+i%3) for i in range(4)]) for t in range(4)]
    base=dict(version=1,seed=4721,selectedPattern="A",patterns=[dict(name="A",length=4,stepsPerBeat=8,tracks=tracks),dict(name="B",length=3,stepsPerBeat=8,tracks=[dict(voices=2,steps=[dict(step=i,notes=[24+i*12],accent=True) for i in range(3)])])],lanes=[dict(moduleId="42",paramId=i,mode="normalized",points=[dict(bar=0,value=v)]) for i,v in enumerate([.25,.75])])
    changed=copy.deepcopy(base)
    changed["patterns"][0]["length"]=2
    for t in changed["patterns"][0]["tracks"]:
        t["voices"]=1;t["steps"]=[dict(step=i,notes=[-24],vel=.125) for i in range(2)]
    changed["lanes"][0]["points"][0]["value"]=.625
    selected=copy.deepcopy(base);selected["selectedPattern"]="B"
    raw=copy.deepcopy(base)
    for lane,v in zip(raw["lanes"],[-2,3]):lane["mode"]="raw";lane["points"][0]["value"]=v
    return [base,changed,selected,raw]

def packets(case,count):
    for frame in range(count):
        action,index,running,button,ext,clock,reset,present,manual,value=0,0,1,0,0,0,0,1,-1,0
        if case=="schmitt_startup_disconnect_reset":
            ext=int(not 160<=frame<192)
            # First high is startup, and reconnect-high must retain Schmitt state.
            clock=10 if frame<4 or 8<=frame<220 or frame%17>=8 else 0
            if frame in (5,6,7):clock=[.100001,.1,1][frame-5]
            reset=10 if frame<4 or 270<=frame<273 else 0
        elif case=="stopped_divider_reset_button":
            running=int(frame>=45 and not 140<=frame<180)
            button=int(frame<4 or 82<=frame<84 or 210<=frame<212)
            reset=10 if 310<=frame<313 else 0
        elif case=="manual_cache_equal_reload":
            if frame in (63,97,200):manual,value=0,.875
            if frame in (129,241):action=1
        elif case=="changed_data":
            if frame==6500:action,index=1,1
        elif case=="missing_initial_return":
            present=int(frame>=64)
            if frame==256:action=1
        elif case=="written_target_disappear_return":
            present=int(not 96<=frame<160)
            # Bare return remains an exact case; load later checks rearming.
            if frame==256:action=1
        yield f"{action} {index} {running} {button} {ext} {clock} {reset} {present} {manual} {value}\n"

def compare(a,b,expected):
    mismatches=[];count=0;different=0
    with a.open() as left,b.open() as right:
        for count,(x,y) in enumerate(itertools.zip_longest(left,right),1):
            for row in (x,y):
                if row is None or len(row.split())!=309:raise RuntimeError(f"malformed trace {a.name}/{b.name} frame {count-1}")
            if x!=y:
                different+=1
                if len(mismatches)<8:
                    xx,yy=x.split(),y.split();fields=[dict(index=i,source=u,native=v) for i,(u,v) in enumerate(zip(xx,yy)) if u!=v]
                    mismatches.append(dict(frame=count-1,fields=fields[:12]))
    if count!=expected:raise RuntimeError(f"expected {expected} rows, got {count}")
    return dict(frames=count,passed=different==0,differing_frames=different,first_mismatches=mismatches)

def main():
    parser=argparse.ArgumentParser()
    for name in ("source","sdk","output","target-dir"):parser.add_argument("--"+name,type=Path,required=True)
    parser.add_argument("--case",choices=list(CASES),action="append")
    args=parser.parse_args();source=args.source.resolve();sdk=args.sdk.resolve();output=args.output.resolve();target=args.target_dir.resolve()
    output.mkdir(parents=True,exist_ok=False)
    frozen=json.loads((source/"manifest.json").read_text());assert frozen["repository_commit"]==PIN
    for name,sha in frozen["files"].items():assert digest(source/name)==sha,name
    for name in FILES:shutil.copy2(HERE/name,output/name)
    save(output/"source-manifest.json",frozen)
    production={str(p.relative_to(ROOT)):digest(p) for p in sorted((ROOT/"crates/openpatch-core/src").rglob("*.rs"))}
    for name in ("Cargo.toml","Cargo.lock","crates/openpatch-core/Cargo.toml"):production[name]=digest(ROOT/name)
    save(output/"production-manifest.json",production)
    headers={str(p.relative_to(sdk)):digest(p) for folder in (sdk/"include",sdk/"dep/include") for p in sorted(folder.rglob("*")) if p.is_file()}
    save(output/"sdk-headers.json",headers)
    build=execute(["cargo","build","--offline","--locked","-p","openpatch-core","--lib","--message-format=json"],output,"cargo",cwd=ROOT,env={**os.environ,"CARGO_TARGET_DIR":str(target)})
    libraries={}
    for line in build.stdout.splitlines():
        item=json.loads(line)
        if item.get("reason")=="compiler-artifact":
            for name in item["filenames"]:
                if name.endswith(".rlib"):libraries[item["target"]["name"]]=Path(name)
    native=output/"native"
    execute(["rustc","--edition=2021","-O",str(output/"native.rs"),"-L","dependency="+str(libraries["serde_json"].parent),"--extern","openpatch_core="+str(libraries["openpatch_core"]),"--extern","serde_json="+str(libraries["serde_json"]),"-o",str(native)],output,"rustc")
    descriptors=json.loads(execute([str(native),"--describe"],output,"descriptors").stdout)
    mac_sdk=execute(["xcrun","--show-sdk-path"],output,"mac-sdk").stdout.strip()
    cpp=output/"source"
    flags=["-std=c++17","-O2","-ffp-contract=off","-DRACKMCP_HAVE_JANSSON=1","-isysroot",mac_sdk,"-isystem",mac_sdk+"/usr/include/c++/v1"]
    execute(["c++",*flags,"-I"+str(source/"src"),"-I"+str(sdk/"include"),"-I"+str(sdk/"dep/include"),str(output/"source.cpp"),str(source/"src/rackside/SequencerModule.cpp"),str(source/"src/core/sequencer.cpp"),str(source/"src/core/plan.cpp"),"-L"+str(sdk),"-lRack","-Wl,-rpath,"+str(sdk),"-Wl,-dead_strip","-o",str(cpp)],output,"cpp")
    chosen=args.case or list(CASES)
    declaration=dict(source_pin=PIN,rate=48000,cases={c:CASES[c] for c in chosen},columns="18*(channel_count+16 canonical f32 bits), target0 bits, target1 bits, target_present",comparison="exact every frame from frame zero; no tolerance",source_boundary="actual SequencerModule::process; genuine SDK Context/Engine/registry/ports/ParamHandles/ParamQuantity; declared native one-sample cable receive boundary",snap="both real native capabilities have snap=false; no snapped equivalence claim",descriptors=descriptors)
    save(output/"declaration.json",declaration)
    fixtures=[]
    for case in chosen:
        fixture=output/(case+".json");packet=output/(case+".txt")
        save(fixture,dict(rate=48000,versions=versions(),initial_version=2 if case=="selected_nonzero_first_advance" else 3 if case=="raw_clamp" else 0,initial_live=case!="missing_initial_return",descriptors=descriptors))
        packet.write_text("".join(packets(case,CASES[case])));fixtures.extend([fixture,packet])
    manifest=dict(source_manifest_sha256=digest(output/"source-manifest.json"),production_manifest_sha256=digest(output/"production-manifest.json"),sdk_headers_sha256=digest(output/"sdk-headers.json"),sdk_library_sha256=digest(sdk/"libRack.dylib"),adapters={n:digest(output/n) for n in FILES},declaration_sha256=digest(output/"declaration.json"),fixtures={p.name:digest(p) for p in fixtures},rust_library_sha256=digest(libraries["openpatch_core"]),binaries={p.name:digest(p) for p in (cpp,native)},toolchain={"rust":execute(["rustc","--version"],output,"rust-version").stdout.strip(),"cpp":execute(["c++","--version"],output,"cpp-version").stdout.splitlines()[0]},cpp_flags=flags)
    save(output/"manifest.json",manifest) # Freeze before any sample execution.
    report=dict(manifest_sha256=digest(output/"manifest.json"),cases=[])
    for case in chosen:
        traces=[]
        for binary in (cpp,native):
            trace=output/(case+"."+binary.name+".trace")
            with trace.open("w") as stream,(output/(case+"."+binary.name+".stderr.log")).open("w") as errors:
                try:
                    result=subprocess.run([str(binary),str(output/(case+".json")),str(output/(case+".txt"))],stdout=stream,stderr=errors,timeout=60,env={**os.environ,"DYLD_LIBRARY_PATH":str(sdk)})
                except subprocess.TimeoutExpired as error:
                    report["failure"]=dict(case=case,binary=binary.name,reason="timeout",seconds=60)
                    save(output/"report.json",report)
                    raise RuntimeError(f"{binary.name}/{case} timed out after 60s; partial trace and stderr retained") from error
            if result.returncode:raise RuntimeError(f"{binary.name}/{case} failed ({result.returncode}); see private stderr log")
            traces.append(trace)
        result=compare(*traces,CASES[case]);result["case"]=case;report["cases"].append(result);save(output/"report.json",report)
    passed=all(c["passed"] for c in report["cases"]);report["passed"]=passed;save(output/"report.json",report)
    print(json.dumps(dict(passed=passed,cases=len(chosen),frames=sum(CASES[c] for c in chosen),report=str(output/"report.json"))))
    if not passed:raise SystemExit(1)

if __name__=="__main__":main()
