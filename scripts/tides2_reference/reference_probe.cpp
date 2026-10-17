// Original OpenPatch host adapter, MIT licensed. DSP and resources remain
// unmodified, separately attributed upstream files from source_manifest.json.
#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <stdexcept>
#include <vector>
#include "tides2/poly_slope_generator.h"
#include "tides2/ramp/ramp_extractor.h"

static uint32_t bits(float value) {
  uint32_t result;
  std::memcpy(&result, &value, sizeof(result));
  return result;
}
static void require(bool value, const char* message) {
  if (!value) throw std::runtime_error(message);
}

int main() {
  try {
    // The real firmware owns zero-initialized global instances. One fresh
    // process per case supplies that same construction domain without memset.
    static tides::PolySlopeGenerator generator;
    static tides::RampExtractor extractor;
    float rate;
    size_t size, blocks;
    int mode, output_mode, range, trigger_patched, clock_patched;
    size_t gate_period, gate_high, clock_period, clock_high;
    float ratio;
    int denominator;
    require(bool(std::cin >> rate >> size >> blocks >> mode >> output_mode >> range
      >> trigger_patched >> clock_patched >> gate_period >> gate_high
      >> clock_period >> clock_high >> ratio >> denominator), "invalid header");
    require(rate > 0 && size > 0 && size <= 64 && blocks <= 20000, "invalid dimensions");
    require(mode >= 0 && mode < tides::RAMP_MODE_LAST && output_mode >= 0 && output_mode < tides::OUTPUT_MODE_LAST
      && range >= 0 && range < tides::RANGE_LAST, "invalid enum");
    generator.Init();
    extractor.Init(rate, 40.0f / rate);
    stmlib::GateFlags previous_gate = stmlib::GATE_FLAG_LOW;
    stmlib::GateFlags previous_clock = stmlib::GATE_FLAG_LOW;
    std::vector<stmlib::GateFlags> gates(size), clocks(size);
    std::vector<float> ramp(size);
    std::vector<tides::PolySlopeGenerator::OutputSample> output(size);
    for (size_t block = 0; block < blocks; ++block) {
      float frequency, slope, shape, smoothness, shift;
      int reset_filter, reset_extractor;
      require(bool(std::cin >> frequency >> slope >> shape >> smoothness >> shift >> reset_filter >> reset_extractor), "invalid controls");
      if (reset_filter) generator.Reset();  // Source Reset resets the filter only.
      if (reset_extractor) extractor.Reset();
      for (size_t i = 0; i < size; ++i) {
        const size_t frame = block * size + i;
        previous_gate = stmlib::ExtractGateFlags(previous_gate, trigger_patched && gate_period && frame % gate_period < gate_high);
        previous_clock = stmlib::ExtractGateFlags(previous_clock, clock_patched && clock_period && frame % clock_period < clock_high);
        gates[i] = previous_gate;
        clocks[i] = previous_clock;
      }
      if (clock_patched) {
        frequency = extractor.Process(range == tides::RANGE_AUDIO,
          range == tides::RANGE_AUDIO && mode == tides::RAMP_MODE_AR,
          tides::Ratio{ratio, denominator}, clocks.data(), ramp.data(), size);
      }
      const bool use_ramp = clock_patched && !trigger_patched;
      generator.Render(tides::RampMode(mode), tides::OutputMode(output_mode), tides::Range(range),
        frequency, slope, shape, smoothness, shift, gates.data(), use_ramp ? ramp.data() : nullptr,
        output.data(), size);
      for (size_t i = 0; i < size; ++i) {
        require(std::isfinite(frequency), "nonfinite frequency");
        std::cout << "{\"frame\":" << block * size + i << ",\"block\":" << block
          << ",\"frequency_bits\":" << bits(frequency) << ",\"gate_flags\":" << unsigned(gates[i])
          << ",\"clock_flags\":" << unsigned(clocks[i]) << ",\"extracted_ramp_bits\":";
        if (clock_patched) {
          require(std::isfinite(ramp[i]), "nonfinite extracted ramp");
          std::cout << bits(ramp[i]);
        } else std::cout << "null";
        std::cout << ",\"output_bits\":[";
        for (size_t channel = 0; channel < 4; ++channel) {
          require(std::isfinite(output[i].channel[channel]), "nonfinite output");
          if (channel) std::cout << ',';
          std::cout << bits(output[i].channel[channel]);
        }
        std::cout << "]}\n";
      }
    }
    require(bool(std::cout), "output write failed");
    std::string trailing;
    require(!(std::cin >> trailing), "unexpected trailing input");
    return 0;
  } catch (const std::exception& error) {
    std::cerr << error.what() << '\n';
    return 1;
  }
}
