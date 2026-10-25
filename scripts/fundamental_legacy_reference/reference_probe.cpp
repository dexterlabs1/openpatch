// SPDX-License-Identifier: MIT
// Original OpenPatch host adapter. The genuine pinned VCA and Unity classes
// are extracted unmodified (only `plugin.hpp` replaced by the SDK include) into
// temporary headers by run.py; this file never contains upstream code.
//
// Protocol: stdin is a stream of whitespace-separated tokens; every float is
// an unsigned 32-bit hex bit pattern. Rack Engine::stepBlock installs nearest
// rounding plus flush-to-zero before modules run, so the probe calls the
// genuine rack::system::resetFpuFlags once before any processing.
//
// vca mode, per frame line:  F level1 level2 {channels lane0..lane15} x6
//   prints: channels1 active lanes... channels2 active lanes...
// unity mode:
//   F avg1 avg2 {channels lane0} x12   -> mix1 inv1 mix2 inv2 light0..9 meter1 meter2 clock merge
//   M 0|1                               -> context menu "Merge channels 1 & 2"
//   R                                   -> parameter defaults, then genuine Unity::onReset()
//   D <json>                            -> fresh module dataFromJson, prints merge and dataToJson
#include "reference_vca.hpp"
#include "reference_unity.hpp"
#include <cinttypes>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <memory>
#include <string>

static float bits(const std::string& token) {
    uint32_t u = (uint32_t) std::stoul(token, nullptr, 16);
    float f;
    std::memcpy(&f, &u, 4);
    return f;
}

static void put(float f) {
    uint32_t u;
    std::memcpy(&u, &f, 4);
    std::printf(" %08" PRIx32, u);
}

static std::string next() {
    std::string token;
    if (!(std::cin >> token)) throw 1;
    return token;
}

// A Rack cable sets the input's channel count and copies only active lanes;
// Port::setChannels and cable removal leave higher lanes at zero.
static void load(rack::engine::Input& input, int channels, const float* lanes) {
    input.channels = channels;
    for (int lane = 0; lane < 16; lane++)
        input.voltages[lane] = lane < channels ? lanes[lane] : 0.f;
}

static int run_vca(float rate) {
    VCA module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = rate;
    args.sampleTime = 1.f / rate;
    // Both outputs patched: a connected output starts mono.
    for (auto& output : module.outputs) output.channels = 1;
    std::string op;
    while (std::cin >> op) {
        if (op != "F") return 3;
        module.params[VCA::LEVEL1_PARAM].setValue(bits(next()));
        module.params[VCA::LEVEL2_PARAM].setValue(bits(next()));
        for (auto& input : module.inputs) {
            int channels = std::stoi(next());
            if (channels < 0 || channels > 16) return 3;
            float lanes[16];
            for (float& lane : lanes) lane = bits(next());
            load(input, channels, lanes);
        }
        module.process(args);
        for (auto& output : module.outputs) {
            int channels = output.getChannels();
            std::printf(channels == 0 ? "%d" : " %d", channels);
            for (int lane = 0; lane < channels; lane++) put(output.getVoltage(lane));
        }
        std::printf("\n");
        args.frame++;
    }
    return 0;
}

static int run_unity(float rate) {
    auto module = std::make_unique<Unity>();
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = rate;
    args.sampleTime = 1.f / rate;
    for (auto& output : module->outputs) output.channels = 1;
    std::string op;
    while (std::cin >> op) {
        if (op == "F") {
            module->params[Unity::AVG1_PARAM].setValue(bits(next()));
            module->params[Unity::AVG2_PARAM].setValue(bits(next()));
            for (auto& input : module->inputs) {
                int channels = std::stoi(next());
                if (channels < 0 || channels > 16) return 3;
                float lanes[16] = {};
                lanes[0] = bits(next());
                // Unity reads only the first channel; higher lanes of a
                // polyphonic cable carry an unrelated marker value.
                for (int lane = 1; lane < channels; lane++) lanes[lane] = 7.f;
                load(input, channels, lanes);
            }
            module->process(args);
            for (int i = 0; i < 4; i++) put(module->outputs[i].getVoltage());
            for (int i = 0; i < 10; i++) put(module->lights[i].getBrightness());
            put(module->vuMeters[0].v);
            put(module->vuMeters[1].v);
            std::printf(" %u %d\n", module->lightDivider.getClock(), module->merge ? 1 : 0);
            args.frame++;
        }
        else if (op == "M") {
            module->merge = std::stoi(next()) != 0;
        }
        else if (op == "R") {
            // Module::onReset(ResetEvent) returns every resettable parameter
            // to its configured default, then calls the deprecated
            // Unity::onReset(). Its ParamQuantity::reset path needs a running
            // app, so apply the same defaults from the genuine quantities.
            for (rack::engine::ParamQuantity* pq : module->paramQuantities) {
                if (pq->resetEnabled)
                    module->params[pq->paramId].setValue(pq->getDefaultValue());
            }
            module->onReset();
            std::printf("R");
            put(module->params[Unity::AVG1_PARAM].getValue());
            put(module->params[Unity::AVG2_PARAM].getValue());
            std::printf(" %d\n", module->merge ? 1 : 0);
        }
        else if (op == "D") {
            std::string line;
            std::getline(std::cin, line);
            json_error_t error;
            json_t* root = json_loads(line.c_str(), JSON_DECODE_ANY, &error);
            if (!root) return 4;
            Unity fresh;
            fresh.dataFromJson(root);
            json_decref(root);
            json_t* saved = fresh.dataToJson();
            char* text = json_dumps(saved, JSON_COMPACT | JSON_SORT_KEYS);
            std::printf("D %d %s\n", fresh.merge ? 1 : 0, text);
            std::free(text);
            json_decref(saved);
        }
        else {
            return 3;
        }
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc != 3) return 2;
    rack::system::resetFpuFlags();
    try {
        std::string mode = argv[1];
        float rate = std::stof(argv[2]);
        if (mode == "vca") return run_vca(rate);
        if (mode == "unity") return run_unity(rate);
        if (mode == "constants") {
            // Constants the Rust port hardcodes: logf(50) as simd::pow uses it.
            const float expBase = 50.f;
            put(std::log(expBase));
            std::printf("\n");
            return 0;
        }
    }
    catch (int) {
        return 3;
    }
    return 2;
}
