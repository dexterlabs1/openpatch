// Independent orchestration using actual SDK primitives; no upstream module copy.
#include <dsp/digital.hpp>
#include <engine/Light.hpp>
#include <engine/Port.hpp>
#include <iomanip>
#include <iostream>

int main(int argc, char** argv) {
    const float sampleTime = 1.f / std::stof(argv[1]);
    rack::engine::Input input;
    rack::engine::Light lights[32];
    rack::dsp::ClockDivider divider;
    divider.setDivision(16);
    int channels = 0, count, adopt;
    std::cout << std::setprecision(9);
    while (std::cin >> adopt >> count) {
        // Emulate engine port bookkeeping (including disconnected zero lanes).
        input.channels = count;
        for (int c = 0; c < 16; ++c) {
            float value;
            std::cin >> value;
            input.setVoltage(c < count ? value : 0.f, c);
        }
        // Rust rebuild/adoption must equal this uninterrupted reference state.
        if (divider.process()) {
            channels = input.getChannels();
            for (int c = 0; c < 16; ++c) {
                float voltage = input.getVoltage(c) / 10.f;
                lights[c * 2].setSmoothBrightness(voltage, sampleTime * divider.getDivision());
                lights[c * 2 + 1].setSmoothBrightness(-voltage, sampleTime * divider.getDivision());
            }
        }
        std::cout << channels;
        for (auto& light : lights) std::cout << ' ' << light.getBrightness();
        std::cout << '\n';
    }
}
