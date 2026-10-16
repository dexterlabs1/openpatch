# SPDX-License-Identifier: MIT
"""Original deterministic synthetic inputs; contains no upstream DSP."""
import io
import json
import math
from pathlib import Path
import struct

HERE = Path(__file__).resolve().parent
MANIFEST = json.loads((HERE / 'fixtures.json').read_text())
HEADER = struct.Struct('<IIfIIIII')
OUTPUT = struct.Struct('<I67f8I')
MAGIC = 0x3154574f

def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]

def bank(name, length=128):
    if name == 'default':
        return 1024, []
    if name == 'dc_endpoints':
        return length, [f32(x) for x in [-0.75, 0.25, 0.875] for _ in range(length)]
    if name == 'saw':
        return length, [f32(2*i/length-1) for i in range(length)]
    if name == 'harmonic':
        waves = [lambda p: math.sin(p),
                 lambda p: 0.6*math.sin(p)+0.25*math.sin(3*p)+0.1*math.cos(7*p),
                 lambda p: 0.375,
                 lambda p: 0.5*math.cos(2*p)-0.2*math.sin(5*p)]
        return length, [f32(fn(2*math.pi*i/length)) for fn in waves for i in range(length)]
    raise ValueError(name)

def rows(kind, name, frames, rate):
    for frame in range(frames):
        # Source defaults, with higher LFO frequency for several full cycles.
        p = [0., 0., 0., 0.25, 0., 0., 0.] if kind == 0 else [1., 0., 7., 0.25, 0., 0.]
        pos_index, fm_index, cv_index = (3, 4, 5) if kind == 0 else (3, 4, 5)
        ports = [(0, [997.-3*i for i in range(16)]) for _ in range(4)]
        event, payload = 0, None
        def port(index, value, width=1):
            values = value if isinstance(value, list) else [value]*width
            assert len(values) >= width
            ports[index] = (width, [f32(x) for x in values[:width]] + [997.-3*i for i in range(width, 16)])
        def primary(width):
            if kind == 0:
                port(3, [-0.5+lane*0.055 for lane in range(width)], width)
            else:
                port(0, [(lane-4)*0.09 for lane in range(width)], width)
                p[fm_index] = 0.25
        if name in ['default', 'harmonic', 'saw', 'dc_endpoints']:
            pass
        elif name.startswith('position_'):
            p[pos_index] = {'position_0': 0., 'position_1': 1., 'position_between': 0.5}[name]
        elif name == 'signed_position_cv':
            primary(5)
            p[pos_index] = 0.5
            p[cv_index] = [-1., -0.5, 0., 0.5, 1.][frame//127%5]
            port(2, [-20., -5., 0., 5., 20.], 5)
        elif name in ['exponential_fm', 'positive_linear_fm']:
            primary(5)
            p[fm_index] = 0.25
            port(0, [math.sin((frame+lane*7)/19) for lane in range(5)], 5)
            if kind == 0 and name == 'positive_linear_fm':
                p[6] = 1.
        elif name in ['hard_sync', 'soft_sync']:
            primary(5)
            p[1] = float(name == 'soft_sync')
            port(1, [4. if (frame+lane*11)%137 < 41 else -3. for lane in range(5)], 5)
        elif name == 'reset':
            primary(5)
            # Start low, straddle both hysteresis thresholds, hold high and retrigger.
            levels = [0., 0.1, 0.1001, 1.999, 2., 10., 1., 0.1, 0., 3.]
            port(1, [levels[(frame//19+lane)%len(levels)] for lane in range(3)], 3)
        elif name == 'offset_invert':
            p[0] = float((frame//251)%2)
            p[1] = float((frame//503)%2)
            primary(5)
        elif name in ['clock', 'clock_reject_fast']:
            p[2] = 2.
            # Clock width cannot create polyphony; only first clock channel is read.
            if name == 'clock':
                if frame < 300 or frame >= 2700:
                    port(3, 0., 0)
                else:
                    period = 239 if frame < 1500 else 479
                    port(3, [10. if frame%period < 3 else 0., 10.], 2)
            else:
                port(3, 10. if frame%7 == 3 else 0.)
        elif name == 'mono_broadcast':
            primary(16)
            port(0, 0.5)
            # For LFO the FM port itself defines output width; mono FM does not broadcast.
            if kind == 1:
                port(0, [0.5]*16, 16)
            p[fm_index] = 0.3
            p[cv_index] = 0.6
            port(2, 2.)
            port(1, -3. if frame%211<107 else 3.)
        elif name == 'short_poly':
            primary(16)
            p[fm_index], p[cv_index] = 0.2, -0.7
            if kind == 0:
                port(0, [0.2, -0.1, 0.3], 3)
            port(2, [-3., 1., 4.], 3)
            port(1, [0. if frame%139<79 else 4., 0.], 2)
        elif name.startswith('width_'):
            primary(int(name[6:]))
        elif name == 'group_freeze':
            primary([16, 1, 5, 4, 0, 16][frame//503%6])
            port(1, -3. if frame%127<71 else 3.)
        elif name == 'parameter_adoption':
            primary([5, 16, 1, 16][frame//317%4])
            p[pos_index] = (frame//109%5)/4
            p[fm_index] = [-0.5, 0., 0.5][frame//173%3]
            p[cv_index] = -0.5
            port(2, 2.)
            port(1, -2. if frame%97<47 else 3.)
            if kind == 0:
                p[1] = float(frame//457%2)
            else:
                p[0], p[1] = float(frame//227%2), float(frame//389%2)
                port(3, 10. if frame%251<3 else 0.)
            if frame%109 == 0:
                event = 8
        elif name == 'loading_freeze':
            primary(5)
            if frame in [257, 1029, 2307]: event = 3
            if frame in [769, 1537, 3073]: event = 4
            if kind == 1: port(3, 10. if frame%239<3 else 0.)
        elif name == 'invalid_bank':
            primary(5)
            if frame == 257: event = 5
            if frame == 1029: event, payload = 2, (1, [0.25])
            if frame == 2307: event, payload = 2, bank('harmonic')
        elif name == 'bank_replace':
            primary(5)
            if frame in [257, 1029, 2307]:
                event, payload = 2, bank(['saw', 'dc_endpoints', 'harmonic'][[257,1029,2307].index(frame)], 64)
        elif name == 'wave_length_rebuild':
            primary(5)
            if frame in [257, 1029, 2307]:
                event, payload = 7, [64, 256, 128][[257,1029,2307].index(frame)]
        elif name == 'source_reset':
            primary(5)
            port(1, -3. if frame%127<71 else 3.)
            if frame in [257, 1029, 2307]: event = 1
        else:
            raise ValueError(name)
        yield event, payload, [f32(x) for x in p], ports

def generate(kind, name, rate, frames=None):
    if frames is None: frames = MANIFEST['frames_default']
    bank_name = name if name in MANIFEST['banks'] else ('dc_endpoints' if name.startswith('position_') else 'harmonic')
    length, samples = bank(bank_name)
    out = io.BytesIO()
    out.write(HEADER.pack(MAGIC, kind, rate, frames, 1, length, len(samples), bank_name == 'default'))
    out.write(struct.pack('<%df'%len(samples), *samples))
    for event, payload, params, ports in rows(kind, name, frames, rate):
        out.write(struct.pack('<I', event))
        if event == 2:
            wave_length, new_samples = payload
            out.write(struct.pack('<II%df'%len(new_samples), wave_length, len(new_samples), *new_samples))
        elif event == 7:
            out.write(struct.pack('<I', payload))
        out.write(struct.pack('<%df'%len(params), *params))
        for width, values in ports:
            out.write(struct.pack('<I16f', width, *values))
    return out.getvalue()
