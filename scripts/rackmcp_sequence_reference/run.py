#!/usr/bin/env python3
"""Compare actual pinned C++ sequence engine/parser and the compiled Rust port.

No Rack module scheduling, target writes, live capture or app equivalence claim.
All floating values are compared bit-for-bit, including beat position.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PIN = "285e84c5ff2d3ecef25890da07f578af6a0c5ca6"
RATES = [44100, 48000, 96000]
CASES = ["swing_poly_probability", "slide_ratchet_stop", "pattern_selection",
         "external_stall", "seek_reset", "data_adoption"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def execute(command, **kwargs):
    result = subprocess.run(command, text=True, capture_output=True, **kwargs)
    if result.returncode:
        raise RuntimeError(f"{command[0]} failed ({result.returncode}):\n{result.stderr[-5000:]}")
    return result


def versions():
    steps = [dict(step=i, notes=[-12+i*3, i*2, 12-i], vel=(i+1)/8,
                  gate=[0, .25, .8, 1][i % 4], prob=[1, .33, .78, 0][i % 4],
                  ratchet=1+i % 8, accent=i % 2 == 0, slide=i % 3 == 0) for i in range(8)]
    tracks = [dict(voices=t+1, glideMs=0 if t == 0 else 65.3,
                   steps=copy.deepcopy(steps)) for t in range(4)]
    # A note-less hit and a rest both retain pitch but break ties differently.
    tracks[1]["steps"][2]["notes"] = []
    tracks[2]["steps"].pop(5)
    base = dict(version=1, seed=123456789, selectedPattern="A",
                patterns=[dict(name="A", length=8, stepsPerBeat=8, tracks=tracks),
                          dict(name="B", length=3, stepsPerBeat=3,
                               tracks=[dict(voices=4, glideMs=350,
                                            steps=[dict(step=i, notes=[12*i], slide=True, accent=True)
                                                   for i in range(3)])])], lanes=[])
    curves = ["linear", "easeIn", "easeOut", "easeInOut", "hold"]
    for i, curve in enumerate(curves):
        base["lanes"].append(dict(moduleId=str(i+1), paramId=i, mode="raw", loop=i % 2 == 0,
                                  lengthBars=.5, points=[dict(bar=0, value=-3),
                                                       dict(bar=.125, value=4, curve=curve),
                                                       dict(bar=.125, value=.25),
                                                       dict(bar=.5, value=-1, curve=curve)]))
    changed = copy.deepcopy(base)
    changed["patterns"][0]["length"] = 2
    for track in changed["patterns"][0]["tracks"]:
        track["voices"] = 1
        track["steps"] = [s for s in track["steps"] if s["step"] < 2]
        for step in track["steps"]:
            step["notes"] = [-24]
    selected = copy.deepcopy(base)
    selected["selectedPattern"] = "B"
    return [base, changed, selected]


def packets(case, rate):
    # Scaled duration covers several internal pattern wraps at all rates.
    count = round(rate * .55)
    for frame in range(count):
        action, index, value = 0, 0, 0
        bpm, swing, running, external, rising = 299.25, 67.125, 1, 0, 0
        at = lambda seconds: frame == round(rate * seconds)
        if case == "slide_ratchet_stop":
            running = int(not .12 <= frame/rate < .21)
        elif case == "pattern_selection":
            if at(.03): action,index,value = 3,1,1
            if at(.18): action,index,value = 3,0,0
            if at(.32): action,index,value = 3,0,1
        elif case == "external_stall":
            external = 1
            period = max(1,round(rate*.0031))
            rising = int(frame % period == 0 and not .08 <= frame/rate < .2)
            if frame/rate > .32:
                rising = int(frame % (period * 3) == 0)
            running = int(not .24 <= frame/rate < .27)
        elif case == "seek_reset":
            external = 1
            rising = int(frame % 97 == 0)
            if at(.015): action,index,value = 3,1,0
            if at(.03): action,value = 2,2.125
            if at(.16): action = 1
            if at(.3): action,value = 2,123.75
            if at(.4): action = 1
        elif case == "data_adoption":
            if at(.01): action,index = 4,2
            if at(.1): action,index = 4,1
            if at(.25): action,index = 4,0
            if at(.35): action,index = 4,2
        yield f"{action} {index} {value} {rate} {bpm} {swing} {running} {external} {rising}\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    source, sdk, output = args.source.resolve(), args.sdk.resolve(), args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    frozen = json.loads((source / "manifest.json").read_text())
    assert frozen["repository_commit"] == PIN
    for path, expected in frozen["files"].items():
        assert digest(source/path) == expected, path
    declaration = {"source_pin": PIN, "cases": CASES, "rates": RATES,
                   "duration_seconds": .55, "comparison": "all output/position/lane bits, every frame"}
    (output/"declaration.json").write_text(json.dumps(declaration,indent=2)+"\n")
    for path in (HERE/"run.py", HERE/"source.cpp", HERE/"native.rs"):
        (output/path.name).write_bytes(path.read_bytes())
    mac_sdk = Path(execute(["xcrun", "--show-sdk-path"]).stdout.strip())
    cpp = output/"source"
    flags = ["-std=c++17","-O2","-ffp-contract=off","-DRACKMCP_HAVE_JANSSON=1",
             "-isysroot",str(mac_sdk),"-isystem",str(mac_sdk/"usr/include/c++/v1")]
    command = ["c++",*flags,"-I"+str(source/"src"),"-I"+str(sdk/"dep/include"),
               str(HERE/"source.cpp"),str(source/"src/core/sequencer.cpp"),
               str(source/"src/core/plan.cpp"),"-L"+str(sdk),"-lRack",
               "-Wl,-rpath,"+str(sdk),"-Wl,-dead_strip","-o",str(cpp)]
    (output/"cpp-command.json").write_text(json.dumps(command))
    execute(command)
    build = execute(["cargo","build","--offline","--locked","-p","openpatch-core",
                     "--lib","--message-format=json"],cwd=ROOT,
                    env={**os.environ,"CARGO_TARGET_DIR":str(args.target.resolve())})
    (output/"cargo.log").write_text(build.stderr)
    libraries={}
    for line in build.stdout.splitlines():
        item=json.loads(line)
        if item.get("reason")=="compiler-artifact":
            for filename in item["filenames"]:
                if filename.endswith(".rlib"): libraries[item["target"]["name"]]=Path(filename)
    native=output/"native"
    execute(["rustc","--edition=2021","-O",str(HERE/"native.rs"),"-L",
             "dependency="+str(libraries["serde_json"].parent),"--extern",
             "openpatch_core="+str(libraries["openpatch_core"]),"--extern",
             "serde_json="+str(libraries["serde_json"]),"-o",str(native)])
    fixture=output/"versions.json"
    fixture.write_text(json.dumps(versions(),indent=2)+"\n")
    report={"declaration":declaration,"cases":[],"source_manifest_sha256":digest(source/"manifest.json"),
            "rust_library_sha256":digest(libraries["openpatch_core"]),"cpp_flags":flags,
            "cpp_version":execute(["c++","--version"]).stdout.splitlines()[0],
            "rust_version":execute(["rustc","--version"]).stdout.strip(),
            "adapters":{p.name:digest(p) for p in [HERE/"run.py",HERE/"source.cpp",HERE/"native.rs"]}}
    for rate in RATES:
        for case in CASES:
            path=output/f"{case}-{rate}.txt"
            path.write_text("".join(packets(case,rate)))
            results=[]
            for binary in [cpp,native]:
                trace=path.with_suffix("."+binary.name+".trace")
                with trace.open("w") as stream:
                    process=subprocess.run([str(binary),str(fixture),str(path)],stdout=stream,stderr=subprocess.PIPE,text=True,
                                           env={**os.environ,"DYLD_LIBRARY_PATH":str(sdk)})
                if process.returncode: raise RuntimeError(process.stderr[-4000:])
                results.append(trace)
            mismatches=[]
            count=0
            with results[0].open() as a,results[1].open() as b:
                from itertools import zip_longest
                for count,(left,right) in enumerate(zip_longest(a,b),1):
                    if left is not None and len(left.split()) != 47:
                        raise RuntimeError(f"malformed source trace at frame {count-1}")
                    if right is not None and len(right.split()) != 47:
                        raise RuntimeError(f"malformed native trace at frame {count-1}")
                    if left!=right and len(mismatches)<8:
                        mismatches.append({"frame":count-1,"source":left,"native":right})
            if count != round(rate * .55):
                raise RuntimeError(f"wrong trace duration: {case}/{rate}: {count}")
            report["cases"].append({"case":case,"rate":rate,"frames":count,"passed":not mismatches,"first_mismatches":mismatches})
            (output/"report.json").write_text(json.dumps(report,indent=2)+"\n")
    passed=all(c["passed"] for c in report["cases"])
    print(json.dumps({"passed":passed,"cases":len(report["cases"]),"frames":sum(c["frames"] for c in report["cases"]),"report":str(output/"report.json")}))
    if not passed: raise SystemExit(1)


if __name__=="__main__": main()
