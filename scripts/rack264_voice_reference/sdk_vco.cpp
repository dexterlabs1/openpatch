// SPDX-License-Identifier: MIT
// Original observation adapter. reference_module.hpp contains the unchanged
// pinned Fundamental class prefix and remains with its license in local evidence.
// No DSP implementation, replacement kernels or reference fixes live here.
#include "reference_module.hpp"
#include <array>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>

using Oscillator = VoltageControlledOscillator<16, 16, rack::simd::float_4>;
using Correction = rack::dsp::MinBlepGenerator<16, 16, rack::simd::float_4>;

static const Correction& correction(const Oscillator& oscillator, int wave) {
    switch (wave) {
        case 0: return oscillator.sinMinBlep;
        case 1: return oscillator.triMinBlep;
        case 2: return oscillator.sawMinBlep;
        default: return oscillator.sqrMinBlep;
    }
}

static float waveValue(const Oscillator& oscillator, int wave, int lane) {
    switch (wave) {
        case 0: return oscillator.sinValue[lane];
        case 1: return oscillator.triValue[lane];
        case 2: return oscillator.sawValue[lane];
        default: return oscillator.sqrValue[lane];
    }
}

// libc++ formatted float extraction rejects some valid subnormal strings.
// strtof preserves the parsed f32, including an ERANGE subnormal result.
static bool parseRow(const std::string& line, std::array<float, 77>& row) {
    std::istringstream stream(line);
    for (float& value : row) {
        std::string token;
        if (!(stream >> token)) return false;
        char* end = nullptr;
        value = std::strtof(token.c_str(), &end);
        if (end == token.c_str() || *end != '\0' || !std::isfinite(value)) return false;
    }
    std::string extra;
    return !(stream >> extra);
}

static bool integerInRange(float value, int low, int high) {
    return value >= low && value <= high && value == std::trunc(value);
}

int main(int argc, char** argv) {
    if (argc != 2 || rack::APP_VERSION != "2.6.6" || std::fegetround() != FE_TONEAREST) return 2;
    char* end = nullptr;
    const float rate = std::strtof(argv[1], &end);
    if (end == argv[1] || *end != '\0' || !integerInRange(rate, 8000, 192000)) return 2;
    std::cerr << "FE_TONEAREST=1\n";
#if defined(__aarch64__) || defined(__arm64__)
    std::uint64_t fpcr;
    asm volatile("mrs %0, fpcr" : "=r"(fpcr));
    std::cerr << "FPCR=" << fpcr << '\n';
#endif
    std::cout << std::setprecision(9);
    VCO module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = rate;
    args.sampleTime = 1.f / rate;
    rack::engine::Module::SampleRateChangeEvent changed{};
    changed.sampleRate = args.sampleRate;
    changed.sampleTime = args.sampleTime;
    module.onSampleRateChange(changed);

    // Input controls are frequency, PW, FM depth, PWM depth, linear, soft.
    const int paramIds[] = {VCO::FREQ_PARAM, VCO::PW_PARAM, VCO::FM_PARAM,
                            VCO::PW_CV_PARAM, VCO::LINEAR_PARAM, VCO::SYNC_PARAM};
    unsigned previousMask = 0;
    std::string line;
    while (std::getline(std::cin, line)) {
        std::array<float, 77> row{};
        if (!parseRow(line, row) || !integerInRange(row[0], 0, 2)
            || !integerInRange(row[1], 0, 1) || !integerInRange(row[2], 0, 15)) return 3;
        const int action = static_cast<int>(row[0]);
        const bool observe = row[1] == 1.f;
        const unsigned mask = static_cast<unsigned>(row[2]);
        if (row[3] < -54.f || row[3] > 54.f || row[4] < .01f || row[4] > .99f
            || row[5] < -1.f || row[5] > 1.f || row[6] < -1.f || row[6] > 1.f
            || !integerInRange(row[7], 0, 1) || !integerInRange(row[8], 0, 1)) return 3;
        for (int i = 0; i < 6; ++i) {
            module.params[paramIds[i]].setValue(i == 5 ? 1.f - row[3 + i] : row[3 + i]);
        }
        // Genuine inherited runtime-reset hook. This VCO does not override it.
        // Supplied controls model host Initialize defaults/overrides separately.
        if (action == 1) module.onReset();
        // Action2 models same-rate host runtime transfer by retaining this exact
        // class instance. Rust rebuilds controls and adopts its production state.
        for (int portIndex = 0; portIndex < 4; ++portIndex) {
            const int start = 9 + 17 * portIndex;
            if (!integerInRange(row[start], 0, 16)) return 3;
            const int width = static_cast<int>(row[start]);
            auto& port = module.inputs[portIndex];
            port.channels = width;
            for (int lane = 0; lane < 16; ++lane) {
                const float value = row[start + 1 + lane];
                // Genuine SDK SIMD input reads include hidden quartet lanes.
                // Require canonical +0 padding to model cleared host storage.
                if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
                port.voltages[lane] = value;
            }
        }
        for (int wave = 0; wave < 4; ++wave) {
            const unsigned bit = 1u << wave;
            if (!(mask & bit)) module.outputs[wave].channels = 0;
            else if (!(previousMask & bit)) module.outputs[wave].channels = 1;
        }
        previousMask = mask;
        module.process(args);
        ++args.frame;

        const int width = std::max(module.inputs[VCO::PITCH_INPUT].getChannels(), 1);
        std::cout << width;
        auto emit = [](auto value) { std::cout << ' ' << value; };
        for (int wave = 0; wave < 4; ++wave) {
            const auto& port = module.outputs[wave];
            // Canonical publication deliberately excludes inactive/raw storage.
            for (int lane = 0; lane < 16; ++lane) {
                emit((mask & (1u << wave)) && lane < width ? port.voltages[lane] : 0.f);
            }
        }
        if (observe) {
            // Exact Rack264VcoSnapshot field order, lane-major within each field.
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].phase[lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].freq[lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].pulseWidth[lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].syncDirection[lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].lastSyncValue[lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].sqrState[lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].sqrFilter.xstate[0][lane % 4]);
            for (int lane = 0; lane < 16; ++lane) emit(module.oscillators[lane / 4].sqrFilter.ystate[0][lane % 4]);
            for (int wave = 0; wave < 4; ++wave) {
                for (int lane = 0; lane < 16; ++lane) {
                    const auto& ring = correction(module.oscillators[lane / 4], wave);
                    for (int future = 0; future < 32; ++future) emit(ring.buf[(ring.pos + future) % 32][lane % 4]);
                }
            }
            for (const auto& oscillator : module.oscillators) emit(oscillator.sinMinBlep.pos);
            for (int wave = 0; wave < 4; ++wave) {
                for (int lane = 0; lane < 16; ++lane) emit(waveValue(module.oscillators[lane / 4], wave, lane % 4));
            }
            emit(module.lightDivider.getClock());
            for (auto& light : module.lights) emit(light.getBrightness());
        }
        std::cout << '\n';
    }
    return std::cin.bad() ? 3 : 0;
}
