#!/usr/bin/env python3
"""Deterministic bounded source contract corpus; never executes DSP."""
import json
from pathlib import Path

def settings(pack, xyz, density): return [0,1,pack,*xyz,*density]
def case(name, rows, seed=33): return {'id':name,'seed':seed,'operations':rows}
def generate():
    cases=[case('resources_and_map',[[30,1]]+[[20,1,x,y] for x in [0,63,64,127,128,191,192,255] for y in [0,63,64,127,128,191,192,255]])]
    # Every source-defined packed option byte; enum LAST=3 is declared then clamped.
    cases.append(case('packed_options', [settings(pack,[127,128,255],[1,128,255]) for pack in range(256) if (pack&7)<=3]))
    for resolution,increment in [(0,6),(1,3),(2,1)]:
        rows=[]
        for density in [0,1,63,64,127,128,191,192,254,255]:
            for clock in [0,1]:
                rows += [settings(0x40|0x88|resolution|(clock<<5),[63,192,0],[density,255-density,density]),[1,192,increment]]
        cases.append(case(f'drums_resolution_{resolution}',rows))
    for seed in [0,1,33,65535,0x12345678]:
        rows=[]
        for chaos in [0,1,127,255]:
            for swing in [0,1]:
                rows += [settings(0x40|0x80|2|(0 if swing else 8),[255,255,chaos],[95,160,224]),[5,1],[1,289,1],[2,2],[3,260],[4,1]]
        cases.append(case(f'chaos_seed_{seed}',rows,seed))
    # All 32 lengths and 32 density addresses, three offset lanes, two full cycles.
    for length in range(1,33):
        rows=[]
        for density in range(32):
            lengths=[length,1+(length+6)%32,1+(length+14)%32]
            densities=[density,(density+11)%32,(density+23)%32]
            rows += [settings(0x8a|((density&1)<<5),[(v-1)<<3 for v in lengths],[v<<3 for v in densities]),[5,1],[1,64,6]]
        cases.append(case(f'euclidean_length_{length}',rows))
    rows=[]
    for pack in [0xca,0x4a,0xea,0x6a,0x82,0x02,0xa2,0x22]:
        rows += [settings(pack,[255,193,254],[255,200,175]),[5,1],[2,1],[3,7],[3,1],[3,249],[3,10],
                 [1,3,1],[4,1],[2,1],[5,1],[2,1],[1,100,3],[4,1]]
    cases.append(case('live_modes_reset_retrigger_falling_counter_wrap',rows))
    # Actual phase traversal plus wraps on either side of boundaries, reset and locks.
    for resolution in range(3):
        rows=[]
        for bpm in [0,1,30,120,240,480,511]:
            rows += [[10,1,bpm,resolution],[13,1],[14,1],[11,1700],[12,1,0],[15,1]]
            for swing in [42,214,1,255,127,128]:
                rows += [[11,1400],[12,1,swing]]
        cases.append(case(f'clock_resolution_{resolution}',rows))
    cases.append(case('clock_all_lut_entries',[[10,1,bpm,resolution] for bpm in range(512) for resolution in range(3)]))
    # Frozen composite source scan: >513 scans, active-low physical inputs only.
    # Helpers are original firmware; ADC/button/MIDI/shift-register remain excluded.
    rows=[]
    for resolution in range(3):
        for internal in [0,1]:
            for pack in [0xca,0x4a,0xea,0x6a,0x82,0x02,0xa2,0x22]:
                pack=(pack&~7)|resolution
                rows += [settings(pack,[255,193,254],[255,200,175]),[10,1,480 if internal else 0,resolution],
                         [40,17,0,0],[40,1,1,1],[40,8,1,1],[40,9,0,0],
                         [40,1,1,0],[40,8,1,0],[40,1,0,1],[40,9,0,1],[40,1100 if internal else 17,0,0]]
    cases.append(case('native_scan_composite',rows))
    return {'schema':1,'cases':cases,'negative_source_cases':['--undefined-clock','--undefined-map','--undefined-options-4','--undefined-options-7']}
if __name__=='__main__': Path(__file__).with_name('cases.json').write_text(json.dumps(generate(),indent=2)+'\n')
