#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Execute the actual pinned Rack MIDI-CC class against frozen production Rust."""
import argparse, hashlib, json, math, os, platform, struct, subprocess, sys, tempfile, time, zlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent
PIN="8ea470f343058ade6f75f16d503a1eb330e1700c"
URL=f"https://raw.githubusercontent.com/VCVRack/Rack/{PIN}/src/core/MIDICC_CV.cpp"
SOURCE_SHA="9256e38c046a1c497b1c919ad78d8da81da4f3a589aac3b3aec31491f691d405"
ABS=0.0001
REL=0.000001
# This manifest and all deterministic fixture logic are written before the first comparison.
CASES={
 "all_cells_all_wire_channels_7bit":4096,
 "all_128_controllers_mapping_windows":4096,
 "normal_mode_channel_merge":4096,
 "unmapped_and_collision_arrays":4096,
 "smoothing_strict_jump_threshold_and_convergence":16384,
 "paired_14bit_all_channels_msb_lsb_orderings":4096,
 "paired_14bit_upper_bank_and_saturation":4096,
 "disconnected_and_unmapped_filter_freeze":4096,
 "mpe_toggle_inactive_lane_history":4096,
 "selected_channel_filter_and_unrelated_messages":4096,
 "learn_equal_rejected_collision_and_precommit":4096,
 "unmapped_controller_reassignment":4096,
 "json_channel_zero_snapshot_restore":4096,
 "adopt_mapping_option_channel_changes":4096,
 "same_sample_event_ordering":4096,
}

def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def fixture(name,frames):
 for f in range(frames):
  options=[1,0,0,0];mask=65535;op=0;learn=0;maps=list(range(1,17));events=[];loaded=None
  def cc(ch,c,v):events.append([11,ch,c,v])
  if name=="all_cells_all_wire_channels_7bit":
   options=[0,1,0,0]
   if f%127==0:
    for ch in range(16):
     for c in range(1,17):cc(ch,c,(ch*17+c*7+f//127)%128)
  elif name=="all_128_controllers_mapping_windows":
   options=[0,1,0,0];bank=(f//512)%8;maps=list(range(bank*16,bank*16+16))
   if f%512==0:
    for ch in range(16):
     for c in range(bank*16,bank*16+16):cc(ch,c,(c+ch*11+1)%128)
  elif name=="normal_mode_channel_merge":
   options=[0,0,0,0]
   if f%17==0:
    for ch in range(16):cc(ch,1+(f//17)%16,(ch*7+f//17)%128)
  elif name=="unmapped_and_collision_arrays":
   options=[1,1,0,0];maps=[-1 if i%3==0 else (i//2)+1 for i in range(16)]
   if f%79==0:
    for ch in range(16):
     for c in range(1,9):cc(ch,c,(c*13+ch*5+f//79)%128)
  elif name=="smoothing_strict_jump_threshold_and_convergence":
   values=[127,0,126,1,64,0,127,126];step=f//2048
   if f%2048==0:
    for c in range(1,17):cc(0,c,values[(step+c-1)%8])
  elif name.startswith("paired_14bit"):
   options=[1,1,1,0];base=16 if "upper_bank" in name else 0;maps=list(range(base,base+8))+list(range(base+32,base+40));step=f%193
   for ch in range(16):
    for offset in range(8):
     c=base+offset;pattern=(offset+ch)%5
     if step==0:
      if pattern in (0,3):cc(ch,c,127 if pattern==3 else (ch+offset+f//193)%128)
      else:cc(ch,c+32,127 if pattern==4 else (ch*7+offset)%128)
     if step==1 and pattern!=2:
      if pattern in (0,3):cc(ch,c+32,127 if pattern==3 else (offset*13)%128)
      else:cc(ch,c,127 if pattern==4 else (offset*17+ch)%128)
     if step==3 and pattern==1:cc(ch,c+32,64)
   # Pattern 2 is LSB-only; pattern 3 saturates 127+127; pattern 4 reversed pair.
  elif name=="disconnected_and_unmapped_filter_freeze":
   mask=65535 if (f//257)%3==0 else 0xaaaa
   if (f//509)%2:maps[0]=-1;maps[15]=-1
   if f%233==0:
    for c in range(1,17):cc(0,c,((f//233)*31+c*3)%128)
  elif name=="mpe_toggle_inactive_lane_history":
   options=[1,int((f//509)%3!=1),0,0]
   if f%127==0:
    for ch in range(16):
     for c in range(1,17):cc(ch,c,(ch*7+c+f//127)%128)
  elif name=="selected_channel_filter_and_unrelated_messages":
   options=[0,1,1,1+(f//257)%16]
   if f%31==0:
    for ch in range(16):cc(ch,1,(ch*9+f//31)%128);cc(ch,33,(ch*13+f//31)%128);events.append([9,ch,1,127])
  elif name=="learn_equal_rejected_collision_and_precommit":
   options=[1,1,1,2]
   phase=f%257
   if phase==0:learn=1+(f//257)%16
   if phase==1:cc(0,2,100);cc(1,2,0)
   if phase==2:events.append([9,1,2,100])
   if phase==3:cc(1,2,1+(f//257)%127)
   if phase==4:cc(1,34,4)
   if phase==5:cc(1,2,1+(f//257)%127)
  elif name=="unmapped_controller_reassignment":
   maps=list(range(96,112)) if f>=2048 else list(range(1,17))
   if f==0:
    for ch in range(16):
     for c in range(96,112):cc(ch,c,(c+ch)%128)
   options=[1,1,0,0]
  elif name=="json_channel_zero_snapshot_restore":
   options=[1,1,0,0]
   if f==0:
    op=1;loaded=[(i*17+3)%128 for i in range(128)]
   if f==1024:op=2
   if f==3072:op=3
   if f in (512,2048):
    for ch in range(16):
     for c in range(1,17):cc(ch,c,(c+ch+f//512*11)%128)
  elif name=="adopt_mapping_option_channel_changes":
   options=[(f//257)%2,(f//509)%2,(f//769)%2,(f//1021)%3]
   maps=[(i+f//383*17)%128 for i in range(16)]
   if f%43==0:
    for ch in range(16):
     for c in maps:cc(ch,c,(c*7+ch*11+f//43)%128)
  elif name=="same_sample_event_ordering":
   options=[0,1,1,0];maps=list(range(0,8))+list(range(32,40))
   if f%97==0:
    for ch in range(16):
     for c in range(8):cc(ch,c,100);cc(ch,c+32,17);cc(ch,c,23);cc(ch,c+32,99);cc(ch,c,127)
  else:raise ValueError(name)
  row=[*options,mask,op,learn,len(events),*maps]
  if loaded is not None:row+=loaded
  row += [number for event in events for number in event]
  yield " ".join(map(str,row))

def serde_library():
 cargo=str(Path.home()/".cargo/bin/cargo")
 built=subprocess.run([cargo,"build","--offline","--locked","-p","openpatch-core","--lib","--message-format=json"],cwd=ROOT,check=True,capture_output=True,text=True)
 for line in built.stdout.splitlines():
  if line.startswith("{"):
   item=json.loads(line)
   if item.get("reason")=="compiler-artifact" and item.get("target",{}).get("name")=="serde_json":return next(Path(p) for p in item["filenames"] if p.endswith(".rlib"))
 raise RuntimeError("serde_json artifact missing")

def main():
 parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--sdk",type=Path,default=Path.home()/"Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK");parser.add_argument("--output",type=Path,default=ROOT/"docs/benchmarks/midi-cc-reference.json");args=parser.parse_args();args.sdk=args.sdk.resolve()
 started=time.time();source=subprocess.check_output(["curl","--fail","--silent","--show-error","--location",URL]);assert hashlib.sha256(source).hexdigest()==SOURCE_SHA,"pinned source SHA mismatch"
 upstream=source.decode();assert upstream.count('struct MIDICC_CVWidget')==1 and upstream.count('#include "plugin.hpp"')==1
 prefix=upstream.split("struct MIDICC_CVWidget",1)[0].replace('#include "plugin.hpp"','#include <rack.hpp>')+'\n} // namespace core\n} // namespace rack\n'
 filter_source=subprocess.check_output(["curl","--fail","--silent","--show-error","--location",f"https://raw.githubusercontent.com/VCVRack/Rack/{PIN}/include/dsp/filter.hpp"])
 assert hashlib.sha256(filter_source).hexdigest()==sha(args.sdk/"include/dsp/filter.hpp"),"SDK filter differs from pinned filter header"
 midi_source=subprocess.check_output(["curl","--fail","--silent","--show-error","--location",f"https://raw.githubusercontent.com/VCVRack/Rack/{PIN}/src/midi.cpp"])
 cpp_flags=["-std=c++17","-O2","-ffp-contract=off"];rust_flags=["--edition=2021","-O","-C","target-feature=-fma"]
 headers=["rack.hpp","midi.hpp","dsp/filter.hpp","engine/Port.hpp","engine/Module.hpp"]
 report={"reference":{"rack_commit":PIN,"source_url":URL,"source_sha256":SOURCE_SHA,"compiled_prefix_sha256":hashlib.sha256(prefix.encode()).hexdigest(),"midi_routing_source_sha256":hashlib.sha256(midi_source).hexdigest(),"pinned_filter_header_sha256":hashlib.sha256(filter_source).hexdigest(),"pinned_filter_matches_sdk":True,"sdk_library_sha256":sha(args.sdk/("libRack.dylib" if sys.platform=="darwin" else "libRack.so")),"sdk_header_sha256":{h:sha(args.sdk/"include"/h) for h in headers}},"method":"Actual pinned MIDICC_CV class; only widget/registration trimmed, two namespaces closed and plugin.hpp replaced with rack.hpp. SDK InputQueue receives explicitly timestamped standard MIDI messages. Driver applies exact pinned InputDevice channel rule before queue insertion without opening a driver/device/context. Production midi_cc.rs compiled unchanged using minimal host data types. Raw frame-index comparison begins at first process call, all exposed cells/lanes each frame.","qualification_history":{"initial_failure_artifact_sha256":sha(HERE/"initial-failure.json"),"initial_failure":"Original production Euler filter omitted the pinned SDK rounded-no-change snap; fixture and numerical gate unchanged for rerun."},"predeclared_cases":CASES,"tolerances":{"absolute_volts":ABS,"relative":REL,"maps_values_learning_completion_frame_output_width":"exact","alignment":"same index; no warm-up, offsets, exclusions or threshold exemptions"},"limitations":["No application, window, native MIDI driver/device, hardware endpoint or audio device is opened", "SDK channel filtering normally occurs in InputDevice::onMessage; exact source rule is reproduced in driver to avoid context/device initialization, while class process and actual SDK queue remain genuine", "MIDI nonstandard signed gamepad extension excluded; ordinary 7-bit messages only", "Host configuration changes use source public configuration/actual JSON mapping loader versus Rust rebuild/adoption; Learn cancellation on configuration change is explicit host policy", "JSON snapshots invoke actual Rack dataToJson/dataFromJson; hardware MIDI port JSON omitted so no endpoint selection occurs", "Bypass, initialization/reset/panic host policy, Engine cables/probe consumer bookkeeping, Session capture/save/Undo, transport durability/backpressure/stale identity, native GUI and performance are separate gates", "This SDK/toolchain/platform combination is qualified; other platforms must execute their own reference comparison"],"production_source_sha256":sha(ROOT/"crates/openpatch-core/src/midi_cc.rs"),"driver_sha256":{p:sha(HERE/p) for p in ["run.py","sdk_probe.cpp","rust_probe.rs"]},"toolchain":{"cpp":subprocess.check_output(["c++","--version"],text=True).splitlines()[0],"rust":subprocess.check_output([str(Path.home()/".cargo/bin/rustc"),"--version"],text=True).strip(),"platform":platform.platform(),"machine":platform.machine(),"cpp_flags":cpp_flags,"rust_flags":rust_flags,"FMA":"C++ fp contraction disabled; Rust fma target feature disabled, ordinary separate operations, no fast-math"},"cases":[]}
 serde=serde_library()
 with tempfile.TemporaryDirectory(prefix="openpatch-midi-cc-reference-")as directory:
  tmp=Path(directory);(tmp/"reference_module.hpp").write_text(prefix);cpp=tmp/"sdk_probe";rust=tmp/"rust_probe";extra=[]
  if sys.platform=="darwin":
   mac_sdk=subprocess.check_output(["xcrun","--show-sdk-path"],text=True).strip();extra=["-isysroot",mac_sdk,"-isystem",str(Path(mac_sdk)/"usr/include/c++/v1")]
  subprocess.run(["c++",*cpp_flags,*extra,"-I"+str(args.sdk/"include"),"-I"+str(args.sdk/"dep/include"),"-I"+str(tmp),str(HERE/"sdk_probe.cpp"),"-L"+str(args.sdk),"-lRack","-Wl,-rpath,"+str(args.sdk),"-o",str(cpp)],check=True)
  (tmp/"probe.rs").write_text((HERE/"rust_probe.rs").read_text().replace("__MIDI_CC_SOURCE__",json.dumps(str(ROOT/"crates/openpatch-core/src/midi_cc.rs"))))
  subprocess.run([str(Path.home()/".cargo/bin/rustc"),*rust_flags,str(tmp/"probe.rs"),"--extern","serde_json="+str(serde),"-L","dependency="+str(serde.parent),"-o",str(rust)],check=True)
  report["executable_sha256"]={"cpp":sha(cpp),"rust":sha(rust)};env=os.environ.copy();env["DYLD_LIBRARY_PATH" if sys.platform=="darwin" else "LD_LIBRARY_PATH"]=str(args.sdk)
  for rate in [44100,48000,96000]:
   for name,frames in CASES.items():
    data=("\n".join(fixture(name,frames))+"\n").encode();t0=time.monotonic();traces=[subprocess.run([str(exe),str(rate)],input=data,capture_output=True,check=True,env=env).stdout for exe in [cpp,rust]];elapsed=time.monotonic()-t0
    row_size=(162+256)*4;assert all(len(t)==frames*row_size for t in traces),(name,[len(t)for t in traces]);failures=[];worst={"absolute_error_volts":-1};max_ratio=0;metadata_mismatches=0;active_samples=0
    for frame in range(frames):
     offset=frame*row_size;meta=[struct.unpack_from("<162i",t,offset) for t in traces];volts=[struct.unpack_from("<256f",t,offset+648)for t in traces]
     if meta[0]!=meta[1]:
      metadata_mismatches+=1
      if len(failures)<8:failures.append({"frame_zero_based":frame,"metadata_differences":[{"field":i,"reference":a,"actual":b} for i,(a,b)in enumerate(zip(*meta))if a!=b][:16]})
     for cell,width in enumerate(meta[0][146:]):
      for lane in range(width):
       i=cell*16+lane;a,b=volts[0][i],volts[1][i];assert math.isfinite(a)and math.isfinite(b);error=abs(a-b);tolerance=ABS+REL*abs(a);ratio=error/tolerance;active_samples+=1;max_ratio=max(max_ratio,ratio)
       if error>worst["absolute_error_volts"]:worst={"frame_zero_based":frame,"cell_one_based":cell+1,"lane_one_based":lane+1,"reference_volts":a,"actual_volts":b,"absolute_error_volts":error,"tolerance_volts":tolerance}
       if ratio>1 and len(failures)<8:failures.append({"frame_zero_based":frame,"cell_one_based":cell+1,"lane_one_based":lane+1,"reference":a,"actual":b,"error":error,"tolerance":tolerance})
    case={"name":name,"sample_rate":rate,"frames":frames,"active_lane_samples":active_samples,"input_sha256":hashlib.sha256(data).hexdigest(),"input_crc32":f"{zlib.crc32(data):08x}","trace_sha256":{"reference":hashlib.sha256(traces[0]).hexdigest(),"production":hashlib.sha256(traces[1]).hexdigest()},"trace_crc32":{"reference":f"{zlib.crc32(traces[0]):08x}","production":f"{zlib.crc32(traces[1]):08x}"},"worst_sample":worst,"maximum_tolerance_ratio":max_ratio,"metadata_mismatches":metadata_mismatches,"driver_execution_seconds":elapsed,"passed":not failures,"first_failures":failures};report["cases"].append(case);print(json.dumps({"name":name,"rate":rate,"passed":case["passed"],"maximum_tolerance_ratio":max_ratio}),flush=True)
 report["passed"]=all(c["passed"]for c in report["cases"]);report["total_frames"]=sum(c["frames"]for c in report["cases"]);report["total_active_lane_samples"]=sum(c["active_lane_samples"]for c in report["cases"]);report["elapsed_seconds"]=time.time()-started;report["run_utc"]=time.strftime("%Y-%m-%dT%H:%M:%SZ",time.gmtime());args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+"\n");print(json.dumps({"passed":report["passed"],"frames":report["total_frames"],"report":str(args.output)}));raise SystemExit(0 if report["passed"]else 1)
if __name__=="__main__":main()
