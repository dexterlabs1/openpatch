// SPDX-License-Identifier: MIT
// Original fixture adapter. Detection and cadence execute in genuine libRack.
#include <rack.hpp>
#include <cstdint>
#include <iomanip>
#include <iostream>
struct FixtureModule : rack::engine::Module {
    uint64_t normalCalls = 0, bypassCalls = 0;
    FixtureModule() { config(0, 1, 1, 0); }
    void process(const ProcessArgs&) override { ++normalCalls; }
    void processBypass(const ProcessArgs&) override { ++bypassCalls; }
};
int main(int argc, char** argv) {
    if (argc != 2) return 2;
    rack::settings::cpuMeter = false;
    FixtureModule module;
    rack::engine::Module::ProcessArgs args{};
    args.sampleRate = std::stof(argv[1]); args.sampleTime = 1.f / args.sampleRate;
    std::cout << std::setprecision(9);
    int bypass;
    while (std::cin >> bypass) {
        module.setBypassed(bypass != 0);
        rack::engine::Port* ports[] = {&module.inputs[0], &module.outputs[0]};
        for (auto* port : ports) {
            int channels;
            if (!(std::cin >> channels) || channels < 0 || channels > 16) return 3;
            // Fixture establishes physical connection. setChannels() alone
            // intentionally cannot connect a disconnected SDK port.
            port->channels = channels;
            for (int lane = 0; lane < 16; ++lane)
                if (!(std::cin >> port->voltages[lane])) return 3;
        }
        module.doProcess(args);
        std::cout << args.frame;
        for (auto* port : ports) {
            std::cout << ' ' << port->getChannels();
            for (const auto& light : port->plugLights) std::cout << ' ' << light.value;
        }
        std::cout << '\n';
        ++args.frame;
    }
    std::cerr << rack::APP_VERSION << ' ' << module.normalCalls << ' ' << module.bypassCalls << '\n';
}
