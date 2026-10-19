#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Experimental one-plugin Rack worker client; does not alter the desktop app."""
from __future__ import annotations

import argparse
from contextlib import AbstractContextManager
import hashlib
import json
import math
import os
from pathlib import Path
import selectors
import struct
import subprocess
import sys
import tempfile
import time
import wave

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DEFAULT_SDK = Path.home() / "Library/Caches/VCVRackMCP/build/ck-sdk/vendor/Rack-SDK"
DEFAULT_PLUGINS = Path.home() / "Library/Application Support/Rack2/plugins-mac-arm64"
DEFAULT_SYSTEM = Path.home() / "Applications/VCV Rack 2 Free.app/Contents/Resources"
BUILD = ROOT / "target/rack-host"


class HostError(RuntimeError):
    pass


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def build(sdk: Path = DEFAULT_SDK) -> Path:
    """Build only our adapter against an existing SDK; never patch a plugin."""
    if sys.platform != "darwin":
        raise HostError("This experimental builder currently supports macOS only")
    sdk = sdk.expanduser().resolve()
    library = sdk / "libRack.dylib"
    if not library.is_file() or not (sdk / "include/rack.hpp").is_file():
        raise HostError("Rack SDK not found; pass --sdk /path/to/Rack-SDK")
    compiler = os.environ.get("CXX", "c++")
    mac_sdk = Path(subprocess.check_output(["xcrun", "--show-sdk-path"], text=True).strip())
    dependencies = [HERE / "worker.cpp", library]
    for directory in (sdk / "include", sdk / "dep/include"):
        dependencies.extend(sorted(p for p in directory.rglob("*") if p.is_file()))
    identity = hashlib.sha256()
    identity.update(subprocess.check_output([compiler, "--version"]))
    identity.update(str(mac_sdk).encode())
    identity.update(Path(__file__).read_bytes())
    for path in dependencies:
        identity.update(str(path).encode())
        identity.update(bytes.fromhex(digest(path)))
    fingerprint = identity.hexdigest()
    # Each build has an immutable name, so replacing a worker cannot invalidate
    # an already running process. This also allows separate SDK installations.
    output = BUILD / f"worker-{fingerprint[:20]}"
    if output.is_file():
        return output
    BUILD.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="build-", dir=BUILD) as directory:
        temporary = Path(directory) / "worker"
        subprocess.run([
            compiler, "-std=c++17", "-O2", "-Wall", "-Wextra",
            # A host uses Engine's private API, unlike a normal Rack plugin.
            "-Wno-deprecated-declarations", "-isysroot", str(mac_sdk),
            "-isystem", str(mac_sdk / "usr/include/c++/v1"),
            "-isystem", str(sdk / "include"), "-isystem", str(sdk / "dep/include"),
            str(HERE / "worker.cpp"), "-L" + str(sdk), "-lRack",
            "-Wl,-rpath," + str(sdk), "-o", str(temporary),
        ], check=True)
        temporary.replace(output)
    return output


def catalog(plugins: Path = DEFAULT_PLUGINS) -> dict:
    """Read manifests only: browsing does not dlopen any plugin binaries."""
    if not plugins.is_dir():
        raise HostError(f"Plugin directory does not exist: {plugins}")
    modules = []
    errors = []
    for manifest in sorted(plugins.glob("*/plugin.json")):
        try:
            data = json.loads(manifest.read_text())
            if not isinstance(data, dict) or not isinstance(data.get("modules"), list):
                raise ValueError("Expected plugin object with a modules array")
            for module in data["modules"]:
                if not isinstance(module, dict):
                    raise ValueError("Expected module object")
                if module.get("hidden"):
                    continue
                modules.append({
                    "plugin": data["slug"], "plugin_name": data.get("name", data["slug"]),
                    "plugin_version": data["version"], "license": data.get("license", "unknown"),
                    "model": module["slug"], "name": module["name"],
                    "description": module.get("description", ""), "tags": module.get("tags", []),
                    "plugin_dir": str(manifest.parent.resolve()),
                })
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append({"manifest": str(manifest), "error": str(error)})
    return {"format": "openpatch-rack-local-catalog", "version": 1,
            "modules": modules, "errors": errors, "plugin_binaries_loaded": 0}


class Worker(AbstractContextManager):
    """Synchronous offline IPC. Never call this from a realtime audio callback."""

    def __init__(self, plugin: Path, model: str, *, sdk: Path = DEFAULT_SDK,
                 system: Path = DEFAULT_SYSTEM, rate: int = 48000,
                 executable: Path | None = None, timeout: float = 15):
        if type(rate) is not int or not 8000 <= rate <= 192000:
            raise HostError("Sample rate must be an integer between 8000 and 192000")
        self.plugin = plugin.expanduser().resolve()
        manifest = self.plugin / "plugin.json"
        binary = self.plugin / "plugin.dylib"
        if not manifest.is_file() or not binary.is_file():
            raise HostError(f"Expected plugin.json and plugin.dylib in {self.plugin}")
        self.binary_hash = digest(binary)
        self.sdk = sdk.expanduser().resolve()
        self.runtime_hash = digest(self.sdk / "libRack.dylib")
        self.system = system.expanduser().resolve()
        self.timeout = timeout
        self._buffer = b""
        self._closed = False
        executable = executable or build(self.sdk)
        self._storage = tempfile.TemporaryDirectory(prefix="openpatch-rack-worker-")
        self._stderr = tempfile.TemporaryFile()
        env = os.environ.copy()
        # dyld resolves plugin imports of /tmp/Rack2/libRack.dylib to this SDK.
        # No shared /tmp/Rack2 link or installed application is changed.
        env["DYLD_LIBRARY_PATH"] = str(self.sdk)
        try:
            self.process = subprocess.Popen([
                str(executable), str(self.plugin), model, str(rate),
                str(self.system), self._storage.name,
            ], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self._stderr,
               bufsize=0, env=env)
            self._selector = selectors.DefaultSelector()
            self._selector.register(self.process.stdout, selectors.EVENT_READ)
            startup = self._read()
            self.info = startup.get("ready")
            if not isinstance(self.info, dict) or self.info.get("protocol") != "openpatch-rack-worker":
                raise HostError("Worker did not complete protocol handshake")
        except BaseException:
            self.close()
            raise

    def _read(self) -> dict:
        deadline = time.monotonic() + self.timeout
        while b"\n" not in self._buffer:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not self._selector.select(remaining):
                self.close()
                raise HostError("Rack worker timed out and was terminated")
            data = os.read(self.process.stdout.fileno(), 65536)
            if not data:
                self._stderr.seek(0)
                error = self._stderr.read(4096).decode(errors="replace")
                self.close()
                raise HostError(f"Rack worker exited: {error.strip()}")
            self._buffer += data
            if len(self._buffer) > 64 * 1024 * 1024:
                self.close()
                raise HostError("Rack worker response exceeded 64 MiB")
        line, self._buffer = self._buffer.split(b"\n", 1)
        try:
            result = json.loads(line)
        except ValueError as error:
            self.close()
            raise HostError("Invalid worker response") from error
        if not isinstance(result, dict):
            self.close()
            raise HostError("Expected worker response object")
        return result

    def request(self, op: str, **values) -> dict:
        if self._closed:
            raise HostError("Rack worker is closed")
        data = json.dumps({"op": op, **values}, allow_nan=False, separators=(",", ":")).encode() + b"\n"
        if len(data) > 1024 * 1024:
            raise HostError("Request exceeds 1 MiB")
        try:
            pending = memoryview(data)
            while pending:
                written = self.process.stdin.write(pending)
                if not written:
                    raise BrokenPipeError("Worker stdin closed")
                pending = pending[written:]
            response = self._read()
        except (BrokenPipeError, OSError) as error:
            self.close()
            raise HostError("Rack worker disconnected") from error
        if "error" in response:
            raise HostError(response["error"])
        if not isinstance(response.get("result"), dict):
            self.close()
            raise HostError("Missing worker result object")
        return response["result"]

    def save(self) -> dict:
        state = self.request("save")
        state["plugin_binary_sha256"] = self.binary_hash
        state["rack_library_sha256"] = self.runtime_hash
        return state

    def restore(self, state: dict):
        if not isinstance(state, dict):
            raise HostError("Saved state must be a JSON object")
        if state.get("plugin_binary_sha256") != self.binary_hash:
            raise HostError("Saved state requires a different plugin binary")
        if state.get("rack_library_sha256") != self.runtime_hash:
            raise HostError("Saved state requires a different Rack runtime")
        self.request("restore", state=state)

    def close(self):
        if self._closed:
            return
        self._closed = True
        process = getattr(self, "process", None)
        if process:
            if process.stdin:
                process.stdin.close()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            if process.stdout:
                process.stdout.close()
        selector = getattr(self, "_selector", None)
        if selector:
            selector.close()
        self._stderr.close()
        self._storage.cleanup()

    def __exit__(self, *_):
        self.close()


def atomic_json(path: Path, value: dict):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", dir=path.parent, delete=False) as temporary:
        pending = Path(temporary.name)
        try:
            json.dump(value, temporary, indent=2, allow_nan=False)
            temporary.write("\n")
            temporary.close()
            pending.replace(path)
        finally:
            pending.unlink(missing_ok=True)


def render(worker: Worker, path: Path, *, seconds: float, outputs: list[int],
           channel: int = 0, gain: float = 0.12, block: int = 256) -> dict:
    """Write selected output lanes as PCM16; full polyphony stays in IPC."""
    if not math.isfinite(seconds) or not 0 < seconds <= 30:
        raise HostError("Render duration must be greater than zero and at most 30 seconds")
    if not outputs or len(outputs) > 2 or not 0 <= channel <= 15:
        raise HostError("WAV supports one or two selected outputs and voice channel 0..15")
    if not math.isfinite(gain) or not 0 < gain <= 1 or not 1 <= block <= 4096:
        raise HostError("Gain must be 0..1 and block length 1..4096")
    frames = round(seconds * worker.info["sample_rate"])
    if frames < 1:
        raise HostError("Render duration is shorter than one sample")
    path.parent.mkdir(parents=True, exist_ok=True)
    peak = squared = clipped = 0
    with tempfile.NamedTemporaryFile(dir=path.parent, suffix=".wav", delete=False) as temporary:
        pending = Path(temporary.name)
    try:
        with wave.open(str(pending), "wb") as wav:
            wav.setnchannels(len(outputs))
            wav.setsampwidth(2)
            wav.setframerate(worker.info["sample_rate"])
            for start in range(0, frames, block):
                result = worker.request("process", frames=min(block, frames-start), outputs=outputs)
                pcm = bytearray()
                for frame in range(result["frames"]):
                    for port in result["outputs"]:
                        voices = port["samples"][frame]
                        value = (voices[channel] if channel < len(voices) else 0) * gain
                        if not math.isfinite(value):
                            raise HostError("Nonfinite audio returned from worker")
                        peak = max(peak, abs(value))
                        squared += value * value
                        clipped += abs(value) > 1
                        pcm.extend(struct.pack("<h", round(max(-1, min(1, value)) * 32767)))
                wav.writeframesraw(pcm)
        pending.replace(path)
    finally:
        pending.unlink(missing_ok=True)
    return {"path": str(path.resolve()), "frames": frames, "channels": len(outputs),
            "sample_rate": worker.info["sample_rate"], "peak": peak,
            "rms": math.sqrt(squared / (frames * len(outputs))), "clipped_samples": clipped}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=DEFAULT_SDK)
    parser.add_argument("--system", type=Path, default=DEFAULT_SYSTEM)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("build")
    configuration = commands.add_parser("configure")
    configuration.add_argument("--plugin", type=Path, default=DEFAULT_PLUGINS / "AudibleInstruments")
    configuration.add_argument("--output", type=Path, default=Path.home() / "OpenPatch/rack-host.json")
    listing = commands.add_parser("catalog")
    listing.add_argument("--plugins", type=Path, default=DEFAULT_PLUGINS)
    listing.add_argument("--query", default="")
    for name in ("describe", "render"):
        command = commands.add_parser(name)
        command.add_argument("--plugin", type=Path, required=True)
        command.add_argument("--model", required=True)
        command.add_argument("--rate", type=int, default=48000)
        command.add_argument("--state", type=Path)
        if name == "render":
            command.add_argument("--output", type=Path, required=True)
            command.add_argument("--seconds", type=float, default=3)
            command.add_argument("--outputs", type=int, nargs="+", default=[0, 1])
            command.add_argument("--channel", type=int, default=0)
            command.add_argument("--gain", type=float, default=0.12)
            command.add_argument("--save-state", type=Path)
    args = parser.parse_args()
    if args.command == "build":
        print(build(args.sdk))
        return
    if args.command == "configure":
        executable = build(args.sdk)
        atomic_json(args.output, {"version": 1, "worker": str(executable),
                    "sdk": str(args.sdk.expanduser().resolve()),
                    "plugin": str(args.plugin.expanduser().resolve()),
                    "system": str(args.system.expanduser().resolve())})
        print(args.output.resolve())
        return
    if args.command == "catalog":
        result = catalog(args.plugins)
        query = args.query.casefold()
        result["modules"] = [m for m in result["modules"] if query in json.dumps(m).casefold()]
    else:
        with Worker(args.plugin, args.model, sdk=args.sdk, system=args.system, rate=args.rate) as worker:
            if args.state:
                worker.restore(json.loads(args.state.read_text()))
            if args.command == "describe":
                result = worker.request("describe")
            else:
                result = render(worker, args.output, seconds=args.seconds,
                                outputs=args.outputs, channel=args.channel, gain=args.gain)
                if args.save_state:
                    atomic_json(args.save_state, worker.save())
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    try:
        main()
    except (HostError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Rack host: {error}", file=sys.stderr)
        sys.exit(1)
