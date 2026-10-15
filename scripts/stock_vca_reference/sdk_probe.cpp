// SPDX-License-Identifier: MIT
// Original host adapter; the actual pinned class lives in a temporary header.
#include "reference_module.hpp"
#include <cassert>
#include <iomanip>
#include <iostream>
int main(int argc, char** argv) {
    if (argc != 2) return 2;
    VCA_1 module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]); args.sampleTime = 1.f / args.sampleRate;
    std::cout << std::setprecision(9);
    int connected;
    while (std::cin >> connected) {
        float level, response;
        if (!(std::cin >> level >> response)) return 3;
        module.params[VCA_1::LEVEL_PARAM].setValue(level);
        module.params[VCA_1::EXP_PARAM].setValue(response);
        for (auto& input : module.inputs) {
            int width; if (!(std::cin >> width) || width < 0 || width > 16) return 3;
            input.channels = width;
            for (int lane = 0; lane < 16; lane++) {
                float value; if (!(std::cin >> value)) return 3;
                // Rack engine cables zero missing lanes. Rust receives poisoned
                // padding directly to verify its host PortFrame read boundary.
                input.voltages[lane] = lane < width ? value : 0.f;
            }
        }
        module.outputs[0].channels = connected ? 1 : 0;
        module.process(args);
        // These are actual SDK channel metadata, not recomputed from inputs.
        std::cout << module.outputs[0].getChannels() << ' ' << module.lastChannels;
        for (int lane = 0; lane < 16; lane++)
            std::cout << ' ' << (connected && lane < module.lastChannels ? module.outputs[0].getVoltage(lane) : 0.f);
        for (int lane = 0; lane < 16; lane++)
            std::cout << ' ' << (lane < module.lastChannels ? module.lastGains[lane] : 0.f);
        std::cout << '\n'; args.frame++;
    }
}
