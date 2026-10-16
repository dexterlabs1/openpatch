// SPDX-License-Identifier: MIT
// Original text adapter; reference_module.hpp is the pinned actual LFO class.
#include "reference_module.hpp"
#include <array>
#include <cmath>
#include <cfenv>
#include <cstdint>
#include <iomanip>
#include <iostream>
#include <memory>
#include <sstream>
#include <string>

int main(int argc, char** argv) {
    if (argc != 2 || rack::APP_VERSION != "2.6.6") {
        std::cerr << "Sample rate and genuine Rack SDK 2.6.6 required\n";
        return 2;
    }
    if (std::fegetround() != FE_TONEAREST) return 5;
    std::cerr << "FE_TONEAREST=" << (std::fegetround() == FE_TONEAREST) << '\n';
#if defined(__aarch64__) || defined(__arm64__)
    std::uint64_t fpcr;
    asm volatile("mrs %0, fpcr" : "=r"(fpcr));
    std::cerr << "FPCR=" << fpcr << '\n';
#endif
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    args.sampleTime = 1.f / args.sampleRate;
    std::unique_ptr<LFO> module;
    unsigned previousMask = 0;
    std::cout << std::setprecision(9);
    const int paramIds[] = {LFO::FREQ_PARAM, LFO::FM_PARAM, LFO::PW_PARAM,
                           LFO::PWM_PARAM, LFO::OFFSET_PARAM, LFO::INVERT_PARAM};
    const int inputIds[] = {LFO::FM_INPUT, LFO::PW_INPUT, LFO::RESET_INPUT, LFO::CLOCK_INPUT};
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream input(line);
        std::array<float, 76> row{};
        for (float& value : row) {
            if (!(input >> value) || !std::isfinite(value)) return 3;
        }
        std::string extra;
        if (input >> extra) return 3;
        const int event = static_cast<int>(row[0]);
        if (event < 0 || event > 2 || row[0] != event) return 3;
        if (event == 1 || !module) {
            module = std::make_unique<LFO>();
            for (int lane=0; lane<16; ++lane) {
                if (module->phases[lane/4][lane%4] != 0.f ||
                    module->resetTriggers[lane/4].state[lane%4] == 0.f) return 4;
            }
            if (module->clockTrigger.s != rack::dsp::SchmittTrigger::UNINITIALIZED ||
                module->clockFreq != 1.f || module->clockTimer.getTime() != 0.f ||
                module->lightDivider.getClock() != 0) return 4;
            for (auto& light : module->lights) if (light.getBrightness() != 0.f) return 4;
            previousMask = 0;
        }
        for (int i = 0; i < 6; ++i) module->params[paramIds[i]].setValue(row[1 + i]);
        if (event == 2) module->onReset();
        for (int i = 0; i < 4; ++i) {
            const int at = 7 + 17 * i;
            const int width = static_cast<int>(row[at]);
            if (width < 0 || width > 16 || row[at] != width) return 3;
            auto& port = module->inputs[inputIds[i]];
            port.channels = width;
            for (int lane = 0; lane < 16; ++lane) {
                const float value = row[at + 1 + lane];
                if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
                port.voltages[lane] = value;
            }
        }
        const unsigned mask = static_cast<unsigned>(row[75]);
        if (mask > 15 || row[75] != mask) return 3;
        for (int output = 0; output < 4; ++output) {
            const unsigned bit = 1u << output;
            if (!(mask & bit)) module->outputs[output].channels = 0;
            else if (!(previousMask & bit)) module->outputs[output].channels = 1;
        }
        previousMask = mask;
        module->process(args);
        args.frame++;
        // No output DSP is reconstructed by this adapter.
        bool first = true;
        auto emit = [&](float value) {
            if (!first) std::cout << ' ';
            first = false;
            std::cout << value;
        };
        for (auto& output : module->outputs) emit(output.getChannels());
        for (auto& output : module->outputs)
            for (int lane = 0; lane < 16; ++lane) emit(output.getVoltage(lane));
        for (int lane = 0; lane < 16; ++lane) emit(module->phases[lane / 4][lane % 4]);
        for (int lane = 0; lane < 16; ++lane)
            emit(module->resetTriggers[lane / 4].state[lane % 4] != 0.f ? 1.f : 0.f);
        emit(static_cast<unsigned>(module->clockTrigger.s));
        emit(module->clockFreq);
        emit(module->clockTimer.getTime());
        emit(module->lightDivider.getClock());
        for (auto& light : module->lights) emit(light.getBrightness());
        std::cout << '\n';
    }
    return std::cin.bad() ? 3 : 0;
}
