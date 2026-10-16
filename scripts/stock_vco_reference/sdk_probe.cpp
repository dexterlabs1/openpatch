// SPDX-License-Identifier: MIT
// Original observation adapter for the separately compiled pinned VCO class.
// reference_module.hpp and its license remain in ignored comparison evidence.
#include "reference_module.hpp"
#include <array>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <memory>
#include <sstream>
#include <string>

static MinBlepBuffer<32, rack::simd::float_4>& correction(VCOProcessor<rack::simd::float_4>& p, int wave) {
    switch (wave) {
        case 0: return p.sinMinBlep;
        case 1: return p.triMinBlep;
        case 2: return p.sawMinBlep;
        default: return p.sqrMinBlep;
    }
}
static float filter(const VCOProcessor<rack::simd::float_4>& p, int wave, int lane) {
    switch (wave) {
        case 0: return p.dcFilterStateSin.y[lane];
        case 1: return p.dcFilterStateTri.y[lane];
        case 2: return p.dcFilterStateSaw.y[lane];
        default: return p.dcFilterStateSqr.y[lane];
    }
}
static bool fresh(VCO& module) {
    for (int lane = 0; lane < 16; ++lane) {
        auto& p = module.processors[lane / 4]; const int j = lane % 4;
        if (p.phase[j] != 0.f || p.syncDirection[j] != 1.f || p.lastSync[j] != 0.f || p.lastSqrState[j] != 1.f) return false;
        for (int wave = 0; wave < 4; ++wave) {
            if (filter(p, wave, j) != 0.f) return false;
            auto& buffer = correction(p, wave);
            if (buffer.bufferIndex != 0) return false;
            for (const auto& value : buffer.buffer) if (value[j] != 0.f) return false;
        }
    }
    if (module.lightDivider.getClock() != 0) return false;
    for (auto& light : module.lights) if (light.getBrightness() != 0.f) return false;
    return true;
}
int main(int argc, char** argv) {
    if (argc != 2 || rack::APP_VERSION != "2.6.6" || std::fegetround() != FE_TONEAREST) return 2;
    std::cerr << "FE_TONEAREST=1\n";
#if defined(__aarch64__) || defined(__arm64__)
    std::uint64_t fpcr; asm volatile("mrs %0, fpcr" : "=r"(fpcr));
    std::cerr << "FPCR=" << fpcr << '\n';
#endif
    std::cout << std::setprecision(9);
    if (std::string(argv[1]) == "--kernel") {
        const auto& table = getMinBlep();
        for (int z = 0; z < 32; ++z) for (int o = 0; o < 16; ++o) std::cout << table.impulseReordered[o][z] << '\n';
        for (int z = 0; z < 32; ++z) for (int o = 0; o < 16; ++o) std::cout << table.rampReordered[o][z] << '\n';
        return 0;
    }
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]); args.sampleTime = 1.f / args.sampleRate;
    if (!std::isfinite(args.sampleRate) || args.sampleRate <= 0) return 3;
    std::unique_ptr<VCO> module;
    unsigned previousMask = 0;
    const int paramIds[] = {VCO::FREQ_PARAM, VCO::FM_PARAM, VCO::PW_PARAM, VCO::PW_CV_PARAM, VCO::LINEAR_PARAM, VCO::SYNC_PARAM};
    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream input(line); std::array<float, 78> row{};
        // Formatted float extraction rejects legitimate subnormal tokens on
        // libc++. strtof preserves their f32 value even when errno is ERANGE.
        // Keep full-token and finite checks; do not clip or flush input values.
        for (float& value : row) {
            std::string token; if (!(input >> token)) return 3;
            char* end = nullptr;
            value = std::strtof(token.c_str(), &end);
            if (end == token.c_str() || *end != '\0' || !std::isfinite(value)) return 3;
        }
        std::string extra; if (input >> extra) return 3;
        const int event = static_cast<int>(row[0]);
        if (event < 0 || event > 3 || row[0] != event) return 3;
        if (event == 1 || !module) {
            module = std::make_unique<VCO>();
            rack::engine::Module::SampleRateChangeEvent changed{};
            changed.sampleRate = args.sampleRate; changed.sampleTime = args.sampleTime;
            module->onSampleRateChange(changed);
            if (!fresh(*module)) return 4;
            previousMask = 0;
        }
        for (int i = 0; i < 6; ++i) module->params[paramIds[i]].setValue(i == 5 ? 1.f - row[1 + i] : row[1 + i]);
        // Event2 is the deprecated runtime reset hook only. The actual VCO does
        // not override it. Host Initialize's parameter defaults are tested apart.
        if (event == 2) module->onReset();
        // Event3 is compatible control-graph replacement. The SDK reference
        // remains in place; production Rust rebuilds and adopts its real state.
        for (int i = 0; i < 4; ++i) {
            const int at = 7 + 17 * i; const int width = static_cast<int>(row[at]);
            if (width < 0 || width > 16 || row[at] != width) return 3;
            auto& port = module->inputs[i]; port.channels = width;
            for (int lane = 0; lane < 16; ++lane) {
                const float value = row[at + 1 + lane];
                if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
                port.voltages[lane] = value;
            }
        }
        const unsigned mask = static_cast<unsigned>(row[75]);
        if (mask > 15 || row[75] != mask || (row[76] != 0.f && row[76] != 1.f) || (row[77] != 0.f && row[77] != 1.f)) return 3;
        for (int output = 0; output < 4; ++output) {
            const unsigned bit = 1u << output;
            if (!(mask & bit)) module->outputs[output].channels = 0;
            else if (!(previousMask & bit)) module->outputs[output].channels = 1;
        }
        previousMask = mask;
        const bool bypass = row[76] == 1.f;
        // Explicit host-policy shim: skip the class and expose zero output.
        // It does not claim Rack Engine bypass equivalence.
        if (!bypass) module->process(args);
        args.frame++;
        bool first = true;
        auto emit = [&](float value) { if (!first) std::cout << ' '; first = false; std::cout << value; };
        for (auto& output : module->outputs) emit(bypass ? 1.f : output.getChannels());
        for (auto& output : module->outputs) for (int lane = 0; lane < 16; ++lane) emit(bypass ? 0.f : output.getVoltage(lane));
        for (int lane = 0; lane < 16; ++lane) emit(module->processors[lane / 4].phase[lane % 4]);
        for (int lane = 0; lane < 16; ++lane) emit(module->processors[lane / 4].syncDirection[lane % 4]);
        for (int lane = 0; lane < 16; ++lane) emit(module->processors[lane / 4].lastSync[lane % 4]);
        for (int lane = 0; lane < 16; ++lane) emit(module->processors[lane / 4].lastSqrState[lane % 4]);
        for (int wave = 0; wave < 4; ++wave) for (int lane = 0; lane < 16; ++lane) emit(filter(module->processors[lane / 4], wave, lane % 4));
        for (int wave = 0; wave < 4; ++wave) for (int group = 0; group < 4; ++group) emit(correction(module->processors[group], wave).bufferIndex);
        emit(module->lightDivider.getClock());
        for (auto& light : module->lights) emit(light.getBrightness());
        if (row[77] == 1.f) {
            for (int wave = 0; wave < 4; ++wave) for (int lane = 0; lane < 16; ++lane) {
                const auto& b = correction(module->processors[lane / 4], wave);
                for (int sample = 0; sample < 32; ++sample) emit(b.buffer[b.bufferIndex + sample][lane % 4]);
            }
        }
        std::cout << '\n';
    }
    return std::cin.bad() ? 3 : 0;
}
