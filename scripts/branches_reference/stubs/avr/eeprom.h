#pragma once
#include <cstdint>
inline uint8_t eeprom_read_byte(const uint8_t*) { return 0xff; }
inline void eeprom_write_byte(uint8_t*, uint8_t) {}
