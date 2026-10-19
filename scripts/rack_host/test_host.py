#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Qualification against original installed Rack binaries, not mocks or ports."""
import argparse
import copy
import json
import math
from pathlib import Path
import platform
import tempfile
import time
import unittest
from unittest.mock import patch
import wave

import host


def rising_frequency(samples, rate):
    crossings = []
    for i in range(1, len(samples)):
        if samples[i-1] <= 0 < samples[i]:
            crossings.append(i-1-samples[i-1]/(samples[i]-samples[i-1]))
    return (len(crossings)-1)*rate/(crossings[-1]-crossings[0])


class Qualification(unittest.TestCase):
    sdk = host.DEFAULT_SDK
    plugins = host.DEFAULT_PLUGINS
    system = host.DEFAULT_SYSTEM

    @classmethod
    def setUpClass(cls):
        cls.executable = host.build(cls.sdk)

    def worker(self, plugin="Fundamental", model="VCO", rate=48000):
        return host.Worker(self.plugins / plugin, model, sdk=self.sdk,
                           system=self.system, rate=rate, executable=self.executable)

    def test_catalog_reads_manifests_without_loading_binaries(self):
        with patch("subprocess.Popen", side_effect=AssertionError("Catalog attempted to execute code")):
            result = host.catalog(self.plugins)
        self.assertEqual(result["plugin_binaries_loaded"], 0)
        self.assertFalse(result["errors"])
        self.assertTrue(any(m["plugin"] == "AudibleInstruments" and m["model"] == "Plaits"
                            for m in result["modules"]))

    def test_original_vco_pitch_and_polyphonic_width(self):
        for rate in (44100, 48000, 96000):
            with self.subTest(rate=rate), self.worker(rate=rate) as worker:
                self.assertEqual(worker.info["plugin"], "Fundamental")
                worker.request("set_inputs", inputs=[{"port": 0, "voltages": [0, 1, 2]}])
                trace = worker.request("process", frames=4096, outputs=[0])["outputs"][0]["samples"]
                self.assertTrue(all(len(frame) == 3 for frame in trace))
                for channel, expected in enumerate((261.625565, 523.25113, 1046.50226)):
                    values = [frame[channel] for frame in trace]
                    measured = rising_frequency(values[128:], rate)
                    self.assertLess(abs(measured / expected - 1), 0.003)
                    self.assertGreater(max(map(abs, values)), 4)
                    self.assertLessEqual(max(map(abs, values)), 5.1)
                for width in (16, 1, 0, 5, 16):
                    worker.request("set_inputs", inputs=[{"port": 0, "voltages": [0.1*i for i in range(width)]}])
                    out = worker.request("process", frames=17, outputs=[0])["outputs"][0]["samples"]
                    self.assertTrue(all(len(frame) == max(1, width) for frame in out))
                    self.assertTrue(all(math.isfinite(v) for frame in out for v in frame))

    def test_audio_is_independent_of_ipc_block_boundaries(self):
        traces = []
        for partitions in ([1024], [1, 3, 17, 129, 256, 512, 106]):
            with self.worker() as worker:
                worker.request("set_inputs", inputs=[{"port": 0, "voltages": [0, 0.5, 1]}])
                trace = []
                for length in partitions:
                    result = worker.request("process", frames=length, outputs=[0, 2])
                    trace.extend(zip(result["outputs"][0]["samples"], result["outputs"][1]["samples"]))
                traces.append(trace)
        self.assertEqual(traces[0], traces[1])

    def test_plaits_custom_state_and_parameters_survive_process_restart(self):
        directory = self.plugins / "AudibleInstruments"
        before = host.digest(directory / "plugin.dylib")
        with self.worker("AudibleInstruments", "Plaits") as worker:
            self.assertEqual([p["name"] for p in worker.info["outputs"]], ["Main", "Auxiliary"])
            self.assertEqual(len(worker.info["params"]), 11)
            state = worker.save()
            state["module"]["data"] = {"model": 4, "lowCpu": True}
            worker.restore(state)
            worker.request("set_params", params=[{"id": 3, "value": 0.23}, {"id": 4, "value": 0.66}, {"id": 5, "value": 0.37}])
            worker.request("set_inputs", inputs=[{"port": 7, "voltages": [0, 4/12, 7/12, 1]}])
            expected = worker.save()
        with self.worker("AudibleInstruments", "Plaits") as worker:
            worker.restore(expected)
            actual = worker.save()
            self.assertEqual(actual, expected)
            result = worker.request("process", frames=4096, outputs=[0, 1])
            for output in result["outputs"]:
                self.assertTrue(all(len(frame) == 4 for frame in output["samples"]))
                values = [v for frame in output["samples"] for v in frame]
                self.assertTrue(all(math.isfinite(v) for v in values))
                self.assertGreater(math.sqrt(sum(v*v for v in values)/len(values)), 0.02)
            worker.request("set_inputs", inputs=[{"port": 7, "voltages": [i/12 for i in range(16)]}])
            result = worker.request("process", frames=513, outputs=[0, 1])
            for output in result["outputs"]:
                self.assertTrue(all(len(frame) == 16 for frame in output["samples"]))
                self.assertTrue(all(math.isfinite(v) for frame in output["samples"] for v in frame))
        self.assertEqual(host.digest(directory / "plugin.dylib"), before)

    def test_plaits_engine_selection_changes_original_dsp(self):
        traces = []
        for model in (0, 2, 4, 5):
            with self.worker("AudibleInstruments", "Plaits") as worker:
                state = worker.save()
                state["module"]["data"]["model"] = model
                worker.restore(state)
                trace = worker.request("process", frames=4096, outputs=[0])["outputs"][0]["samples"]
                traces.append([frame[0] for frame in trace])
        for a in range(len(traces)):
            for b in range(a):
                difference = math.sqrt(sum((x-y)**2 for x,y in zip(traces[a],traces[b]))/len(traces[a]))
                self.assertGreater(difference, 0.05)

    def test_invalid_commands_leave_controls_unchanged_and_worker_alive(self):
        with self.worker() as worker:
            before = worker.save()
            with self.assertRaises(host.HostError):
                worker.restore([])
            bad = [
                ("set_params", {"params": [{"id": 2, "value": 12}, {"id": 5, "value": 7}]}),
                ("set_params", {"params": [{"id": 2, "value": 12}, {"id": 2, "value": 14}]}),
                ("set_inputs", {"inputs": [{"port": 0, "voltages": [1]}, {"port": 1, "voltages": [0]*17}]}),
                ("process", {"frames": 4097, "outputs": [0]}),
                ("process", {"frames": 1, "outputs": [999]}),
                ("process", {"frames": 1, "outputs": [0, 0]}),
                ("unknown", {}),
            ]
            for operation, values in bad:
                with self.subTest(operation=operation, values=values), self.assertRaises(host.HostError):
                    worker.request(operation, **values)
                self.assertEqual(worker.save(), before)
            # Malformed JSON reaches the real worker parser, not Python's parser.
            worker.process.stdin.write(b'{"op": "describe", "op": "save"}\n')
            self.assertIn("error", worker._read())
            worker.process.stdin.write(b'not json\n')
            self.assertIn("error", worker._read())
            self.assertEqual(worker.request("describe")["model"], "VCO")

    def test_restore_rejects_mismatched_identity_version_runtime_and_params(self):
        with self.worker() as worker:
            before = worker.save()
            for field in ("plugin", "model", "plugin_version", "plugin_binary_sha256", "rack_library_sha256", "format"):
                changed = copy.deepcopy(before)
                changed[field] = "mismatch"
                with self.subTest(field=field), self.assertRaises(host.HostError):
                    worker.restore(changed)
                self.assertEqual(worker.save(), before)
            for mutate in (
                lambda state: state.update(sample_rate=44100),
                lambda state: state.update(version=2),
                lambda state: state["module"].update(plugin="different"),
                lambda state: state["module"]["params"][2].update(value=1000),
                lambda state: state.update(inputs=[]),
            ):
                changed = copy.deepcopy(before)
                mutate(changed)
                with self.assertRaises(host.HostError):
                    worker.restore(changed)
                self.assertEqual(worker.save(), before)

    def test_worker_failure_is_contained_and_new_worker_recovers(self):
        with self.worker() as worker:
            worker.process.kill()
            worker.process.wait()
            with self.assertRaises(host.HostError):
                worker.request("describe")
        with self.worker() as worker:
            self.assertEqual(worker.request("process", frames=3, outputs=[0])["frames"], 3)
        with self.assertRaises(host.HostError):
            self.worker(model="does-not-exist")

    def test_wav_export_reports_real_audio_and_failure_preserves_existing_file(self):
        with tempfile.TemporaryDirectory() as directory, self.worker("AudibleInstruments", "Plaits") as worker:
            path = Path(directory) / "preview.wav"
            result = host.render(worker, path, seconds=0.1, outputs=[0, 1], block=129)
            self.assertEqual(result["frames"], 4800)
            self.assertGreater(result["rms"], 0.01)
            self.assertEqual(result["clipped_samples"], 0)
            with wave.open(str(path)) as wav:
                self.assertEqual((wav.getnchannels(), wav.getframerate(), wav.getnframes()), (2, 48000, 4800))
            before = path.read_bytes()
            with self.assertRaises(host.HostError):
                host.render(worker, path, seconds=0.1, outputs=[999])
            self.assertEqual(path.read_bytes(), before)
            self.assertEqual(list(Path(directory).iterdir()), [path])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, default=host.DEFAULT_SDK)
    parser.add_argument("--plugins", type=Path, default=host.DEFAULT_PLUGINS)
    parser.add_argument("--system", type=Path, default=host.DEFAULT_SYSTEM)
    parser.add_argument("--report", type=Path, default=host.ROOT / "docs/benchmarks/rack-host-qualification.json")
    parser.add_argument("--preview", type=Path)
    args = parser.parse_args()
    Qualification.sdk = args.sdk.resolve()
    Qualification.plugins = args.plugins.resolve()
    Qualification.system = args.system.resolve()
    binaries = {p: host.digest(args.plugins / p / "plugin.dylib") for p in ("Fundamental", "AudibleInstruments")}
    started = time.monotonic()
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Qualification)
    names = [test.id() for test in suite]
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    unchanged = all(host.digest(args.plugins / p / "plugin.dylib") == value for p,value in binaries.items())
    report = {
        "format": "openpatch-rack-host-qualification", "version": 1,
        "platform": platform.platform(), "machine": platform.machine(),
        "sdk_library_sha256": host.digest(args.sdk / "libRack.dylib"),
        "worker_source_sha256": host.digest(host.HERE / "worker.cpp"),
        "client_source_sha256": host.digest(host.HERE / "host.py"),
        "qualification_source_sha256": host.digest(Path(__file__)),
        "plugin_binary_sha256": binaries,
        "plugin_versions": {p: json.loads((args.plugins/p/"plugin.json").read_text())["version"] for p in binaries},
        "plugin_binaries_unchanged": unchanged, "tests": names,
        "tests_run": result.testsRun, "failures": len(result.failures), "errors": len(result.errors),
        "elapsed_seconds": time.monotonic()-started,
        "local_catalog_modules": len(host.catalog(args.plugins)["modules"]),
        "limitations": [
            "Offline one-module worker; no desktop integration or real-time audio qualification",
            "No original module UI, expanders, inter-module graph, MIDI or file-backed module storage",
            "Module JSON preserves Rack preset state, not unsaved oscillator/DSP phase histories",
            "Only original Fundamental VCO and Audible Instruments Plaits binaries qualified",
            "macOS arm64 only; no library service authentication or remote plugin installation",
        ],
    }
    if args.preview and result.wasSuccessful() and unchanged:
        with host.Worker(args.plugins / "AudibleInstruments", "Plaits", sdk=args.sdk,
                         system=args.system, executable=Qualification.executable) as worker:
            state = worker.save()
            state["module"]["data"]["model"] = 4
            worker.restore(state)
            worker.request("set_params", params=[{"id": 3, "value": 0.35}, {"id": 4, "value": 0.3}, {"id": 5, "value": 0.6}])
            worker.request("set_inputs", inputs=[{"port": 7, "voltages": [0]}])
            report["preview"] = host.render(worker, args.preview, seconds=4, outputs=[0, 1])
            host.atomic_json(args.preview.with_suffix(".rack-module.json"), worker.save())
    host.atomic_json(args.report, report)
    if not result.wasSuccessful() or not unchanged:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
