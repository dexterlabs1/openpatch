#pragma once
#include <cstdint>
#include <cstring>
#define PROGMEM
using prog_char = char;
using prog_uint8_t = uint8_t;
using prog_uint16_t = uint16_t;
using prog_uint32_t = uint32_t;
#define pgm_read_byte(p) (*reinterpret_cast<const uint8_t*>(p))
#define pgm_read_word(p) (*reinterpret_cast<const uint16_t*>(p))
#define pgm_read_dword(p) (*reinterpret_cast<const uint32_t*>(p))
#define strncpy_P std::strncpy
#define memcpy_P std::memcpy
