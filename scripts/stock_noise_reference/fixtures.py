#!/usr/bin/env python3
"""Input-only Stock Noise declaration. No source DSP or production imports."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct

RATES = (44100, 48000, 96000)
COLORS = ('white', 'pink', 'red', 'violet', 'blue', 'gray', 'black')
# Controlled values replace source PRNG calls. They intentionally make no claim
# about the distribution or identity of Rack's uncontrolled global PRNG.
TAPE_LENGTH = 8192
NORMAL_BITS = (0x00000000, 0x3f800000, 0xbf800000, 0x3f000000,
               0xbf000000, 0x40000000, 0xc0000000, 0x3e800000,
               0xbe800000, 0x3fc00000, 0xbfc00000, 0x3e000000,
               0xbe000000, 0x3f400000, 0xbf400000, 0x00000000)
UNIFORM_BITS = (0x00000000, 0x3e800000, 0x3f000000, 0x3f400000,
                0x3e000000, 0x3f600000, 0x3ec00000, 0x3f200000,
                0x3d800000, 0x3f700000, 0x3e400000, 0x3f500000,
                0x3e900000, 0x3f100000, 0x3ed00000, 0x3f300000)

def float_bits(value): return struct.unpack('<I',struct.pack('<f',value))[0]
def derived_bits(label,index,uniform):
    # Input synthesis only: SHA-256 counter derivation deliberately differs
    # from any Rack or production random implementation.
    word=int.from_bytes(hashlib.sha256(f'stock-noise-reference-v1/{label}/{index}'.encode()).digest()[:4],'little')
    value=(word&0x7fff)/32768 if uniform else ((word&0xffff)-32768)/16384
    return float_bits(value)
NORMAL_TAPE = tuple(NORMAL_BITS) + tuple(derived_bits('normal',i,False) for i in range(16,TAPE_LENGTH))
UNIFORM_TAPE = tuple(UNIFORM_BITS) + tuple(derived_bits('uniform',i,True) for i in range(16,TAPE_LENGTH))
def profile_bits(profile):
    if profile=='broad':return NORMAL_TAPE
    result=[]
    for i in range(TAPE_LENGTH):
        if profile=='dc':value=1.0
        elif profile=='nyquist':value=1.0 if i%2==0 else -1.0
        elif profile=='low':value=math.sin(2*math.pi*i/1024)
        elif profile=='inside':value=math.sin(2*math.pi*128*i/1024)
        elif profile=='outside-high':value=math.sin(2*math.pi*511*i/1024)
        elif profile=='impulse':value=1.0 if i==0 else 0.0
        elif profile=='signed-zero':return tuple(0 if j%2==0 else 0x80000000 for j in range(TAPE_LENGTH))
        else:raise ValueError(profile)
        result.append(float_bits(value))
    return tuple(result)
PROFILES={name:profile_bits(name) for name in ('broad','dc','nyquist','low','inside','outside-high','impulse','signed-zero')}
def step(frames=1): return {'op':'step'} if frames==1 else {'op':'step','frames':frames}
def connect(ident, port, input_port=None):
    return {'op': 'connect', 'id': ident, 'source': 'module', 'port': port,
            'destination': 'sink', 'input': port if input_port is None else input_port}
def disconnect(ident): return {'op': 'disconnect', 'id': ident}
def case(rate, name, mask, events, profile='broad'):
    return {'id': f'{rate}/{name}', 'sample_rate': rate,
            'initial': {'output_mask': mask,'normal_profile':profile,
                        'normal_bits':PROFILES[profile],'uniform_bits':UNIFORM_TAPE},
            'events': events}

def cases():
    for rate in RATES:
        # All 128 physical output-consumer combinations, including zero.
        for mask in range(128):
            events=([step(),step(1022),step(),step()] if mask&(1<<5) else [step(),step()])
            yield case(rate, f'masks/{mask:03}', mask, events)
        yield case(rate, 'all-boundaries', 127, [step() for _ in range(2050)])
        yield case(rate, 'gray-solo-boundaries', 1 << 5,
                   [step() for _ in range(2050)])
        # Gray's cursor/output history must stop when disconnected. Both
        # no-sample cable events and all gap samples remain in the trace.
        yield case(rate, 'gray-freeze', 1 << 5,
                   [step() for _ in range(511)] + [disconnect('out5')] +
                   [step() for _ in range(17)] + [connect('gray-return', 5)] +
                   [step() for _ in range(514)])
        # Voss frame 255 -> 0 changes all eight registers; frame 256 must be
        # observed together with black's position in the shared uniform stream.
        yield case(rate, 'voss-wrap', (1 << 1) | (1 << 4) | (1 << 6),
                   [step() for _ in range(260)])
        yield case(rate, 'source-family-freezes', 0,
                   [step(), connect('w', 0), step(), connect('r', 2), step(),
                    disconnect('w'), step(), connect('v', 3), step(),
                    disconnect('r'), step(), disconnect('v'), step(),
                    connect('p', 1), step(), connect('b', 4), step(),
                    disconnect('p'), step(), disconnect('b'), step(),
                    connect('k', 6), step(), disconnect('k'), step(),
                    connect('g', 5)] + [step() for _ in range(8)])
        yield case(rate, 'cable-bypass-reset-adopt-recreate', 127,
                   [step(), connect('extra-gray', 5, 0), step(),
                    disconnect('out5'), step(), {'op': 'bypass', 'value': True},
                    step(), step(), {'op': 'reset'}, step(),
                    {'op': 'adopt'}, step(), {'op': 'bypass', 'value': False},
                    step(), disconnect('extra-gray'), step(),
                    {'op': 'recreate'}, step(), connect('after-recreate', 5),
                    step()])
        # Genuine nonzero gray data retained through stacked consumers, mixed
        # sink inputs, host clearing, bypass and receiving-incarnation changes.
        yield case(rate,'post-fft-topology',(1<<0)|(1<<5)|(1<<6),
                   [step(),step(1022),step(),
                    connect('gray-mix-white',5,0),step(),
                    disconnect('out5'),step(),
                    connect('gray-mix-black',5,6),step(),
                    disconnect('gray-mix-white'),step(),
                    disconnect('gray-mix-black'),step(),
                    connect('gray-return',5),step(),
                    {'op':'bypass','value':True},
                    {'op':'bypass','value':True},step(),
                    {'op':'reset'},{'op':'adopt'},step(),
                    {'op':'bypass','value':False},step(),
                    disconnect('gray-return'),step(),
                    {'op':'recreate'},step(),
                    connect('gray-after-recreate',5),step()])
        for profile in ('dc','nyquist','low','inside','impulse','signed-zero'):
            yield case(rate,f'gray-profile/{profile}',1<<5,
                       [step(),step(1022),step(),step()],profile)
        if rate==96000:
            yield case(rate,'gray-profile/outside-high',1<<5,
                       [step(),step(1022),step(),step()],'outside-high')

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open('w') as stream:
        for row in cases():
            stream.write(json.dumps(row, sort_keys=True, separators=(',', ':')) + '\n')

if __name__ == '__main__': main()
