#!/usr/bin/env python3
"""Build/run only the genuine source two-sequencer ownership audit; no native claim."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
from run import HERE,PIN,digest,execute,save

def main():
    parser=argparse.ArgumentParser()
    for name in ("source","sdk","output"):parser.add_argument("--"+name,type=Path,required=True)
    args=parser.parse_args();source=args.source.resolve();sdk=args.sdk.resolve();output=args.output.resolve();output.mkdir(parents=True,exist_ok=False)
    frozen=json.loads((source/"manifest.json").read_text());assert frozen["repository_commit"]==PIN
    for name,sha in frozen["files"].items():assert digest(source/name)==sha,name
    for name in ("ownership.cpp","ownership.py","run.py","README.md"):shutil.copy2(HERE/name,output/name)
    save(output/"source-manifest.json",frozen)
    headers={str(p.relative_to(sdk)):digest(p) for directory in (sdk/"include",sdk/"dep/include") for p in sorted(directory.rglob("*")) if p.is_file()};save(output/"sdk-headers.json",headers)
    declaration=dict(source_pin=PIN,sdk="2.6.6",sample_rate=48000,source_modules=[100,200],target=[42,0],lane_values=[.25,.75],frames_per_stage=32,stages=["A_acquires","B_takes_over","equal_incumbent_B_reload_self_unbinds","displaced_A_reloads_and_acquires","B_takes_over_again","delete_winner_B_does_not_restore_A"],scope="genuine source wrapper/SDK ownership only; original expected stages derived from pinned overwrite=true calls and SDK2.6.6 observed self-unbind; no native comparison or Rack scheduler equivalence")
    save(output/"declaration.json",declaration)
    mac_sdk=execute(["xcrun","--show-sdk-path"],output,"mac-sdk").stdout.strip();flags=["-std=c++17","-O2","-ffp-contract=off","-DRACKMCP_HAVE_JANSSON=1","-isysroot",mac_sdk,"-isystem",mac_sdk+"/usr/include/c++/v1"]
    binary=output/"ownership"
    execute(["c++",*flags,"-I"+str(source/"src"),"-I"+str(sdk/"include"),"-I"+str(sdk/"dep/include"),str(output/"ownership.cpp"),str(source/"src/rackside/SequencerModule.cpp"),str(source/"src/core/sequencer.cpp"),str(source/"src/core/plan.cpp"),"-L"+str(sdk),"-lRack","-Wl,-rpath,"+str(sdk),"-Wl,-dead_strip","-o",str(binary)],output,"cpp")
    save(output/"manifest.json",dict(declaration_sha256=digest(output/"declaration.json"),source_manifest_sha256=digest(output/"source-manifest.json"),sdk_headers_sha256=digest(output/"sdk-headers.json"),sdk_library_sha256=digest(sdk/"libRack.dylib"),adapters={n:digest(output/n) for n in ("ownership.cpp","ownership.py","run.py","README.md")},binary_sha256=digest(binary),cpp_flags=flags,cpp_version=execute(["c++","--version"],output,"cpp-version").stdout.splitlines()[0]))
    with (output/"source-report.json").open("w") as stream,(output/"source.stderr.log").open("w") as errors:
        try:result=subprocess.run([str(binary)],stdout=stream,stderr=errors,timeout=60,env={**os.environ,"DYLD_LIBRARY_PATH":str(sdk)})
        except subprocess.TimeoutExpired as error:
            save(output/"failure.json",dict(reason="timeout",seconds=60));raise RuntimeError("source ownership timed out; partial report and stderr retained") from error
    if result.returncode not in (0,1):raise RuntimeError(f"source ownership failed ({result.returncode}); see retained source.stderr.log")
    report=json.loads((output/"source-report.json").read_text());save(output/"report.json",dict(manifest_sha256=digest(output/"manifest.json"),source_report_sha256=digest(output/"source-report.json"),passed=report["passed"],stages=len(report["stages"]),frames=report["frames"]))
    print(json.dumps(dict(passed=report["passed"],stages=len(report["stages"]),frames=report["frames"],report=str(output/"report.json"))))
    if not report["passed"]:raise SystemExit(1)

if __name__=="__main__":main()
