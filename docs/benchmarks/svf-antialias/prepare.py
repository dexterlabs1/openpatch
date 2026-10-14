#!/usr/bin/env python3
"""Prepare an isolated, pinned SVF antialias candidate; never build or run it.

The checked-in runtime is not modified. Factor 0 preserves the baseline DSP.
Factors 4/8/16 reuse its Blackman FIR resampler with 257/513/1025 taps and add
the study's drive preprocessing and matched 64-sample cutoff-CV delay.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import subprocess
import tarfile

BASELINE = "147c8300b7dab2ab58569ee186d0e8fe4a58f1ad"
CORE_PREFIX = "crates/openpatch-core/"
PROBE_PREFIX = "docs/benchmarks/svf-spectral/"
HERE = Path(__file__).resolve().parent


def replace_exact(source, old, new, count=1):
    """Fail closed if a pinned-source assumption changes."""
    actual = source.count(old)
    if actual != count:
        raise ValueError(f"Expected {count} matches, found {actual}: {old!r}")
    return source.replace(old, new)


def pinned_files(repository):
    archive = subprocess.run(
        ["git", "archive", "--format=tar", BASELINE, CORE_PREFIX.rstrip("/"),
         PROBE_PREFIX.rstrip("/")], cwd=repository, check=True, capture_output=True
    ).stdout
    files = {}
    # Do not extract arbitrary tar members to the filesystem. Validate all
    # paths/types, then read only regular files into memory.
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
        for member in tar:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or "\\" in member.name:
                raise ValueError(f"Unsafe archive path: {member.name!r}")
            if not (member.isfile() or member.isdir()):
                raise ValueError(f"Unsupported archive member: {member.name!r}")
            if member.isfile():
                if not member.name.startswith((CORE_PREFIX, PROBE_PREFIX)):
                    raise ValueError(f"Unexpected archive file: {member.name!r}")
                if member.name in files:
                    raise ValueError(f"Duplicate archive file: {member.name!r}")
                files[member.name] = tar.extractfile(member).read()
    return files


def candidate_voice_tools(source):
    """Apply the exact study patch while retaining original source formatting.

    The host-rate TPT filter equations remain unchanged. Only its input drive
    is moved into the existing oversampler; CV receives the same host latency.
    """
    source = replace_exact(source,
        "use crate::{parameter, Node, PortFrame, MAX_CHANNELS};",
        "use crate::{parameter, Node, PortFrame, MAX_CHANNELS};\nuse crate::ladder::{Oversampler, LATENCY_SAMPLES};")
    source = replace_exact(source,
        "#[derive(Clone, Debug)]\npub(crate) struct Filter {", """#[derive(Clone, Debug)]
struct DriveLane {
    oversampler: Oversampler,
    cv: [f64; LATENCY_SAMPLES],
    head: usize,
}
impl DriveLane {
    fn new() -> Self { Self { oversampler: Oversampler::new(), cv: [0.0; LATENCY_SAMPLES], head: 0 } }
    fn process(&mut self, x: f64, cv: f64, gain: f64, blend: f64) -> (f64, f64) {
        let delayed_cv = self.cv[self.head]; self.cv[self.head] = cv;
        self.head = (self.head+1)%LATENCY_SAMPLES;
        let denom = gain.tanh();
        (self.oversampler.process_pair(x, 0.0, |x, _| x + blend * (5.0 * (x*gain/5.0).tanh()/denom - x)), delayed_cv)
    }
    fn copy_state_from(&mut self, old: &Self) {
        self.oversampler.copy_state_from(&old.oversampler); self.cv=old.cv; self.head=old.head;
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Filter {""")
    source = replace_exact(source, "    states: [[f64; 2]; MAX_CHANNELS],",
        "    states: [[f64; 2]; MAX_CHANNELS],\n    drive_lanes: Box<[DriveLane]>,")
    source = replace_exact(source, "                states: [[0.0; 2]; MAX_CHANNELS],",
        "                states: [[0.0; 2]; MAX_CHANNELS],\n                drive_lanes: (0..MAX_CHANNELS).map(|_| DriveLane::new()).collect(),")
    source = replace_exact(source, """                for lane in count..MAX_CHANNELS {
                    f.states[lane] = [0.0; 2];
                }
                for lane in f.active..count {
                    f.states[lane] = [0.0; 2];
                }""", """                for lane in count.min(f.active)..count.max(f.active) {
                    f.states[lane] = [0.0; 2];
                    f.drive_lanes[lane] = DriveLane::new();
                }""")
    source = replace_exact(source, """                for lane in 0..count {
                    let cutoff""", """                let gain = 1.0 + 8.0 * f.drive;
                let t = (f.drive/0.1).clamp(0.0,1.0);
                let blend = t*t*(3.0-2.0*t);
                for lane in 0..count {
                    let raw = finite(signal.voice(lane, 0.0)).clamp(-1e6, 1e6);
                    let (x, delayed_cv) = f.drive_lanes[lane].process(raw, finite(cv.voice(lane, 0.0)), gain, blend);
                    let cutoff""")
    source = replace_exact(source, "finite(cv.voice(lane, 0.0)) * f.cv", "delayed_cv * f.cv")
    source = replace_exact(source, """                    let x = finite(signal.voice(lane, 0.0)).clamp(-1e6, 1e6);
                    let x = if f.drive > 0.0 {
                        let gain = 1.0 + 8.0 * f.drive;
                        5.0 * (x * gain / 5.0).tanh() / gain.tanh()
                    } else {
                        x
                    };
""", "")
    source = replace_exact(source, """                a.states = b.states;
                a.active = b.active;""", """                a.states = b.states;
                a.active = b.active;
                for (a,b) in a.drive_lanes.iter_mut().zip(&b.drive_lanes) { a.copy_state_from(b); }""")
    source = replace_exact(source, """                f.states = [[0.0; 2]; MAX_CHANNELS];
                f.active = 0;""", """                f.states = [[0.0; 2]; MAX_CHANNELS];
                for lane in f.drive_lanes.iter_mut() { *lane=DriveLane::new(); }
                f.active = 0;""")
    return source


def prepared_files(baseline_files, factor, matrix):
    files = {"core/" + name[len(CORE_PREFIX):]: data
             for name, data in baseline_files.items() if name.startswith(CORE_PREFIX)}
    files["core/Cargo.toml"] += b"\n[workspace]\n"
    if factor:
        name = "core/src/voice_tools.rs"
        files[name] = candidate_voice_tools(files[name].decode()).encode()
        name = "core/src/ladder.rs"
        source = files[name].decode()
        source = replace_exact(source, "pub const OVERSAMPLING: usize = 8;",
                               f"pub const OVERSAMPLING: usize = {factor};")
        source = replace_exact(source, "const TAPS: usize = 513;",
                               f"const TAPS: usize = {64 * factor + 1};")
        files[name] = source.encode()
    manifest = baseline_files[PROBE_PREFIX + "Cargo.toml"].decode()
    files["probe/Cargo.toml"] = replace_exact(manifest, "../../../crates/openpatch-core", "../core").encode()
    files["probe/Cargo.lock"] = baseline_files[PROBE_PREFIX + "Cargo.lock"]
    files["probe/src/main.rs"] = baseline_files[PROBE_PREFIX + "src/main.rs"]
    if matrix in ("highband", "dense"):
        # The byte-preserved direct-component probe bypasses oscillator frequency
        # limits and supplies its analytic f32 sine with two explicit delays.
        probe = (HERE / "highband_probe.rs").read_text()
        if matrix == "dense":
            # Sweep the difficult upper host band at high drive, retaining
            # explicit rational frequencies where aliases can overlap harmonics.
            probe = replace_exact(probe, """  let mut bins=vec![4,43,171,683,1501,3441,3687,3933,4014];
  bins.push((18000.0/rate as f64*N as f64).round() as usize);
  bins.push((20000.0/rate as f64*N as f64).round() as usize);""", """  let mut bins:Vec<usize>=(2868..=4014).step_by(16).collect();
  bins.extend([3072,3277,3441,3686,3932,4014]);""")
            probe = replace_exact(probe, "for amplitude in [0.1_f64,5.0]", "for amplitude in [5.0_f64]")
            probe = replace_exact(probe, "for drive in [0.0_f64,0.1,1.0]", "for drive in [1.0_f64]")
        files["core/src/spectral_probe.rs"] = probe.encode()
        files["core/src/lib.rs"] += b"\n#[doc(hidden)] pub mod spectral_probe;\n"
        files["probe/src/main.rs"] = b'fn main(){openpatch_core::spectral_probe::export(std::path::Path::new(&std::env::args().nth(1).unwrap()));}\n'
    elif matrix == "performance":
        # Use the exact timed public-Engine workload. Profile flags are local
        # to this probe; spectral-probe compilation settings stay unchanged.
        files["probe/src/main.rs"] = (HERE / "performance.rs").read_bytes()
        manifest = replace_exact(files["probe/Cargo.toml"].decode(),
                                 'name = "openpatch-svf-spectral-probe"',
                                 'name = "openpatch-svf-performance-probe"')
        files["probe/Cargo.toml"] = (manifest + "\n[profile.release]\nlto = \"thin\"\ncodegen-units = 1\n").encode()
        lock = replace_exact(files["probe/Cargo.lock"].decode(),
                             'name = "openpatch-svf-spectral-probe"',
                             'name = "openpatch-svf-performance-probe"')
        files["probe/Cargo.lock"] = lock.encode()
    return files


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--factor", type=int, choices=(0, 4, 8, 16), required=True)
    parser.add_argument("--matrix", choices=("engine", "highband", "dense", "performance"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists() or args.output.is_symlink():
        parser.error("--output must be a new directory; existing paths are never overwritten")
    repository = HERE.parents[2]
    baseline_files = pinned_files(repository)
    files = prepared_files(baseline_files, args.factor, args.matrix)
    provenance = {
        "baseline_commit": BASELINE,
        "factor": args.factor, "matrix": args.matrix,
        "fir_taps": 64 * args.factor + 1 if args.factor else None,
        "candidate_delay_host_samples": 64 if args.factor else 0,
        "source_delay_host_samples": 3 if args.matrix == "performance" else 2,
        "baseline_files_sha256": {name: sha256(data) for name, data in sorted(baseline_files.items())},
        "prepared_files_sha256": {name: sha256(data) for name, data in sorted(files.items())},
        "study_files_sha256": {name: sha256((HERE / name).read_bytes())
                               for name in ("prepare.py", "highband_probe.rs", "performance.rs", "reference.py")},
    }
    args.output.mkdir(parents=True, exist_ok=False)
    for name, data in sorted(files.items()):
        destination = args.output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as output:
            output.write(data)
    with (args.output / "provenance.json").open("x") as output:
        json.dump(provenance, output, indent=2)
        output.write("\n")
    print(f"Prepared factor {args.factor} {args.matrix} study at {args.output}")


if __name__ == "__main__":
    main()
