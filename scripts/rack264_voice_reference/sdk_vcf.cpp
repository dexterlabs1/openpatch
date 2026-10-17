// SPDX-License-Identifier: MIT
// Original observation adapter. reference_module.hpp retains the unchanged
// pinned Fundamental 2.6.4 VCF class prefix, with its license in local evidence.
// No DSP replacement or change to the upstream constructor/process/reset body.
#include "reference_module.hpp"
#include <array>
#include <cassert>
#include <cerrno>
#include <cfenv>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iomanip>
#include <iostream>
#include <limits>
#include <sstream>
#include <string>

// strtof preserves subnormals that libc++ formatted float extraction rejects.
static bool parseFloat(const std::string& token, float& value) {
    char* end = nullptr;
    value = std::strtof(token.c_str(), &end);
    return end != token.c_str() && *end == '\0' && std::isfinite(value);
}

static bool parseUnsigned(const std::string& token, std::uint64_t& value) {
    if (token.empty() || token[0] == '-' || token[0] == '+') return false;
    char* end = nullptr;
    errno = 0;
    const bool hex = token.size() > 2 && token[0] == '0' && (token[1] == 'x' || token[1] == 'X');
    value = std::strtoull(token.c_str(), &end, hex ? 16 : 10);
    return end != token.c_str() && *end == '\0' && errno != ERANGE;
}

static bool parseRow(const std::string& line, std::array<float, 77>& row) {
    std::istringstream stream(line);
    for (float& value : row) {
        std::string token;
        if (!(stream >> token) || !parseFloat(token, value)) return false;
    }
    std::string extra;
    return !(stream >> extra);
}

static bool integerInRange(float value, int low, int high) {
    return value >= low && value <= high && value == std::trunc(value);
}

int main(int argc, char** argv) {
    if (argc != 5 || rack::APP_VERSION != "2.6.6" || std::fegetround() != FE_TONEAREST) return 2;
    std::uint64_t integerRate, seed0, seed1;
    float history;
    if (!parseUnsigned(argv[1], integerRate) || integerRate == 0
        || integerRate > std::numeric_limits<std::uint32_t>::max()
        || !parseUnsigned(argv[2], seed0) || !parseUnsigned(argv[3], seed1)
        || (seed0 == 0 && seed1 == 0) || !parseFloat(argv[4], history)) return 2;
    const float rate = static_cast<float>(integerRate);
    std::cerr << std::setprecision(9) << "FE_TONEAREST=1\n";
#if defined(__aarch64__) || defined(__arm64__)
    std::uint64_t fpcr;
    asm volatile("mrs %0, fpcr" : "=r"(fpcr));
    std::cerr << "FPCR=" << fpcr << '\n';
#endif
    std::cout << std::setprecision(9);
    VCF module;
    // Upstream's previous input is uninitialized. Assign all SIMD groups
    // explicitly before the first process, and report the chosen harness state.
    // This is not an assertion about undefined upstream cold-start behavior.
    for (auto& filter : module.filters) filter.input = rack::simd::float_4(history);
    rack::random::local().seed(seed0, seed1);
    std::cerr << "INITIAL_HISTORY_ASSIGNED=16 INITIAL_PREVIOUS_INPUT=" << history << '\n';
    std::cerr << "INITIAL_RNG=" << rack::random::local().state[0] << ' '
              << rack::random::local().state[1] << '\n';
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = rate;
    args.sampleTime = 1.f / rate;
    const int paramIds[] = {VCF::FREQ_PARAM, VCF::RES_PARAM, VCF::FREQ_CV_PARAM,
                            VCF::RES_CV_PARAM, VCF::DRIVE_PARAM, VCF::DRIVE_CV_PARAM};
    const float minFrequency = (std::log2(rack::dsp::FREQ_C4 / 8000.f) + 5.f) / 10.f;
    const float maxFrequency = (std::log2(8000.f / rack::dsp::FREQ_C4) + 5.f) / 10.f;
    unsigned previousMask = 0;
    std::string line;
    while (std::getline(std::cin, line)) {
        std::array<float, 77> row{};
        if (!parseRow(line, row) || !integerInRange(row[0], 0, 2)
            || !integerInRange(row[1], 0, 1) || !integerInRange(row[2], 0, 3)) return 3;
        const int action = static_cast<int>(row[0]);
        const bool observe = row[1] == 1.f;
        const unsigned mask = static_cast<unsigned>(row[2]);
        if (row[3] < minFrequency || row[3] > maxFrequency || row[4] < 0.f || row[4] > 1.f) return 3;
        for (int i = 2; i < 6; ++i) if (row[3 + i] < -1.f || row[3 + i] > 1.f) return 3;
        for (int i = 0; i < 6; ++i) module.params[paramIds[i]].setValue(row[3 + i]);
        if (action == 1) module.onReset();
        // Action2 retains this actual C++ class instance; Rust rebuilds controls
        // and invokes the production same-rate adoption path.
        for (int portIndex = 0; portIndex < 4; ++portIndex) {
            const int start = 9 + 17 * portIndex;
            if (!integerInRange(row[start], 0, 16)) return 3;
            const int width = static_cast<int>(row[start]);
            auto& port = module.inputs[portIndex];
            port.channels = width;
            for (int lane = 0; lane < 16; ++lane) {
                const float value = row[start + 1 + lane];
                if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
                port.voltages[lane] = value;
            }
        }
        for (int output = 0; output < 2; ++output) {
            const unsigned bit = 1u << output;
            if (!(mask & bit)) module.outputs[output].channels = 0;
            else if (!(previousMask & bit)) module.outputs[output].channels = 1;
        }
        previousMask = mask;
        module.process(args);
        ++args.frame;
        const int width = std::max(module.inputs[VCF::IN_INPUT].getChannels(), 1);
        std::cout << width;
        auto emit = [](float value) {
            assert(std::isfinite(value));
            std::cout << ' ' << value;
        };
        for (int output = 0; output < 2; ++output) {
            const bool connected = mask & (1u << output);
            auto& port = module.outputs[output];
            // Inspect actual genuine SDK connected-channel behavior before
            // converting disconnected/hidden storage to host publication zeros.
            assert(port.getChannels() == (connected ? width : 0));
            for (int lane = 0; lane < 16; ++lane) {
                emit(connected && lane < width ? port.voltages[lane] : 0.f);
            }
        }
        std::cout << ' ' << rack::random::local().state[0] << ' ' << rack::random::local().state[1];
        if (observe) {
            for (int lane = 0; lane < 16; ++lane) {
                const auto& filter = module.filters[lane / 4];
                for (int state = 0; state < 4; ++state) emit(filter.state[state][lane % 4]);
                emit(filter.input[lane % 4]);
            }
        }
        std::cout << '\n';
    }
    return std::cin.bad() ? 3 : 0;
}
