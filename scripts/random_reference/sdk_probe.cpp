// SPDX-License-Identifier: MIT
// Original host adapter. Genuine pinned source is supplied only in temporary storage.
#include <rack.hpp>
#include <array>
#include <cmath>
#include <iomanip>
#include <iostream>
#include <memory>
#include <stdexcept>

namespace {
std::array<float, 2> suppliedDraws{};
unsigned drawCount = 0;
float referenceUniform() {
    if (drawCount >= suppliedDraws.size()) throw std::runtime_error("uniform stream exhausted");
    return suppliedDraws[drawCount++];
}
}

#include "reference_module.hpp"

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    if (rack::APP_VERSION != "2.6.6") throw std::runtime_error("Rack SDK 2.6.6 required");
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    if (!(args.sampleRate > 0.f)) return 2;
    args.sampleTime = 1.f / args.sampleRate;
    auto module = std::make_unique<Random>();
    for (auto& output : module->outputs) output.channels = 1;
    std::cout << std::setprecision(9);
    unsigned event;
    while (std::cin >> event) {
        if (event > 3) return 3;
        if (event == 2) {
            module = std::make_unique<Random>();
            for (auto& output : module->outputs) output.channels = 1;
        }
        // Events 1/3 are parameter reset/adoption. The actual source runtime
        // remains intact; all source-ordered parameters arrive in the row.
        for (auto& param : module->params) {
            float value;
            if (!(std::cin >> value) || !std::isfinite(value)) return 3;
            param.setValue(value);
        }
        for (auto& input : module->inputs) {
            int width;
            if (!(std::cin >> width) || width < 0 || width > 16) return 3;
            input.channels = width;
            for (int lane = 0; lane < 16; lane++) {
                float value;
                if (!(std::cin >> value) || !std::isfinite(value)) return 3;
                // Rack cable bookkeeping guarantees zeros beyond input width.
                input.voltages[lane] = lane < width ? value : 0.f;
            }
        }
        for (auto& draw : suppliedDraws)
            if (!(std::cin >> draw) || !(draw >= 0.f && draw < 1.f)) return 3;
        const float rate = module->params[Random::RATE_PARAM].getValue()
            + module->inputs[Random::RATE_INPUT].getVoltage()
            * module->params[Random::RATE_CV_PARAM].getValue();
        if (!std::isfinite(rate) || rate < -120.f || rate > 120.f) return 3;
        drawCount = 0;
        module->process(args);
        bool first = true;
        auto emit = [&](auto value) {
            if (!first) std::cout << ' ';
            first = false;
            std::cout << value;
        };
        for (auto& output : module->outputs) emit(output.getChannels());
        for (auto& output : module->outputs)
            for (int lane = 0; lane < 16; lane++) emit(output.getVoltage(lane));
        for (float value : module->lastVoltage) emit(value);
        for (float value : module->nextVoltage) emit(value);
        emit(module->phase);
        emit(module->clockFreq);
        emit(module->clockPhase);
        emit(module->clockTimer.time);
        emit(module->pulseGenerator.remaining);
        emit(unsigned(module->clockTrigger.s));
        for (auto& light : module->lights) emit(light.getBrightness());
        emit(drawCount);
        std::cout << '\n';
        args.frame++;
    }
    if (!std::cin.eof()) return 3;
}
