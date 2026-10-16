// SPDX-License-Identifier: MIT
// Original adapter. reference_module.hpp contains the pinned actual Merge class.
#include "reference_module.hpp"
#include <cmath>
#include <iomanip>
#include <iostream>

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    if (rack::APP_VERSION != "2.6.6") {
        std::cerr << "Rack SDK 2.6.6 required\n";
        return 2;
    }
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]);
    args.sampleTime = 1.f / args.sampleRate;
    std::cout << std::setprecision(9);
    int selection;
    while (std::cin >> selection) {
        if (selection < -1 || selection > 16) return 3;
        Merge module;
        module.channels = selection == 0 ? -1 : selection == -1 ? 0 : selection;
        for (auto& input : module.inputs) {
            int width;
            if (!(std::cin >> width) || width < 0 || width > 16) return 3;
            input.channels = width;
            for (int lane = 0; lane < 16; lane++) {
                float value;
                if (!(std::cin >> value) || !std::isfinite(value)) return 3;
                // Rack's engine cables supply zero in missing input lanes.
                input.voltages[lane] = lane < width ? value : 0.f;
            }
        }
        module.process(args);
        std::cout << module.outputs[Merge::POLY_OUTPUT].getChannels()
                  << ' ' << module.automaticChannels;
        for (int lane = 0; lane < 16; lane++)
            std::cout << ' ' << module.outputs[Merge::POLY_OUTPUT].getVoltage(lane);
        std::cout << '\n';
    }
    if (!std::cin.eof()) return 3;
}
