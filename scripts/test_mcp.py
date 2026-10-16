"""Integration smoke tests for the OpenPatch MCP stdio server.

Run from the repository root with ``python -m unittest discover -s scripts``.
Set ``OPENPATCH_BIN`` to override the default release binary location.
"""

from __future__ import annotations

import base64
import hashlib
import json
import math
import os
import queue
import subprocess
import struct
import tempfile
import threading
import time
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

    def compact_wavetable_payload(self, result: dict[str, Any]) -> dict[str, Any]:
        # Measure the complete MCP result, including its text/structured copies.
        self.assertLess(len(json.dumps(result, separators=(",", ":")).encode()), 32 * 1024)
        payload = self.payload(result)
        self.assertIsInstance(payload, dict)
        assert payload is not None

        def check(value: Any) -> None:
            if isinstance(value, dict):
                self.assertNotIn("data", value, "ordinary reply leaked an encoded asset payload")
                for child in value.values():
                    check(child)
            elif isinstance(value, list):
                for child in value:
                    check(child)

        check(payload)
        return payload

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

    def test_scope_manual_takeover_and_following_drag_share_one_mcp_undo_step(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "nodes": [
                {"id": "scope", "kind": "scope", "params": {"auto_range": 1, "x_offset": 0}},
                {"id": "other", "kind": "scope"},
            ],
        }})
        before = self.patch_snapshot()
        request = {
            "expected_revision": before["revision"],
            "history_group": "scope-offset-drag",
            "history_parameter": {"id": "scope", "parameter": "x_offset"},
            "edits": [
                {"tool": "set_parameter", "arguments": {"id": "scope", "parameter": "x_offset", "value": 0.25}},
                {"tool": "set_parameter", "arguments": {"id": "scope", "parameter": "auto_range", "value": 0}},
            ],
        }
        current = self.payload(self.require_tool_success("apply_edits", request))
        for value in (0.5, 0.75, 1.0):
            current = self.payload(self.require_tool_success("set_parameter", {
                "expected_revision": current["revision"], "id": "scope", "parameter": "x_offset",
                "value": value, "history_group": "scope-offset-drag",
            }))
        self.assertEqual(current["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        self.assertEqual(current["patch"]["nodes"][0]["params"], {"auto_range": 0.0, "x_offset": 1.0})
        self.require_tool_error("apply_edits", request)
        self.assertEqual(self.patch_snapshot(), current, "stale takeover must not affect history")
        invalid = {**request, "expected_revision": current["revision"], "edits": [
            request["edits"][0],
            {"tool": "set_parameter", "arguments": {"id": "other", "parameter": "auto_range", "value": 0}},
        ]}
        self.require_tool_error("apply_edits", invalid)
        self.assertEqual(self.patch_snapshot(), current, "a grouped batch cannot edit another module")
        undone = self.payload(self.require_tool_success("undo", {"expected_revision": current["revision"]}))
        self.assertEqual(undone["patch"], before["patch"])
        redone = self.payload(self.require_tool_success("redo", {"expected_revision": undone["revision"]}))
        self.assertEqual(redone["patch"], current["patch"])

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

    def test_merge_forced_zero_preserves_legacy_auto_and_atomic_history(self) -> None:
        module = next(m for m in self.payload(self.require_tool_success("catalog"))["modules"]
                      if m["kind"] == "poly_merge")
        self.assertEqual(module["params"]["channels"]["values"], [0, -1, *range(1, 17)])
        self.assertEqual(module["params"]["channels"]["labels"], ["Auto", *map(str, range(17))])
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "source", "kind": "poly_constant", "params": {"channels": 3, "v1": 5, "v2": -13}},
            {"id": "merge", "kind": "poly_merge"}], "cables": [
                {"from": "source", "to": "merge", "input": "in16"}], "output": "merge"}})
        auto = self.patch_snapshot()
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "merge", "seconds": 0.001}))
        self.assertEqual(measured["max_channels"], 16)
        self.assertEqual(measured["channels"][15]["peak"], 5)
        self.require_tool_success("set_parameter", {"id": "merge", "parameter": "channels", "value": -1,
                                                    "expected_revision": auto["revision"]})
        zero = self.patch_snapshot()
        self.assertEqual(zero["history"]["undo_steps"], auto["history"]["undo_steps"] + 1)
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "merge", "seconds": 0.001}))
        self.assertEqual(measured["max_channels"], 0)
        self.assertEqual(measured["channels"], [])
        for value in [-2, -0.5, 0.5, 17]:
            self.require_tool_error("set_parameter", {"id": "merge", "parameter": "channels", "value": value})
            self.assertEqual(self.patch_snapshot(), zero)
        self.require_tool_error("set_parameter", {"id": "merge", "parameter": "channels", "value": 16,
                                                 "expected_revision": auto["revision"]})
        self.assertEqual(self.patch_snapshot(), zero)
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], auto["patch"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], zero["patch"])
        # Older clients can still select Auto through the existing zero value.
        self.require_tool_success("set_parameter", {"id": "merge", "parameter": "channels", "value": 0})
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "merge", "seconds": 0.001}))
        self.assertEqual(measured["max_channels"], 16)

    def test_merge_selection_survives_presets_duplication_files_and_initialize(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "merge", "kind": "poly_merge", "params": {"channels": -1}},
            {"id": "target", "kind": "poly_merge", "params": {"channels": 7}}]}})
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "merge"}))["preset"]
        self.assertEqual(preset["params"]["channels"], -1)
        self.require_tool_success("save_module_preset", {"filename": "zero.opreset", "preset": preset})
        reviewed = self.payload(self.require_tool_success("read_module_preset", {"filename": "zero.opreset"}))["preset"]
        self.assertEqual(reviewed, preset)
        self.require_tool_success("apply_module_preset", {"id": "target", "preset": reviewed})
        self.require_tool_success("duplicate_modules", {"modules": [{"id": "merge", "new_id": "copy"}],
                                                       "cable_mode": "none"})
        saved_patch = self.patch_snapshot()["patch"]
        self.assertTrue(all(n["params"]["channels"] == -1 for n in saved_patch["nodes"]))
        self.require_tool_success("save_patch", {"filename": "merge.json"})
        self.require_tool_success("randomize_modules", {"ids": ["merge", "target", "copy"], "seed": 719})
        self.assertEqual(self.patch_snapshot()["patch"], saved_patch)
        self.require_tool_success("initialize_modules", {"ids": ["merge"]})
        initialized = self.patch_snapshot()
        nodes = {n["id"]: n for n in initialized["patch"]["nodes"]}
        self.assertEqual(nodes["merge"]["params"]["channels"], 0)
        self.assertEqual(nodes["target"]["params"]["channels"], -1)
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], saved_patch)
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], initialized["patch"])
        self.require_tool_success("load_patch", {"filename": "merge.json"})
        self.assertEqual(self.patch_snapshot()["patch"], saved_patch)

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

    def test_poly_viz_has_no_outputs_and_invalid_edits_are_atomic(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog", {}))
        module = next(m for m in catalog["modules"] if m["kind"] == "poly_viz")
        self.assertEqual(module["outputs"], [])
        self.assertEqual(module["params"], {})
        self.assertEqual(len(module["light_names"]), 16)
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "source", "kind": "poly_constant", "params": {"channels": 1, "v1": 2}}],
            "output": "source"}})
        self.require_tool_success("add_module", {"id": "viz", "kind": "poly_viz"})
        self.require_tool_success("connect", {"from": "source", "to": "viz", "input": "signal"})
        before = self.patch_snapshot()
        for tool, args in [
            ("set_output", {"id": "viz"}),
            ("connect", {"from": "viz", "to": "viz", "input": "signal", "mode": "stack"}),
            ("set_parameter", {"id": "viz", "parameter": "level", "value": 1}),
            ("analyze_signal", {"id": "viz", "seconds": 0.001}),
            ("capture_signal", {"id": "viz", "seconds": 0.001})]:
            self.require_tool_error(tool, args)
            self.assertEqual(self.patch_snapshot(), before)
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "source", "seconds": 0.001}))
        self.assertEqual(measured["channels"][0]["peak"], 2)
        self.require_tool_success("save_patch", {"filename": "viz.json"})
        saved = json.loads((self.workspace / "viz.json").read_text())
        self.assertEqual(saved["nodes"][1]["kind"], "poly_viz")
        self.require_tool_success("remove_module", {"id": "viz"})
        self.require_tool_success("undo", {})
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo", {})
        self.assertEqual(len(self.patch_snapshot()["patch"]["nodes"]), 1)

    def test_random_values_golden_draws_polyphony_and_saved_state(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        model = modules["random_values"]
        self.assertEqual(model["inputs"], ["trig"])
        self.assertEqual(model["outputs"], [f"out{i}" for i in range(1, 8)])
        self.assertEqual(model["runtime_actions"], ["push_press", "push_release"])
        self.assertEqual(sum(spec.get("hidden", False) for spec in model["params"].values()), 114)

        def f32(value):
            return struct.unpack("<f", struct.pack("<f", value))[0]

        def draw(state, low, width):
            state ^= (state << 13) & 0xffffffff
            state ^= state >> 17
            state ^= (state << 5) & 0xffffffff
            return state, f32(low + f32(width * ((state >> 8) / 16777216)))

        # Public sequence contract checked independently through JSON-RPC, all
        # 112 retained voltages, two boundary seeds and every supported range.
        for seed in [0, 16777215]:
            for range_index, (low, width) in enumerate([(0, 10), (0, 5), (0, 1), (-10, 20), (-5, 10), (-1, 2)]):
                self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
                    "nodes": [{"id": "values", "kind": "random_values", "params": {"seed": seed, "range": range_index}}],
                    "output": "values", "output_port": "out1"}})
                saved = self.payload(self.require_tool_success("capture_project_patch"))["patch"]
                params = saved["nodes"][0]["params"]
                state = seed ^ 0xA341316C
                expected = [[0.0] * 16 for _ in range(7)]
                for lane in range(16):
                    for output in range(7):
                        state, expected[output][lane] = draw(state, low, width)
                        self.assertAlmostEqual(params[f"value{output+1}_{lane+1}"], expected[output][lane], delta=1e-6)
                self.assertEqual(int(params["rng_hi"]) * 65536 + int(params["rng_lo"]), state)
                # A fresh render starts from the saved state, not the seed's
                # beginning. The first manual press consumes exactly seven draws.
                self.require_tool_success("replace_patch", {"patch": saved})
                _, after = draw(state, low, width)
                response = self.payload(self.require_tool_success("capture_signal", {
                    "id": "values", "port": "out1", "seconds": .002, "points": 96,
                    "module_events": [{"frame": 32, "id": "values", "action": "push_press"},
                                      {"frame": 33, "id": "values", "action": "push_release"}]}))
                for frame, point in enumerate(response["bins"]):
                    self.assertAlmostEqual(point["mean"], expected[0][0] if frame < 32 else after, delta=1e-6)
        before = self.patch_snapshot()
        for param, value in [("range", 6), ("range", 0.5), ("seed", -1), ("seed", 16777216)]:
            self.require_tool_error("set_parameter", {"id": "values", "parameter": param, "value": value})
        self.assertEqual(self.patch_snapshot(), before)
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "values"}))["preset"]
        self.assertEqual(len([key for key in preset["params"] if key.startswith("value")]), 112)
        self.require_tool_success("save_patch", {"filename": "values.json"})
        disk = json.loads((self.workspace / "values.json").read_text())
        # File JSON uses shortest float32 decimals; RPC JSON can promote them
        # to float64 decimals. Compare the actual stored float32 bits.
        bits = lambda params: {key: struct.pack("<f", value) for key, value in params.items()}
        self.assertEqual(bits(disk["nodes"][0]["params"]), bits(before["patch"]["nodes"][0]["params"]))
        self.require_tool_success("load_patch", {"filename": "values.json"})
        self.assertEqual(bits(self.patch_snapshot()["patch"]["nodes"][0]["params"]), bits(disk["nodes"][0]["params"]))
        self.require_tool_success("randomize_modules", {"ids": ["values"], "seed": 42})
        randomized = self.payload(self.require_tool_success("capture_project_patch"))["patch"]
        self.assertNotEqual(randomized["nodes"][0]["params"], disk["nodes"][0]["params"])
        self.require_tool_success("undo", {})
        restored = self.payload(self.require_tool_success("capture_project_patch"))["patch"]
        self.assertEqual(bits(restored["nodes"][0]["params"]), bits(disk["nodes"][0]["params"]))
        # Public polyphonic interface keeps all seven outputs at 16 channels.
        self.require_tool_success("add_module", {"id": "lanes", "kind": "poly_constant", "params": {"channels": 16}})
        self.require_tool_success("connect", {"from": "lanes", "to": "values", "input": "trig"})
        for port in ["out1", "out7"]:
            measured = self.payload(self.require_tool_success("analyze_signal", {"id": "values", "port": port, "seconds": .002}))
            self.assertEqual(len(measured["channels"]), 16)
            self.assertTrue(all(c["active_frames"] > 90 for c in measured["channels"]))
        example = json.loads((REPOSITORY / "examples/random-values-voice.json").read_text())
        self.require_tool_success("replace_patch", {"patch": example})
        self.require_tool_success("render_audio", {"filename": "values-voice.wav", "seconds": 0.2})
        with wave.open(str(self.workspace / "values-voice.wav")) as rendered:
            samples = self._first_channel_samples(rendered.readframes(9600), rendered.getsampwidth(), rendered.getnchannels())
        self.assertTrue(any(abs(x) > 100 for x in samples))
        self.assertLess(max(abs(x) for x in samples), 32767)

    def test_noise_colors_named_outputs_seed_history_and_persistence(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        colors = ["white", "pink", "red", "violet", "blue", "gray", "black"]
        self.assertEqual(modules["colored_noise"]["inputs"], [])
        self.assertEqual(modules["colored_noise"]["outputs"], colors)
        patch = json.loads((REPOSITORY / "examples/noise-colors.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch})
        original = self.patch_snapshot()
        captures = {}
        for color in colors:
            args = {"id": "noise", "port": color, "seconds": 0.05, "points": 64}
            response = self.payload(self.require_tool_success("capture_signal", args))
            captures[color] = response["bins"]
            self.assertGreater(response["summary"]["rms"], 0.01)
            self.assertTrue(all(math.isfinite(response["summary"][key]) for key in ("rms", "dc", "peak")))
            self.assertEqual(response["bins"], self.payload(self.require_tool_success("capture_signal", args))["bins"])
            if color == "black":
                self.assertLessEqual(response["summary"]["peak"], 1.0)
        self.assertEqual(self.patch_snapshot(), original)
        self.require_tool_success("set_parameter", {"id": "noise", "parameter": "seed", "value": 42})
        changed = self.payload(self.require_tool_success("capture_signal", {
            "id": "noise", "port": "white", "seconds": 0.05, "points": 64}))
        self.assertNotEqual(changed["bins"], captures["white"])
        self.require_tool_success("undo", {})
        self.assertEqual(self.patch_snapshot()["patch"], original["patch"])
        for param, value in [("seed", -1), ("seed", 0.5), ("seed", 16777216), ("level", 2.01)]:
            self.require_tool_error("set_parameter", {"id": "noise", "parameter": param, "value": value})
        self.require_tool_error("set_output", {"id": "noise", "port": "missing"})
        self.assertEqual(self.patch_snapshot()["patch"], original["patch"])
        self.require_tool_success("set_output", {"id": "noise", "port": "gray"})
        self.require_tool_success("save_patch", {"filename": "noise.json"})
        saved = json.loads((self.workspace / "noise.json").read_text())
        self.assertEqual(saved["output_port"], "gray")
        self.assertEqual(saved["nodes"][0]["params"]["seed"], 1)
        self.require_tool_success("undo", {})
        self.require_tool_success("render_audio", {"filename": "noise.wav", "seconds": 0.1})
        with wave.open(str(self.workspace / "noise.wav")) as rendered:
            samples = self._first_channel_samples(rendered.readframes(4800), rendered.getsampwidth(), rendered.getnchannels())
        self.assertTrue(any(abs(x) > 100 for x in samples))
        self.assertLess(max(abs(x) for x in samples), 32767)

    def test_physical_resonator_catalog_render_edit_and_restore(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        model = modules["modal_resonator"]
        self.assertEqual(model["inputs"], ["pitch", "strike", "excite"])
        self.assertEqual(model["outputs"], ["out"])
        self.assertEqual(model["params"]["decay"]["unit"], "s")
        patch = json.loads((REPOSITORY / "examples/struck-resonator.json").read_text())
        self.require_tool_success("replace_patch", {"patch": patch})
        original = self.patch_snapshot()
        args = {"id": "body", "seconds": 0.1, "points": 256}
        response = self.payload(self.require_tool_success("capture_signal", args))
        self.assertEqual(response["units"], "V")
        self.assertGreater(response["summary"]["peak"], 0.01)
        self.assertEqual(self.patch_snapshot(), original)
        # Physical strike/pickup reciprocity is observable through the public API.
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "body", "parameter": "strike_position", "value": 0.61}},
            {"tool": "set_parameter", "arguments": {"id": "body", "parameter": "pickup_position", "value": 0.23}},
        ], "expected_revision": original["revision"]})
        reciprocal = self.payload(self.require_tool_success("capture_signal", args))
        for a, b in zip(response["bins"], reciprocal["bins"]):
            for field in ("min", "max", "mean"):
                if a[field] is not None:
                    self.assertAlmostEqual(a[field], b[field], delta=1e-6)
        self.require_tool_success("undo", {})
        self.assertEqual(self.patch_snapshot()["patch"], original["patch"])
        for param, value in [("stiffness", 1.01), ("decay", 0), ("strike_position", 0)]:
            self.require_tool_error("set_parameter", {"id": "body", "parameter": param, "value": value})
        self.assertEqual(self.patch_snapshot()["patch"], original["patch"])
        self.require_tool_success("save_patch", {"filename": "resonator.json"})
        saved = json.loads((self.workspace / "resonator.json").read_text())
        self.assertEqual(saved["nodes"][0]["kind"], "modal_resonator")
        self.require_tool_success("render_audio", {"filename": "resonator.wav", "seconds": 0.1})
        with wave.open(str(self.workspace / "resonator.wav")) as rendered:
            samples = self._first_channel_samples(rendered.readframes(4800), rendered.getsampwidth(), rendered.getnchannels())
        self.assertTrue(any(abs(x) > 100 for x in samples))
        self.assertLess(max(abs(x) for x in samples), 32767)

    def test_rack_scope_contract_and_offline_inspection_boundary(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog", {}))
        module = next(m for m in catalog["modules"] if m["kind"] == "scope")
        self.assertEqual(module["inputs"], ["x", "y", "trigger"])
        self.assertEqual(module["outputs"], ["x", "y"])
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "source", "kind": "poly_constant", "params": {"channels": 16, "v1": 12, "v16": -14}},
            {"id": "scope", "kind": "scope", "params": {"time": 0.005}}],
            "cables": [{"from": "source", "to": "scope", "input": "x"}],
            "output": "scope", "output_port": "x"}})
        before = self.patch_snapshot()
        unavailable = self.payload(self.require_tool_success("inspect_scope", {"id": "scope", "channel": 16}))
        self.assertFalse(unavailable["supported"])
        self.assertIsNone(unavailable["scope"])
        for channel in [0, 17, -1, 1.5, "1", None]:
            self.require_tool_error("inspect_scope", {"id": "scope", "channel": channel})
        self.require_tool_error("inspect_scope", {"id": "source"})
        self.require_tool_error("inspect_scope", {"id": "missing"})
        self.require_tool_error("inspect_scope", {"id": "scope", "expected_revision": 0})
        self.assertEqual(self.patch_snapshot(), before)
        measured = self.payload(self.require_tool_success("analyze_signal", {
            "id": "scope", "port": "x", "seconds": 0.001}))
        self.assertEqual(measured["max_channels"], 16)
        self.assertEqual(measured["channels"][0]["peak"], 12)
        self.assertEqual(measured["channels"][15]["peak"], 14)
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_success("set_parameter", {"id": "scope", "parameter": "xy", "value": 1})
        self.require_tool_success("save_patch", {"filename": "scope.json"})
        saved = json.loads((self.workspace / "scope.json").read_text())
        self.assertAlmostEqual(saved["nodes"][1]["params"]["time"], 0.005)
        self.assertEqual(saved["nodes"][1]["params"]["xy"], 1)
        self.require_tool_success("undo", {})
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo", {})
        self.assertEqual(self.patch_snapshot()["patch"]["nodes"][1]["params"]["xy"], 1)

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

    def test_annotations_wire_transactions_unicode_presets_history_and_signal_isolation(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog"))
        self.assertEqual(len(catalog["modules"]), 72)
        for kind in ("notes", "blank"):
            module = next(m for m in catalog["modules"] if m["kind"] == kind)
            self.assertEqual(module["inputs"], [])
            self.assertEqual(module["outputs"], [])
            self.assertEqual(module["min_patch_version"], 2)
            self.assertIn("content_schema", module)
        text = "DRUM LAB\n\t日本語 · café · 🎛️\nMIDI → gates → drums"
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "nodes": [
            {"id": "notes", "kind": "notes"}, {"id": "blank", "kind": "blank"},
            {"id": "source", "kind": "constant", "params": {"value": 0.25}},
            {"id": "sink", "kind": "gain"}], "output": "source"}})
        before = self.patch_snapshot()
        arguments = {"expected_revision": before["revision"], "edits": [
            {"tool": "set_module_content", "arguments": {"id": "notes", "content": {"type": "notes", "text": text}}},
            {"tool": "set_module_content", "arguments": {"id": "blank", "content": {"type": "blank", "width_hp": 24}}},
            {"tool": "set_parameter", "arguments": {"id": "source", "parameter": "value", "value": 0.5}}]}
        preview = self.payload(self.require_tool_success("preview_edits", arguments))
        self.assertEqual(self.patch_snapshot(), before)
        committed = self.payload(self.require_tool_success("apply_edits", arguments))
        self.assertEqual(committed["patch"], preview["patch"])
        self.assertEqual(committed["revision"], before["revision"] + 1)
        self.assertEqual(committed["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        self.require_tool_error("apply_edits", arguments)
        for content in ({"type": "notes", "text": "é" * 4097}, {"type": "blank", "width_hp": 10},
                        {"type": "notes", "text": text, "unknown": True}):
            self.require_tool_error("set_module_content", {"id": "notes", "content": content})
            self.assertEqual(self.patch_snapshot(), committed)
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "source", "parameter": "value", "value": 2}},
            {"tool": "set_module_content", "arguments": {"id": "blank", "content": {"type": "blank", "width_hp": 2}}}]})
        self.assertEqual(self.patch_snapshot(), committed)
        for kind in ("notes", "blank"):
            self.require_tool_error("set_output", {"id": kind})
            self.require_tool_error("analyze_signal", {"id": kind, "seconds": 0.001})
            self.require_tool_error("connect", {"from": kind, "to": "sink", "input": "signal"})
            self.require_tool_error("connect", {"from": "source", "to": kind, "input": "signal"})
        self.assertEqual(self.patch_snapshot(), committed)
        measured = self.payload(self.require_tool_success("analyze_signal", {"id": "source", "seconds": 0.001}))
        self.assertEqual(measured["channels"][0]["peak"], 0.5)
        for kind in ("notes", "blank"):
            preset = self.payload(self.require_tool_success("capture_module_preset", {"id": kind}))["preset"]
            self.require_tool_success("save_module_preset", {"filename": kind + ".opreset", "preset": preset})
            loaded = self.payload(self.require_tool_success("read_module_preset", {"filename": kind + ".opreset"}))["preset"]
            self.assertEqual(loaded, preset)
        self.require_tool_success("save_patch", {"filename": "annotations.json"})
        self.require_tool_success("undo", {})
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("redo", {})
        self.assertEqual(self.patch_snapshot()["patch"], committed["patch"])
        self.require_tool_success("duplicate_modules", {"modules": [{"id": "notes", "new_id": "notes_copy"},
            {"id": "blank", "new_id": "blank_copy"}], "cable_mode": "none"})
        nodes = {n["id"]: n for n in self.patch_snapshot()["patch"]["nodes"]}
        self.assertEqual(nodes["notes_copy"]["content"], {"type": "notes", "text": text})
        self.assertEqual(nodes["blank_copy"]["content"], {"type": "blank", "width_hp": 24})
        self.require_tool_success("load_patch", {"filename": "annotations.json"})
        self.assertEqual(self.patch_snapshot()["patch"], committed["patch"])

    def test_midi_cc_catalog_probes_mapping_history_learning_boundary_and_saved_voice(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog"))
        self.assertEqual(len(catalog["modules"]), 72)
        module = next(item for item in catalog["modules"] if item["kind"] == "midi_cc")
        self.assertEqual(module["inputs"], [])
        self.assertEqual(module["outputs"], [f"out{i}" for i in range(1, 17)])
        self.assertEqual(module["min_patch_version"], 2)
        for i in range(1, 17):
            self.assertEqual(module["params"][f"cc{i}"]["default"], i)
            self.assertEqual(module["params"][f"cc{i}"]["min"], -1)
            self.assertEqual(module["params"][f"cc{i}"]["max"], 127)
        self.assertTrue(all(module["params"][f"cc_value{i}"]["hidden"] for i in range(128)))
        patch = {"version": 2, "sample_rate": 48000, "nodes": [{
            "id": "controller", "kind": "midi_cc", "params": {
                "cc1": 7, "cc7": -1, "smooth": 0, "cc_value7": 64, "cc_value90": 37,
            }}]}
        self.require_tool_success("replace_patch", {"patch": patch})

        def cc(frame: int, controller: int, value: int, channel: int = 0) -> dict[str, Any]:
            return {"frame": frame, "event": {"type": "control_change", "channel": channel,
                                              "controller": controller, "value": value}}

        before = self.patch_snapshot()
        seven = self.payload(self.require_tool_success("capture_signal", {
            "id": "controller", "port": "out1", "seconds": 0.001, "points": 64,
            "midi_events": [cc(0, 7, 127), cc(24, 7, 0)],
            "expected_revision": before["revision"],
        }))
        self.assertEqual(seven["summary"]["active_frames"], 48)
        self.assertEqual(seven["summary"]["dc"], 5)
        self.assertEqual([item["mean"] for item in seven["bins"]], [10] * 24 + [0] * 24)
        self.assertEqual(self.patch_snapshot(), before, "offline CCs cannot alter saved values/history")

        self.require_tool_success("set_parameter", {
            "id": "controller", "parameter": "high_resolution", "value": 1})
        before = self.patch_snapshot()
        paired = self.payload(self.require_tool_success("capture_signal", {
            "id": "controller", "port": "out2", "seconds": 0.001, "points": 64,
            "midi_events": [cc(0, 2, 64), cc(12, 34, 127)],
        }))
        target = (64 * 128 + 127) / 16256 * 10
        self.assertTrue(all(item["mean"] == 0 for item in paired["bins"][:12]),
                        "an unpaired MSB must not commit a new voltage")
        for item in paired["bins"][12:]:
            self.assertAlmostEqual(item["mean"], target, delta=0.00001)
        self.assertEqual(self.patch_snapshot(), before)

        self.require_tool_success("set_parameter", {"id": "controller", "parameter": "mpe", "value": 1})
        before = self.patch_snapshot()
        mpe = self.payload(self.require_tool_success("analyze_signal", {
            "id": "controller", "port": "out2", "seconds": 0.001,
            "midi_events": [cc(0, 2, 32), cc(0, 34, 0), cc(0, 2, 96, 15), cc(0, 34, 64, 15)],
        }))
        self.assertEqual(mpe["max_channels"], 16)
        self.assertAlmostEqual(mpe["channels"][0]["mean"], 32 * 128 / 16256 * 10, delta=0.00001)
        self.assertAlmostEqual(mpe["channels"][15]["mean"], (96 * 128 + 64) / 16256 * 10, delta=0.00001)
        self.assertEqual(self.patch_snapshot(), before)

        mapped = self.payload(self.require_tool_success("set_parameter", {
            "id": "controller", "parameter": "cc2", "value": 7,
            "expected_revision": before["revision"],
        }))
        params = mapped["patch"]["nodes"][0]["params"]
        self.assertEqual((params["cc1"], params["cc2"]), (-1, 7))
        self.assertEqual((params["cc_value7"], params["cc_value90"]), (64, 37))
        self.assertEqual(mapped["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        undone = self.payload(self.require_tool_success("undo", {"expected_revision": mapped["revision"]}))
        params = undone["patch"]["nodes"][0]["params"]
        self.assertEqual((params["cc1"], params.get("cc2", 2)), (7, 2))
        self.assertEqual((params["cc_value7"], params["cc_value90"]), (64, 37))
        for tool, arguments in [("learn_midi_cc", {"id": "controller", "cell": 16}),
                                ("cancel_midi_cc_learn", {"id": "controller"})]:
            result = self.require_tool_error(tool, {**arguments, "expected_revision": undone["revision"]})
            self.assertIn("live audio backend", json.dumps(result))
            self.assertEqual(self.patch_snapshot(), undone)

        self.require_tool_success("save_patch", {"filename": "controllers.json"})
        saved = json.loads((self.workspace / "controllers.json").read_text())
        self.assertEqual(saved["nodes"][0]["params"]["cc_value7"], 64)
        self.assertEqual(saved["nodes"][0]["params"]["cc_value90"], 37)
        self.require_tool_success("set_parameter", {"id": "controller", "parameter": "cc_value7", "value": 0})
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "controllers.json"}))
        self.assertEqual(loaded["patch"], saved)

        voice = json.loads((REPOSITORY / "examples" / "midi-controller-voice.json").read_text())
        self.require_tool_success("replace_patch", {"patch": voice})
        self.require_tool_success("render_audio", {"filename": "controller-voice.wav", "seconds": 0.1})
        with wave.open(str(self.workspace / "controller-voice.wav")) as wav:
            self.assertEqual(wav.getnchannels(), 2)
            samples = self._first_channel_samples(wav.readframes(4800), wav.getsampwidth(), wav.getnchannels())
        self.assertTrue(any(abs(value) > 100 for value in samples), "saved controller values provide an audible voice")
        self.assertLess(max(abs(value) for value in samples), 32767)

    def test_midi_gate_catalog_pulses_mpe_mapping_presets_panic_and_connected_drums(self) -> None:
        catalog = self.payload(self.require_tool_success("catalog"))
        self.assertEqual(len(catalog["modules"]), 72)
        module = next(item for item in catalog["modules"] if item["kind"] == "midi_gate")
        defaults = [48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39]
        self.assertEqual(module["inputs"], [])
        self.assertEqual(module["outputs"], [f"gate{i}" for i in range(1, 17)])
        self.assertEqual(module["min_patch_version"], 2)
        self.assertEqual([module["params"][f"note{i}"]["default"] for i in range(1, 17)], defaults)
        self.assertEqual(len(module["params"]), 19)
        self.assertTrue(all(module["params"][f"note{i}"]["min"] == -1 and
                            module["params"][f"note{i}"]["max"] == 127 for i in range(1, 17)))

        def note(frame: int, number: int, velocity: int, channel: int = 0, off: bool = False) -> dict[str, Any]:
            return {"frame": frame, "event": {"type": "note_off" if off else "note_on",
                    "channel": channel, "note": number, "velocity": velocity}}

        for rate, active_samples in [(44100, 45), (48000, 48), (96000, 97)]:
            self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": rate,
                "nodes": [{"id": "pads", "kind": "midi_gate"}]}})
            before = self.patch_snapshot()
            pulse = self.payload(self.require_tool_success("capture_signal", {
                "id": "pads", "port": "gate1", "seconds": 0.003, "points": 2048,
                "midi_events": [note(0, 48, 127), note(0, 48, 0, off=True)],
            }))
            samples = [item["mean"] for item in pulse["bins"]]
            self.assertEqual(samples[:active_samples], [10] * active_samples, f"{rate}: exact f32 minimum pulse")
            self.assertEqual(samples[active_samples:], [0] * (len(samples) - active_samples))
            self.assertEqual(self.patch_snapshot(), before)

        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
            "nodes": [{"id": "pads", "kind": "midi_gate", "params": {"velocity": 1, "mpe": 1}}]}})
        before = self.patch_snapshot()
        voices = self.payload(self.require_tool_success("analyze_signal", {
            "id": "pads", "port": "gate1", "seconds": 0.003,
            "midi_events": [note(0, 48, 1), note(0, 48, 0, off=True),
                            note(0, 48, 127, 15), note(96, 48, 0, 15, off=True)],
        }))
        self.assertEqual(voices["max_channels"], 16)
        self.assertAlmostEqual(voices["channels"][0]["peak"], 10 / 127, delta=0.000001)
        self.assertAlmostEqual(voices["channels"][0]["mean"], (10 / 127) / 3, delta=0.000001)
        self.assertEqual(voices["channels"][14]["peak"], 0)
        self.assertEqual(voices["channels"][15]["peak"], 10)
        self.assertAlmostEqual(voices["channels"][15]["mean"], 10 * 96 / 144, delta=0.000001)
        self.assertEqual(self.patch_snapshot(), before, "MIDI gates and velocities are transient")

        mapped = self.payload(self.require_tool_success("set_parameter", {
            "id": "pads", "parameter": "note1", "value": 49, "expected_revision": before["revision"],
        }))
        self.assertEqual((mapped["patch"]["nodes"][0]["params"]["note1"],
                          mapped["patch"]["nodes"][0]["params"]["note2"]), (49, -1))
        self.assertEqual(mapped["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        undone = self.payload(self.require_tool_success("undo", {"expected_revision": mapped["revision"]}))
        params = undone["patch"]["nodes"][0]["params"]
        self.assertEqual((params.get("note1", 48), params.get("note2", 49)), (48, 49))
        self.assertEqual((params["velocity"], params["mpe"]), (1, 1))
        for tool, arguments in [("learn_midi_gate", {"id": "pads", "cell": 16}),
                                ("cancel_midi_gate_learn", {"id": "pads"})]:
            error = self.require_tool_error(tool, {**arguments, "expected_revision": undone["revision"]})
            self.assertIn("live audio backend", json.dumps(error))
            self.assertEqual(self.patch_snapshot(), undone)

        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "pads"}))["preset"]
        self.assertEqual([preset["params"][f"note{i}"] for i in range(1, 17)], defaults)
        self.require_tool_success("save_module_preset", {"filename": "pads.opreset", "preset": preset})
        reviewed = self.payload(self.require_tool_success("read_module_preset", {"filename": "pads.opreset"}))["preset"]
        self.assertEqual(reviewed, preset)
        self.require_tool_success("set_parameter", {"id": "pads", "parameter": "note1", "value": 127})
        applied = self.payload(self.require_tool_success("apply_module_preset", {"id": "pads", "preset": reviewed}))
        self.assertEqual(applied["patch"]["nodes"][0]["params"]["note1"], 48)
        self.require_tool_success("save_patch", {"filename": "pads.json"})
        saved = json.loads((self.workspace / "pads.json").read_text())
        self.require_tool_success("set_parameter", {"id": "pads", "parameter": "note1", "value": -1})
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "pads.json"}))
        self.assertEqual(loaded["patch"], saved)
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "pads", "parameter": "velocity", "value": 0}},
            {"tool": "set_parameter", "arguments": {"id": "pads", "parameter": "mpe", "value": 0}},
        ]})
        before = self.patch_snapshot()
        panicked = self.payload(self.require_tool_success("capture_signal", {
            "id": "pads", "port": "gate1", "seconds": 0.001, "points": 64,
            "midi_events": [note(0, 48, 127)],
            "module_events": [{"frame": 12, "id": "pads", "action": "midi_panic"}],
        }))
        self.assertEqual([item["mean"] for item in panicked["bins"]], [10] * 12 + [0] * 36)
        self.assertEqual(self.patch_snapshot(), before, "offline Panic is not a saved parameter/history change")

        drums = json.loads((REPOSITORY / "examples" / "midi-gate-drums.json").read_text())
        phrase = json.loads((REPOSITORY / "examples" / "midi-gate-phrase.events.json").read_text())
        self.require_tool_success("replace_patch", {"patch": drums})
        self.require_tool_success("render_audio", {"filename": "pads.wav", "seconds": 2, "midi_events": phrase})
        with wave.open(str(self.workspace / "pads.wav")) as wav:
            self.assertEqual((wav.getnchannels(), wav.getnframes()), (2, 96000))
            samples = self._first_channel_samples(wav.readframes(96000), wav.getsampwidth(), wav.getnchannels())
        self.assertTrue(any(abs(value) > 100 for value in samples))
        self.assertLess(max(abs(value) for value in samples), 32767, "the connected percussion mix leaves headroom")

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

    def test_maximum_wavetable_uses_bounded_stdio_replies_and_portable_workspace_files(self) -> None:
        # Signed zero, a subnormal and values outside [-1,1] expose unintended
        # normalization or a lossy sample representation during persistence.
        pattern = struct.pack("<8I", 0, 0x80000000, 0x3f800000, 0xbf800000,
                              0x3eaaaaab, 1, 0x3f000001, 0xc0200000)
        source = pattern * (524288 // 8)
        (self.workspace / "maximum.f32").write_bytes(source)
        self.require_tool_success("add_module", {"id": "wave", "kind": "wavetable_lfo"})
        imported = self.compact_wavetable_payload(self.require_tool_success("import_wavetable", {
            "id": "wave", "filename": "maximum.f32", "wave_points": 1024,
        }))
        digest = imported["asset"]["sha256"]
        self.assertEqual(imported["asset"]["sample_count"], 524288)
        inspected = self.compact_wavetable_payload(self.require_tool_success("inspect_patch"))
        self.assertEqual(inspected["patch"]["assets"][digest]["sample_count"], 524288)
        preview = self.compact_wavetable_payload(self.require_tool_success("preview_edits", {
            "edits": [{"tool": "set_parameter", "arguments": {
                "id": "wave", "parameter": "position", "value": 0.25,
            }}], "expected_revision": inspected["revision"],
        }))
        self.assertFalse(preview["committed"])
        self.assertEqual(self.patch_snapshot(), inspected)
        changed = self.compact_wavetable_payload(self.require_tool_success("set_parameter", {
            "id": "wave", "parameter": "position", "value": 0.5,
        }))
        undone = self.compact_wavetable_payload(self.require_tool_success("undo"))
        self.assertEqual(undone["patch"], inspected["patch"])
        redone = self.compact_wavetable_payload(self.require_tool_success("redo"))
        self.assertEqual(redone["patch"], changed["patch"])

        self.compact_wavetable_payload(self.require_tool_success("save_patch", {"filename": "maximum.json"}))
        saved_bytes = (self.workspace / "maximum.json").read_bytes()
        self.assertGreater(len(saved_bytes), 1024 * 1024)
        saved = json.loads(saved_bytes)
        self.assertEqual(saved["assets"][digest]["encoding"], "f32le-base64-v1")
        self.assertEqual(base64.b64decode(saved["assets"][digest]["data"], validate=True), source)
        self.require_tool_success("replace_patch", {"patch": {"version": 2}})
        loaded = self.compact_wavetable_payload(self.require_tool_success("load_patch", {"filename": "maximum.json"}))
        self.assertEqual(loaded["patch"], changed["patch"])
        for offset in (0, 524288 - 8):
            chunk = self.compact_wavetable_payload(self.require_tool_success("inspect_wavetable", {
                "id": "wave", "offset": offset, "limit": 8,
            }))
            self.assertEqual(chunk["asset"]["sha256"], digest)
            self.assertEqual(struct.pack("<8f", *chunk["samples"]), pattern)
        self.compact_wavetable_payload(self.require_tool_success("export_wavetable", {
            "id": "wave", "filename": "maximum-restored.f32",
        }))
        restored = (self.workspace / "maximum-restored.f32").read_bytes()
        self.assertEqual(hashlib.sha256(restored).digest(), hashlib.sha256(source).digest())
        before_rejection = self.patch_snapshot()
        self.require_tool_error("export_wavetable", {"id": "wave", "filename": "maximum-restored.f32"})
        self.assertEqual((self.workspace / "maximum-restored.f32").read_bytes(), restored)
        self.assertEqual(self.patch_snapshot(), before_rejection)

    def test_wavetable_preset_file_review_digest_is_required_for_atomic_apply(self) -> None:
        source = struct.pack("<4f", 0.0, -0.0, 0.25, -0.75) * 16
        (self.workspace / "small.f32").write_bytes(source)
        self.require_tool_success("add_module", {"id": "wave", "kind": "wavetable_lfo"})
        imported = self.compact_wavetable_payload(self.require_tool_success("import_wavetable", {
            "id": "wave", "filename": "small.f32", "wave_points": 32,
        }))
        digest = imported["asset"]["sha256"]
        self.require_tool_success("set_parameter", {"id": "wave", "parameter": "position", "value": 0.25})
        # Explicit full data is requested only for this small portable preset.
        captured = self.payload(self.require_tool_success("capture_module_preset", {
            "id": "wave", "include_asset_data": True,
        }))["preset"]
        saved = self.compact_wavetable_payload(self.require_tool_success("save_module_preset", {
            "filename": "reviewed-wave.opreset", "preset": captured,
        }))
        self.assertNotIn("preset", saved)
        reviewed = self.compact_wavetable_payload(self.require_tool_success("read_module_preset", {
            "filename": "reviewed-wave.opreset",
        }))
        self.assertNotIn("preset", reviewed)
        self.assertFalse(reviewed["preset_summary"]["portable_preset"])
        preset_path = self.workspace / "reviewed-wave.opreset"
        reviewed_bytes = preset_path.read_bytes()
        self.assertEqual(reviewed["file_sha256"], hashlib.sha256(reviewed_bytes).hexdigest())
        self.require_tool_success("replace_patch", {"patch": {"version": 2}})
        self.require_tool_success("add_module", {"id": "target", "kind": "wavetable_lfo"})
        before = self.patch_snapshot()
        self.require_tool_error("apply_module_preset", {"id": "target", "filename": "reviewed-wave.opreset"})
        self.assertEqual(self.patch_snapshot(), before)
        applied = self.compact_wavetable_payload(self.require_tool_success("apply_module_preset", {
            "id": "target", "filename": "reviewed-wave.opreset",
            "expected_file_sha256": reviewed["file_sha256"], "expected_revision": before["revision"],
        }))
        target = next(node for node in applied["patch"]["nodes"] if node["id"] == "target")
        self.assertEqual(target["params"]["position"], 0.25)
        self.assertEqual(target["content"]["asset_id"], digest)
        samples = self.compact_wavetable_payload(self.require_tool_success("inspect_wavetable", {
            "id": "target", "offset": 0, "limit": 4,
        }))["samples"]
        self.assertEqual(struct.pack("<4f", *samples), source[:16])
        # Even semantically identical JSON with changed bytes requires review.
        preset_path.write_bytes(reviewed_bytes + b"\n")
        before_rejection = self.patch_snapshot()
        self.require_tool_error("apply_module_preset", {
            "id": "target", "filename": "reviewed-wave.opreset",
            "expected_file_sha256": reviewed["file_sha256"],
            "expected_revision": before_rejection["revision"],
        })
        self.assertEqual(self.patch_snapshot(), before_rejection)

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
             "trigger_module", "inspect_performance", "rotate_sequence", "set_bypass", "invert_mutes",
             "learn_midi_cc", "cancel_midi_cc_learn", "learn_midi_gate", "cancel_midi_gate_learn"}.issubset(names),
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
        saved = self.payload(self.require_tool_success("capture_project_patch"))["patch"]
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
        expected = self.payload(self.require_tool_success("capture_project_patch"))["patch"]
        self.assertEqual(serialized, expected)
        self.assertEqual(self.patch_snapshot(), before)  # Saving does not edit the graph.
        for node in serialized["nodes"]:
            self.assertNotIn("pressed", node)
            self.assertNotIn("bypassed", node)
            self.assertEqual(node["params"], {"running": 1.0} if node["kind"] == "seq3" else {})
        self.require_tool_success("set_bypass", {"id": "process", "bypassed": True})
        self.require_tool_error("render_audio", {"filename": "invalid-bypassed-press.wav", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "process", "action": "gate_press"}]})
        self.assertFalse((self.workspace / "invalid-bypassed-press.wav").exists())
        self.require_tool_success("capture_signal", {"id": "process", "port": "sh1", "seconds": 0.001,
            "module_events": [{"frame": 0, "id": "process", "action": "gate_release"}]})
        self.require_tool_success("load_patch", {"filename": "manual-utilities.json"})
        self.assertEqual(self.patch_snapshot()["patch"], expected)

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

    def test_stock_vca_modes_render_presets_duplicate_history_and_atomic_rejection(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["stock_vca"]
        self.assertEqual(descriptor["min_patch_version"], 2)
        self.assertEqual(descriptor["inputs"], ["cv", "signal"])
        self.assertEqual(descriptor["outputs"], ["out"])
        self.assertEqual(set(descriptor["params"]), {"level", "response"})
        self.assertEqual([descriptor["params"]["level"][key] for key in ("default", "min", "max")], [1, 0, 1])
        self.assertEqual(descriptor["params"]["level"]["display"], {"multiplier": 100, "unit": "%"})
        self.assertEqual([descriptor["params"]["response"][key] for key in ("default", "min", "max", "step")], [1, 0, 1, 1])
        self.assertEqual(descriptor["params"]["response"]["values"], [0, 1])
        self.assertEqual(descriptor["light_names"], [f"gain_{voice}" for voice in range(1, 17)])
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "sample_rate": 48000,
            "nodes": [
                {"id": "cv", "kind": "poly_constant", "params": {"channels": 5, "v1": 0, "v2": 5, "v3": 10, "v4": -5, "v5": 15}},
                {"id": "signal", "kind": "constant", "params": {"value": .5}},
                {"id": "amplifier", "kind": "stock_vca", "params": {"level": .5}},
                {"id": "split", "kind": "poly_split"},
            ],
            "cables": [
                {"from": "cv", "to": "amplifier", "input": "cv"},
                {"from": "signal", "to": "amplifier", "input": "signal"},
                {"from": "amplifier", "to": "split", "input": "signal"},
            ], "output": "split", "output_port": "out2",
        }})
        before = self.patch_snapshot()
        measured = self.payload(self.require_tool_success("analyze_signal", {
            "id": "amplifier", "seconds": .001, "expected_revision": before["revision"],
        }))
        self.assertEqual(measured["max_channels"], 5)
        self.assertEqual([channel["mean"] for channel in measured["channels"]], [0, .125, .25, 0, .25])
        self.assertEqual([channel["active_frames"] for channel in measured["channels"]], [48, 47, 47, 47, 47])
        self.require_tool_success("render_audio", {"filename": "vca-linear.wav", "seconds": .001})
        self.assertEqual(self.patch_snapshot(), before, "offline analysis/render must preserve patch, revision and history")
        self.require_tool_success("set_parameter", {"id": "amplifier", "parameter": "response", "value": 0})
        exponential = self.patch_snapshot()
        captured = self.payload(self.require_tool_success("capture_signal", {
            "id": "amplifier", "channel": 2, "seconds": .001, "points": 64,
            "expected_revision": exponential["revision"],
        }))
        self.assertEqual(captured["summary"]["active_frames"], 47)
        self.assertEqual(captured["summary"]["dc"], .015625)
        self.assertEqual(captured["bins"][0]["active_frames"], 0)
        self.assertEqual([point["mean"] for point in captured["bins"]][1:], [.015625] * 47)
        self.require_tool_success("render_audio", {"filename": "vca-exponential.wav", "seconds": .001})
        for filename, expected in [("vca-linear.wav", 4096), ("vca-exponential.wav", 512)]:
            with wave.open(str(self.workspace / filename)) as rendered:
                self.assertEqual((rendered.getframerate(), rendered.getnchannels(), rendered.getnframes()), (48000, 1, 48))
                samples = self._first_channel_samples(rendered.readframes(48), rendered.getsampwidth(), rendered.getnchannels())
            self.assertEqual(samples[:2], [0, 0])
            self.assertAlmostEqual(samples[-1], expected, delta=1)
        # This stdio process has no live callback: it must not fabricate gain telemetry.
        telemetry = self.payload(self.require_tool_success("inspect_performance", {"expected_revision": exponential["revision"]}))
        self.assertFalse(telemetry["supported"])
        self.assertIsNone(telemetry["performance"])
        self.assertEqual(self.patch_snapshot(), exponential)

        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "amplifier"}))["preset"]
        self.assertEqual(preset["params"], {"level": .5, "response": 0})
        self.require_tool_success("save_module_preset", {"filename": "vca-exponential.opreset", "preset": preset})
        reviewed = self.payload(self.require_tool_success("read_module_preset", {"filename": "vca-exponential.opreset"}))["preset"]
        self.assertEqual(reviewed, preset)
        self.require_tool_success("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "amplifier", "parameter": "response", "value": 1}},
            {"tool": "set_parameter", "arguments": {"id": "amplifier", "parameter": "level", "value": .25}},
        ]})
        changed = self.patch_snapshot()
        restored = self.payload(self.require_tool_success("apply_module_preset", {
            "id": "amplifier", "preset": reviewed, "expected_revision": changed["revision"],
        }))
        self.assertEqual(restored["patch"], exponential["patch"])
        self.assertEqual(restored["history"]["undo_steps"], changed["history"]["undo_steps"] + 1)
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], changed["patch"])
        self.assertEqual(self.payload(self.require_tool_success("redo"))["patch"], restored["patch"])
        before_duplicate = self.patch_snapshot()
        duplicated = self.payload(self.require_tool_success("duplicate_modules", {
            "modules": [{"id": "amplifier", "new_id": "copy"}], "cable_mode": "inputs",
            "expected_revision": before_duplicate["revision"],
        }))
        copy = next(node for node in duplicated["patch"]["nodes"] if node["id"] == "copy")
        self.assertEqual(copy["params"], {"level": .5, "response": 0})
        duplicate_capture = self.payload(self.require_tool_success("capture_signal", {
            "id": "copy", "channel": 2, "seconds": .001, "points": 64,
        }))
        self.assertEqual(duplicate_capture["summary"]["dc"], .015625)
        self.assertEqual(duplicated["history"]["undo_steps"], before_duplicate["history"]["undo_steps"] + 1)
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], before_duplicate["patch"])
        self.assertEqual(self.payload(self.require_tool_success("redo"))["patch"], duplicated["patch"])

        before_rejection = self.patch_snapshot()
        self.require_tool_error("set_parameter", {"id": "amplifier", "parameter": "response", "value": .5})
        self.assertEqual(self.patch_snapshot(), before_rejection)
        self.require_tool_error("apply_edits", {"expected_revision": before_rejection["revision"], "edits": [
            {"tool": "set_parameter", "arguments": {"id": "amplifier", "parameter": "level", "value": .75}},
            {"tool": "set_parameter", "arguments": {"id": "copy", "parameter": "response", "value": .5}},
        ]})
        self.assertEqual(self.patch_snapshot(), before_rejection, "fractional response must roll back the earlier level edit and history")
        invalid_preset = {**reviewed, "params": {"level": .75, "response": .5}}
        self.require_tool_error("apply_module_preset", {"id": "amplifier", "preset": invalid_preset})
        self.assertEqual(self.patch_snapshot(), before_rejection)

    def test_stock_seq3_physical_buttons_and_all_named_outputs_through_mcp(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["stock_seq3"]
        self.assertEqual(descriptor["inputs"], ["tempo", "clock", "run", "reset", "steps"])
        expected = ["cv1", "cv2", "cv3", "trigger"] + [f"step{i}" for i in range(1, 9)] + ["steps", "clock", "run", "reset"]
        self.assertEqual(descriptor["outputs"], expected)
        self.assertEqual([descriptor["params"]["tempo"][k] for k in ("min", "max", "default")], [-2, 4, 1])
        parameters = {f"row{row}_{step}": row + step / 10 for row in range(1, 4) for step in range(1, 9)}
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
            "nodes": [{"id": "seq", "kind": "stock_seq3", "params": parameters}]}})
        before = self.patch_snapshot()
        events = [{"frame": 10, "id": "seq", "action": "clock_press"},
                  {"frame": 20, "id": "seq", "action": "clock_release"},
                  {"frame": 40, "id": "seq", "action": "reset_press"},
                  {"frame": 41, "id": "seq", "action": "reset_release"}]
        for port in expected:
            trace = self.payload(self.require_tool_success("capture_signal", {
                "id": "seq", "port": port, "seconds": .002, "points": 96,
                "module_events": events, "expected_revision": before["revision"]}))
            self.assertEqual(trace["module_event_count"], 4)
            self.assertEqual(trace["summary"]["active_frames"], 96)
            values = [point["mean"] for point in trace["bins"]]
            self.assertTrue(all(math.isfinite(value) for value in values))
            if port == "cv1":
                self.assertAlmostEqual(values[0], 1.1, places=6)
                self.assertAlmostEqual(values[10], 1.2, places=6)
                self.assertAlmostEqual(values[40], 1.1, places=6)
            if port == "clock":
                self.assertEqual(values[0], 0)
                self.assertEqual(values[10], 10)
                self.assertEqual(values[40], 10, "reset to another index triggers the source clock pulse")
        self.assertEqual(self.patch_snapshot(), before)

    def test_stock_seq3_rotation_presets_initialize_and_atomic_validation(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {"version": 2,
            "nodes": [{"id": "seq", "kind": "stock_seq3", "params": {
                "running": 0, "gate1": 0, "passthrough": 1, "row1_1": 3, "row3_8": -4}}]}})
        before = self.patch_snapshot()
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "seq"}))["preset"]
        self.assertEqual(preset["params"]["running"], 0)
        self.assertEqual(preset["params"]["gate1"], 0)
        self.assertEqual(preset["params"]["passthrough"], 1)
        self.require_tool_success("rotate_sequence", {"id": "seq", "direction": "right", "expected_revision": before["revision"]})
        rotated = self.patch_snapshot()["patch"]["nodes"][0]["params"]
        self.assertEqual(rotated["row1_2"], 3)
        self.assertEqual(rotated["row3_1"], -4)
        self.assertEqual(rotated["gate2"], 0)
        self.require_tool_success("undo")
        self.assertEqual(self.patch_snapshot()["patch"], before["patch"])
        self.require_tool_success("initialize_modules", {"ids": ["seq"]})
        initialized = self.payload(self.require_tool_success("capture_module_preset", {"id": "seq"}))["preset"]["params"]
        self.assertEqual(initialized["running"], 0, "Initialize preserves source Run")
        self.assertEqual(initialized["passthrough"], 0)
        self.assertTrue(all(initialized[f"gate{i}"] == 1 for i in range(1, 9)))
        self.assertTrue(all(initialized[f"row{r}_{i}"] == 0 for r in range(1, 4) for i in range(1, 9)))
        stable = self.patch_snapshot()
        self.require_tool_error("apply_edits", {"edits": [
            {"tool": "set_parameter", "arguments": {"id": "seq", "parameter": "row1_1", "value": 8}},
            {"tool": "set_parameter", "arguments": {"id": "seq", "parameter": "gate2", "value": .5}}]})
        self.assertEqual(self.patch_snapshot(), stable)

    def test_stock_vco_named_outputs_and_semitone_controls_through_mcp(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["stock_vco"]
        self.assertEqual(descriptor["inputs"], ["pitch", "fm", "sync", "pwm"])
        self.assertEqual(descriptor["outputs"], ["sine", "triangle", "saw", "square"])
        self.assertEqual([descriptor["params"]["frequency"][k] for k in ("min", "max", "default")], [-76, 76, 0])
        for name in ["fm_amount", "pwm_amount"]:
            self.assertEqual(descriptor["params"][name]["randomization"], "disabled")
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
            "nodes": [{"id": "vco", "kind": "stock_vco"}], "output": "vco", "output_port": "sine"}})
        original = self.patch_snapshot()
        for port in ["sine", "triangle", "saw", "square"]:
            trace = self.payload(self.require_tool_success("capture_signal", {
                "id": "vco", "port": port, "seconds": .01, "points": 256,
                "expected_revision": original["revision"],
            }))
            values = [point["mean"] for point in trace["bins"]]
            self.assertEqual(trace["summary"]["active_frames"], 480)
            self.assertTrue(all(math.isfinite(value) for value in values))
            self.assertGreater(max(values) - min(values), .1, port)
        self.assertEqual(self.patch_snapshot(), original, "offline taps do not edit or save DSP history")
        measured = self.payload(self.require_tool_success("inspect_performance"))
        self.assertFalse(measured["supported"])
        self.assertIsNone(measured["performance"], "headless MCP must not invent live VCO lamps")

    def test_stock_vco_presets_initialize_and_atomic_control_validation(self) -> None:
        parameters = {"frequency": -12, "pulse_width": .25, "fm_amount": -.5,
                      "pwm_amount": .75, "linear_fm": 1, "soft_sync": 1}
        self.require_tool_success("replace_patch", {"patch": {"version": 2,
            "nodes": [{"id": "vco", "kind": "stock_vco", "params": parameters}],
            "output": "vco", "output_port": "triangle"}})
        original = self.patch_snapshot()
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "vco"}))["preset"]
        self.assertEqual(preset["params"], parameters)
        self.assertNotIn("phase", preset)
        self.require_tool_success("save_patch", {"filename": "stock-vco.json"})
        duplicate = self.payload(self.require_tool_success("duplicate_modules", {
            "modules": [{"id": "vco", "new_id": "copy"}], "cable_mode": "inputs",
            "expected_revision": original["revision"],
        }))
        self.assertEqual(next(n for n in duplicate["patch"]["nodes"] if n["id"] == "copy")["params"], parameters)
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], original["patch"])
        self.assertEqual(self.payload(self.require_tool_success("redo"))["patch"], duplicate["patch"])
        before = self.patch_snapshot()
        for name, value in [("frequency", 77), ("frequency", -77), ("pulse_width", 0), ("linear_fm", .5), ("soft_sync", .5)]:
            self.require_tool_error("apply_edits", {"expected_revision": before["revision"], "edits": [
                {"tool": "set_parameter", "arguments": {"id": "vco", "parameter": "frequency", "value": 3}},
                {"tool": "set_parameter", "arguments": {"id": "copy", "parameter": name, "value": value}},
            ]})
            self.assertEqual(self.patch_snapshot(), before)
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "stock-vco.json"}))
        self.assertEqual(loaded["patch"], original["patch"])
        self.require_tool_success("initialize_modules", {"ids": ["vco"]})
        current = self.patch_snapshot()
        self.assertEqual(next(n for n in current["patch"]["nodes"] if n["id"] == "vco")["params"], {
            "frequency": 0, "pulse_width": .5, "fm_amount": 0, "pwm_amount": 0, "linear_fm": 0, "soft_sync": 0})
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], original["patch"])

    def test_stock_lfo_source_polarity_phase_origins_and_named_outputs_through_mcp(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["stock_lfo"]
        self.assertEqual(descriptor["inputs"], ["fm", "pwm", "reset", "clock"])
        self.assertEqual(descriptor["outputs"], ["sine", "triangle", "saw", "square"])
        self.assertEqual(descriptor["params"]["offset"]["default"], 1)
        self.assertEqual([descriptor["params"]["rate"][k] for k in ("min", "max", "default")], [-8, 10, 1])
        for name in ["fm_amount", "pwm_amount"]:
            self.assertEqual(descriptor["params"][name]["randomization"], "disabled")
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
            "nodes": [{"id": "lfo", "kind": "stock_lfo"}], "output": "lfo", "output_port": "sine"}})
        original = self.patch_snapshot()
        for port in ["sine", "triangle", "saw", "square"]:
            trace = self.payload(self.require_tool_success("capture_signal", {
                "id": "lfo", "port": port, "seconds": .001, "points": 64,
                "expected_revision": original["revision"],
            }))
            values = [point["mean"] for point in trace["bins"]]
            self.assertEqual(trace["summary"]["active_frames"], 48)
            if port == "square":
                self.assertEqual(values, [10] * 48)
            else:
                self.assertTrue(all(0 <= value < .05 for value in values), f"{port} must begin near unipolar zero")
                self.assertTrue(all(a <= b for a, b in zip(values, values[1:])))
        self.assertEqual(self.patch_snapshot(), original, "offline inspection is observational")
        self.require_tool_success("set_parameter", {"id": "lfo", "parameter": "invert", "value": 1})
        trace = self.payload(self.require_tool_success("capture_signal", {"id": "lfo", "port": "square", "seconds": .001}))
        self.assertTrue(all(point["mean"] == 0 for point in trace["bins"]))
        measured = self.payload(self.require_tool_success("inspect_performance"))
        self.assertFalse(measured["supported"])
        self.assertIsNone(measured["performance"], "headless MCP must not fabricate live clock or lamp readings")

    def test_stock_lfo_presets_duplicate_persistence_and_invalid_atomic_edits(self) -> None:
        parameters = {"rate": -2.5, "pulse_width": .25, "fm_amount": -.5,
                      "pwm_amount": .75, "offset": 0, "invert": 1}
        self.require_tool_success("replace_patch", {"patch": {"version": 2,
            "nodes": [{"id": "lfo", "kind": "stock_lfo", "params": parameters}],
            "output": "lfo", "output_port": "triangle"}})
        original = self.patch_snapshot()
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "lfo"}))["preset"]
        self.assertEqual(preset["params"], parameters)
        self.assertNotIn("clock_frequency", json.dumps(preset))
        self.require_tool_success("save_patch", {"filename": "stock-lfo.json"})
        duplicate = self.payload(self.require_tool_success("duplicate_modules", {
            "modules": [{"id": "lfo", "new_id": "copy"}], "cable_mode": "inputs",
            "expected_revision": original["revision"],
        }))
        self.assertEqual(next(n for n in duplicate["patch"]["nodes"] if n["id"] == "copy")["params"], parameters)
        self.assertEqual(duplicate["history"]["undo_steps"], original["history"]["undo_steps"] + 1)
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], original["patch"])
        self.assertEqual(self.payload(self.require_tool_success("redo"))["patch"], duplicate["patch"])
        before = self.patch_snapshot()
        for name, value in [("rate", 11), ("rate", -9), ("pulse_width", 0), ("offset", .5), ("invert", .5)]:
            self.require_tool_error("apply_edits", {"expected_revision": before["revision"], "edits": [
                {"tool": "set_parameter", "arguments": {"id": "lfo", "parameter": "rate", "value": 3}},
                {"tool": "set_parameter", "arguments": {"id": "copy", "parameter": name, "value": value}},
            ]})
            self.assertEqual(self.patch_snapshot(), before)
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "stock-lfo.json"}))
        self.assertEqual(loaded["patch"], original["patch"])
        self.require_tool_success("initialize_modules", {"ids": ["lfo"]})
        current = self.patch_snapshot()
        self.assertEqual(next(n for n in current["patch"]["nodes"] if n["id"] == "lfo")["params"], {
            "rate": 1, "pulse_width": .5, "fm_amount": 0, "pwm_amount": 0, "offset": 1, "invert": 0})
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], original["patch"])

    def test_split_named_outputs_silent_occupancy_and_stateless_mcp_roundtrips(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        split = modules["poly_split"]
        self.assertEqual(split["inputs"], ["signal"])
        self.assertEqual(split["outputs"], [f"out{i}" for i in range(1, 17)])
        self.assertEqual(split["params"], {})
        self.assertEqual(split["light_names"], [])
        self.require_tool_success("replace_patch", {"patch": {
            "version": 2, "sample_rate": 48000,
            "nodes": [{"id": "source", "kind": "poly_constant", "params": {
                "channels": 16, "v1": 0, "v2": -3, "v16": 7}},
                {"id": "split", "kind": "poly_split"}],
            "cables": [{"from": "source", "to": "split", "input": "signal"}],
            "output": "split", "output_port": "out16",
        }})
        original = self.patch_snapshot()
        for port, expected in [("out1", 0), ("out2", -3), ("out16", 7)]:
            trace = self.payload(self.require_tool_success("capture_signal", {
                "id": "split", "port": port, "seconds": .001, "points": 64,
                "expected_revision": original["revision"],
            }))
            self.assertEqual(trace["summary"]["active_frames"], 48)
            self.assertEqual([b["mean"] for b in trace["bins"]][1:], [expected] * 47)
        self.assertEqual(self.patch_snapshot(), original)
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "split"}))["preset"]
        self.assertEqual(preset["params"], {})
        self.assertNotIn("input_channels", json.dumps(preset))
        self.require_tool_success("save_patch", {"filename": "split.json"})
        self.require_tool_success("set_parameter", {"id": "source", "parameter": "channels", "value": 1})
        trace = self.payload(self.require_tool_success("capture_signal", {
            "id": "split", "port": "out16", "seconds": .001, "points": 64,
        }))
        self.assertTrue(all(b["mean"] == 0 for b in trace["bins"]))
        # This stdio process has no callback: topology must not become a fabricated live count.
        measured = self.payload(self.require_tool_success("inspect_performance"))
        self.assertFalse(measured["supported"])
        self.assertIsNone(measured["performance"])
        changed = self.patch_snapshot()
        self.require_tool_error("set_parameter", {"id": "split", "parameter": "input_channels", "value": 16})
        self.assertEqual(self.patch_snapshot(), changed)
        duplicate = self.payload(self.require_tool_success("duplicate_modules", {
            "modules": [{"id": "split", "new_id": "copy"}], "cable_mode": "inputs",
            "expected_revision": changed["revision"],
        }))
        copy = next(n for n in duplicate["patch"]["nodes"] if n["id"] == "copy")
        self.assertEqual(copy.get("params", {}), {})
        self.assertEqual(duplicate["history"]["undo_steps"], changed["history"]["undo_steps"] + 1)
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], changed["patch"])
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "split.json"}))
        self.assertEqual(loaded["patch"], original["patch"])

    def test_random_clock_interpolation_seed_presets_and_atomic_mcp_history(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["random"]
        self.assertEqual(descriptor["inputs"], ["rate_cv", "shape_cv", "trigger", "external", "probability_cv", "spread_cv"])
        self.assertEqual(descriptor["outputs"], ["stepped", "linear", "smooth", "exponential", "trigger"])
        self.assertEqual(descriptor["min_patch_version"], 2)
        params = {"rate": 7, "probability": 1, "spread": .75, "shape": .5,
                  "rate_cv": -.5, "probability_cv": .25, "spread_cv": -.75,
                  "shape_cv": .5, "unipolar": 1, "seed": 123}
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 8000, "nodes": []}})
        before = self.patch_snapshot()
        configured = self.payload(self.require_tool_success("apply_edits", {"expected_revision": before["revision"], "edits": [
            {"tool": "add_module", "arguments": {"id": "motion", "kind": "random", "params": params}},
            {"tool": "set_output", "arguments": {"id": "motion", "port": "linear"}},
        ]}))
        self.assertEqual(configured["history"]["undo_steps"], before["history"]["undo_steps"] + 1)
        for port in descriptor["outputs"]:
            args = {"id": "motion", "port": port, "seconds": .125, "points": 128,
                    "expected_revision": configured["revision"]}
            capture = self.payload(self.require_tool_success("capture_signal", args))
            self.assertEqual(capture["frames"], 1000)
            self.assertGreater(capture["summary"]["peak"], .001)
            self.assertEqual(capture, self.payload(self.require_tool_success("capture_signal", args)))
        self.assertEqual(self.patch_snapshot(), configured, "offline captures must not advance editable state/history")
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "motion"}))["preset"]
        changed = self.payload(self.require_tool_success("set_parameter", {
            "id": "motion", "parameter": "shape", "value": .25, "expected_revision": configured["revision"],
        }))
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], configured["patch"])
        self.assertEqual(self.payload(self.require_tool_success("redo"))["patch"], changed["patch"])
        restored = self.payload(self.require_tool_success("apply_module_preset", {"id": "motion", "preset": preset}))
        self.assertEqual(restored["patch"]["nodes"][0]["params"], params)
        for key, value in [("shape", 1.1), ("seed", 1.5), ("unipolar", .5), ("rate_cv", -1.1)]:
            self.require_tool_error("set_parameter", {"id": "motion", "parameter": key, "value": value})
            self.assertEqual(self.patch_snapshot(), restored)
        self.require_tool_success("save_patch", {"filename": "random-motion.json"})
        self.assertEqual(json.loads((self.workspace / "random-motion.json").read_text()), restored["patch"])
        self.require_tool_success("set_parameter", {"id": "motion", "parameter": "seed", "value": 456})
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "random-motion.json"}))
        self.assertEqual(loaded["patch"], restored["patch"])
        # External CV controls four output widths; the accepted-event pulse stays mono.
        external = {"version": 2, "sample_rate": 8000, "nodes": [
            {"id": "external", "kind": "poly_constant", "params": {"channels": 5, "v1": .25, "v2": -.5, "v5": .75}},
            {"id": "clock", "kind": "gate", "params": {"frequency": 13, "duty": .25}},
            {"id": "motion", "kind": "random", "params": {"shape": 0}},
        ], "cables": [{"from": "external", "to": "motion", "input": "external"},
                       {"from": "clock", "to": "motion", "input": "trigger"}]}
        self.require_tool_success("replace_patch", {"patch": external})
        snapshot = self.patch_snapshot()
        for port in descriptor["outputs"]:
            analysis = self.payload(self.require_tool_success("analyze_signal", {"id": "motion", "port": port, "seconds": .125}))
            self.assertEqual(analysis["max_channels"], 1 if port == "trigger" else 5)
            self.assertGreater(analysis["channels"][0]["peak"], .001)
        self.assertEqual(self.patch_snapshot(), snapshot)

    def test_stock_delay_signed_cv_edits_history_persistence_and_offline_render(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["stock_delay"]
        controls = ("time", "feedback", "tone", "mix")
        depths = {f"{name}_cv_amount" for name in controls}
        self.assertEqual(descriptor["min_patch_version"], 2)
        self.assertEqual(descriptor["inputs"], ["time_cv", "feedback_cv", "tone_cv", "mix_cv", "signal", "clock"])
        self.assertEqual(descriptor["outputs"], ["mix", "wet"])
        self.assertEqual(set(descriptor["params"]), set(controls) | depths)
        for name in controls:
            self.assertEqual([descriptor["params"][name][key] for key in ("min", "max")], [0, 1])
        for name in depths:
            self.assertEqual([descriptor["params"][name][key] for key in ("default", "min", "max")], [0, -1, 1])

        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 8000, "nodes": []}})
        empty = self.patch_snapshot()
        # Dyadic values must survive the complete RPC/history/JSON path exactly.
        params = {"time": .25, "feedback": .5, "tone": .5, "mix": .5,
                  "time_cv_amount": -.5, "feedback_cv_amount": .25,
                  "tone_cv_amount": -.75, "mix_cv_amount": .5}
        edits = [
            {"tool": "add_module", "arguments": {"id": "signal", "kind": "poly_constant", "params": {
                "channels": 3, "v1": .125, "v2": -.25, "v3": .375}}},
            {"tool": "add_module", "arguments": {"id": "cv", "kind": "constant", "params": {"value": 2.5}}},
            {"tool": "add_module", "arguments": {"id": "clock", "kind": "gate", "params": {"frequency": 13, "duty": .25}}},
            {"tool": "add_module", "arguments": {"id": "echo", "kind": "stock_delay", "params": params}},
        ]
        routes = {"signal": "signal", "clock": "clock", **{f"{name}_cv": "cv" for name in controls}}
        edits.extend({"tool": "connect", "arguments": {
            "cable_id": f"echo_{port}", "from": source, "to": "echo", "input": port,
        }} for port, source in routes.items())
        edits.append({"tool": "set_output", "arguments": {"id": "echo", "port": "mix"}})
        configured = self.payload(self.require_tool_success("apply_edits", {
            "edits": edits, "expected_revision": empty["revision"],
        }))
        self.assertEqual(configured["history"]["undo_steps"], empty["history"]["undo_steps"] + 1)
        echo = next(node for node in configured["patch"]["nodes"] if node["id"] == "echo")
        self.assertEqual(echo["params"], params)
        self.assertEqual({cable["input"]: cable["from"] for cable in configured["patch"]["cables"]
                          if cable["to"] == "echo"}, routes)
        self.assertEqual(len({cable["id"] for cable in configured["patch"]["cables"]}), 6)

        changed = self.payload(self.require_tool_success("set_parameter", {
            "id": "echo", "parameter": "time_cv_amount", "value": -.875,
            "expected_revision": configured["revision"],
        }))
        changed_params = {**params, "time_cv_amount": -.875}
        self.assertEqual(next(node for node in changed["patch"]["nodes"] if node["id"] == "echo")["params"], changed_params)
        self.assertEqual(changed["history"]["undo_steps"], configured["history"]["undo_steps"] + 1)
        self.assertEqual(self.payload(self.require_tool_success("undo"))["patch"], configured["patch"])
        redone = self.payload(self.require_tool_success("redo"))
        self.assertEqual(redone["patch"], changed["patch"])
        for arguments in [
            {"id": "echo", "parameter": "time_cv_amount", "value": -1.125},
            {"id": "echo", "parameter": "mix", "value": 1.125},
            {"id": "echo", "parameter": "tone", "value": .25, "expected_revision": configured["revision"]},
        ]:
            self.require_tool_error("set_parameter", arguments)
            self.assertEqual(self.patch_snapshot(), redone, "rejected edits must not alter revision, controls or history")

        self.require_tool_success("save_patch", {"filename": "stock-delay.json"})
        saved = json.loads((self.workspace / "stock-delay.json").read_text())
        self.assertEqual(saved, redone["patch"])
        self.assertEqual(self.patch_snapshot(), redone)
        self.require_tool_success("set_parameter", {"id": "echo", "parameter": "mix", "value": .25})
        loaded = self.payload(self.require_tool_success("load_patch", {"filename": "stock-delay.json"}))
        self.assertEqual(loaded["patch"], saved)
        self.assertEqual(next(node for node in loaded["patch"]["nodes"] if node["id"] == "echo")["params"], changed_params)

        # Each bounded probe starts from reset and must leave the editable patch,
        # its revision and undo history intact. Both Delay outputs stay mono.
        for port in ["mix", "wet"]:
            arguments = {"id": "echo", "port": port, "seconds": .125, "points": 128,
                         "expected_revision": loaded["revision"]}
            capture = self.payload(self.require_tool_success("capture_signal", arguments))
            self.assertEqual((capture["frames"], capture["points"]), (1000, 128))
            self.assertEqual(capture["summary"]["active_frames"], 1000)
            self.assertGreater(capture["summary"]["peak"], .001)
            self.assertTrue(all(math.isfinite(capture["summary"][field]) for field in ("min", "max", "dc", "rms", "peak")))
            self.assertEqual(capture, self.payload(self.require_tool_success("capture_signal", arguments)))
            analysis = self.payload(self.require_tool_success("analyze_signal", {
                "id": "echo", "port": port, "seconds": .125, "expected_revision": loaded["revision"],
            }))
            self.assertEqual(analysis["max_channels"], 1)
            self.assertEqual(analysis["channels"][0]["active_frames"], 1000)
        rendered = self.payload(self.require_tool_success("render_audio", {
            "filename": "stock-delay.wav", "seconds": .125,
        }))
        self.assertEqual((rendered["sample_rate"], rendered["channels"], rendered["frames"]), (8000, 1, 1000))
        self.assertEqual(rendered["revision"], loaded["revision"])
        self.assertGreater(rendered["peak"], .001)
        self.assertLess(rendered["peak"], 1)
        self.assertTrue(math.isfinite(rendered["rms"]))
        with wave.open(str(self.workspace / "stock-delay.wav"), "rb") as wav:
            self.assertEqual((wav.getframerate(), wav.getnchannels(), wav.getnframes()), (8000, 1, 1000))
            samples = self._first_channel_samples(wav.readframes(1000), wav.getsampwidth(), wav.getnchannels())
        self.assertTrue(any(abs(sample) > 100 for sample in samples))
        self.assertEqual(self.patch_snapshot(), loaded)

    def test_stock_vcf_controls_capture_consumers_bypass_and_history(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["stock_vcf"]
        self.assertEqual(descriptor["inputs"], ["cutoff_cv", "resonance_cv", "drive_cv", "signal"])
        self.assertEqual(descriptor["outputs"], ["lowpass", "highpass"])
        self.assertEqual(descriptor["polyphony"]["primary_input"], "signal")
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
            "nodes": [{"id": "signal", "kind": "poly_constant", "params": {"channels": 5, "v1": 2}},
                      {"id": "filter", "kind": "stock_vcf", "params": {"frequency": .7}}],
            "cables": [{"from": "signal", "to": "filter", "input": "signal"}]}})
        before = self.patch_snapshot()
        for port in ["lowpass", "highpass"]:
            args = {"id": "filter", "port": port, "seconds": .02, "points": 128}
            capture = self.payload(self.require_tool_success("capture_signal", args))
            self.assertGreater(max(abs(b["mean"]) for b in capture["bins"]), .001)
            self.assertEqual(capture, self.payload(self.require_tool_success("capture_signal", args)))
        self.assertEqual(before, self.patch_snapshot())
        self.require_tool_error("set_parameter", {"id": "filter", "parameter": "drive", "value": 2})
        self.require_tool_success("set_parameter", {"id": "filter", "parameter": "cutoff_cv_amount", "value": -.75})
        self.require_tool_success("undo")
        self.assertEqual(before["patch"], self.patch_snapshot()["patch"])
        self.require_tool_success("set_bypass", {"id": "filter", "bypassed": True})
        for port in ["lowpass", "highpass"]:
            capture = self.payload(self.require_tool_success("capture_signal", {"id": "filter", "port": port, "seconds": .002, "points": 96}))
            self.assertEqual([b["mean"] for b in capture["bins"]][2:], [2.0] * 94)

    def test_adsr_eg_cv_envelope_and_sampled_manual_events(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["adsr_eg"]
        self.assertEqual(descriptor["inputs"], ["attack_cv", "decay_cv", "sustain_cv", "release_cv", "gate", "retrigger"])
        self.assertEqual(descriptor["runtime_actions"], ["push_press", "push_release"])
        self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000, "nodes": [
            {"id": "env", "kind": "adsr_eg", "params": {"attack": 0, "decay": 0, "sustain": .25, "release": 0}}]}})
        before = self.patch_snapshot()
        args = {"id": "env", "port": "out", "seconds": .004, "points": 192, "module_events": [
            {"frame": 0, "id": "env", "action": "push_press"},
            {"frame": 160, "id": "env", "action": "push_release"}]}
        capture = self.payload(self.require_tool_success("capture_signal", args))
        values = [point["mean"] for point in capture["bins"]]
        self.assertEqual(values[:15], [0] * 15)
        self.assertGreater(values[15], 0)
        self.assertGreater(max(values), 10)
        self.assertLess(max(values), 10.1)
        self.assertLess(values[160], values[159])
        self.assertGreater(values[-1], 0)
        self.assertEqual(capture, self.payload(self.require_tool_success("capture_signal", args)))
        args["module_events"][-1]["frame"] = 0
        silent = self.payload(self.require_tool_success("capture_signal", args))
        self.assertTrue(all(point["mean"] == 0 for point in silent["bins"]))
        self.assertEqual(self.patch_snapshot(), before)
        for action in ["hold_press", "reset_press", "clock"]:
            self.require_tool_error("capture_signal", {"id": "env", "seconds": .004,
                "module_events": [{"frame": 0, "id": "env", "action": action}]})
        self.require_tool_error("set_parameter", {"id": "env", "parameter": "attack", "value": -1})
        self.require_tool_error("set_parameter", {"id": "env", "parameter": "sustain_amount", "value": 2})

    def test_shift_register_normaling_mono_actions_and_persistent_state(self) -> None:
        modules = {m["kind"]: m for m in self.payload(self.require_tool_success("catalog"))["modules"]}
        descriptor = modules["sh_asr"]
        self.assertEqual(descriptor["inputs"], [f"in{i}" for i in range(1, 9)] + [f"trig{i}" for i in range(1, 9)])
        self.assertEqual(descriptor["outputs"], [f"out{i}" for i in range(1, 9)])
        self.assertEqual(descriptor["runtime_actions"], ["push_press", "push_release", "reset_press", "reset_release"])
        self.assertEqual(sum(p.get("hidden", False) for p in descriptor["params"].values()), 10)
        self.assertTrue(all(p["maximum_channels"] == 1 for p in descriptor["output_metadata"]))
        rng = 1 ^ 0xA341316C
        params = {f"value{i}": float(i) for i in range(1, 9)} | {"rng_hi": rng >> 16, "rng_lo": rng & 0xffff}
        patch = {"version": 2, "sample_rate": 48000, "nodes": [
            {"id": "register", "kind": "sh_asr", "params": params},
            {"id": "source", "kind": "poly_constant", "params": {"channels": 16, "v1": 4.5, "v2": 15}},
            {"id": "low", "kind": "constant", "params": {"value": 0}},
        ], "cables": [{"from": "source", "to": "register", "input": "in1"}]}
        self.require_tool_success("replace_patch", {"patch": patch})
        events = [{"frame": frame, "id": "register", "action": action}
                  for frame, action in [(32, "push_press"), (33, "push_release"), (64, "push_press"), (65, "push_release")]]
        for row in (1, 3, 8):
            capture = self.payload(self.require_tool_success("capture_signal", {
                "id": "register", "port": f"out{row}", "seconds": .002, "module_events": events}))
            expected = [float(row)] * 32
            expected += [4.5 if row == 1 else float(row - 1)] * 32
            expected += [4.5 if row <= 2 else float(row - 2)] * 32
            self.assertEqual([b["mean"] for b in capture["bins"]], expected)
            analyzed = self.payload(self.require_tool_success("analyze_signal", {
                "id": "register", "port": f"out{row}", "seconds": .002, "module_events": events}))
            self.assertEqual(analyzed["max_channels"], 1)
        # A patched LOW clock at row 4 breaks inherited PUSH propagation.
        patch["cables"].append({"from": "low", "to": "register", "input": "trig4"})
        self.require_tool_success("replace_patch", {"patch": patch})
        held = self.payload(self.require_tool_success("capture_signal", {
            "id": "register", "port": "out8", "seconds": .002, "module_events": events}))
        self.assertTrue(all(b["mean"] == 8 for b in held["bins"]))
        # CLEAR wins over captures regardless of the two buttons' queue order.
        for actions in [("reset_press", "push_press"), ("push_press", "reset_press")]:
            cleared = self.payload(self.require_tool_success("capture_signal", {
                "id": "register", "port": "out8", "seconds": .002,
                "module_events": [{"frame": 32, "id": "register", "action": a} for a in actions]}))
            self.assertEqual([b["mean"] for b in cleared["bins"]], [8] * 32 + [0] * 64)
        # Rendering is isolated; saved/preset/duplicate state remains the configured snapshot.
        captured = self.payload(self.require_tool_success("capture_project_patch"))
        state = captured["shift_registers"]["register"]
        self.assertEqual(state, {"values": list(range(1, 9)), "rng_state": rng})
        preset = self.payload(self.require_tool_success("capture_module_preset", {"id": "register"}))["preset"]
        self.assertEqual(preset["params"]["value8"], 8)
        self.require_tool_success("duplicate_modules", {"modules": [{"id": "register", "new_id": "copy"}]})
        self.assertEqual(self.payload(self.require_tool_success("capture_project_patch"))["shift_registers"]["copy"], state)
        before = self.patch_snapshot()
        self.require_tool_success("randomize_modules", {"ids": ["register"], "seed": 42})
        self.assertEqual(self.patch_snapshot(), before)
        self.require_tool_success("set_parameter", {"id": "register", "parameter": "seed", "value": 123})
        reseeded = self.payload(self.require_tool_success("capture_project_patch"))["shift_registers"]["register"]
        self.assertEqual(reseeded["values"], state["values"])
        self.assertEqual(reseeded["rng_state"], 123 ^ 0xA341316C)
        self.require_tool_success("initialize_modules", {"ids": ["register"]})
        self.assertEqual(self.payload(self.require_tool_success("capture_project_patch"))["shift_registers"]["register"],
                         {"values": [0] * 8, "rng_state": rng})
        self.require_tool_success("undo")
        self.assertEqual(self.payload(self.require_tool_success("capture_project_patch"))["shift_registers"]["register"], reseeded)
        self.require_tool_success("save_patch", {"filename": "shift-register.json"})
        self.require_tool_success("load_patch", {"filename": "shift-register.json"})
        self.assertEqual(self.payload(self.require_tool_success("capture_project_patch"))["shift_registers"]["register"], reseeded)
        # Independently reproduce random draws: RND consumes once even with IN1 patched.
        def f32(value: float) -> float:
            return struct.unpack("<f", struct.pack("<f", value))[0]
        for range_, low, width in [(0, 0, 10), (1, 0, 5), (2, 0, 1), (3, -10, 20), (4, -5, 10), (5, -1, 2)]:
            self.require_tool_success("replace_patch", {"patch": {"version": 2, "sample_rate": 48000,
                "nodes": [{"id": "register", "kind": "sh_asr", "params": {"random": 1, "range": range_}}]}})
            stream = rng
            values = []
            for _ in range(2):
                stream ^= (stream << 13) & 0xffffffff
                stream ^= stream >> 17
                stream ^= (stream << 5) & 0xffffffff
                values.append(f32(low + f32(width * f32((stream >> 8) / 16777216))))
            random_capture = self.payload(self.require_tool_success("capture_signal", {
                "id": "register", "port": "out1", "seconds": .002, "module_events": events}))
            actual = [b["mean"] for b in random_capture["bins"]]
            self.assertEqual([struct.pack("<f", v) for v in actual],
                             [struct.pack("<f", v) for v in [0] * 32 + [values[0]] * 32 + [values[1]] * 32])
        for invalid in [{"value1": 1}, {"rng_hi": 1}, {"range": .5}, {"random": 2}, {"seed": -1}]:
            before = self.patch_snapshot()
            self.require_tool_error("add_module", {"id": "invalid", "kind": "sh_asr", "params": invalid})
            self.assertEqual(self.patch_snapshot(), before)
        demo = json.loads((REPOSITORY / "examples" / "shift-register-canon.json").read_text())
        self.require_tool_success("replace_patch", {"patch": demo})
        result = self.payload(self.require_tool_success("render_audio", {"filename": "canon.wav", "seconds": 1}))
        self.assertLess(result["peak"], 1)
        with wave.open(str(self.workspace / "canon.wav"), "rb") as rendered:
            samples = self._first_channel_samples(rendered.readframes(rendered.getnframes()), rendered.getsampwidth(), rendered.getnchannels())
        self.assertGreater(max(abs(s) for s in samples), 100)
        self.assertLess(max(abs(s) for s in samples), 32767)

    def test_construction_receipts_are_real_bounded_and_revision_matched(self) -> None:
        before = self.patch_snapshot()
        steps = [
            {"label": "Add a source", "edits": [{"tool": "add_module", "arguments": {"id": "source", "kind": "constant"}}]},
            {"label": "Add a level control", "edits": [{"tool": "add_module", "arguments": {"id": "level", "kind": "gain"}}]},
            {"label": "Connect the signal", "edits": [{"tool": "connect", "arguments": {"from": "source", "to": "level", "input": "signal", "cable_id": "visible"}}]},
            {"label": "Adjust the level", "edits": [
                {"tool": "set_parameter", "arguments": {"id": "level", "parameter": "gain", "value": .75}},
                {"tool": "set_parameter", "arguments": {"id": "level", "parameter": "gain", "value": .25}},
            ]},
            {"label": "Recreate the cable", "edits": [
                {"tool": "disconnect", "arguments": {"cable_id": "visible"}},
                {"tool": "connect", "arguments": {"from": "source", "to": "level", "input": "signal", "cable_id": "visible"}},
            ]},
        ]
        observed = []
        for index, step in enumerate(steps):
            # One-step plans retain the exact completed receipt indefinitely;
            # client scheduling cannot skip it while subsequent steps commit.
            started = self.payload(self.require_tool_success("start_construction", {
                "expected_revision": before["revision"] + index,
                "interval_ms": 100, "steps": [step],
            }))
            self.assertIsNone(started["last_commit"])
            deadline = time.monotonic() + 10
            while True:
                state = self.payload(self.require_tool_success("inspect_construction", {"plan_id": started["plan_id"]}))
                if state["status"] == "completed":
                    break
                self.assertEqual(state["status"], "running")
                self.assertLess(time.monotonic(), deadline)
                time.sleep(.01)
            receipt = state["last_commit"]
            self.assertEqual(receipt["plan_id"], started["plan_id"])
            self.assertEqual(receipt["revision"], before["revision"] + index + 1)
            self.assertEqual(receipt["revision"], receipt["before_revision"] + 1)
            self.assertEqual(receipt["revision"], state["last_revision"])
            self.assertEqual(receipt["step_index"], 0)
            self.assertEqual(state["completed_steps"], 1)
            self.assertIsInstance(receipt["age_ms"], int)
            self.assertGreaterEqual(receipt["age_ms"], 0)
            self.assertIsInstance(receipt["invalidate_existing_cables"], bool)
            self.assertLessEqual(sum(len(receipt[k]) for k in ("new_modules", "new_cables", "moved_cables", "parameter_changes")), 128)
            observed.append(receipt)
        self.assertEqual(observed[0]["new_modules"], [{"id": "source", "kind": "constant"}])
        self.assertEqual(observed[2]["new_cables"], [{"id": "visible", "from": "source", "output": "out", "to": "level", "input": "signal"}])
        self.assertEqual(observed[3]["parameter_changes"], [{"id": "level", "kind": "gain", "parameter": "gain", "old_effective": 1, "new_effective": .25}])
        self.assertEqual(observed[4]["new_cables"], [])
        self.assertEqual(observed[4]["moved_cables"], [])
        self.assertTrue(observed[4]["invalidate_existing_cables"])
        final = self.patch_snapshot()
        self.assertEqual(final["history"]["undo_steps"], before["history"]["undo_steps"] + 5)
        self.assertEqual(final["revision"], before["revision"] + 5)
        self.require_tool_success("undo")
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], final["patch"])

    def test_direct_and_paced_cable_moves_publish_exact_successful_routes(self) -> None:
        self.require_tool_success("replace_patch", {"patch": {"version": 2,
            "nodes": [{"id": "s", "kind": "constant"}, {"id": "s2", "kind": "constant"},
                      {"id": "a", "kind": "gain"}, {"id": "b", "kind": "gain"}],
            "cables": [{"id": "moving", "from": "s", "to": "a", "input": "signal"}]}})
        self.assertIsNone(self.payload(self.require_tool_success("inspect_construction"))["last_move"])
        before = self.patch_snapshot()
        self.require_tool_success("move_cables", {"cable_ids": ["moving"], "end": "input",
            "node": "b", "port": "signal", "expected_revision": before["revision"]})
        state = self.payload(self.require_tool_success("inspect_construction"))
        self.assertEqual(state["status"], "idle")
        direct = state["last_move"]
        route_a = {"from": "s", "output": "out", "to": "a", "input": "signal"}
        route_b = {"from": "s", "output": "out", "to": "b", "input": "signal"}
        expected = [{"id": "moving", "before": route_a, "after": route_b}]
        self.assertEqual(direct["moved_cables"], expected)
        self.assertEqual(direct["before_revision"], before["revision"])
        self.assertEqual(direct["revision"], before["revision"] + 1)
        self.assertEqual(self.patch_snapshot()["patch"]["cables"][0]["to"], "b")
        self.require_tool_error("move_cables", {"cable_ids": ["moving"], "end": "input",
            "node": "b", "port": "missing"})
        retained = self.payload(self.require_tool_success("inspect_construction"))["last_move"]
        self.assertGreaterEqual(retained["age_ms"], direct["age_ms"])
        self.assertEqual({k: v for k, v in retained.items() if k != "age_ms"},
                         {k: v for k, v in direct.items() if k != "age_ms"})
        started = self.payload(self.require_tool_success("start_construction", {
            "expected_revision": direct["revision"], "interval_ms": 100,
            "steps": [{"label": "Move the source", "edits": [{"tool": "move_cables", "arguments": {
                "cable_ids": ["moving"], "end": "output", "node": "s2", "port": "out"}}]}]}))
        self.assertIsNone(started["last_move"])
        deadline = time.monotonic() + 10
        while True:
            state = self.payload(self.require_tool_success("inspect_construction", {"plan_id": started["plan_id"]}))
            if state["status"] == "completed":
                break
            self.assertEqual(state["status"], "running")
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.01)
        expected = [{"id": "moving", "before": route_b, "after": {**route_b, "from": "s2"}}]
        self.assertEqual(state["last_commit"]["moved_cables"], expected)
        self.assertEqual(state["last_move"]["moved_cables"], expected)
        self.assertEqual(state["last_commit"]["revision"], state["last_move"]["revision"])
        final = self.patch_snapshot()
        self.assertEqual(final["patch"]["cables"][0]["from"], "s2")
        self.assertEqual(final["history"]["undo_steps"], before["history"]["undo_steps"] + 2)
        self.require_tool_success("undo")
        self.assertIsNone(self.payload(self.require_tool_success("inspect_construction"))["last_move"])
        self.require_tool_success("redo")
        self.assertEqual(self.patch_snapshot()["patch"], final["patch"])
        self.assertIsNone(self.payload(self.require_tool_success("inspect_construction"))["last_move"])

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
