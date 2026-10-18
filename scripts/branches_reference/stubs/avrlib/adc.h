#pragma once
#include <cstdint>
namespace avrlib {
enum { ADC_DEFAULT, ADC_LEFT_ALIGNED };
struct Adc {
  void Init() {}
  void set_reference(int) {}
  void set_alignment(int) {}
  void StartConversion(uint8_t) {}
  bool ready() { return false; }
  uint8_t ReadOut8() { return 0; }
};
}
