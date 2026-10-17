// OpenPatch companion protocol v1, original code MIT. macOS ARM64 little endian.
// All integers/floats are fixed-width native little endian; no C++ object layout.
// Immutable header is published before child launch. Mutable control/state words
// use lock-free acquire/release uint32 atomics. Payload access requires ownership.
#pragma once
#include <cstddef>
#include <cstdint>
#include <cstring>
namespace openpatch_host {
constexpr uint32_t VERSION=1, QUANTUM=128, RATE=48000, SLOTS=4, EVENTS=64;
constexpr uint32_t INPUTS=3, OUTPUTS=1, CHANNELS=16;
constexpr size_t HEADER=128, SLOT=34368, BYTES=HEADER+SLOTS*SLOT;
constexpr char MAGIC[8]={'O','P','H','S','T','0','0','1'};
// Header offsets: version8, bytes12, quantum16, rate20, slots24, maxEvents28,
// incarnation32:u64, generation40:u64, revision48:u64, inputPorts56,
// outputPorts60, maxChannels64, status68, stop72, error76, processed80,
// inputConnectionMask84. All other header bytes must remain zero.
constexpr size_t STATUS=68, STOP=72, ERROR=76, PROCESSED=80, INPUT_MASK=84;
constexpr uint32_t BOOT=0, READY=1, FAULT=2, STOPPED=3;
// Slot ownership: parent Free->Writing->InputReady, child InputReady->Processing
// ->OutputReady, parent OutputReady->Reading->Free. No owner spins or steals.
constexpr uint32_t FREE=0, INPUT_READY=1, PROCESSING=2, OUTPUT_READY=3, READING=4, WRITING=5;
// slot offsets: state0, error4, sequence8:u64, incarnation16:u64,
// generation24:u64, revision32:u64, eventCount40. Reserved44..63 zero.
constexpr size_t COUNTS=64, INPUT_VALUES=448, EVENT_VALUES=25024;
constexpr size_t OUTPUT_COUNTS=26048, OUTPUT_VALUES=26176;
// counts[port][frame]:u8; values[port][frame][channel]:f32.
// output counts[frame]:u8; output values[frame][channel]:f32.
// Each 16-byte event: sampleOffset:u16,param:u16,value:f32,order:u32,reserved:u32.
// Events sorted strictly by (sampleOffset,order); param0/1 in [-1,1]. All data
// must be validated BEFORE processing. Mask-fixed disconnected input counts=0;
// connected input counts=1..16; unused voltages are zero. Output count0..16.
template<class T> T get(const uint8_t* p,size_t offset) {
    T value;std::memcpy(&value,p+offset,sizeof(value));return value;
}
template<class T> void put(uint8_t* p,size_t offset,T value) {
    std::memcpy(p+offset,&value,sizeof(value));
}
inline uint32_t load(const uint8_t* p,size_t offset) {
    return __atomic_load_n(reinterpret_cast<const uint32_t*>(p+offset),__ATOMIC_ACQUIRE);
}
inline void store(uint8_t* p,size_t offset,uint32_t value) {
    __atomic_store_n(reinterpret_cast<uint32_t*>(p+offset),value,__ATOMIC_RELEASE);
}
inline bool claim(uint8_t* p,uint32_t from,uint32_t to) {
    return __atomic_compare_exchange_n(reinterpret_cast<uint32_t*>(p),&from,to,false,__ATOMIC_ACQUIRE,__ATOMIC_RELAXED);
}
static_assert(__atomic_always_lock_free(4,nullptr),"shared state requires lock-free uint32");
static_assert(sizeof(float)==4 && sizeof(uint64_t)==8,"wire scalar sizes");
} // namespace openpatch_host
