"""Integration smoke tests for the OpenPatch MCP stdio server.

Run from the repository root with ``python -m unittest discover -s scripts``.
Set ``OPENPATCH_BIN`` to override the default release binary location.
"""

from __future__ import annotations

import json
import math
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

    def test_score_edit_roundtrip_and_bounded_arrangement_render(self) -> None:
        score = {"length":4,"tracks":[{"steps":[{"step":0,"notes":[0]}]}],"lanes":[{"points":[{"bar":0,"value":0.5}]}]}
        self.require_tool_success("replace_patch", {"patch":{"version":2,"sample_rate":8000,"nodes":[{"id":"score","kind":"score_sequencer","score":score}],"output":"score","output_port":"automation"}})
        before = self.patch_snapshot()
        changed = dict(score, length=8)
        self.require_tool_success("set_score", {"id":"score","score":changed})
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_error("set_score", {"id":"score","score":{"length":0}})
        self.require_tool_success("save_patch", {"filename":"score.json"})
        result = self.payload(self.require_tool_success("render_arrangement", {"filename":"arrangement.wav","seconds":31}))
        self.assertEqual(result["stats"]["frames"], 248000)
        self.assertEqual(result["stats"]["channels"], 1)
        self.assertEqual(result["stats"]["clipped_samples"], 0)
        with wave.open(str(self.workspace / "arrangement.wav"), "rb") as wav:
            self.assertEqual(wav.getnframes(), 248000)
            self.assertEqual(wav.getnchannels(), 1)
        self.require_tool_error("render_arrangement", {"filename":"arrangement.wav","seconds":1})
        self.require_tool_error("render_arrangement", {"filename":"too-long.wav","seconds":301})
        self.assertFalse((self.workspace / "too-long.wav").exists())

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

    def test_cable_rgb_schema_persistence_and_malformed_color_rollback(self) -> None:
        tools = self.client.request("tools/list")["result"]["tools"]
        schema = next(tool["inputSchema"] for tool in tools if tool["name"] == "connect")
        self.assertEqual(schema["properties"]["color"], {
            "type": "array", "minItems": 3, "maxItems": 3,
            "items": {"type": "integer", "minimum": 0, "maximum": 255},
        })
        self.assertNotIn("color", schema["required"])
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [
                {"id": "source", "kind": "constant", "params": {"value": 0.25}},
                {"id": "sink", "kind": "gain"},
            ], "output": "sink",
        }})
        before = self.patch_snapshot()
        arguments = {"from": "source", "to": "sink", "input": "signal",
                     "color": [0, 128, 255], "expected_revision": before["revision"]}
        committed = self.payload(self.require_tool_success("connect", arguments))
        self.assertEqual(self.patch_snapshot(), committed)
        cable = committed["patch"]["cables"][0]
        self.assertEqual(cable["color"], [0, 128, 255])
        self.assertEqual(cable["id"], "cable_1")
        self.require_tool_success("save_patch", {"filename": "colored.json"})
        saved = json.loads((self.workspace / "colored.json").read_text())
        self.assertEqual(saved, committed["patch"])
        self.assertEqual(self.patch_snapshot(), committed, "saving must preserve history")

        undone = self.payload(self.require_tool_success("undo", {
            "expected_revision": committed["revision"],
        }))
        self.assertEqual(undone["patch"], before["patch"])
        self.assertEqual(undone["history"]["redo_steps"], 1)
        for color in (None, False, "#0080ff", {}, [], [0, 1], [0, 1, 2, 3],
                      [-1, 0, 0], [0, 256, 0], [0, 0.5, 0], [0, True, 0], [0, "1", 0]):
            with self.subTest(color=color):
                self.require_tool_error("connect", {
                    **arguments, "color": color, "expected_revision": undone["revision"],
                })
                self.assertEqual(self.patch_snapshot(), undone,
                                 "invalid RGB must preserve patch, revision, undo and redo")
        self.require_tool_error("apply_edits", {"expected_revision": undone["revision"], "edits": [
            {"tool": "connect", "arguments": {
                "from": "source", "to": "sink", "input": "cv", "color": [255, 0, 0],
            }},
            {"tool": "connect", "arguments": {
                "from": "source", "to": "sink", "input": "signal", "color": [0, 256, 0],
            }},
        ]})
        self.assertEqual(self.patch_snapshot(), undone, "a malformed RGB must roll back the batch")
        redone = self.payload(self.require_tool_success("redo", {
            "expected_revision": undone["revision"],
        }))
        self.assertEqual(redone["patch"], committed["patch"])
        self.assertEqual(self.patch_snapshot()["patch"]["cables"][0], cable)
        self.assertEqual(json.loads((self.workspace / "colored.json").read_text()), saved)

    def test_exact_cable_edit_tools_preserve_identity_history_and_atomicity(self) -> None:
        listed = self.client.request("tools/list")["result"]["tools"]
        schemas = {tool["name"]: tool["inputSchema"] for tool in listed}
        for name in ("disconnect_cables", "move_cables"):
            self.assertEqual(schemas[name]["properties"]["cable_ids"], {
                "type": "array", "minItems": 1, "maxItems": 1024, "uniqueItems": True,
                "items": schemas["set_cable_color"]["properties"]["cable_id"],
            })
        self.assertEqual(schemas["move_cables"]["properties"]["end"]["enum"], ["input", "output"])
        self.assertEqual(schemas["move_cables"]["properties"]["duplicate_mode"]["enum"],
                         ["reject", "discard", "stack"])
        self.assertEqual(schemas["set_cable_color"]["properties"]["color"],
                         schemas["connect"]["properties"]["color"])
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [
                {"id": "a", "kind": "constant"}, {"id": "b", "kind": "constant"},
                {"id": "left", "kind": "gain"}, {"id": "right", "kind": "gain"},
                {"id": "target", "kind": "gain"},
            ], "cables": [
                {"id": "first", "from": "a", "to": "left", "input": "signal", "color": [1, 2, 3]},
                {"id": "second", "from": "b", "to": "right", "input": "signal", "color": [4, 5, 6]},
                {"id": "existing", "from": "a", "to": "target", "input": "signal", "color": [7, 8, 9]},
            ], "output": "target",
        }})
        before = self.patch_snapshot()
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "set_cable_color", "arguments": {"cable_id": "second", "color": [0, 128, 255]}},
            {"tool": "move_cables", "arguments": {
                "cable_ids": ["first"], "end": "input", "node": "target", "port": "signal",
            }},
        ]})
        self.assertEqual(self.patch_snapshot(), before, "duplicate rejection must roll back prior color edits")
        colored = self.payload(self.require_tool_success("set_cable_color", {
            "cable_id": "second", "color": [0, 128, 255], "expected_revision": before["revision"],
        }))
        move = {"cable_ids": ["second"], "end": "input", "node": "target", "port": "signal"}
        preview = self.payload(self.require_tool_success("preview_edits", {
            "edits": [{"tool": "move_cables", "arguments": move}],
        }))
        self.assertEqual(self.patch_snapshot(), colored)
        moved = self.payload(self.require_tool_success("move_cables", {
            **move, "expected_revision": colored["revision"],
        }))
        self.assertEqual(moved["patch"], preview["patch"])
        self.assertEqual([c["id"] for c in moved["patch"]["cables"]], ["first", "second", "existing"])
        self.assertEqual(moved["patch"]["cables"][1]["color"], [0, 128, 255])
        expected_sibling = dict(before["patch"]["cables"][2])
        expected_sibling["input_order"] = moved["patch"]["cables"][2]["input_order"]
        self.assertEqual(moved["patch"]["cables"][2], expected_sibling)
        self.require_tool_success("save_patch", {"filename": "cable-edits.json"})
        self.assertEqual(json.loads((self.workspace / "cable-edits.json").read_text()), moved["patch"])
        self.require_tool_success("undo")
        undone = self.patch_snapshot()
        self.assertEqual(undone["patch"], colored["patch"])
        self.assertEqual(self.payload(self.require_tool_success("set_cable_color", {
            "cable_id": "second", "color": [0, 128, 255], "expected_revision": undone["revision"],
        })), undone)
        self.assertEqual(self.payload(self.require_tool_success("move_cables", {
            "cable_ids": ["first"], "end": "input", "node": "left", "port": "signal",
            "duplicate_mode": "discard", "expected_revision": undone["revision"],
        })), undone)
        for name, args in [
            ("set_cable_color", {"cable_id": "second", "color": [0, 256, 0]}),
            ("set_cable_color", {"cable_id": "second", "color": [0, 128, 255], "expected_revision": 0}),
            ("disconnect_cables", {"cable_ids": ["first", "missing"]}),
            ("disconnect_cables", {"cable_ids": ["first", "first"]}),
            ("move_cables", {**move, "node": "a", "port": "out", "duplicate_mode": "discard"}),
            ("move_cables", {**move, "expected_revision": 0}),
        ]:
            with self.subTest(tool=name, arguments=args):
                self.require_tool_error(name, args)
                self.assertEqual(self.patch_snapshot(), undone)
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], moved["patch"])
        discarded = self.payload(self.require_tool_success("move_cables", {
            "cable_ids": ["first"], "end": "input", "node": "target", "port": "signal",
            "duplicate_mode": "discard",
        }))
        self.assertEqual([c["id"] for c in discarded["patch"]["cables"]], ["second", "existing"])
        cleared = self.payload(self.require_tool_success("disconnect_cables", {
            "cable_ids": ["existing", "second"], "expected_revision": discarded["revision"],
        }))
        self.assertEqual(cleared["patch"]["cables"], [])
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], discarded["patch"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], cleared["patch"])

    def test_independent_endpoint_order_promote_persistence_and_noop_redo(self) -> None:
        schemas = {tool["name"]: tool["inputSchema"]
                   for tool in self.client.request("tools/list")["result"]["tools"]}
        self.assertEqual(schemas["promote_cables"]["properties"]["cable_ids"]["maxItems"], 1024)
        self.assertEqual(schemas["promote_cables"]["properties"]["end"]["enum"], ["input", "output"])
        self.assertEqual(schemas["move_cables"]["properties"]["promote"], {"type": "boolean", "default": False})
        self.assertEqual(schemas["move_cables"]["properties"]["plug_order"],
                         schemas["promote_cables"]["properties"]["cable_ids"])
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [{"id": "source", "kind": "constant", "params": {"value": 1}},
                                     {"id": "sink", "kind": "gain"}], "cables": [
                {"id": "first", "from": "source", "to": "sink", "input": "signal",
                 "input_order": 65535, "output_order": 0},
                {"id": "second", "from": "source", "to": "sink", "input": "signal",
                 "input_order": 0, "output_order": 65535},
            ], "output": "sink",
        }})
        before = self.patch_snapshot()
        edits = [{"tool": "promote_cables", "arguments": {"cable_ids": ["first"], "end": "output"}}]
        preview = self.payload(self.require_tool_success("preview_edits", {"edits": edits}))
        self.assertEqual(self.patch_snapshot(), before)
        self.assertEqual(preview["changes"]["added_cables"], [])
        self.assertEqual(preview["changes"]["removed_cables"], [])
        self.assertEqual(preview["changes"]["changed_cables"], [])
        self.assertFalse(preview["changes"]["cable_order_changed"])
        self.assertTrue(preview["changes"]["changed_cable_order"])
        promoted = self.payload(self.require_tool_success("apply_edits", {"edits": edits}))
        self.assertEqual(promoted["patch"], preview["patch"])
        self.assertEqual([c["id"] for c in promoted["patch"]["cables"]], ["first", "second"])
        self.assertEqual([c["input_order"] for c in promoted["patch"]["cables"]], [65535, 0])
        self.assertGreater(promoted["patch"]["cables"][0]["output_order"],
                           promoted["patch"]["cables"][1]["output_order"])
        self.require_tool_success("save_patch", {"filename": "plug-order.json"})
        self.assertEqual(json.loads((self.workspace / "plug-order.json").read_text()), promoted["patch"])
        capture = self.payload(self.require_tool_success("capture_signal", {"id": "sink", "seconds": 0.002}))
        self.assertEqual(capture["bins"][-1]["mean"], 2)
        self.require_tool_success("undo")
        undone = self.patch_snapshot()
        self.assertEqual(undone["patch"], before["patch"])
        self.assertEqual(self.payload(self.require_tool_success("promote_cables", {
            "cable_ids": ["second"], "end": "output", "expected_revision": undone["revision"],
        })), undone)
        self.assertEqual(undone["history"]["redo_steps"], 1)
        for args in ({"cable_ids": ["first", "first"], "end": "output"},
                     {"cable_ids": ["first", "missing"], "end": "output"},
                     {"cable_ids": ["first"], "end": "output", "expected_revision": 0}):
            self.require_tool_error("promote_cables", args)
            self.assertEqual(self.patch_snapshot(), undone)
        self.require_tool_error("move_cables", {"cable_ids": ["first", "second"], "end": "output",
            "node": "source", "port": "out", "promote": True, "plug_order": ["first"]})
        self.assertEqual(self.patch_snapshot(), undone)
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], promoted["patch"])
        self.require_tool_success("load_patch", {"filename": "plug-order.json"})
        self.assertEqual(self.patch_snapshot()["patch"], promoted["patch"])
        # A successful same-origin move explicitly chooses visual bundle order
        # without changing routes or the independent input-end order.
        loaded = self.patch_snapshot()
        move = {"cable_ids": ["second", "first"], "end": "output", "node": "source",
                "port": "out", "promote": True, "plug_order": ["first", "second"]}
        preview = self.payload(self.require_tool_success("preview_edits", {
            "edits": [{"tool": "move_cables", "arguments": move}],
        }))
        self.assertTrue(preview["changes"]["changed_cable_order"])
        self.assertEqual(preview["changes"]["added_cables"], [])
        self.assertEqual(preview["changes"]["removed_cables"], [])
        moved = self.payload(self.require_tool_success("move_cables", {
            **move, "expected_revision": loaded["revision"],
        }))
        self.assertEqual(moved["patch"], preview["patch"])
        self.assertEqual([c["input_order"] for c in moved["patch"]["cables"]], [65535, 0])
        self.assertLess(moved["patch"]["cables"][0]["output_order"],
                        moved["patch"]["cables"][1]["output_order"])
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], loaded["patch"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], moved["patch"])

    def test_clear_cables_schema_empty_noop_stale_guard_and_redo_preservation(self) -> None:
        tools = self.client.request("tools/list")["result"]["tools"]
        schema = next(tool["inputSchema"] for tool in tools if tool["name"] == "clear_cables")
        self.assertEqual(set(schema["properties"]), {"expected_revision"})
        self.assertFalse(schema["additionalProperties"])
        self.assertFalse(schema.get("required", []))
        empty = self.patch_snapshot()
        self.assertEqual(self.payload(self.require_tool_success("clear_cables")), empty)
        self.assertEqual(self.patch_snapshot(), empty)
        self.add_node("temporary", "constant")
        changed = self.patch_snapshot()
        self.require_tool_success("undo")
        undone = self.patch_snapshot()
        self.assertEqual(undone["patch"], empty["patch"])
        self.assertEqual(undone["history"]["redo_steps"], 1)
        self.assertEqual(self.payload(self.require_tool_success("clear_cables", {
            "expected_revision": undone["revision"],
        })), undone)
        self.require_tool_error("clear_cables", {"expected_revision": changed["revision"]})
        self.require_tool_error("clear_cables", {"to": "temporary"})
        self.assertEqual(self.patch_snapshot(), undone)
        redone = self.payload(self.require_tool_success("redo"))
        self.assertEqual(redone["patch"], changed["patch"])

    def test_clear_cables_v1_and_maximum_v2_stack_preview_undo_and_file_roundtrip(self) -> None:
        for version, count in [(1, 2), (2, 1024)]:
            with self.subTest(version=version, cables=count):
                patch = {"version": version, "sample_rate": 48000, "nodes": [
                    {"id": "source", "kind": "constant", "params": {"value": 0.25}},
                    {"id": "sink", "kind": "gain", "params": {"gain": 0.5}, "bypassed": True},
                    {"id": "other", "kind": "gain"},
                ], "cables": [
                    {"id": f"route_{index}", "from": "source", "output": "out",
                     "to": "sink" if version == 2 or index == 0 else "other", "input": "signal"}
                    for index in range(count)
                ], "output": "sink", "output_port": "out"}
                # replace_patch exercises the full cable limit without a 256-edit batch.
                self.require_tool_success("replace_patch", {"patch": patch})
                before = self.patch_snapshot()
                request = {"expected_revision": before["revision"], "edits": [
                    {"tool": "clear_cables", "arguments": {}},
                ]}
                preview = self.payload(self.require_tool_success("preview_edits", request))
                self.assertFalse(preview["committed"])
                self.assertEqual(len(preview["changes"]["removed_cables"]), count)
                self.assertEqual(preview["changes"]["added_cables"], [])
                self.assertEqual(self.patch_snapshot(), before)
                cleared = self.payload(self.require_tool_success("clear_cables", {
                    "expected_revision": before["revision"],
                }))
                expected = dict(before["patch"], cables=[])
                self.assertEqual(cleared["patch"], expected)
                self.assertEqual(cleared["patch"], preview["patch"])
                self.assertEqual(cleared["revision"], before["revision"] + 1)
                self.assertEqual(cleared["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
                self.require_tool_error("clear_cables", {"expected_revision": before["revision"]})
                self.assertEqual(self.patch_snapshot(), cleared)
                filename = f"cleared-v{version}.json"
                self.require_tool_success("save_patch", {"filename": filename})
                saved = json.loads((self.workspace / filename).read_text())
                self.assertEqual(saved, expected)
                undone = self.payload(self.require_tool_success("undo"))
                self.assertEqual(undone["patch"], before["patch"], "one undo restores every ID and cable order")
                redone = self.payload(self.require_tool_success("redo"))
                self.assertEqual(redone["patch"], expected)
                self.require_tool_success("undo")
                loaded = self.payload(self.require_tool_success("load_patch", {"filename": filename}))
                self.assertEqual(loaded["patch"], expected)

    def test_clear_cables_transaction_preview_rollback_and_empty_subedit_commit(self) -> None:
        self.make_audio_graph()
        before = self.patch_snapshot()
        for tool in ("preview_edits", "apply_edits"):
            self.require_tool_error(tool, {"expected_revision": before["revision"], "edits": [
                {"tool": "clear_cables", "arguments": {}},
                {"tool": "set_parameter", "arguments": {
                    "id": "missing", "parameter": "gain", "value": 1,
                }},
            ]})
            self.assertEqual(self.patch_snapshot(), before)
        request = {"expected_revision": before["revision"], "edits": [
            {"tool": "clear_cables", "arguments": {}},
            {"tool": "connect", "arguments": {
                "from": "osc", "to": "out", "input": "signal", "cable_id": "replacement_route",
            }},
        ]}
        preview = self.payload(self.require_tool_success("preview_edits", request))
        self.assertEqual(self.patch_snapshot(), before)
        committed = self.payload(self.require_tool_success("apply_edits", request))
        self.assertEqual(committed["patch"], preview["patch"])
        self.assertEqual([c["id"] for c in committed["patch"]["cables"]], ["replacement_route"])
        self.assertEqual(committed["revision"], before["revision"] + 1)
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], committed["patch"])
        self.require_tool_success("clear_cables")
        empty = self.patch_snapshot()
        empty_request = {"expected_revision": empty["revision"], "edits": [
            {"tool": "clear_cables", "arguments": {}},
        ]}
        empty_preview = self.payload(self.require_tool_success("preview_edits", empty_request))
        self.assertEqual(empty_preview["patch"], empty["patch"])
        self.assertEqual(self.patch_snapshot(), empty)
        empty_committed = self.payload(self.require_tool_success("apply_edits", empty_request))
        self.assertEqual(empty_committed["patch"], empty["patch"])
        self.assertEqual(empty_committed["revision"], empty["revision"] + 1)
        self.assertEqual(empty_committed["history"]["undo_steps"], empty["history"]["undo_steps"] + 1)

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
            {"modules": mapping, "cable_mode": "outputs"},
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

    def test_duplicate_all_preserves_outgoing_stacks_and_sums_actual_sink_audio(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [
                {"id": "source", "kind": "poly_constant", "params": {"channels": 2, "v1": 1, "v2": 2}},
                {"id": "bus", "kind": "mult"}, {"id": "sink", "kind": "audio_out"},
            ], "cables": [
                {"id": "input", "from": "source", "to": "bus", "input": "signal"},
                {"id": "output1", "from": "bus", "output": "out2", "to": "sink", "input": "signal"},
                {"id": "output2", "from": "bus", "output": "out2", "to": "sink", "input": "signal"},
            ], "output": "sink",
        }})
        before = self.patch_snapshot()

        def sink_channels() -> list[float]:
            return [self.payload(self.require_tool_success("capture_signal", {
                "id": "sink", "channel": channel, "seconds": 0.002, "points": 64,
            }))["bins"][-1]["mean"] for channel in (1, 2)]

        for actual, expected in zip(sink_channels(), (0.2, 0.4)):
            self.assertAlmostEqual(actual, expected, places=6)
        arguments = {"modules": [{"id": "bus", "new_id": "bus_copy"}], "cable_mode": "all"}
        request = {"edits": [{"tool": "duplicate_modules", "arguments": arguments}],
                   "expected_revision": before["revision"]}
        preview = self.payload(self.require_tool_success("preview_edits", request))
        self.assertEqual(self.patch_snapshot(), before)
        duplicated = self.payload(self.require_tool_success("apply_edits", request))
        self.assertEqual(duplicated["patch"], preview["patch"])
        self.assertEqual(duplicated["patch"]["cables"][:3], before["patch"]["cables"])
        copied = duplicated["patch"]["cables"][3:]
        self.assertEqual([(c["from"], c.get("output", "out"), c["to"], c["input"]) for c in copied], [
            ("source", "out", "bus_copy", "signal"),
            ("bus_copy", "out2", "sink", "signal"),
            ("bus_copy", "out2", "sink", "signal"),
        ])
        ids = [c["id"] for c in duplicated["patch"]["cables"]]
        self.assertEqual(len(set(ids)), 6)
        self.assertEqual(duplicated["patch"]["output"], "sink")
        for actual, expected in zip(sink_channels(), (0.4, 0.8)):
            self.assertAlmostEqual(actual, expected, places=6)
        rendered = self.payload(self.require_tool_success("render_audio", {"filename": "all-outgoing.wav", "seconds": 0.002}))
        self.assertAlmostEqual(rendered["peak"], 0.4, places=6)
        with wave.open(str(self.workspace / "all-outgoing.wav"), "rb") as wav:
            self.assertEqual(wav.getnchannels(), 1)
            frames = wav.readframes(wav.getnframes())
        self.assertAlmostEqual(struct.unpack("<h", frames[-2:])[0] / 32767, 0.4, places=4)
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], duplicated["patch"])
        stable = self.patch_snapshot()
        self.require_tool_error("duplicate_modules", {**arguments, "expected_revision": before["revision"]})
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "duplicate_modules", "arguments": {"modules": [{"id": "bus", "new_id": "failed"}], "cable_mode": "all"}},
            {"tool": "set_output", "arguments": {"id": "missing"}},
        ]})
        self.assertEqual(self.patch_snapshot(), stable)
        schemas = self.client.request("tools/list")["result"]["tools"]
        schema = next(tool["inputSchema"] for tool in schemas if tool["name"] == "duplicate_modules")
        self.assertIn("all", schema["properties"]["cable_mode"]["enum"])

    def test_duplicate_all_version_one_rejects_outgoing_stack_without_partial_edit(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 1, "nodes": [{"id": "source", "kind": "constant", "params": {"value": 0.25}},
                                       {"id": "sink", "kind": "output"}],
            "cables": [{"from": "source", "to": "sink", "input": "signal"}], "output": "sink",
        }})
        before = self.patch_snapshot()
        self.require_tool_error("duplicate_modules", {
            "modules": [{"id": "source", "new_id": "source_copy"}], "cable_mode": "all",
        })
        self.assertEqual(self.patch_snapshot(), before)
        copied_sink = self.payload(self.require_tool_success("duplicate_modules", {
            "modules": [{"id": "sink", "new_id": "sink_copy"}], "cable_mode": "all",
        }))
        self.assertEqual(len(copied_sink["patch"]["cables"]), 2)
        self.assertEqual(copied_sink["patch"]["cables"][-1]["to"], "sink_copy")
        self.assertEqual(copied_sink["patch"]["output"], "sink")

    def test_stacked_cables_sum_polyphony_and_edit_by_identity_atomically(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "mono", "kind": "constant", "params": {"value": 1}},
            {"id": "poly", "kind": "poly_constant", "params": {"channels": 3, "v1": 2, "v2": -2, "v3": 4}},
            {"id": "bus", "kind": "mult"}, {"id": "other", "kind": "mult"},
        ], "cables": [{"from": "poly", "to": "bus", "input": "signal"}]}})
        before = self.patch_snapshot()
        original_id = before["patch"]["cables"][0]["id"]
        self.require_tool_error("connect", {"from": "mono", "to": "bus", "input": "signal"})
        self.assertEqual(self.patch_snapshot(), before)
        edits = [{"tool": "connect", "arguments": {
            "from": source, "to": "bus", "input": "signal", "mode": "stack", "cable_id": cable_id,
        }} for source, cable_id in [("mono", "mono_route"), ("poly", "duplicate_route")]]
        request = {"edits": edits, "expected_revision": before["revision"]}
        preview = self.payload(self.require_tool_success("preview_edits", request))
        self.assertEqual(self.patch_snapshot(), before)
        stacked = self.payload(self.require_tool_success("apply_edits", request))
        self.assertEqual(stacked["patch"], preview["patch"])
        self.assertEqual([c["id"] for c in stacked["patch"]["cables"]],
                         [original_id, "mono_route", "duplicate_route"])
        self.assertEqual(stacked["history"]["undo_steps"], before["history"]["undo_steps"] + 1)

        def voltages(module: str = "bus") -> list[float]:
            return [self.payload(self.require_tool_success("capture_signal", {
                "id": module, "port": "out8", "channel": channel, "seconds": 0.002, "points": 64,
            }))["bins"][-1]["mean"] for channel in [1, 2, 3]]

        self.assertEqual(voltages(), [5, -3, 9], "mono broadcasts, and duplicate routes contribute twice")
        copied = self.payload(self.require_tool_success("duplicate_modules", {
            "modules": [{"id": "bus", "new_id": "bus_copy"}], "cable_mode": "inputs",
        }))
        cable_ids = [c["id"] for c in copied["patch"]["cables"]]
        self.assertEqual(len(cable_ids), 6)
        self.assertEqual(len(set(cable_ids)), 6)
        self.assertEqual(voltages("bus_copy"), [5, -3, 9])
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], stacked["patch"])
        self.require_tool_success("disconnect", {"cable_id": "mono_route"})
        remaining = self.patch_snapshot()
        self.assertEqual([c["id"] for c in remaining["patch"]["cables"]], [original_id, "duplicate_route"])
        self.assertEqual(voltages(), [4, -4, 8])
        moved = self.payload(self.require_tool_success("apply_edits", {"edits": [
            {"tool": "disconnect", "arguments": {"cable_id": "duplicate_route"}},
            {"tool": "connect", "arguments": {"cable_id": "duplicate_route", "from": "poly", "to": "other", "input": "signal"}},
        ]}))
        self.assertEqual(voltages(), [2, -2, 4])
        self.assertEqual(voltages("other"), [2, -2, 4])
        self.assertEqual(moved["patch"]["cables"][-1]["id"], "duplicate_route")
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], remaining["patch"])
        stable = self.patch_snapshot()
        reordered = self.payload(self.require_tool_success("preview_edits", {"edits": [
            {"tool": "disconnect", "arguments": {"cable_id": original_id}},
            {"tool": "connect", "arguments": {"cable_id": original_id, "from": "poly", "to": "bus", "input": "signal", "mode": "stack"}},
        ]}))
        self.assertTrue(reordered["changes"]["cable_order_changed"])
        self.assertEqual([c["id"] for c in reordered["patch"]["cables"]], ["duplicate_route", original_id])
        self.assertEqual(self.patch_snapshot(), stable)
        for arguments in [{"cable_id": original_id, "to": "bus", "input": "signal"},
                          {"cable_id": "missing"}, {"to": "bus"}, {}]:
            self.require_tool_error("disconnect", arguments)
            self.assertEqual(self.patch_snapshot(), stable)
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "disconnect", "arguments": {"cable_id": original_id}},
            {"tool": "connect", "arguments": {"cable_id": "duplicate_route", "from": "mono", "to": "bus", "input": "signal", "mode": "stack"}},
        ]})
        self.assertEqual(self.patch_snapshot(), stable)
        self.require_tool_success("save_patch", {"filename": "stack.json"})
        self.require_tool_success("disconnect", {"to": "bus", "input": "signal"})
        self.assertEqual(self.patch_snapshot()["patch"]["cables"], [])
        self.require_tool_success("load_patch", {"filename": "stack.json"})
        self.assertEqual(self.patch_snapshot()["patch"], stable["patch"])
        self.assertEqual(voltages(), [4, -4, 8])

    def test_legacy_cable_ids_migrate_without_rewriting_source_or_changing_v1_routing(self) -> None:
        legacy = {"nodes": [{"id": "dc", "kind": "constant", "params": {"value": 0.25}},
                            {"id": "out", "kind": "gain", "params": {"gain": 1}}],
                  "cables": [{"from": "dc", "to": "out", "input": "signal"}], "output": "out"}
        raw = json.dumps(legacy).encode()
        (self.workspace / "legacy-cables.json").write_bytes(raw)
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "legacy-cables.json"}))
        self.assertEqual(loaded["patch"]["version"], 1)
        self.assertEqual(loaded["patch"]["cables"][0]["id"], "cable_1")
        self.assertEqual((self.workspace / "legacy-cables.json").read_bytes(), raw)
        result = self.payload(self.require_tool_success("capture_signal", {"id": "out", "seconds": 0.001, "points": 64}))
        self.assertEqual(result["summary"]["dc"], 0.25)
        self.assertEqual(result["summary"]["active_frames"], 48)
        self.require_tool_error("connect", {"from": "dc", "to": "out", "input": "signal", "mode": "stack"})
        self.assertEqual(self.patch_snapshot(), loaded)
        self.require_tool_success("save_patch", {"filename": "migrated-cables.json"})
        self.assertEqual(json.loads((self.workspace / "migrated-cables.json").read_text()), loaded["patch"])

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

    def test_poly_sum_level_contract_edits_and_persistence(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog", {}))
        module = next(m for m in catalog["modules"] if m["kind"] == "poly_sum")
        self.assertEqual(module["params"]["level"]["default"], 1)
        self.assertEqual(len(module["light_names"]), 6)
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "sample_rate": 48000, "nodes": [
                {"id": "bus", "kind": "poly_constant", "params": {
                    "channels": 16, **{f"v{i}": 1 for i in range(1, 17)}}},
                {"id": "sum", "kind": "poly_sum"}],
            "cables": [{"from": "bus", "to": "sum", "input": "signal"}],
            "output": "sum"}})
        unity = self.patch_snapshot()
        self.require_tool_success("set_parameter", {
            "id": "sum", "parameter": "level", "value": 0.25,
            "expected_revision": unity["revision"]})
        attenuated = self.patch_snapshot()
        measured = self.payload(self.require_tool_success("analyze_signal", {
            "id": "sum", "seconds": 0.001}))
        self.assertEqual(measured["max_channels"], 1)
        self.assertEqual(measured["channels"][0]["peak"], 4)
        self.assertEqual(self.patch_snapshot(), attenuated)
        self.require_tool_error("set_parameter", {
            "id": "sum", "parameter": "level", "value": 1.01})
        self.assertEqual(self.patch_snapshot(), attenuated)
        self.require_tool_success("save_patch", {"filename": "sum.json"})
        saved = json.loads((self.workspace / "sum.json").read_text())
        self.assertEqual(saved["nodes"][1]["params"]["level"], 0.25)
        self.require_tool_success("undo", {})
        self.assertEqual(self.patch_snapshot()["patch"], unity["patch"])
        self.require_tool_success("redo", {})
        self.assertEqual(self.patch_snapshot()["patch"], attenuated["patch"])

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

    def test_project_capture_is_read_only_and_exposes_offline_run_provenance(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "a", "kind": "seq3"},
            {"id": "b", "kind": "seq3", "params": {"running": 0}},
        ]}})
        before = self.patch_snapshot()
        captured = self.payload(self.require_tool_success("capture_project_patch", {
            "expected_revision": before["revision"],
        }))
        self.assertEqual(captured["revision"], before["revision"])
        self.assertEqual(captured["state_source"], "offline_reset")
        self.assertIsNone(captured["frame"])
        self.assertEqual(captured["running"], {"a": True, "b": False})
        self.assertEqual([node["params"]["running"] for node in captured["patch"]["nodes"]], [1, 0])
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_error("capture_project_patch", {"expected_revision": before["revision"] + 1})
        self.require_tool_error("capture_project_patch", {"filename": "unexpected.openpatch"})
        self.assertFalse((self.workspace / "unexpected.openpatch").exists())
        self.assertEqual(self.patch_snapshot(), before)

    def test_native_module_preset_capture_exact_save_partial_apply_and_history(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [
                {"id": "vco", "kind": "multi_vco", "bypassed": True,
                 "params": {"frequency": 220, "pulse_width": 0.99}},
                {"id": "seq", "kind": "seq3", "params": {"running": 0, "row1_1": 3}},
            ], "cables": [{"id": "pitch", "from": "seq", "output": "cv1", "to": "vco", "input": "pitch"}],
            "output": "vco", "output_port": "sine",
        }})
        before = self.patch_snapshot()
        captured = self.payload(self.require_tool_success("capture_module_preset", {
            "id": "vco", "expected_revision": before["revision"],
        }))
        self.assertEqual(captured["state_source"], "saved_parameters")
        self.assertIsNone(captured["frame"])
        self.assertEqual(captured["preset"]["params"]["frequency"], 220)
        self.assertEqual(captured["preset"]["params"]["fm_amount"], 0)
        self.assertTrue(captured["preset"]["bypassed"])
        self.assertNotIn("id", captured["preset"])
        self.assertEqual(self.patch_snapshot(), before)
        offline = self.payload(self.require_tool_success("capture_module_preset", {"id": "seq"}))
        self.assertEqual(offline["state_source"], "offline_reset")
        self.assertEqual(offline["preset"]["params"]["running"], 0)
        self.require_tool_success("set_parameter", {"id": "vco", "parameter": "frequency", "value": 440})
        changed = self.patch_snapshot()
        saved = self.payload(self.require_tool_success("save_module_preset", {
            "filename": "reviewed-vco.opreset", "preset": captured["preset"],
        }))
        self.assertEqual(saved["preset"], captured["preset"])
        self.assertEqual(self.patch_snapshot(), changed)
        read = self.payload(self.require_tool_success("read_module_preset", {"filename": "reviewed-vco.opreset"}))
        self.assertEqual(read["preset"], captured["preset"])
        self.assertEqual(json.loads((self.workspace / "reviewed-vco.opreset").read_text()), captured["preset"])
        # Later disk edits do not change the reviewed inline operation.
        replacement = {"format": "openpatch-module-preset", "version": 1, "kind": "multi_vco", "params": {"frequency": 880}}
        (self.workspace / "reviewed-vco.opreset").write_text(json.dumps(replacement))
        request = {"edits": [{"tool": "apply_module_preset", "arguments": {"id": "vco", "preset": read["preset"]}}],
                   "expected_revision": changed["revision"]}
        preview = self.payload(self.require_tool_success("preview_edits", request))
        self.assertEqual(self.patch_snapshot(), changed)
        applied = self.payload(self.require_tool_success("apply_edits", request))
        self.assertEqual(applied["patch"], preview["patch"])
        self.assertEqual(applied["patch"]["nodes"][0]["params"]["frequency"], 220)
        self.assertEqual(applied["patch"]["cables"], before["patch"]["cables"])
        self.assertEqual(applied["patch"]["output"], "vco")
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], changed["patch"])
        self.assertEqual(self.payload(self.require_tool_success("redo"))["patch"], applied["patch"])
        partial = {"format": "openpatch-module-preset", "version": 1, "kind": "seq3", "params": {"row2_1": 7}}
        merged = self.payload(self.require_tool_success("apply_module_preset", {"id": "seq", "preset": partial}))
        seq = next(node for node in merged["patch"]["nodes"] if node["id"] == "seq")
        self.assertEqual(seq["params"]["row1_1"], 3)
        self.assertEqual(seq["params"]["row2_1"], 7)
        self.assertEqual(seq["params"]["running"], 0)

    def test_native_preset_library_search_validation_and_file_safety(self) -> None:
        value = {"format": "openpatch-module-preset", "version": 1, "kind": "multi_vco", "params": {"pulse_width": 0.01}}
        for filename in ("a-vco.opreset", "z-vco.opreset"):
            self.require_tool_success("save_module_preset", {"filename": filename, "preset": value})
        (self.workspace / "broken.opreset").write_text("not json")
        listing = self.payload(self.require_tool_success("list_module_presets", {"limit": 1}))
        self.assertEqual(listing["total"], 3)
        self.assertEqual(listing["presets"][0]["filename"], "a-vco.opreset")
        searched = self.payload(self.require_tool_success("list_module_presets", {"query": "Z-VCO", "limit": 1, "kind": "multi_vco"}))
        self.assertEqual(searched["total"], 1)
        self.assertEqual(searched["presets"][0]["filename"], "z-vco.opreset")
        broken = self.payload(self.require_tool_success("list_module_presets", {"query": "broken"}))
        self.assertFalse(broken["presets"][0]["valid"])
        self.assertIsNotNone(broken["presets"][0]["error"])
        before = self.patch_snapshot()
        for filename in ("a-vco.opreset", "../escape.opreset", ".hidden.opreset", "bad name.opreset", "wrong.json"):
            self.require_tool_error("save_module_preset", {"filename": filename, "preset": value})
        (self.workspace / "large.opreset").write_bytes(b" " * 65537)
        self.require_tool_error("read_module_preset", {"filename": "large.opreset"})
        (self.workspace / "directory.opreset").mkdir()
        self.require_tool_error("read_module_preset", {"filename": "directory.opreset"})
        if os.name != "nt":
            (self.workspace / "link.opreset").symlink_to(self.workspace / "a-vco.opreset")
            self.require_tool_error("read_module_preset", {"filename": "link.opreset"})
            self.require_tool_error("save_module_preset", {"filename": "link.opreset", "preset": value})
        for arguments in ({"limit": 0}, {"limit": 101}, {"offset": -1}, {"query": "x" * 121}, {"query": True}):
            self.require_tool_error("list_module_presets", arguments)
        for invalid in (
            {**value, "version": 2}, {**value, "bypassed": 1}, {**value, "id": "forbidden"},
            {**value, "params": {"soft_sync": 0.5}}, {**value, "params": {"pulse_width": 2}},
            {**value, "params": {"unknown": 1}},
        ):
            self.require_tool_error("save_module_preset", {"filename": "invalid.opreset", "preset": invalid})
            self.assertFalse((self.workspace / "invalid.opreset").exists())
        self.assertEqual(self.patch_snapshot(), before)
        self.assertFalse(any(path.name.startswith(".openpatch-preset-") for path in self.workspace.iterdir()))
        schemas = self.client.request("tools/list")["result"]["tools"]
        names = {tool["name"] for tool in schemas}
        self.assertTrue({"capture_module_preset", "save_module_preset", "read_module_preset", "list_module_presets", "apply_module_preset"} <= names)
        save_schema = next(tool["inputSchema"] for tool in schemas if tool["name"] == "save_module_preset")
        self.assertNotIn("id", save_schema["properties"])
        self.assertFalse(save_schema["properties"]["preset"]["additionalProperties"])

    def test_initialize_modules_atomic_preview_and_parameter_history(self) -> None:
        patch = {
            "version": 2,
            "nodes": [
                {"id": "vco", "kind": "multi_vco", "bypassed": True,
                 "params": {"frequency": 440, "fm_amount": 0.5, "pwm_amount": -0.5}},
                {"id": "seq", "kind": "seq3",
                 "params": {"running": 0, "tempo": 240, "row1_1": 3, "gate1": 0}},
            ],
            "cables": [{"id": "pitch", "from": "seq", "output": "cv1",
                        "to": "vco", "input": "pitch"}],
            "output": "vco", "output_port": "sine",
        }
        self.require_tool_success("replace_patch", {"patch": patch})
        before = self.patch_snapshot()
        edits = [{"tool": "initialize_modules", "arguments": {"ids": ["vco", "seq"]}}]
        request = {"edits": edits, "expected_revision": before["revision"]}
        preview = self.payload(self.require_tool_success("preview_edits", request))
        self.assertEqual(self.patch_snapshot(), before)
        committed = self.payload(self.require_tool_success("apply_edits", request))
        self.assertEqual(committed["patch"], preview["patch"])
        self.assertEqual(committed["revision"], before["revision"] + 1)
        self.assertEqual(committed["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        nodes = {node["id"]: node for node in committed["patch"]["nodes"]}
        self.assertTrue(nodes["vco"]["bypassed"])
        self.assertEqual(nodes["vco"]["params"]["fm_amount"], 0)
        self.assertEqual(nodes["seq"]["params"]["running"], 0)
        self.assertEqual(nodes["seq"]["params"]["row1_1"], 0)
        self.assertEqual(nodes["seq"]["params"]["gate1"], 1)
        for key in ("cables", "output", "output_port"):
            self.assertEqual(committed["patch"][key], before["patch"][key])
        undone = self.payload(self.require_tool_success("undo"))
        self.assertEqual(undone["patch"], before["patch"])
        redone = self.payload(self.require_tool_success("redo"))
        self.assertEqual(redone["patch"], committed["patch"])
        # Even initialization at saved defaults remains an explicit operation.
        initialized = self.payload(self.require_tool_success("initialize_modules", {"ids": ["seq"]}))
        self.assertEqual(initialized["revision"], redone["revision"] + 1)
        self.assertEqual(initialized["patch"], redone["patch"])

    def test_randomize_modules_seed_policy_strict_arguments_and_rollback(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [
                {"id": "a", "kind": "multi_vco", "bypassed": True,
                 "params": {"frequency": 220, "fm_amount": 0.25, "pwm_amount": -0.5}},
                {"id": "b", "kind": "seq3",
                 "params": {"tempo": 240, "steps": 4, "running": 0, "passthrough": 1}},
            ], "cables": [], "output": "a", "output_port": "sine",
        }})
        before = self.patch_snapshot()
        arguments = {"ids": ["b", "a"], "seed": 4294967295}
        edits = [{"tool": "randomize_modules", "arguments": arguments}]
        preview = self.payload(self.require_tool_success("preview_edits", {"edits": edits}))
        self.assertEqual(self.patch_snapshot(), before)
        committed = self.payload(self.require_tool_success("randomize_modules", arguments))
        self.assertEqual(committed["patch"], preview["patch"])
        nodes = {node["id"]: node for node in committed["patch"]["nodes"]}
        self.assertTrue(nodes["a"]["bypassed"])
        self.assertEqual(nodes["a"]["params"]["fm_amount"], 0.25)
        self.assertEqual(nodes["a"]["params"]["pwm_amount"], -0.5)
        for key, value in (("tempo", 240), ("steps", 4), ("running", 0), ("passthrough", 1)):
            self.assertEqual(nodes["b"]["params"][key], value)
        self.assertNotEqual(committed["patch"]["nodes"], before["patch"]["nodes"])
        self.require_tool_success("undo")
        reordered = self.payload(self.require_tool_success("randomize_modules", {
            "ids": ["a", "b"], "seed": 4294967295,
        }))
        self.assertEqual(reordered["patch"], committed["patch"])
        stable = self.patch_snapshot()
        for arguments in (
            {"ids": []}, {"ids": ["a", "a"]}, {"ids": ["a", "missing"]},
            {"ids": "a"}, {"ids": [False]}, {"ids": ["a"], "extra": True},
            {"ids": ["a"], "expected_revision": 0},
        ):
            self.require_tool_error("initialize_modules", arguments)
            self.assertEqual(self.patch_snapshot(), stable)
        for seed in (-1, 4294967296, 0.5, True, None, "1"):
            self.require_tool_error("randomize_modules", {"ids": ["a"], "seed": seed})
            self.assertEqual(self.patch_snapshot(), stable)
        self.require_tool_error("randomize_modules", {"ids": ["a"]})
        for tool in ("preview_edits", "apply_edits"):
            self.require_tool_error(tool, {"edits": [
                {"tool": "initialize_modules", "arguments": {"ids": ["a"]}},
                {"tool": "set_parameter", "arguments": {"id": "missing", "parameter": "frequency", "value": 440}},
            ]})
            self.assertEqual(self.patch_snapshot(), stable)
        schemas = self.client.request("tools/list")["result"]["tools"]
        for name in ("initialize_modules", "randomize_modules"):
            schema = next(tool["inputSchema"] for tool in schemas if tool["name"] == name)
            self.assertFalse(schema["additionalProperties"])
            self.assertTrue(schema["properties"]["ids"]["uniqueItems"])
            self.assertEqual(schema["properties"]["ids"]["maxItems"], 256)

    def test_multi_vco_catalog_named_outputs_and_version_boundary(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog"))
        modules = {module["kind"]: module for module in catalog["modules"]}
        module = modules["multi_vco"]
        self.assertEqual(module["model"], "functional")
        self.assertEqual(module["min_patch_version"], 2)
        self.assertEqual(module["inputs"], ["pitch", "fm", "sync", "pwm"])
        self.assertEqual(module["outputs"], ["sine", "triangle", "saw", "square"])
        self.assertEqual(set(module["params"]), {
            "frequency", "fm_amount", "pulse_width", "pwm_amount", "linear_fm", "soft_sync",
        })
        for flag in ("linear_fm", "soft_sync"):
            self.assertEqual([module["params"][flag][key] for key in ("default", "min", "max", "step")], [0, 0, 1, 1])
        self.assertEqual(modules["analog_vco"]["min_patch_version"], 1)
        self.require_tool_success("replace_patch", {"patch": {
            "version": 1, "nodes": [{"id": "old", "kind": "constant"}], "output": "old",
        }})
        legacy = self.patch_snapshot()
        self.require_tool_error("add_module", {"id": "vco", "kind": "multi_vco"})
        self.require_tool_error("replace_patch", {"patch": {
            "version": 1, "nodes": [{"id": "vco", "kind": "multi_vco"}],
            "output": "vco", "output_port": "sine",
        }})
        self.assertEqual(self.patch_snapshot(), legacy)
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "sample_rate": 48000,
            "nodes": [{"id": "vco", "kind": "multi_vco", "params": {"frequency": 220}}],
            "output": "vco", "output_port": "sine",
        }})
        before = self.patch_snapshot()
        traces = []
        for port in module["outputs"]:
            captured = self.payload(self.require_tool_success("capture_signal", {
                "id": "vco", "port": port, "seconds": 0.01, "points": 128,
                "expected_revision": before["revision"],
            }))
            self.assertEqual(captured["port"], port)
            self.assertEqual(captured["summary"]["active_frames"], 480)
            self.assertGreater(captured["summary"]["rms"], 0)
            self.assertTrue(all(math.isfinite(captured["summary"][key]) for key in ("dc", "rms", "peak", "min", "max")))
            traces.append(tuple(point["mean"] for point in captured["bins"]))
        self.assertEqual(len(set(traces)), 4, "Named VCO jacks must expose distinct waveform traces")
        for tool, arguments in [
            ("set_output", {"id": "vco", "port": "out"}),
            ("capture_signal", {"id": "vco", "port": "missing", "seconds": 0.001}),
            ("set_parameter", {"id": "vco", "parameter": "waveform", "value": 1}),
        ]:
            self.require_tool_error(tool, arguments)
        self.assertEqual(self.patch_snapshot(), before)

    def test_multi_vco_polyphonic_render_and_atomic_mode_edit_roundtrip(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "sample_rate": 48000,
            "nodes": [
                {"id": "pitch", "kind": "poly_constant", "params": {"channels": 4, "v1": 0, "v2": 1, "v3": -1, "v4": 0.5}},
                {"id": "aux", "kind": "constant", "params": {"value": 0.25}},
                {"id": "sync", "kind": "multi_lfo", "params": {"frequency": 97}},
                {"id": "vco", "kind": "multi_vco", "params": {"frequency": 220, "fm_amount": 0.25, "pulse_width": 0.35, "pwm_amount": 0.5}},
                {"id": "audio", "kind": "audio_stereo"},
            ],
            "cables": [
                {"from": "pitch", "to": "vco", "input": "pitch"},
                {"from": "aux", "to": "vco", "input": "fm"},
                {"from": "aux", "to": "vco", "input": "pwm"},
                {"from": "sync", "output": "sine", "to": "vco", "input": "sync"},
                {"from": "vco", "output": "sine", "to": "audio", "input": "left"},
                {"from": "vco", "output": "square", "to": "audio", "input": "right"},
            ], "output": "audio",
        }})
        before = self.patch_snapshot()
        for port in ("sine", "triangle", "saw", "square"):
            measured = self.payload(self.require_tool_success("analyze_signal", {
                "id": "vco", "port": port, "seconds": 0.02,
            }))
            self.assertEqual(measured["max_channels"], 4)
            self.assertEqual([channel["channel"] for channel in measured["channels"]], [1, 2, 3, 4])
            self.assertTrue(all(channel["active_frames"] > 900 and channel["rms"] > 0 for channel in measured["channels"]))
        rendered = self.payload(self.require_tool_success("render_audio", {"filename": "multi-vco-before.wav", "seconds": 0.02}))
        self.assertEqual((rendered["frames"], rendered["channels"]), (960, 2))
        with wave.open(str(self.workspace / "multi-vco-before.wav")) as wav:
            self.assertEqual((wav.getframerate(), wav.getnchannels(), wav.getnframes()), (48000, 2, 960))
            samples = struct.unpack("<1920h", wav.readframes(960))
        self.assertTrue(any(abs(sample) > 100 for sample in samples[::2]))
        self.assertTrue(any(abs(sample) > 100 for sample in samples[1::2]))
        self.assertNotEqual(samples[::2], samples[1::2])
        self.assertEqual(self.patch_snapshot(), before)
        edits = [
            {"tool": "set_parameter", "arguments": {"id": "vco", "parameter": "linear_fm", "value": 1}},
            {"tool": "set_parameter", "arguments": {"id": "vco", "parameter": "soft_sync", "value": 1}},
        ]
        arguments = {"edits": edits, "expected_revision": before["revision"]}
        preview = self.payload(self.require_tool_success("preview_edits", arguments))
        self.assertEqual(self.patch_snapshot(), before)
        changed = self.payload(self.require_tool_success("apply_edits", arguments))
        self.assertEqual(changed["patch"], preview["patch"])
        self.assertEqual(changed["revision"], before["revision"] + 1)
        self.assertEqual(changed["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        parameters = next(node for node in changed["patch"]["nodes"] if node["id"] == "vco")["params"]
        self.assertEqual((parameters["linear_fm"], parameters["soft_sync"]), (1, 1))
        self.require_tool_error("apply_edits", arguments)
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "vco", "parameter": "linear_fm", "value": 0}},
            {"tool": "set_parameter", "arguments": {"id": "vco", "parameter": "soft_sync", "value": 0.5}},
        ]})
        self.assertEqual(self.patch_snapshot(), changed)
        self.require_tool_success("render_audio", {"filename": "multi-vco-modes.wav", "seconds": 0.02})
        self.assertNotEqual((self.workspace / "multi-vco-modes.wav").read_bytes(),
                            (self.workspace / "multi-vco-before.wav").read_bytes())
        self.assertEqual(self.patch_snapshot(), changed)
        self.require_tool_success("undo", {"expected_revision": changed["revision"]})
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("render_audio", {"filename": "multi-vco-restored.wav", "seconds": 0.02})
        self.assertEqual((self.workspace / "multi-vco-restored.wav").read_bytes(),
                         (self.workspace / "multi-vco-before.wav").read_bytes())
        self.require_tool_success("redo")
        self.require_tool_success("save_patch", {"filename": "multi-vco.json"})
        saved_patch = json.loads((self.workspace / "multi-vco.json").read_text())
        # File serialization uses f32's shortest round-trip decimal; MCP Values
        # expose the same stored parameters widened to f64. Compare exact f32
        # values while retaining every node, parameter key, cable ID and port.
        for node in saved_patch["nodes"]:
            node["params"] = {
                name: struct.unpack("<f", struct.pack("<f", value))[0]
                for name, value in node["params"].items()
            }
        self.assertEqual(saved_patch, changed["patch"])
        self.require_tool_success("set_parameter", {"id": "vco", "parameter": "frequency", "value": 330})
        self.require_tool_success("load_patch", {"filename": "multi-vco.json"})
        self.assertEqual(self.patch_snapshot()["patch"], changed["patch"])

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
             "connect", "disconnect", "clear_cables", "set_parameter", "set_output", "save_patch",
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

    def test_pulses_indexed_events_ports_and_transient_project_state(self) -> None:
        modules = {item["kind"]: item for item in self.payload(self.require_tool_success("catalog"))["modules"]}
        self.assertEqual(modules["pulses"]["inputs"], [])
        self.assertEqual(len(modules["pulses"]["runtime_actions"]), 20)
        self.require_tool_success("add_module", {"id": "buttons", "kind": "pulses"})
        before = self.patch_snapshot()
        events = [{"frame": 0, "id": "buttons", "action": "pulse10_press"},
                  {"frame": 0, "id": "buttons", "action": "pulse10_release"},
                  {"frame": 4, "id": "buttons", "action": "pulse1_press"},
                  {"frame": 8, "id": "buttons", "action": "pulse1_release"}]
        for port, expected in [("trig10", [10] * 48 + [0] * 48),
                               ("gate10", [0] * 96),
                               ("gate1", [10 if 4 <= n < 8 else 0 for n in range(96)])]:
            capture = self.payload(self.require_tool_success("capture_signal", {
                "id": "buttons", "port": port, "seconds": 0.002, "module_events": events}))
            self.assertEqual([point["mean"] for point in capture["bins"]], expected)
        for action in ["pulse0_press", "pulse11_release", "pulse01_press", "pulse1_toggle", "gate_press", {"PulsePress": 1}]:
            self.require_tool_error("render_audio", {"filename": "invalid-pulses.wav", "seconds": 0.002,
                "module_events": [{"frame": 0, "id": "buttons", "action": action}]})
            self.assertFalse((self.workspace / "invalid-pulses.wav").exists())
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_success("save_patch", {"filename": "pulses.json"})
        saved = json.loads((self.workspace / "pulses.json").read_text())
        node = next(node for node in saved["nodes"] if node["id"] == "buttons")
        self.assertEqual(node["params"], {})
        self.assertNotIn("pressed_mask", node)
        self.require_tool_success("set_bypass", {"id": "buttons", "bypassed": True})
        self.require_tool_error("capture_signal", {"id": "buttons", "port": "gate10", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "buttons", "action": "pulse10_press"}]})
        self.require_tool_success("capture_signal", {"id": "buttons", "port": "gate10", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "buttons", "action": "pulse10_release"}]})
        self.require_tool_success("load_patch", {"filename": "pulses.json"})
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])

    def test_gates_edges_delay_reset_and_protocol(self) -> None:
        modules = {item["kind"]: item for item in self.payload(self.require_tool_success("catalog"))["modules"]}
        self.assertEqual(modules["gates"]["inputs"], ["length", "in", "reset"])
        self.assertEqual(modules["gates"]["outputs"], ["rise", "fall", "flip", "flop", "gate", "delay"])
        self.assertEqual(modules["gates"]["runtime_actions"], ["reset_press", "reset_release"])
        self.require_tool_success("add_module", {"id": "source", "kind": "pulses"})
        self.require_tool_success("add_module", {"id": "timing", "kind": "gates", "params": {"length": 1 / 512}})
        self.require_tool_success("connect", {"from": "source", "output": "gate1", "to": "timing", "input": "in"})
        before = self.patch_snapshot()
        edges = [{"frame": 0, "id": "source", "action": "pulse1_press"},
                 {"frame": 1, "id": "source", "action": "pulse1_release"}]
        expected = {"rise": lambda n: 1 <= n < 49, "fall": lambda n: 2 <= n < 50,
                    "flip": lambda n: n == 0, "flop": lambda n: n >= 1,
                    "gate": lambda n: 1 <= n < 94, "delay": lambda n: n == 95}
        for port, high in expected.items():
            result = self.payload(self.require_tool_success("capture_signal", {
                "id": "timing", "port": port, "seconds": .004, "module_events": edges}))
            self.assertEqual([point["mean"] for point in result["bins"]],
                             [10 if high(n) else 0 for n in range(192)], port)
        resets = edges + [{"frame": 10, "id": "timing", "action": "reset_press"},
                          {"frame": 10, "id": "timing", "action": "reset_release"}]
        for port, high in {"rise": lambda n: 1 <= n < 49, "gate": lambda n: 1 <= n < 10,
                           "flop": lambda n: 1 <= n < 10, "delay": lambda n: False}.items():
            result = self.payload(self.require_tool_success("capture_signal", {
                "id": "timing", "port": port, "seconds": .004, "module_events": resets}))
            self.assertEqual([point["mean"] for point in result["bins"]],
                             [10 if high(n) else 0 for n in range(192)], port)
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_error("capture_signal", {"id": "timing", "port": "gate", "seconds": .004,
            "module_events": [{"frame": 0, "id": "timing", "action": "reset"}]})
        self.require_tool_error("set_parameter", {"id": "timing", "parameter": "length", "value": 0})
        self.require_tool_success("set_bypass", {"id": "timing", "bypassed": True})
        self.require_tool_error("capture_signal", {"id": "timing", "port": "gate", "seconds": .004,
            "module_events": [{"frame": 0, "id": "timing", "action": "reset_press"}]})
        self.require_tool_success("capture_signal", {"id": "timing", "port": "gate", "seconds": .004,
            "module_events": [{"frame": 0, "id": "timing", "action": "reset_release"}]})

    def test_push_xor_edges_persisted_hold_and_protocol_validation(self) -> None:
        modules = {item["kind"]: item for item in self.payload(self.require_tool_success("catalog"))["modules"]}
        self.assertEqual(modules["push"]["inputs"], ["push", "hold"])
        self.assertEqual(modules["push"]["outputs"], ["trig", "gate"])
        self.assertEqual(modules["push"]["runtime_actions"], ["push_press", "push_release", "hold_press", "hold_release"])
        self.assertEqual(len(modules["pulses"]["runtime_actions"]), 20)
        self.require_tool_success("add_module", {"id": "button", "kind": "push"})
        before = self.patch_snapshot()
        events = [{"frame": 0, "id": "button", "action": "hold_press"},
                  {"frame": 1, "id": "button", "action": "hold_release"},
                  {"frame": 60, "id": "button", "action": "push_press"},
                  {"frame": 70, "id": "button", "action": "push_release"}]
        for port, expected in [("trig", [10 if n < 48 or 70 <= n < 118 else 0 for n in range(144)]),
                               ("gate", [0 if 60 <= n < 70 else 10 for n in range(144)])]:
            capture = self.payload(self.require_tool_success("capture_signal", {
                "id": "button", "port": port, "seconds": 0.003, "module_events": events}))
            self.assertEqual([point["mean"] for point in capture["bins"]], expected)
        self.assertEqual(self.patch_snapshot(), before)
        for action in ["pulse1_press", "b_press", "hold_toggle", {"hold_press": None}]:
            self.require_tool_error("render_audio", {"filename": "invalid-push.wav", "seconds": 0.002,
                "module_events": [{"frame": 0, "id": "button", "action": action}]})
            self.assertFalse((self.workspace / "invalid-push.wav").exists())
        self.require_tool_error("set_parameter", {"id": "button", "parameter": "holding", "value": 0.5})
        self.require_tool_success("set_parameter", {"id": "button", "parameter": "holding", "value": 1})
        saved = self.payload(self.require_tool_success("capture_project_patch"))
        self.assertEqual(saved["holding"], {"button": True})
        self.assertEqual(saved["running"], {})
        self.assertEqual(saved["state_source"], "offline_reset")
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "button"}))["preset"]
        self.assertEqual(preset["params"], {"holding": 1.0})
        capture = self.payload(self.require_tool_success("capture_signal", {"id": "button", "port": "trig", "seconds": 0.002}))
        self.assertTrue(all(point["mean"] == 0 for point in capture["bins"]))  # Loaded HOLD starts without a trigger.
        self.require_tool_success("initialize_modules", {"ids": ["button"]})
        self.assertFalse(self.payload(self.require_tool_success("capture_project_patch"))["holding"]["button"])
        self.require_tool_success("undo")
        self.assertTrue(self.payload(self.require_tool_success("capture_project_patch"))["holding"]["button"])
        self.require_tool_success("set_bypass", {"id": "button", "bypassed": True})
        self.require_tool_error("capture_signal", {"id": "button", "port": "gate", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "button", "action": "hold_press"}]})
        for action in ["hold_release", "push_release"]:
            self.require_tool_success("capture_signal", {"id": "button", "port": "gate", "seconds": 0.001,
                "module_events": [{"frame": 0, "id": "button", "action": action}]})

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
