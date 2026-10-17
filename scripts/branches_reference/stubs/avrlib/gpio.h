#pragma once
#include <cstdint>
inline uint8_t TCCR1A = 0, TCCR1B = 0;
inline uint16_t TCNT1 = 0;
namespace avrlib {
struct PortB {};
struct PortC {};
struct PortD {};
enum { DIGITAL_OUTPUT, DIGITAL_INPUT };
template<typename Port, unsigned Pin> struct Gpio {
  inline static uint8_t level = 0;
  static void set_mode(int) {}
  static void Low() { level = 0; }
  static void High() { level = 1; }
  static uint8_t value() { return level; }
};
}
