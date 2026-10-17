#pragma once
#include <cstdint>
#define PROGMEM
using prog_uint16_t = uint16_t;
inline uint16_t pgm_read_word(const uint16_t* p) { return *p; }
