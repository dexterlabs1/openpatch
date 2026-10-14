// Headless reference driver; uses genuine Rack SDK primitives, not Rack's GUI.
// Fundamental Sum orchestration is pinned/documented in run.py.
#include <dsp/digital.hpp>
#include <dsp/vumeter.hpp>
#include <engine/Port.hpp>
#include <iomanip>
#include <iostream>

int main(int argc, char** argv) {
    const float sampleTime = 1.f / std::stof(argv[1]);
    rack::engine::Input input;
    rack::dsp::VuMeter2 meter;
    rack::dsp::ClockDivider vu, lights;
    meter.lambda = 1 / 0.1f;
    vu.setDivision(16);
    lights.setDivision(512);
    int channels = 0, count;
    float level, brightness[6] = {};
    const float lo[] = {0, -3, -6, -12, -24, -36};
    const float hi[] = {0, 0, -3, -6, -12, -24};
    std::cout << std::setprecision(9);
    while (std::cin >> level >> count) {
        input.channels = count; // Stand in for the engine's connected-port bookkeeping.
        for (float& voltage : input.voltages) std::cin >> voltage;
        const float output = input.getVoltageSum() * level;
        if (vu.process()) meter.process(sampleTime * vu.getDivision(), output / 10.f);
        if (lights.process()) {
            channels = input.getChannels();
            for (int i = 0; i < 6; ++i) brightness[i] = meter.getBrightness(lo[i], hi[i]);
        }
        std::cout << output << ' ' << channels;
        for (float value : brightness) std::cout << ' ' << value;
        std::cout << '\n';
    }
}
