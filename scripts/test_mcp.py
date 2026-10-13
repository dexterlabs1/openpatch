"""Integration smoke tests for the OpenPatch MCP stdio server.

Run from the repository root with ``python -m unittest discover -s scripts``.
Set ``OPENPATCH_BIN`` to override the default release binary location.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import struct
import tempfile
import threading
import unittest
import wave
from pathlib import Path
from typing import Any


REPOSITORY = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = REPOSITORY / "target" / "release" / (
    "openpatch.exe" if os.name == "nt" else "openpatch"
)
PROTOCOL_VERSION = "2025-11-25"
READ_TIMEOUT = 5.0


class MCPProcess:
    """Small line-oriented JSON-RPC client with bounded stdout reads."""

    def __init__(self, executable: Path, workspace: Path) -> None:
        self.process = subprocess.Popen(
            [str(executable), "mcp", "--workspace", str(workspace)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self._stdout: queue.Queue[str | None] = queue.Queue()
        self._reader = threading.Thread(target=self._read_stdout, daemon=True)
        self._reader.start()
        self._next_id = 1

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self._stdout.put(line)
        self._stdout.put(None)

    def send_raw(self, line: str) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def request(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        request_id = self._next_id
        self._next_id += 1
        request: dict[str, Any] = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            request["params"] = params
        self.send_raw(json.dumps(request, separators=(",", ":")))
        line = self.read_line()
        response = json.loads(line)
        if response.get("id") != request_id:
            raise AssertionError(f"response id {response.get('id')!r} did not match {request_id}")
        return response

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        notification: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            notification["params"] = params
        self.send_raw(json.dumps(notification, separators=(",", ":")))

    def read_line(self, timeout: float = READ_TIMEOUT) -> str:
        try:
            line = self._stdout.get(timeout=timeout)
        except queue.Empty as exc:
            code = self.process.poll()
            stderr = self._stderr_text()
            raise AssertionError(
                f"timed out waiting for JSON-RPC output (process exit={code}, stderr={stderr!r})"
            ) from exc
        if line is None:
            raise AssertionError(
                f"MCP process closed stdout unexpectedly (exit={self.process.poll()}, "
                f"stderr={self._stderr_text()!r})"
            )
        return line.rstrip("\r\n")

    def assert_no_output(self, timeout: float = 0.2) -> None:
        try:
            unexpected = self._stdout.get(timeout=timeout)
        except queue.Empty:
            return
        raise AssertionError(f"server emitted output for a notification: {unexpected!r}")

    def _stderr_text(self) -> str:
        # stderr is kept separate so diagnostics are available without mixing it
        # into the JSON-RPC stream. The server should not emit enough to fill a
        # pipe during these short smoke tests.
        if self.process.stderr is None:
            return ""
        return "<stderr captured separately>"

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=3)
        if self.process.stdin is not None:
            self.process.stdin.close()
        if self.process.stdout is not None:
            self.process.stdout.close()
        if self.process.stderr is not None:
            self.process.stderr.close()


class OpenPatchMCPTests(unittest.TestCase):
    def setUp(self) -> None:
        configured_binary = os.environ.get("OPENPATCH_BIN")
        self.executable = Path(configured_binary) if configured_binary else DEFAULT_BINARY
        self.assertTrue(
            self.executable.is_file(),
            f"OpenPatch executable not found at {self.executable}; build it or set OPENPATCH_BIN",
        )
        self.temp_dir = tempfile.TemporaryDirectory(prefix="openpatch-mcp-")
        self.workspace = Path(self.temp_dir.name) / "workspace"
        self.workspace.mkdir()
        self.client = MCPProcess(self.executable.resolve(), self.workspace)

        init = self.client.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "openpatch-smoke-tests", "version": "1.0"},
            },
        )
        self.assertIn("result", init, f"initialize failed: {init}")
        self.assertEqual(init["result"].get("protocolVersion"), PROTOCOL_VERSION)
        self.client.notify("notifications/initialized")
        self.client.assert_no_output()

    def tearDown(self) -> None:
        if hasattr(self, "client"):
            self.client.close()
        if hasattr(self, "temp_dir"):
            self.temp_dir.cleanup()

    def tool(self, name: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
        response = self.client.request(
            "tools/call", {"name": name, "arguments": arguments or {}}
        )
        if "error" in response:
            return {"_jsonrpc_error": response["error"]}
        result = response.get("result")
        if not isinstance(result, dict):
            raise AssertionError(f"tools/call returned no result object: {response}")
        return result

    def require_tool_success(
        self, name: str, arguments: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        result = self.tool(name, arguments)
        self.assertNotIn("_jsonrpc_error", result, f"{name} returned JSON-RPC error: {result}")
        self.assertFalse(result.get("isError", False), f"{name} failed: {result}")
        return result

    def require_tool_error(
        self, name: str, arguments: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        result = self.tool(name, arguments)
        self.assertTrue(
            "_jsonrpc_error" in result or result.get("isError", False),
            f"{name} unexpectedly succeeded: {result}",
        )
        return result

    @staticmethod
    def payload(result: dict[str, Any]) -> dict[str, Any] | None:
        structured = result.get("structuredContent")
        if isinstance(structured, dict):
            return structured
        for item in result.get("content", []):
            if item.get("type") == "text":
                try:
                    decoded = json.loads(item.get("text", ""))
                except (TypeError, json.JSONDecodeError):
                    continue
                if isinstance(decoded, dict):
                    return decoded
        return None

    def patch_snapshot(self) -> dict[str, Any]:
        result = self.require_tool_success("inspect_patch")
        payload = self.payload(result)
        self.assertIsInstance(payload, dict, f"inspect_patch did not return JSON content: {result}")
        assert payload is not None
        self.assertIn("revision", payload)
        self.assertIn("patch", payload)
        return payload

    def add_node(self, node_id: str, kind: str) -> None:
        revision = self.patch_snapshot()["revision"]
        self.require_tool_success(
            "add_module", {"id": node_id, "kind": kind, "expected_revision": revision}
        )

    def make_audio_graph(self) -> None:
        self.add_node("osc", "oscillator")
        self.add_node("gain", "gain")
        self.add_node("out", "output")
        revision = self.patch_snapshot()["revision"]
        self.require_tool_success(
            "connect",
            {"from": "osc", "to": "gain", "input": "signal", "expected_revision": revision},
        )
        revision = self.patch_snapshot()["revision"]
        self.require_tool_success(
            "connect",
            {"from": "gain", "to": "out", "input": "signal", "expected_revision": revision},
        )
        revision = self.patch_snapshot()["revision"]
        self.require_tool_success("set_output", {"id": "out", "expected_revision": revision})
        revision = self.patch_snapshot()["revision"]
        self.require_tool_success(
            "set_parameter",
            {"id": "osc", "parameter": "frequency", "value": 440,
             "expected_revision": revision},
        )

    def test_atomic_preview_commit_and_shared_history(self) -> None:
        before = self.patch_snapshot()
        arguments = {"expected_revision": before["revision"], "edits": [
            {"tool": "add_module", "arguments": {"id": "constant", "kind": "constant", "params": {"value": 0.25}}},
            {"tool": "set_output", "arguments": {"id": "constant"}},
        ]}
        preview = self.payload(self.require_tool_success("preview_edits", arguments))
        self.assertEqual(self.patch_snapshot(), before)
        self.assertFalse(preview["committed"])
        committed = self.payload(self.require_tool_success("apply_edits", arguments))
        self.assertEqual(committed["revision"], before["revision"] + 1)
        self.assertEqual(committed["patch"], preview["patch"])
        self.assertEqual(committed["history"]["undo_steps"], 1)
        self.require_tool_error("apply_edits", arguments)
        self.assertEqual(self.patch_snapshot(), committed)
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "remove_module", "arguments": {"id": "constant"}},
            {"tool": "set_output", "arguments": {"id": "missing"}},
        ]})
        self.assertEqual(self.patch_snapshot(), committed)
        undone = self.payload(self.require_tool_success("undo", {"expected_revision": committed["revision"]}))
        self.assertEqual(undone["patch"], before["patch"])
        redone = self.payload(self.require_tool_success("redo", {"expected_revision": undone["revision"]}))
        self.assertEqual(redone["patch"], committed["patch"])
        self.assertEqual(redone["revision"], committed["revision"] + 2)

    def test_duplicate_groups_copy_explicit_cables_atomically_and_roundtrip(self) -> None:
        patch = {"version": 2, "nodes": [
            {"id": "source", "kind": "poly_constant", "params": {"channels": 2, "v1": 2, "v2": 4}},
            {"id": "octave", "kind": "octave", "params": {"octaves": 1}},
            {"id": "split", "kind": "poly_split"},
            {"id": "sink", "kind": "audio_out"},
        ], "cables": [
            {"from": "source", "to": "octave", "input": "pitch"},
            {"from": "octave", "to": "split", "input": "signal"},
            {"from": "split", "output": "out2", "to": "sink", "input": "signal"},
        ], "output": "split", "output_port": "out2"}
        self.require_tool_success("replace_patch", {"patch": patch})
        mapping = [{"id": "octave", "new_id": "octave_copy"},
                   {"id": "split", "new_id": "split_copy"}]
        for mode, expected_cables in [(None, 3), ("none", 3), ("internal", 4), ("inputs", 5)]:
            before = self.patch_snapshot()
            arguments = {"modules": mapping, "expected_revision": before["revision"]}
            if mode is not None:
                arguments["cable_mode"] = mode
            duplicated = self.payload(self.require_tool_success("duplicate_modules", arguments))
            self.assertEqual(len(duplicated["patch"]["nodes"]), 6)
            self.assertEqual(len(duplicated["patch"]["cables"]), expected_cables)
            self.assertEqual(duplicated["patch"]["output"], "split")
            self.assertEqual(duplicated["patch"]["output_port"], "out2")
            self.assertEqual(duplicated["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
            self.assertFalse(any(c["from"] == "split_copy" and c["to"] == "sink"
                                 for c in duplicated["patch"]["cables"]))
            capture = self.payload(self.require_tool_success("capture_signal", {
                "id": "split_copy", "port": "out2", "seconds": 0.002, "points": 64,
            }))
            self.assertEqual(capture["bins"][-1]["mean"], 5 if mode == "inputs" else 0)
            self.require_tool_error("duplicate_modules", arguments)
            self.assertEqual(self.patch_snapshot(), duplicated)
            self.require_tool_success("undo")
            self.assertEqual(self.patch_snapshot()["patch"], before["patch"])

        before = self.patch_snapshot()
        invalid = [
            {"modules": []},
            {"modules": [{"id": "octave", "new_id": "source"}]},
            {"modules": [{"id": "missing", "new_id": "new"}]},
            {"modules": [{"id": "octave", "new_id": "new", "extra": True}]},
            {"modules": mapping, "cable_mode": "all"},
            {"modules": mapping, "cable_mode": False},
            {"modules": mapping, "include_inputs": True},
            {"modules": [mapping[0], mapping[0]]},
        ]
        for arguments in invalid:
            self.require_tool_error("duplicate_modules", arguments)
            self.assertEqual(self.patch_snapshot(), before)
        edits = [
            {"tool": "duplicate_modules", "arguments": {"modules": mapping, "cable_mode": "inputs"}},
            {"tool": "set_parameter", "arguments": {"id": "octave_copy", "parameter": "octaves", "value": 2}},
        ]
        arguments = {"edits": edits, "expected_revision": before["revision"]}
        preview = self.payload(self.require_tool_success("preview_edits", arguments))
        self.assertEqual(self.patch_snapshot(), before)
        committed = self.payload(self.require_tool_success("apply_edits", arguments))
        self.assertEqual(committed["patch"], preview["patch"])
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], committed["patch"])
        self.require_tool_success("save_patch", {"filename": "duplicated.json"})
        self.require_tool_success("remove_module", {"id": "octave_copy"})
        self.require_tool_success("load_patch", {"filename": "duplicated.json"})
        self.assertEqual(self.patch_snapshot()["patch"], committed["patch"])

    def test_v2_named_ports_polyphony_analysis_and_legacy_loading(self) -> None:
        initial = self.patch_snapshot()
        self.assertEqual(initial["patch"]["version"], 2)
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "add_module", "arguments": {"id": "bus", "kind": "poly_constant", "params": {"channels": 2, "v1": 0.25, "v2": -0.5}}},
            {"tool": "add_module", "arguments": {"id": "split", "kind": "poly_split"}},
            {"tool": "connect", "arguments": {"from": "bus", "to": "split", "input": "signal"}},
            {"tool": "set_output", "arguments": {"id": "split", "port": "out2"}},
        ]})
        before = self.patch_snapshot()
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "bus", "seconds": 0.001}))
        self.assertEqual(measured["max_channels"], 2)
        self.assertEqual([channel["mean"] for channel in measured["channels"]], [0.25, -0.5])
        self.assertEqual(measured["revision"], before["revision"])
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_error("set_output", {"id": "split", "port": "out17"})
        self.require_tool_error("analyze_signal", {"id": "bus", "port": "unknown", "seconds": 0.001})
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_success("save_patch", {"filename": "poly.json"})
        saved = json.loads((self.workspace / "poly.json").read_text())
        self.assertEqual(saved["output_port"], "out2")
        self.require_tool_success("render_audio", {"filename": "split.wav", "seconds": 0.001})
        with wave.open(str(self.workspace / "split.wav")) as rendered:
            samples = self._first_channel_samples(rendered.readframes(48), rendered.getsampwidth(), rendered.getnchannels())
        self.assertEqual(samples[0], 0)
        self.assertAlmostEqual(samples[-1], -16384, delta=1)
        # Old files retain legacy timing and cannot silently acquire polyphony.
        legacy = {"version": 1, "nodes": [{"id": "old", "kind": "constant", "params": {"value": 0.25}}], "output": "old"}
        self.require_tool_success("replace_patch", {"patch": legacy})
        self.require_tool_error("add_module", {"id": "poly", "kind": "poly_constant"})
        restored = self.patch_snapshot()
        self.assertEqual(restored["patch"]["version"], 1)
        self.assertEqual(len(restored["patch"]["nodes"]), 1)

    def test_stereo_render_preserves_channels_and_normalization(self) -> None:
        patch = {"version": 2, "sample_rate": 48000, "nodes": [
            {"id": "left", "kind": "poly_constant", "params": {"channels": 2, "v1": 2, "v2": 3}},
            {"id": "right", "kind": "constant", "params": {"value": -2.5}},
            {"id": "out", "kind": "audio_stereo"}], "cables": [
            {"from": "left", "to": "out", "input": "left"},
            {"from": "right", "to": "out", "input": "right"}], "output": "out"}
        self.require_tool_success("replace_patch", {"patch": patch})
        before = self.patch_snapshot()
        rendered = self.payload(self.require_tool_success("render_audio", {"filename": "stereo.wav", "seconds": 0.001}))
        self.assertEqual(rendered["channels"], 2)
        self.assertEqual(rendered["frames"], 48)
        with wave.open(str(self.workspace / "stereo.wav")) as wav:
            self.assertEqual(wav.getnchannels(), 2)
            self.assertEqual(wav.getnframes(), 48)
            samples = struct.unpack("<96h", wav.readframes(48))
        self.assertEqual(samples[:2], (0, 0))
        self.assertAlmostEqual(samples[-2], 16384, delta=1)
        self.assertAlmostEqual(samples[-1], -8192, delta=1)
        self.assertEqual(self.patch_snapshot(), before)
        patch["cables"].pop()
        self.require_tool_success("replace_patch", {"patch": patch})
        self.require_tool_success("render_audio", {"filename": "normal.wav", "seconds": 0.001})
        with wave.open(str(self.workspace / "normal.wav")) as wav:
            samples = struct.unpack("<96h", wav.readframes(48))
        self.assertEqual(samples[::2], samples[1::2])

    def test_midi_trace_is_sample_timed_repeatable_and_read_only(self) -> None:
        patch = {"version": 2, "sample_rate": 48000,
                 "nodes": [{"id": "midi", "kind": "midi_cv", "params": {"channels": 1}}],
                 "output": "midi", "output_port": "gate"}
        self.require_tool_success("replace_patch", {"patch": patch})
        before = self.patch_snapshot()
        events = [
            {"frame": 12, "event": {"type": "note_on", "channel": 0, "note": 60, "velocity": 100}},
            {"frame": 36, "event": {"type": "note_off", "channel": 0, "note": 60, "velocity": 0}}]
        analyzed = self.payload(self.require_tool_success("analyze_signal", {
            "id": "midi", "port": "gate", "seconds": 0.001, "midi_events": events}))
        self.assertEqual(analyzed["midi_event_count"], 2)
        self.assertEqual(analyzed["channels"][0]["mean"], 5.0)
        for filename in ["first.wav", "second.wav"]:
            self.require_tool_success("render_audio", {"filename": filename, "seconds": 0.001, "midi_events": events})
        self.assertEqual((self.workspace / "first.wav").read_bytes(), (self.workspace / "second.wav").read_bytes())
        with wave.open(str(self.workspace / "first.wav")) as wav:
            samples = struct.unpack("<48h", wav.readframes(48))
        self.assertTrue(all(value == 0 for value in samples[:12] + samples[36:]))
        self.assertTrue(all(value == 32767 for value in samples[12:36]))
        bad_traces = [list(reversed(events)),
                      [{"frame": 48, "event": {"type": "panic"}}],
                      [{"frame": 0, "event": {"type": "panic", "extra": 1}}],
                      [{"frame": 0, "event": {"type": "note_on", "channel": 16, "note": 60, "velocity": 100}}]]
        for trace in bad_traces:
            self.require_tool_error("render_audio", {"filename": "invalid.wav", "seconds": 0.001, "midi_events": trace})
            self.require_tool_error("analyze_signal", {"id": "midi", "port": "gate", "seconds": 0.001, "midi_events": trace})
            self.assertFalse((self.workspace / "invalid.wav").exists())
        self.assertEqual(self.patch_snapshot(), before)

    def test_stereo_width_patch_edits_render_and_undo_through_mcp(self) -> None:
        patch = json.loads((REPOSITORY / "examples" / "stereo-width-study.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch})
        self.require_tool_success("render_audio", {"filename": "wide.wav", "seconds": 0.05})
        with wave.open(str(self.workspace / "wide.wav")) as wav:
            self.assertEqual(wav.getnchannels(), 2)
            samples = struct.unpack("<4800h", wav.readframes(2400))
        self.assertNotEqual(samples[::2], samples[1::2])
        self.require_tool_success("set_parameter", {"id": "width", "parameter": "decode_width", "value": 0})
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "width", "port": "left", "seconds": 0.05}))
        self.assertEqual(measured["max_channels"], 1)
        self.assertGreater(measured["channels"][0]["peak"], 1)
        self.require_tool_success("render_audio", {"filename": "mono.wav", "seconds": 0.05})
        with wave.open(str(self.workspace / "mono.wav")) as wav:
            samples = struct.unpack("<4800h", wav.readframes(2400))
        self.assertEqual(samples[::2], samples[1::2])
        self.require_tool_success("undo")
        self.require_tool_success("render_audio", {"filename": "restored.wav", "seconds": 0.05})
        self.assertEqual((self.workspace / "wide.wav").read_bytes(), (self.workspace / "restored.wav").read_bytes())

    def test_multi_lfo_named_outputs_and_modulation_preset_through_mcp(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog"))
        module = next(m for m in catalog["modules"] if m["kind"] == "multi_lfo")
        self.assertEqual(module["outputs"], ["sine", "triangle", "saw", "square"])
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
            "nodes": [{"id": "lfo", "kind": "multi_lfo", "params": {"frequency": 20}}]}})
        before = self.patch_snapshot()
        for port, rms in [("sine", 5 / 2**0.5), ("triangle", 5 / 3**0.5), ("saw", 5 / 3**0.5), ("square", 5)]:
            measured = self.payload(self.require_tool_success("analyze_signal", {"id": "lfo", "port": port, "seconds": 0.05}))
            self.assertEqual(measured["max_channels"], 1)
            self.assertAlmostEqual(measured["channels"][0]["rms"], rms, delta=0.005)
            self.assertAlmostEqual(measured["channels"][0]["mean"], 0, delta=0.005)
        self.assertEqual(self.patch_snapshot(), before)
        patch = json.loads((REPOSITORY / "examples" / "clocked-modulation.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch})
        self.require_tool_success("render_audio", {"filename": "modulation.wav", "seconds": 0.1})
        with wave.open(str(self.workspace / "modulation.wav")) as wav:
            self.assertEqual(wav.getnchannels(), 2)
            samples = struct.unpack("<9600h", wav.readframes(4800))
        self.assertEqual(samples[::2], samples[1::2])
        self.assertTrue(any(abs(value) > 100 for value in samples))
        self.require_tool_success("set_parameter", {"id": "lfo", "parameter": "frequency", "value": 4})
        self.require_tool_success("undo")
        self.require_tool_success("render_audio", {"filename": "restored.wav", "seconds": 0.1})
        self.assertEqual((self.workspace / "modulation.wav").read_bytes(), (self.workspace / "restored.wav").read_bytes())

    def test_capture_signal_calibration_bins_activity_and_snapshot_safety(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "sample_rate": 48000,
            "nodes": [{"id": "bus", "kind": "poly_constant", "params": {"channels": 2, "v1": 2, "v2": -4}},
                      {"id": "copy", "kind": "mult"}],
            "cables": [{"from": "bus", "to": "copy", "input": "signal"}],
        }})
        before = self.patch_snapshot()
        result = self.payload(self.require_tool_success("capture_signal", {
            "id": "copy", "port": "out8", "channel": 2, "seconds": 0.001,
            "points": 2048, "expected_revision": before["revision"],
        }))
        self.assertEqual(result["revision"], before["revision"])
        self.assertEqual((result["frames"], result["sample_rate"], result["points"]), (48, 48000, 48))
        self.assertEqual(result["units"], "V")
        self.assertEqual(result["amplitude_domain"], "voltage")
        self.assertIsNone(result["digital_full_scale"])
        self.assertEqual(result["summary"], {"active_frames": 47, "min": -4, "max": -4, "dc": -4, "rms": 4, "peak": 4})
        self.assertEqual(result["bins"][0], {"start_frame": 0, "end_frame": 1, "active_frames": 0, "min": None, "max": None, "mean": None})
        self.assertEqual(result["bins"][-1]["end_frame"], 48)
        missing = self.payload(self.require_tool_success("capture_signal", {"id": "bus", "channel": 16, "seconds": 0.001}))
        self.assertEqual(missing["summary"], {"active_frames": 0, "min": None, "max": None, "dc": None, "rms": None, "peak": None})
        for key, value in [("points", 63), ("points", 2049), ("points", 64.0),
                           ("channel", 0), ("channel", 17), ("channel", 1.5),
                           ("seconds", 0), ("seconds", 2.001), ("port", "missing"),
                           ("expected_revision", before["revision"] - 1), ("unknown", True)]:
            arguments = {"id": "bus", "seconds": 0.001}
            arguments[key] = value
            self.require_tool_error("capture_signal", arguments)
        self.assertEqual(self.patch_snapshot(), before)

    def test_capture_signal_preserves_one_sample_midi_peak_and_same_frame_order(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [{"id": "m", "kind": "midi_cv", "params": {"channels": 1}}],
        }})
        before = self.patch_snapshot()
        on = {"frame": 97, "event": {"type": "note_on", "channel": 0, "note": 60, "velocity": 127}}
        off = {"frame": 97, "event": {"type": "note_off", "channel": 0, "note": 60, "velocity": 0}}
        arguments = {"id": "m", "port": "gate", "seconds": 0.01, "points": 64,
                     "midi_events": [off, on, {"frame": 98, "event": off["event"]}]}
        captured = self.payload(self.require_tool_success("capture_signal", arguments))
        self.assertEqual(captured["summary"]["peak"], 10)
        self.assertEqual(captured["summary"]["dc"], 10 / 480)
        pulse = next(item for item in captured["bins"] if item["start_frame"] == 97)
        self.assertEqual((pulse["min"], pulse["max"]), (0, 10))
        arguments["midi_events"] = [on, off]
        released = self.payload(self.require_tool_success("capture_signal", arguments))
        self.assertEqual(released["summary"]["peak"], 0)
        self.assertEqual(self.patch_snapshot(), before)

    def test_handshake_catalog_and_tool_listing(self) -> None:
        listed = self.client.request("tools/list")
        self.assertIn("result", listed, f"tools/list failed: {listed}")
        tools = listed["result"].get("tools")
        self.assertIsInstance(tools, list)
        names = {item.get("name") for item in tools if isinstance(item, dict)}
        self.assertTrue(
            {"catalog", "inspect_patch", "replace_patch", "add_module", "remove_module",
             "connect", "disconnect", "set_parameter", "set_output", "save_patch",
             "load_patch", "render_audio", "benchmark", "apply_edits", "preview_edits",
             "inspect_history", "undo", "redo", "analyze_signal", "capture_signal",
             "trigger_module", "inspect_performance", "rotate_sequence", "set_bypass", "invert_mutes"}.issubset(names),
            f"tools/list omitted documented tools: {names}",
        )
        bypass_tool = next(item for item in tools if item["name"] == "set_bypass")
        self.assertEqual(bypass_tool["inputSchema"]["properties"]["bypassed"], {"type": "boolean"})
        catalog = self.require_tool_success("catalog")
        self.assertTrue(catalog.get("content") or catalog.get("structuredContent"), catalog)
        inspected = self.patch_snapshot()
        self.assertEqual(inspected["patch"].get("sample_rate"), 48000)
        self.assertEqual(inspected["patch"].get("nodes"), [])
        self.assertEqual(inspected["patch"].get("cables"), [])
        self.assertIsNone(inspected["patch"].get("output"))

    def test_audio_render_save_load_and_wav_content(self) -> None:
        self.make_audio_graph()
        snapshot = self.patch_snapshot()
        self.require_tool_success("save_patch", {"filename": "roundtrip.json"})
        self.assertTrue((self.workspace / "roundtrip.json").is_file())

        self.require_tool_success(
            "render_audio", {"filename": "tone.wav", "seconds": 0.1}
        )
        wav_path = self.workspace / "tone.wav"
        self.assertTrue(wav_path.is_file(), "render_audio did not create the requested WAV")
        self.assertGreater(wav_path.stat().st_size, 44, "rendered WAV contains no audio data")
        with wave.open(str(wav_path), "rb") as rendered:
            self.assertEqual(rendered.getframerate(), 48000)
            self.assertGreaterEqual(rendered.getnchannels(), 1)
            self.assertEqual(rendered.getnframes(), 4800)
            width = rendered.getsampwidth()
            channels = rendered.getnchannels()
            frames = rendered.readframes(rendered.getnframes())
        samples = self._first_channel_samples(frames, width, channels)
        self.assertTrue(any(sample != 0 for sample in samples), "rendered audio is silent")
        crossings = sum(
            (left <= 0 < right) or (left >= 0 > right)
            for left, right in zip(samples, samples[1:])
        )
        frequency = crossings / (2 * 0.1)
        self.assertAlmostEqual(frequency, 440, delta=20, msg="rendered tone is not near 440 Hz")

        self.require_tool_success(
            "render_audio", {"filename": "fractional.wav", "seconds": 0.100125}
        )
        with wave.open(str(self.workspace / "fractional.wav"), "rb") as rendered:
            self.assertEqual(rendered.getframerate(), 48000)
            self.assertEqual(rendered.getnframes(), 4806)

        revision = self.patch_snapshot()["revision"]
        self.require_tool_success(
            "set_parameter",
            {"id": "osc", "parameter": "frequency", "value": 660,
             "expected_revision": revision},
        )
        revision = self.patch_snapshot()["revision"]
        self.require_tool_success("load_patch", {"filename": "roundtrip.json",
                                                   "expected_revision": revision})
        restored = self.patch_snapshot()
        self.assertEqual(restored["patch"], snapshot["patch"])

    def test_reference_voice_catalog_edit_and_render(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog"))
        self.assertIsNotNone(catalog)
        modules = {module["kind"]: module for module in catalog["modules"]}
        self.assertEqual(modules["ladder_vcf"]["model"], "circuit-derived")
        self.assertEqual(modules["analog_vco"]["model"], "idealized")
        patch = json.loads((Path(__file__).resolve().parents[1] / "examples" / "reference-voice.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch, "expected_revision": self.patch_snapshot()["revision"]})
        self.require_tool_success("set_parameter", {"id": "filter", "parameter": "resonance", "value": 0.7, "expected_revision": self.patch_snapshot()["revision"]})
        self.require_tool_error("set_parameter", {"id": "vco", "parameter": "waveform", "value": 0.4})
        self.require_tool_success("render_audio", {"filename": "reference.wav", "seconds": 0.1})
        with wave.open(str(self.workspace / "reference.wav"), "rb") as rendered:
            self.assertEqual(rendered.getnframes(), 4800)
            samples = self._first_channel_samples(rendered.readframes(4800), rendered.getsampwidth(), rendered.getnchannels())
        self.assertTrue(any(abs(sample) > 100 for sample in samples))

    def test_seq3_rotation_offline_events_and_headless_live_boundary(self) -> None:
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "add_module", "arguments": {"id": "seq", "kind": "seq3", "params": {
                "tempo": 15, "row1_1": 0.1, "row1_2": 0.2, "row1_3": 0.3, "gate1": 0,
            }}},
            {"tool": "set_output", "arguments": {"id": "seq", "port": "cv1"}},
        ]})
        before = self.patch_snapshot()
        telemetry = self.payload(self.require_tool_success("inspect_performance", {
            "expected_revision": before["revision"],
        }))
        self.assertFalse(telemetry["supported"])
        self.assertIsNone(telemetry["performance"])
        self.require_tool_error("trigger_module", {"id": "seq", "action": "clock"})
        self.require_tool_error("inspect_performance", {"expected_revision": 0})
        trace = [
            {"frame": 10, "id": "seq", "action": "clock"},
            {"frame": 20, "id": "seq", "action": "reset"},
        ]
        args = {"id": "seq", "port": "cv1", "seconds": 0.001,
                "module_events": trace, "expected_revision": before["revision"]}
        captured = self.payload(self.require_tool_success("capture_signal", args))
        self.assertEqual(captured["module_event_count"], 2)
        for frame, point in enumerate(captured["bins"]):
            self.assertAlmostEqual(point["mean"], 0.2 if 10 <= frame < 20 else 0.1, places=6)
        analyzed = self.payload(self.require_tool_success("analyze_signal", args))
        self.assertEqual(analyzed["channels"][0]["mean"], captured["summary"]["dc"])
        self.assertEqual(analyzed["channels"][0]["rms"], captured["summary"]["rms"])
        self.assertEqual(self.payload(self.require_tool_success("capture_signal", args)), captured)
        for filename in ("seq-one.wav", "seq-two.wav"):
            rendered = self.payload(self.require_tool_success("render_audio", {
                "filename": filename, "seconds": 0.001, "module_events": trace,
            }))
            self.assertEqual(rendered["module_event_count"], 2)
        self.assertEqual((self.workspace / "seq-one.wav").read_bytes(),
                         (self.workspace / "seq-two.wav").read_bytes())
        self.assertEqual(self.patch_snapshot(), before)
        invalid_traces = [
            [{"frame": 48, "id": "seq", "action": "clock"}],
            [{"frame": 0, "id": "missing", "action": "clock"}],
            [{"frame": 0, "id": "seq", "action": "clock", "extra": True}],
            [{"frame": 0, "id": "seq", "action": "start"}],
            [{"frame": 1, "id": "seq", "action": "clock"},
             {"frame": 0, "id": "seq", "action": "reset"}],
            [{"frame": 0, "id": "seq", "action": "clock"}] * 65,
        ]
        for invalid_trace in invalid_traces:
            self.require_tool_error("render_audio", {
                "filename": "invalid-seq.wav", "seconds": 0.001, "module_events": invalid_trace,
            })
            self.assertFalse((self.workspace / "invalid-seq.wav").exists())
            self.require_tool_error("capture_signal", {**args, "module_events": invalid_trace})
            self.require_tool_error("analyze_signal", {**args, "module_events": invalid_trace})
        self.assertEqual(self.patch_snapshot(), before)
        rotated = self.payload(self.require_tool_success("rotate_sequence", {
            "id": "seq", "direction": "left", "expected_revision": before["revision"],
        }))
        self.assertEqual(rotated["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        params = rotated["patch"]["nodes"][0]["params"]
        self.assertAlmostEqual(params["row1_1"], 0.2, places=6)
        self.assertAlmostEqual(params["row1_8"], 0.1, places=6)
        self.assertEqual(params["gate8"], 0)
        self.assertEqual(params["gate1"], 1)
        self.require_tool_error("rotate_sequence", {
            "id": "seq", "direction": "left", "expected_revision": before["revision"],
        })
        undone = self.payload(self.require_tool_success("undo", {"expected_revision": rotated["revision"]}))
        self.assertEqual(undone["patch"], before["patch"])

    def test_switched_stereo_voice_routes_channels_and_rejects_sequencer_actions(self) -> None:
        patch = json.loads((REPOSITORY / "examples" / "switched-stereo-voice.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch})
        self.require_tool_error("trigger_module", {"id": "route", "action": "clock"})
        self.require_tool_error("render_audio", {"filename": "invalid-switch.wav", "seconds": 0.01,
            "module_events": [{"frame": 0, "id": "route", "action": "clock"}]})
        self.assertFalse((self.workspace / "invalid-switch.wav").exists())
        self.require_tool_success("render_audio", {"filename": "switches.wav", "seconds": 1})
        with wave.open(str(self.workspace / "switches.wav"), "rb") as rendered:
            self.assertEqual(rendered.getnchannels(), 2)
            self.assertEqual(rendered.getsampwidth(), 2)
            frames = rendered.getnframes()
            data = struct.unpack("<" + "h" * frames * 2, rendered.readframes(frames))
        # Exclude the 2.5 ms transitions: successive half-second routes are isolated.
        for start, stop, audible in [(2400, 21600, 1), (26400, 45600, 0)]:
            selected = data[start * 2 + audible:stop * 2:2]
            silent = data[start * 2 + 1 - audible:stop * 2:2]
            self.assertGreater(max(abs(v) for v in selected), 100)
            self.assertLess(max(abs(v) for v in selected), 32767)
            self.assertTrue(all(v == 0 for v in silent))
        saved = self.patch_snapshot()["patch"]
        self.require_tool_success("save_patch", {"filename": "switches.json"})
        self.require_tool_success("set_parameter", {"id": "route", "parameter": "steps", "value": 2})
        self.require_tool_success("load_patch", {"filename": "switches.json"})
        self.assertEqual(self.patch_snapshot()["patch"], saved)

    def test_voltage_mixing_controls_both_fade_outputs_and_roundtrips_settings(self) -> None:
        patch = {"version": 2, "nodes": [
            {"id": "source", "kind": "constant", "params": {"value": 2}},
            {"id": "other", "kind": "constant", "params": {"value": -0.25}},
            {"id": "cv", "kind": "constant", "params": {"value": 5}},
            {"id": "mixer", "kind": "vca_mix", "params": {"level1": 0.5, "mix_level": 0.5}},
            {"id": "fade", "kind": "fade", "params": {"position": 0.5, "cv_amount": 0.5}},
        ], "cables": [
            {"from": "source", "to": "mixer", "input": "in1"},
            {"from": "mixer", "output": "mix", "to": "fade", "input": "a"},
            {"from": "other", "to": "fade", "input": "b"},
            {"from": "cv", "to": "fade", "input": "cv"},
        ], "output": "fade"}
        self.require_tool_success("replace_patch", {"patch": patch})

        def settled(module: str, port: str) -> float:
            capture = self.payload(self.require_tool_success("capture_signal", {
                "id": module, "port": port, "seconds": 0.002, "points": 64,
            }))
            return capture["bins"][-1]["mean"]

        self.assertEqual(settled("mixer", "out1"), 0.5)
        self.assertEqual(settled("mixer", "mix"), 0.25)
        self.assertEqual(settled("fade", "out"), -0.125)
        self.assertEqual(settled("fade", "complement"), 0.125)
        before = self.patch_snapshot()
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "mixer", "parameter": "channel_exp", "value": 1}},
            {"tool": "set_parameter", "arguments": {"id": "fade", "parameter": "law", "value": 0.5}},
        ]})
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "mixer", "parameter": "channel_exp", "value": 1}},
            {"tool": "set_parameter", "arguments": {"id": "mixer", "parameter": "mix_exp", "value": 1}},
            {"tool": "set_parameter", "arguments": {"id": "fade", "parameter": "law", "value": 1}},
        ]})
        self.assertAlmostEqual(settled("fade", "out"), 0.125 - 0.25 * (0.75 ** 0.5), places=6)
        saved = self.patch_snapshot()["patch"]
        self.require_tool_success("save_patch", {"filename": "mixing.json"})
        self.require_tool_success("set_bypass", {"id": "fade", "bypassed": True})
        self.assertEqual(settled("fade", "out"), 0.25)
        self.assertEqual(settled("fade", "complement"), -0.25)
        self.require_tool_success("load_patch", {"filename": "mixing.json"})
        self.assertEqual(self.patch_snapshot()["patch"], saved)
        self.require_tool_error("trigger_module", {"id": "mixer", "action": "gate_press"})
        demo = json.loads((REPOSITORY / "examples" / "layered-stereo-morph.json").read_text())
        self.require_tool_success("replace_patch", {"patch": demo})
        self.require_tool_success("render_audio", {"filename": "morph.wav", "seconds": 0.2})
        with wave.open(str(self.workspace / "morph.wav"), "rb") as rendered:
            self.assertEqual(rendered.getnchannels(), 2)
            self.assertEqual(rendered.getsampwidth(), 2)
            samples = struct.unpack("<" + "h" * rendered.getnframes() * 2,
                                    rendered.readframes(rendered.getnframes()))
        left, right = samples[0::2], samples[1::2]
        for channel in (left, right):
            self.assertGreater(max(abs(v) for v in channel), 100)
            self.assertLess(max(abs(v) for v in channel), 32767)
        self.assertNotEqual(left, right)

    def test_voltage_utility_demo_mute_and_bypass_render_actual_signal(self) -> None:
        patch = json.loads((REPOSITORY / "examples" / "voltage-utilities.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch})
        def measure(port: str = "out1") -> dict:
            return self.payload(self.require_tool_success("capture_signal", {
                "id": "mutes", "port": port, "seconds": 0.05, "points": 256,
            }))
        before = measure()
        self.assertGreater(before["summary"]["rms"], 0.1)
        self.require_tool_success("set_parameter", {"id": "mutes", "parameter": "mute1", "value": 1})
        self.assertEqual(measure()["summary"]["peak"], 0)
        self.assertGreater(measure("out10")["summary"]["rms"], 0.1)
        self.require_tool_success("set_bypass", {"id": "mutes", "bypassed": True})
        resumed = measure()
        self.assertEqual(resumed["bins"], before["bins"])
        self.assertEqual(measure("out10")["summary"]["peak"], 0)

    def test_bypass_and_mute_inversion_are_atomic_roundtrip_edits(self) -> None:
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "add_module", "arguments": {"id": "mutes", "kind": "mutes10", "params": {"mute2": 1}}},
            {"tool": "add_module", "arguments": {"id": "seq", "kind": "seq3"}},
        ]})
        before = self.patch_snapshot()
        for value in (0, 1, "true", None):
            self.require_tool_error("set_bypass", {"id": "mutes", "bypassed": value})
        self.require_tool_error("set_bypass", {"id": "mutes", "bypassed": True, "unknown": 1})
        self.require_tool_error("invert_mutes", {"id": "seq"})
        self.assertEqual(self.patch_snapshot(), before)
        edits = [
            {"tool": "set_bypass", "arguments": {"id": "mutes", "bypassed": True}},
            {"tool": "invert_mutes", "arguments": {"id": "mutes"}},
        ]
        preview = self.payload(self.require_tool_success("preview_edits", {"edits": edits}))
        self.assertTrue(preview["patch"]["nodes"][0]["bypassed"])
        self.assertEqual(self.patch_snapshot(), before)
        changed = self.payload(self.require_tool_success("apply_edits", {
            "edits": edits, "expected_revision": before["revision"],
        }))
        self.assertEqual(changed["revision"], before["revision"] + 1)
        self.assertEqual(changed["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        self.assertEqual(changed["patch"], preview["patch"])
        node = changed["patch"]["nodes"][0]
        for channel in range(1, 11):
            self.assertEqual(node["params"][f"mute{channel}"], 0 if channel == 2 else 1)
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo")
        stable = self.patch_snapshot()
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "invert_mutes", "arguments": {"id": "mutes"}},
            {"tool": "set_bypass", "arguments": {"id": "missing", "bypassed": True}},
        ]})
        self.require_tool_error("set_bypass", {"id": "mutes", "bypassed": False, "expected_revision": before["revision"]})
        self.assertEqual(self.patch_snapshot(), stable)
        self.require_tool_success("set_parameter", {"id": "mutes", "parameter": "mute3", "value": 0})
        saved = self.patch_snapshot()["patch"]
        self.require_tool_success("save_patch", {"filename": "bypassed.json"})
        self.require_tool_success("set_bypass", {"id": "mutes", "bypassed": False})
        self.assertNotIn("bypassed", self.patch_snapshot()["patch"]["nodes"][0])
        self.require_tool_success("load_patch", {"filename": "bypassed.json"})
        self.assertEqual(self.patch_snapshot()["patch"], saved)
        self.require_tool_success("set_bypass", {"id": "seq", "bypassed": True})
        self.require_tool_success("rotate_sequence", {"id": "seq", "direction": "left"})
        self.require_tool_error("trigger_module", {"id": "seq", "action": "clock"})
        self.require_tool_error("render_audio", {"filename": "bypassed-event.wav", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "seq", "action": "clock"}]})
        self.assertFalse((self.workspace / "bypassed-event.wav").exists())

    def test_manual_action_wire_format_catalog_and_json_roundtrip(self) -> None:
        modules = {item["kind"]: item for item in self.payload(self.require_tool_success("catalog"))["modules"]}
        self.assertEqual(modules["logic"]["runtime_actions"], ["b_press", "b_release"])
        self.assertEqual(modules["process"]["runtime_actions"], ["gate_press", "gate_release"])
        self.assertEqual(modules["compare"]["runtime_actions"], [])
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "add_module", "arguments": {"id": "logic", "kind": "logic"}},
            {"tool": "add_module", "arguments": {"id": "process", "kind": "process"}},
            {"tool": "add_module", "arguments": {"id": "rescale", "kind": "rescale"}},
            {"tool": "add_module", "arguments": {"id": "seq", "kind": "seq3"}},
        ]})
        before = self.patch_snapshot()
        trace = [{"frame": 3, "id": "logic", "action": "b_press"},
                 {"frame": 7, "id": "logic", "action": "b_release"}]
        capture = self.payload(self.require_tool_success("capture_signal", {
            "id": "logic", "port": "or", "seconds": 0.001, "module_events": trace,
        }))
        self.assertEqual(capture["module_event_count"], 2)
        self.assertEqual([point["mean"] for point in capture["bins"]],
                         [10 if 3 <= frame < 7 else 0 for frame in range(48)])
        for id_, action in [("seq", "b_press"), ("logic", "clock"), ("logic", "gate_press"), ("process", "b_press"), ("rescale", "gate_release"),
                            ("logic", {"b_press": None}), ("process", "gate_toggle")]:
            self.require_tool_error("render_audio", {"filename": "invalid-manual.wav", "seconds": 0.001,
                "module_events": [{"frame": 0, "id": id_, "action": action}]})
            self.assertFalse((self.workspace / "invalid-manual.wav").exists())
        process_capture = self.payload(self.require_tool_success("capture_signal", {
            "id": "process", "port": "sh1", "seconds": 0.001, "module_events": [
                {"frame": 3, "id": "process", "action": "gate_press"},
                {"frame": 7, "id": "process", "action": "gate_release"},
            ],
        }))
        self.assertEqual(process_capture["module_event_count"], 2)
        for id_, action in [("logic", "b_release"), ("process", "gate_release")]:
            self.require_tool_error("trigger_module", {"id": id_, "action": action})  # Headless backend.
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_success("save_patch", {"filename": "manual-utilities.json"})
        serialized = json.loads((self.workspace / "manual-utilities.json").read_text())
        self.assertEqual(serialized, before["patch"])
        for node in serialized["nodes"]:
            self.assertNotIn("pressed", node)
            self.assertNotIn("bypassed", node)
            self.assertEqual(node["params"], {})
        self.require_tool_success("set_bypass", {"id": "process", "bypassed": True})
        self.require_tool_error("render_audio", {"filename": "invalid-bypassed-press.wav", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "process", "action": "gate_press"}]})
        self.assertFalse((self.workspace / "invalid-bypassed-press.wav").exists())
        self.require_tool_success("capture_signal", {"id": "process", "port": "sh1", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "process", "action": "gate_release"}]})
        self.require_tool_success("load_patch", {"filename": "manual-utilities.json"})
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])

    def test_mutation_errors_preserve_revision(self) -> None:
        self.make_audio_graph()
        before_invalid = self.patch_snapshot()
        self.require_tool_error(
            "set_parameter",
            {"id": "osc", "parameter": "not_a_real_parameter", "value": 1,
             "expected_revision": before_invalid["revision"]},
        )
        self.assertEqual(self.patch_snapshot(), before_invalid)

        current = self.patch_snapshot()["revision"]
        self.require_tool_success(
            "set_parameter",
            {"id": "osc", "parameter": "frequency", "value": 440,
             "expected_revision": current},
        )
        changed = self.patch_snapshot()
        self.assertGreater(changed["revision"], current)
        self.require_tool_error(
            "set_parameter",
            {"id": "osc", "parameter": "frequency", "value": 880,
             "expected_revision": current},
        )
        self.assertEqual(self.patch_snapshot(), changed)

    def test_unknown_tool_and_filename_safety(self) -> None:
        unknown = self.require_tool_error("there_is_no_such_tool")
        self.assertTrue(unknown.get("_jsonrpc_error") or unknown.get("content"), unknown)

        escape_name = "../openpatch-mcp-escape.json"
        outside_target = self.workspace.parent / "openpatch-mcp-escape.json"
        self.require_tool_error("save_patch", {"filename": escape_name})
        self.assertFalse(outside_target.exists(), "save_patch wrote outside its workspace")

        self.require_tool_success("save_patch", {"filename": "safe.json"})
        safe_path = self.workspace / "safe.json"
        original = safe_path.read_bytes()
        self.require_tool_error("save_patch", {"filename": "safe.json"})
        self.assertEqual(safe_path.read_bytes(), original, "rejected overwrite changed the file")

    def test_visualizer_export_creates_html_and_rejects_overwrite(self) -> None:
        output = self.workspace / "visualizer.html"
        exported = subprocess.run(
            [str(self.executable.resolve()), "visualizer", str(output)],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(exported.returncode, 0, exported.stderr or exported.stdout)
        self.assertTrue(output.is_file(), "visualizer command did not create an HTML file")
        original = output.read_bytes()
        self.assertGreater(len(original), 10_000, "exported visualizer HTML is unexpectedly small")
        source = original.decode("utf-8")
        self.assertIn("Visual Lab", source)
        self.assertIn("u_audio", source)

        overwrite = subprocess.run(
            [str(self.executable.resolve()), "visualizer", str(output)],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        self.assertNotEqual(overwrite.returncode, 0, "visualizer command overwrote an existing file")
        self.assertEqual(output.read_bytes(), original, "rejected export changed the existing file")

    def test_parse_error_recovers_and_notifications_are_silent(self) -> None:
        self.client.send_raw("{this is not json")
        parse_error = json.loads(self.client.read_line())
        self.assertIn("error", parse_error, f"invalid JSON did not yield a parse error: {parse_error}")
        self.assertEqual(parse_error["error"].get("code"), -32700, parse_error)
        recovered = self.client.request("tools/list")
        self.assertIn("result", recovered, f"server did not recover after parse error: {recovered}")
        self.client.notify("notifications/progress", {"token": "smoke", "progress": 1})
        self.client.assert_no_output()

    @staticmethod
    def _first_channel_samples(data: bytes, width: int, channels: int) -> list[int]:
        if width not in (1, 2, 3, 4):
            raise AssertionError(f"unsupported PCM width in WAV: {width}")
        frame_width = width * channels
        samples: list[int] = []
        for offset in range(0, len(data) - frame_width + 1, frame_width):
            raw = data[offset:offset + width]
            if width == 1:
                value = raw[0] - 128
            else:
                value = int.from_bytes(raw, "little", signed=True)
            samples.append(value)
        return samples


if __name__ == "__main__":
    unittest.main()
