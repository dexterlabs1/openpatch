"""Synthetic STIW conversion acceptance; no user score or plugin execution.

Run: python3 -m unittest discover -s scripts -p test_rebuild_stiw.py
Set OPENPATCH_BIN to opt into the single headless compiler/catalog/save test.
"""
from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import random
import tempfile
import unittest

from rebuild_stiw import Builder, native_score, stock_vcf_frequency_bounds
from test_mcp import MCPProcess, PROTOCOL_VERSION

SCRIPTS = Path(__file__).resolve().parent
FIXTURE = SCRIPTS / "fixtures" / "stiw-source-structure.json"
SOURCE_MAP = SCRIPTS / "stiw_source_map.json"


def source_fixture():
    return json.loads(FIXTURE.read_text(encoding="utf-8"))


def identities():
    return json.loads(SOURCE_MAP.read_text(encoding="utf-8"))["modules"]


def module_by_alias(source, alias):
    identity = next(item for item in identities() if item["alias"] == alias)
    return next(module for module in source["modules"] if str(module["id"]) == str(identity["id"]))


def endpoint(cable):
    return cable["from"], cable["output"], cable["to"], cable["input"]


def cutoff_endpoint_case(value):
    source = source_fixture()
    # One automated and one direct stock frequency exercise both paths.
    for alias in ("bass_filter", "stab_filter"):
        module = module_by_alias(source, alias)
        next(param for param in module["params"] if param["id"] == 0)["value"] = value
    for point in module_by_alias(source, "seqB")["data"]["lanes"][4]["points"]:
        point["value"] = value
    scores = [native_score(module_by_alias(source, alias)["data"])
              for alias in ("seqA", "seqB")]
    return source, scores


class STIWConversionTests(unittest.TestCase):
    def build(self, source=None, scores=None, **options):
        builder = Builder(source if source is not None else source_fixture(), scores, **options)
        patch, translations = builder.create()
        return builder, patch, translations

    def test_stock_vcf_native_f32_cutoff_endpoints_are_inclusive_for_saved_controls_and_raw_lanes(self):
        # Independently recorded from the native stock_vcf catalog, rather than
        # recalculating the helper's formula or using its returned bounds.
        native_bounds = (-0.0031360150314867496, 1.1393855810165405)
        self.assertEqual(stock_vcf_frequency_bounds(), native_bounds)
        for bound in native_bounds:
            source, scores = cutoff_endpoint_case(bound)
            for override in (None, scores):
                with self.subTest(bound=bound, explicit_override=override is not None):
                    _, patch, _ = self.build(source, override)
                    nodes = {node["id"]: node for node in patch["nodes"]}
                    self.assertEqual(nodes["stab_filter"]["params"]["frequency"], bound)
                    self.assertEqual(nodes["bass_filter"]["params"]["frequency"], 0.5)
                    self.assertEqual(nodes["seqB"]["score"]["lanes"][4]["points"][0]["value"],
                                     10 * bound - 5)
        # Check each input independently so one failing validation cannot mask
        # a missing check in the saved, source-lane or explicit-override path.
        for outside in (native_bounds[0] - 1e-5, native_bounds[1] + 1e-5):
            for path in ("saved_automated", "saved_direct", "raw_lane", "override_lane"):
                source = source_fixture()
                scores = [native_score(module_by_alias(source, alias)["data"])
                          for alias in ("seqA", "seqB")]
                if path.startswith("saved_"):
                    alias = "bass_filter" if path == "saved_automated" else "stab_filter"
                    module = module_by_alias(source, alias)
                    next(param for param in module["params"] if param["id"] == 0)["value"] = outside
                elif path == "raw_lane":
                    module_by_alias(source, "seqB")["data"]["lanes"][4]["points"][0]["value"] = outside
                else:
                    scores[1]["lanes"][4]["points"][0]["value"] = outside
                with self.subTest(outside=outside, path=path):
                    with self.assertRaises(ValueError):
                        self.build(source, scores if path == "override_lane" else None)

    def test_public_fixture_has_55_typed_roles_and_83_unique_routes_without_song_content(self):
        source = source_fixture()
        self.assertEqual(len(source["modules"]), 55)
        self.assertEqual(len(source["cables"]), 83)
        self.assertEqual(len({str(m["id"]) for m in source["modules"]}), 55)
        self.assertEqual(len({str(c["id"]) for c in source["cables"]}), 83)
        self.assertEqual(len(identities()), 55)
        for module in source["modules"]:
            with self.subTest(id=module["id"]):
                self.assertIsInstance(module["plugin"], str)
                self.assertIsInstance(module["model"], str)
                self.assertIsInstance(module["version"], str)
                for param in module["params"]:
                    self.assertIsInstance(param["id"], int)
                    self.assertIsInstance(param["value"], (int, float))
                if module["model"] == "Notes":
                    self.assertIn("Synthetic role", module["data"]["text"])
                if module["model"] == "Sequencer":
                    self.assertEqual(module["data"]["selectedPattern"], "Synthetic")
                    self.assertEqual(module["data"]["patterns"][0]["length"], 4)
        # Independent fixed IDs catch accidental replacement by an unrelated
        # 55-module patch even if a permissive converter accepts its counts.
        self.assertEqual(str(module_by_alias(source, "bass_filter")["id"]), "5079823893699916")
        self.assertEqual(str(module_by_alias(source, "notes_clock")["id"]), "1588907208190973")

    def test_stable_id_mapping_survives_module_and_cable_reordering(self):
        source = source_fixture()
        before = copy.deepcopy(source)
        baseline, patch, translations = self.build(source)
        shuffled = copy.deepcopy(source)
        random.Random(719).shuffle(shuffled["modules"])
        random.Random(913).shuffle(shuffled["cables"])
        reordered, other, other_translations = self.build(shuffled)
        self.assertEqual(source, before, "Conversion must not rewrite caller-owned raw data")
        self.assertEqual(other, patch)
        self.assertEqual(other_translations, translations)
        self.assertEqual(reordered.roles, baseline.roles)
        self.assertEqual(len(baseline.roles), 55)
        for identity in identities():
            self.assertEqual(baseline.roles[identity["alias"]]["source_id"], str(identity["id"]))

    def test_same_size_wrong_identity_missing_duplicate_and_version_are_rejected(self):
        for mistake in ("plugin", "model", "version", "id", "duplicate", "missing"):
            source = source_fixture()
            with self.subTest(mistake=mistake):
                if mistake in ("plugin", "model", "version"):
                    source["modules"][19][mistake] = "not-the-curated-source"
                elif mistake == "id":
                    source["modules"][19]["id"] = 123456789
                elif mistake == "duplicate":
                    source["modules"][19]["id"] = source["modules"][20]["id"]
                else:
                    source["modules"].pop()
                with self.assertRaises(ValueError):
                    self.build(source)

    def test_same_size_wrong_cable_identity_endpoint_port_and_duplicates_are_rejected(self):
        for mistake in ("id", "duplicate", "endpoint", "input_port", "output_port", "missing"):
            source = source_fixture()
            with self.subTest(mistake=mistake):
                cable = source["cables"][0]
                if mistake == "id":
                    cable["id"] = 123456789
                elif mistake == "duplicate":
                    cable["id"] = source["cables"][1]["id"]
                elif mistake == "endpoint":
                    cable["inputModuleId"] = module_by_alias(source, "audio")["id"]
                elif mistake == "input_port":
                    cable["inputId"] = 999
                elif mistake == "output_port":
                    cable["outputId"] = 999
                else:
                    source["cables"].pop()
                with self.assertRaises(ValueError):
                    self.build(source)

    def test_all_83_cables_have_nonempty_exact_provenance(self):
        builder, patch, translations = self.build()
        self.assertEqual(len(translations), 83)
        self.assertEqual({item["source_id"] for item in translations},
                         {str(c["id"]) for c in source_fixture()["cables"]})
        native = {c["id"]: c for c in patch["cables"]}
        self.assertEqual(len(native), len(patch["cables"]))
        for item in translations:
            with self.subTest(source_id=item["source_id"]):
                self.assertTrue(item["native_cable_ids"])
                self.assertIsNone(item["gap"])
                out = builder.outs[item["from"]][item["output_id"]]
                inputs = builder.ins[item["to"]][item["input_id"]]
                inputs = inputs if isinstance(inputs, list) else [inputs]
                self.assertEqual([endpoint(native[ident]) for ident in item["native_cable_ids"]],
                                 [(out[0], out[1], inp[0], inp[1]) for inp in inputs])

    def test_all_14_stock_vcas_preserve_response_and_accept_raw_cv_without_extra_scaler(self):
        builder, patch, _ = self.build()
        nodes = {node["id"]: node for node in patch["nodes"]}
        aliases = [item["alias"] for item in identities() if item["model"] == "VCA-1"]
        self.assertEqual(len(aliases), 14)
        self.assertEqual(sum(node["kind"] == "stock_vca" for node in nodes.values()), 14)
        source = source_fixture()
        for alias in aliases:
            with self.subTest(alias=alias):
                self.assertEqual(nodes[alias]["kind"], "stock_vca")
                response = next(p["value"] for p in module_by_alias(source, alias)["params"] if p["id"] == 1)
                self.assertEqual(nodes[alias]["params"]["response"], response)
                self.assertEqual(builder.ins[alias], {0: (alias, "cv"), 1: (alias, "signal")})
                self.assertNotIn(alias + "_cv", nodes, "Stock VCA owns CV/10 arithmetic")
                self.assertEqual(nodes[alias]["params"]["level"], 1 if alias in builder.automation else 0.375)
        # Both responses occur in the fixture, so a constant default cannot pass.
        self.assertEqual({nodes[a]["params"]["response"] for a in aliases}, {0, 1})

    def test_automated_vca_uses_arrangement_proxy_but_unautomated_vca_stays_direct(self):
        _, patch, translations = self.build()
        routes = {endpoint(cable) for cable in patch["cables"]}
        self.assertIn(("kick_gate", "out", "kick_gate_arrange0", "signal"), routes)
        self.assertIn(("seqA_lanes", "out1", "kick_gate_arrange0", "cv"), routes)
        self.assertIn(("kick_gate_arrange0", "out", "kick", "trigger"), routes)
        self.assertIn(("kick_gate_arrange0", "out", "duck_env", "gate"), routes)
        self.assertNotIn(("kick_gate", "out", "kick", "trigger"), routes)
        self.assertIn(("bass_amp", "out", "bass_duck", "signal"), routes)
        self.assertFalse(any(node["id"] == "bass_amp_arrange0" for node in patch["nodes"]))
        native = {cable["id"]: cable for cable in patch["cables"]}
        # Fixed source cable ID independently ties provenance to the proxy.
        translated = next(item for item in translations if item["source_id"] == "6450160585652392")
        self.assertEqual([endpoint(native[ident]) for ident in translated["native_cable_ids"]],
                         [("kick_gate_arrange0", "out", "kick", "trigger")])

    def test_malformed_saved_stock_controls_reject_even_when_automation_masks_their_values(self):
        invalid = [float("nan"), float("inf"), -float("inf")]
        cases = [("kick_gate", 0, invalid + [-1, 2]),
                 ("bass_filter", 0, invalid + [-100, 100]),
                 ("kick_gate", 1, invalid + [-1, 0.5, 2]),
                 ("bass_filter", 2, invalid + [-0.1, 1.1])]
        cases += [("bass_filter", param, invalid + [-1.1, 1.1]) for param in (3, 4, 5, 6)]
        cases += [("bass_env", param, invalid + [-0.1, 1.1]) for param in range(4)]
        cases += [("bass_env", param, invalid + [-1.1, 1.1]) for param in range(4, 8)]
        for alias, param_id, values in cases:
            for value in values:
                source = source_fixture()
                param = next(param for param in module_by_alias(source, alias)["params"] if param["id"] == param_id)
                param["value"] = value
                scores = [native_score(module_by_alias(source, seq)["data"]) for seq in ("seqA", "seqB")]
                with self.subTest(alias=alias, param=param_id, value=value):
                    # Explicit score overrides are valid; malformed saved knobs
                    # must not disappear when a proxy replaces their value.
                    with self.assertRaises(ValueError):
                        self.build(source, scores)

    def test_all_five_adsr_eg_controls_depths_and_six_input_bindings_are_preserved(self):
        builder, patch, _ = self.build()
        nodes = {node["id"]: node for node in patch["nodes"]}
        aliases = [item["alias"] for item in identities() if item["model"] == "ADSR"]
        self.assertEqual(len(aliases), 5)
        expected = {"attack": 0.1, "decay": 0.2, "sustain": 0.3, "release": 0.4,
                    "attack_amount": -0.5, "decay_amount": 0.6,
                    "sustain_amount": -0.7, "release_amount": 0.8}
        names = ("attack_cv", "decay_cv", "sustain_cv", "release_cv", "gate", "retrigger")
        for alias in aliases:
            with self.subTest(alias=alias):
                self.assertEqual(nodes[alias]["kind"], "adsr_eg")
                self.assertEqual(nodes[alias]["params"], expected)
                self.assertEqual(builder.ins[alias], {i: (alias, name) for i, name in enumerate(names)})
                self.assertEqual(builder.outs[alias], {0: (alias, "out")})

    def test_all_four_stock_vcf_raw_controls_and_both_outputs_are_preserved(self):
        # Fundamental VCF.cpp at 10dd0160: FREQ0, removed FINE1, RES2,
        # FREQ_CV3, DRIVE4, RES_CV5, DRIVE_CV6. Deliberately distinct fixture
        # values catch swapped drive/resonance depth and accidental FINE use.
        builder, patch, _ = self.build()
        nodes = {node["id"]: node for node in patch["nodes"]}
        aliases = [item["alias"] for item in identities() if item["model"] == "VCF"]
        self.assertEqual(len(aliases), 4)
        for alias in aliases:
            with self.subTest(alias=alias):
                self.assertEqual(nodes[alias]["kind"], "stock_vcf")
                self.assertEqual(builder.outs[alias], {0: (alias, "lowpass"), 1: (alias, "highpass")})
                self.assertEqual(nodes[alias]["params"]["resonance"], 0.4)
                self.assertEqual(nodes[alias]["params"]["resonance_cv_amount"], -0.7)
                self.assertEqual(nodes[alias]["params"]["drive"], 0.6)
                self.assertEqual(nodes[alias]["params"]["drive_cv_amount"], 0.8)
                self.assertEqual(builder.ins[alias][1], (alias, "resonance_cv"))
                self.assertEqual(builder.ins[alias][2], (alias, "drive_cv"))
                self.assertEqual(builder.ins[alias][3], (alias, "signal"))
                if alias not in builder.automation:
                    self.assertEqual(nodes[alias]["params"]["frequency"], 0.3)
                    self.assertEqual(nodes[alias]["params"]["cutoff_cv_amount"], -0.5)
                    self.assertEqual(builder.ins[alias][0], (alias, "cutoff_cv"))

    def test_raw_source_scores_and_explicit_overrides_are_independent_and_affine(self):
        source = source_fixture()
        scores = [native_score(module_by_alias(source, alias)["data"]) for alias in ("seqA", "seqB")]
        before = copy.deepcopy(scores)
        _, patch, _ = self.build(source)
        byid = {node["id"]: node for node in patch["nodes"]}
        self.assertEqual(byid["seqA"]["score"]["length"], 4)
        self.assertEqual(byid["seqA"]["score"]["tracks"][0]["steps"][0]["notes"], [0])
        # seqB lane4 targets Fundamental VCF: raw .3/.7 maps to -2/+2
        # octaves (C4 * 2**(-2) and C4 * 2**2), with source .3 not reused
        # as a second fixed cutoff multiplier.
        self.assertEqual([p["value"] for p in byid["seqB"]["score"]["lanes"][4]["points"]], [-2.0, 2.0])
        for score in scores:
            score["seed"] = 4242
            score["tracks"][0]["steps"][0]["notes"] = [-12]
            for lane in score["lanes"]:
                for point in lane["points"]:
                    point["value"] = 0.2
        override_before = copy.deepcopy(scores)
        builder, override, _ = self.build(source, scores)
        override_nodes = {node["id"]: node for node in override["nodes"]}
        self.assertEqual(override_nodes["seqA"]["score"]["seed"], 4242)
        self.assertEqual(override_nodes["seqA"]["score"]["tracks"][0]["steps"][0]["notes"], [-12])
        self.assertEqual([p["value"] for p in override_nodes["seqB"]["score"]["lanes"][4]["points"]], [-3.0, -3.0])
        self.assertEqual(scores, override_before, "Affine conversion must not mutate explicit scores")
        self.assertEqual([native_score(module_by_alias(source, alias)["data"]) for alias in ("seqA", "seqB")], before)
        with self.assertRaises(ValueError):
            self.build(source, scores[:1])
        with self.assertRaises(ValueError):
            self.build(source, scores + [scores[0]])

    def test_stock_cutoff_automation_and_original_signed_cv_sum_without_a_second_base_cutoff(self):
        builder, patch, _ = self.build()
        nodes = {node["id"]: node for node in patch["nodes"]}
        routes = {endpoint(cable) for cable in patch["cables"]}
        for alias, sequencer, channel in (("bass_filter", "seqB_lanes", "out5"),
                                           ("pad_filter", "seqA_lanes", "out8")):
            with self.subTest(alias=alias):
                self.assertEqual(nodes[alias]["kind"], "stock_vcf")
                self.assertEqual(nodes[alias]["params"], {
                    "frequency": 0.5, "resonance": 0.4,
                    "cutoff_cv_amount": 1, "resonance_cv_amount": -0.7,
                    "drive": 0.6, "drive_cv_amount": 0.8})
                self.assertEqual(nodes[alias + "_mod"]["params"], {"gain": -0.5})
                self.assertEqual(builder.ins[alias][0], (alias + "_mod", "signal"))
                self.assertIn((alias + "_mod", "out", alias, "cutoff_cv"), routes)
                self.assertIn((sequencer, channel, alias, "cutoff_cv"), routes)
                self.assertNotIn(alias + "_pole34", nodes)
                # Independent pitch equation at source raw .3 and +4 V CV:
                # (10*.3 - 5) + (-.5 * 4) = -4 octaves from middle C.
                raw_lane = next(node for node in patch["nodes"] if node["id"] == sequencer[:-6])["score"]
                lane_index = int(channel[3:]) - 1
                lane_value = raw_lane["lanes"][lane_index]["points"][0]["value"]
                pitch = 10 * nodes[alias]["params"]["frequency"] - 5
                pitch += lane_value + nodes[alias + "_mod"]["params"]["gain"] * 4
                self.assertEqual(pitch, -4)
                self.assertAlmostEqual(261.625565 * 2 ** pitch, 16.3515978125)
        # Unused source fine parameter ID1=.2 cannot alter raw base frequency.
        self.assertEqual(nodes["stab_filter"]["params"]["frequency"], 0.3)

    def test_automation_duplicate_unknown_or_unsupported_targets_are_rejected(self):
        for mistake in ("duplicate", "unknown", "stock_nonfrequency", "adsr"):
            source = source_fixture()
            lanes = module_by_alias(source, "seqA")["data"]["lanes"]
            with self.subTest(mistake=mistake):
                if mistake == "duplicate":
                    lanes[1]["moduleId"] = lanes[0]["moduleId"]
                elif mistake == "unknown":
                    lanes[0]["moduleId"] = "123456789"
                elif mistake == "stock_nonfrequency":
                    lanes[0]["paramId"] = 1
                else:
                    lanes[0]["moduleId"] = str(module_by_alias(source, "bass_env")["id"])
                with self.assertRaises(ValueError):
                    self.build(source)

    def test_automation_nonfinite_and_out_of_domain_values_reject_in_raw_and_override_scores(self):
        for target_lane in (0, 7):  # Stock VCA level and Fundamental VCF frequency.
            for value in (float("nan"), float("inf"), -float("inf"), -100, 100):
                for override in (False, True):
                    source = source_fixture()
                    scores = [native_score(module_by_alias(source, alias)["data"])
                              for alias in ("seqA", "seqB")]
                    with self.subTest(lane=target_lane, value=value, override=override):
                        if override:
                            scores[0]["lanes"][target_lane]["points"][0]["value"] = value
                        else:
                            module_by_alias(source, "seqA")["data"]["lanes"][target_lane]["points"][0]["value"] = value
                        with self.assertRaises(ValueError):
                            self.build(source, scores if override else None)

    def test_raw_scores_can_be_converted_repeatedly_without_double_affine_conversion(self):
        source = source_fixture()
        scores = [native_score(module_by_alias(source, alias)["data"]) for alias in ("seqA", "seqB")]
        _, first, _ = self.build(source, scores)
        _, repeated, _ = self.build(source, scores)
        self.assertEqual(first, repeated)
        self.assertEqual(scores[1]["lanes"][4]["points"][0]["value"], 0.3)
        self.assertEqual(next(n for n in repeated["nodes"] if n["id"] == "seqB")["score"]["lanes"][4]["points"][0]["value"], -2)

    def test_unmodeled_adsr_push_and_automated_vca_bypass_reject_explicitly(self):
        source = source_fixture()
        envelope = module_by_alias(source, "bass_env")
        next(param for param in envelope["params"] if param["id"] == 8)["value"] = 1
        with self.assertRaises(ValueError):
            self.build(source)
        source = source_fixture()
        module_by_alias(source, "kick_gate")["bypassed"] = True
        with self.assertRaises(ValueError):
            self.build(source)

    def test_surge_filters_remain_explicit_approximations_including_riser_bandpass(self):
        for circuit in (False, True):
            builder, patch, _ = self.build(circuit_filters=circuit)
            nodes = {node["id"]: node for node in patch["nodes"]}
            self.assertEqual(nodes["riser_filter"]["kind"], "svf_filter")
            self.assertEqual(builder.outs["riser_filter"][0], ("riser_filter", "band"))
            self.assertEqual(nodes["lead_filterL"]["kind"], "ladder_vcf" if circuit else "svf_filter")
            self.assertEqual(nodes["lead_filterR"]["kind"], "ladder_vcf" if circuit else "svf_filter")
            for alias in ("lead_filter", "riser_filter"):
                note = builder.roles[alias]["substitution"].lower()
                self.assertTrue(any(word in note for word in ("approximation", "not source", "not plugin", "original")), note)
            for alias in ("bass_filter", "stab_filter", "pad_filter", "hats_highpass"):
                self.assertEqual(nodes[alias]["kind"], "stock_vcf", "Circuit option must not demote stock fidelity")

    def test_four_notes_preserve_content_and_stock_bypass_flags(self):
        source = source_fixture()
        aliases = ("notes_clock", "notes_bass", "notes_lead", "notes_atmos", "bass_env", "bass_filter", "bass_amp")
        for alias in aliases:
            module_by_alias(source, alias)["bypassed"] = True
        builder, patch, _ = self.build(source)
        nodes = {node["id"]: node for node in patch["nodes"]}
        for alias in aliases:
            self.assertTrue(nodes[alias]["bypassed"], alias)
        notes = [node for node in nodes.values() if node["kind"] == "notes"]
        self.assertEqual(len(notes), 4)
        for node in notes:
            self.assertEqual(node["content"], {"type": "notes", "text": module_by_alias(source, node["id"])["data"]["text"]})
            self.assertIn(node["id"], builder.roles[node["id"]]["native_ids"])


@unittest.skipUnless(os.environ.get("OPENPATCH_BIN"), "Set OPENPATCH_BIN for headless compiler/catalog/save acceptance")
class STIWCompilerTests(unittest.TestCase):
    def test_generated_graph_catalog_routes_compile_save_and_invalid_routes_reject(self):
        # One server for the complete graph and malformed route probes; never
        # invokes plugins, opens a device, or changes the user's workspace.
        executable = Path(os.environ["OPENPATCH_BIN"]).resolve()
        self.assertTrue(executable.is_file(), str(executable))
        with tempfile.TemporaryDirectory(prefix="openpatch-stiw-synthetic-") as workspace:
            client = MCPProcess(executable, Path(workspace))
            try:
                initialized = client.request("initialize", {"protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {}, "clientInfo": {"name": "stiw-synthetic-acceptance", "version": "1"}})
                self.assertIn("result", initialized, initialized)
                client.notify("notifications/initialized")

                def call(name, arguments=None, success=True):
                    response = client.request("tools/call", {"name": name, "arguments": arguments or {}})
                    if not success:
                        self.assertTrue("error" in response or response.get("result", {}).get("isError"), response)
                        return response
                    self.assertNotIn("error", response, response)
                    result = response["result"]
                    self.assertFalse(result.get("isError"), result)
                    if "structuredContent" in result:
                        return result["structuredContent"]
                    for item in result.get("content", []):
                        if item.get("type") == "text":
                            try:
                                return json.loads(item["text"])
                            except json.JSONDecodeError:
                                pass
                    return result

                source = source_fixture()
                for alias in ("notes_clock", "bass_env", "stab_filter", "bass_amp"):
                    module_by_alias(source, alias)["bypassed"] = True
                builder = Builder(source)
                patch, translations = builder.create()
                catalog = {module["kind"]: module for module in call("catalog")["modules"]}
                frequency = catalog["stock_vcf"]["params"]["frequency"]
                for bound in (frequency["min"], frequency["max"]):
                    endpoint_source, endpoint_scores = cutoff_endpoint_case(bound)
                    for override in (None, endpoint_scores):
                        with self.subTest(cutoff_endpoint=bound, explicit_override=override is not None):
                            endpoint_builder = Builder(endpoint_source, override)
                            endpoint_patch, _ = endpoint_builder.create()
                            call("replace_patch", {"patch": endpoint_patch})
                            accepted = {node["id"]: node for node in call("inspect_patch")["patch"]["nodes"]}
                            self.assertEqual(accepted["bass_filter"]["params"]["frequency"], 0.5)
                            self.assertAlmostEqual(accepted["stab_filter"]["params"]["frequency"], bound, places=7)
                            self.assertAlmostEqual(accepted["seqB"]["score"]["lanes"][4]["points"][0]["value"],
                                                   10 * bound - 5, places=6)
                nodes = {node["id"]: node for node in patch["nodes"]}
                for cable in patch["cables"]:
                    self.assertIn(cable["input"], catalog[nodes[cable["to"]]["kind"]]["inputs"], cable)
                    self.assertIn(cable["output"], catalog[nodes[cable["from"]]["kind"]]["outputs"], cable)
                self.assertEqual(len(translations), 83)
                call("replace_patch", {"patch": patch})
                inspected = call("inspect_patch")["patch"]
                before_save = copy.deepcopy(inspected)
                self.assertEqual(len(inspected["nodes"]), len(patch["nodes"]))
                call("save_patch", {"filename": "synthetic-stiw.json"})
                saved = json.loads((Path(workspace) / "synthetic-stiw.json").read_text(encoding="utf-8"))
                self.assertEqual(len(saved["cables"]), len(patch["cables"]))
                call("replace_patch", {"patch": {"version": 2, "nodes": []}})
                call("load_patch", {"filename": "synthetic-stiw.json"})
                inspected = call("inspect_patch")["patch"]
                self.assertEqual(inspected, before_save, "Inspect before save must equal inspect after load")
                self.assertEqual(saved["cables"], before_save["cables"])
                for node in saved["nodes"]:
                    original = nodes[node["id"]]
                    if node["kind"] in ("notes", "stock_vca", "stock_vcf", "adsr_eg"):
                        self.assertEqual(bool(node.get("bypassed")), bool(original.get("bypassed")))
                        for name, expected in original["params"].items():
                            self.assertAlmostEqual(node["params"][name], expected, places=6)
                    if node["kind"] == "notes":
                        self.assertEqual(node["content"], original["content"])
                # Exercise all signed CV jacks even though the curated source's
                # four filters do not happen to patch every available input.
                probes = copy.deepcopy(patch)
                for alias in ("bass_filter", "stab_filter", "pad_filter", "hats_highpass"):
                    for port_id in range(3):
                        target = builder.ins[alias][port_id]
                        probes["cables"].append({"id": f"probe-{alias}-{port_id}",
                            "from": "near-probe", "output": "out", "to": target[0], "input": target[1]})
                probes["nodes"].append({"id": "near-probe", "kind": "constant", "params": {"value": 4}})
                call("replace_patch", {"patch": probes})
                call("replace_patch", {"patch": inspected})
                for field, value in (("input", "not-a-stock-input"), ("output", "not-a-stock-output")):
                    bad = copy.deepcopy(patch)
                    bad["cables"][0][field] = value
                    call("replace_patch", {"patch": bad}, success=False)
                    self.assertEqual(call("inspect_patch")["patch"], inspected, "Rejected route changed accepted graph")
            finally:
                client.close()


if __name__ == "__main__":
    unittest.main()
