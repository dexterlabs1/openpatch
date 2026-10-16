// SPDX-License-Identifier: MIT
// Original adapter; reference_module.hpp is the actual pinned Split class prefix.
#include "reference_module.hpp"
#include <cmath>
#include <cstdint>
#include <cstring>
#include <string>
#include <iomanip>
#include <iostream>

int main(int argc, char** argv) {
    if (argc != 2 && argc != 3) return 2;
    const bool raw = argc == 3 && std::string(argv[2]) == "--raw";
    if (argc == 3 && !raw) return 2;
    if (rack::APP_VERSION != "2.6.6") {
        std::cerr << "Rack SDK 2.6.6 required\n";
        return 2;
    }
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    args.sampleTime = 1.f / args.sampleRate;
    std::cout << std::setprecision(9);
    int width;
    while (std::cin >> width) {
        if (width < 0 || width > 16) return 3;
        Split module;
        if (module.lastChannels != 0) return 4;
        auto& input = module.inputs[Split::POLY_INPUT];
        input.channels = width;
        for (int lane = 0; lane < 16; lane++) {
            float value;
            if (raw) {
                std::string token;
                if (!(std::cin >> token) || token.size() != 8) return 3;
                std::size_t consumed = 0;
                const auto bits = static_cast<std::uint32_t>(std::stoul(token, &consumed, 16));
                if (consumed != 8) return 3;
                std::memcpy(&value, &bits, sizeof(value));
            } else {
                if (!(std::cin >> value) || !std::isfinite(value) || std::fabs(value) > 16.f) return 3;
                if (lane >= width && (value != 0.f || std::signbit(value))) return 3;
            }
            input.voltages[lane] = value;
        }
        // Actual Rack Engine.cpp marks a connected output mono before process.
        // setChannels(1) on a disconnected port would intentionally do nothing.
        for (auto& output : module.outputs) output.channels = 1;
        module.process(args);
        std::cout << module.lastChannels;
        for (int lane = 0; lane < 16; lane++) {
            auto& output = module.outputs[Split::MONO_OUTPUTS + lane];
            std::cout << ' ' << output.getChannels() << ' ';
            const float value = output.getVoltage();
            if (raw) {
                std::uint32_t bits;
                std::memcpy(&bits, &value, sizeof(bits));
                std::cout << std::hex << std::setw(8) << std::setfill('0') << bits << std::dec;
            } else {
                std::cout << value;
            }
        }
        std::cout << '\n';
    }
    if (!std::cin.eof()) return 3;
}
