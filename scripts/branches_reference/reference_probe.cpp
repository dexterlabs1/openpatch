// Adapter only: the included, hash-pinned upstream firmware owns all decisions.
#include <cstdint>
#include <cstdlib>
#include <iostream>

bool probe_begin_scan();
struct ProbeDone {};
#define main branches_firmware_main
// Only branches.cc's one while(1) is redirected. No arithmetic is rewritten.
#define while(condition) while (probe_begin_scan())
#include "branches/branches.cc"
#undef while
#undef main

namespace {
uint32_t seed;
uint64_t frame;
bool started;
unsigned remaining;
unsigned values[8];

void emit() {
  std::cout << '[' << frame++ << ','
      << unsigned(out_1_a.value()) << ',' << unsigned(out_1_b.value()) << ','
      << unsigned(out_2_a.value()) << ',' << unsigned(out_2_b.value()) << ','
      << previous_state[0] << ',' << previous_state[1] << ','
      << input_state[0] << ',' << input_state[1] << ','
      << unsigned(led_state[0]) << ',' << unsigned(led_state[1]) << ','
      << led_gate_delay[0] << ',' << led_gate_delay[1] << ','
      << rng_state << "]\n";
}
}

bool probe_begin_scan() {
  if (started) emit();
  else { rng_state = seed; started = true; }
  if (!remaining) {
    if (!(std::cin >> remaining)) throw ProbeDone{};
    if (!remaining) std::abort();
    for (unsigned &value : values) if (!(std::cin >> value)) std::abort();
    for (unsigned i : {0u, 1u, 4u, 5u, 6u, 7u}) if (values[i] > 1) std::abort();
    if (values[2] > 65535 || values[3] > 65535) std::abort();
  }
  --remaining;
  // Hardware input is active low; the kernel API accepts logical asserted gates.
  values[0] ? in_1.Low() : in_1.High();
  values[1] ? in_2.Low() : in_2.High();
  for (unsigned i = 0; i < 2; ++i) {
    p[i] = values[2+i];
    toggle_mode[i] = values[4+i];
    latch_mode[i] = values[6+i];
  }
  return true;
}

int main() {
  if (!(std::cin >> seed)) return 2;
  for (unsigned i = 0; i < 256; ++i)
    std::cout << "[\"adc\"," << i << ',' << pgm_read_word(linear_table+i) << "]\n";
  // Its Init, GateOn, GateOff, Read, LED GPIO writes and complete loop execute.
  // Switches remain unpressed and ADC ready is false: scan-level injection above
  // deliberately excludes their scheduler rather than inventing host timing.
  // Throwing at EOF avoids falling off the renamed int main: only actual main
  // has C++'s implicit return-zero rule. Source statements remain untouched.
  try { branches_firmware_main(); } catch (const ProbeDone&) {}
}
