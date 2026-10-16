#!/usr/bin/env python3
"""Streaming frozen VCO acceptance validator; no oscillator/kernel implementation.

Numeric and protocol failures return retained evidence with passed=False. File or
configuration errors raise. The caller must retain reports and exit nonzero.
"""
from collections import Counter
from contextlib import ExitStack
import math
import struct

from fixtures import cases, rows, safety_cases, safety_rows


def _bits(value):
    try:
        return struct.pack('<f', value)
    except (OverflowError, struct.error):
        return None


def _exact(a, b):
    return _bits(a) is not None and _bits(a) == _bits(b)


def _read(stream, size, label, reasons, counts):
    line = stream.readline()
    if not line:
        reasons.append(f'{label}: missing row at EOF')
        return None
    counts[label] += 1
    tokens = line.split()
    if len(tokens) != size:
        reasons.append(f'{label}: expected {size} fields, got {len(tokens)}')
        return None
    try:
        values = [float(t) for t in tokens]
    except ValueError as error:
        reasons.append(f'{label}: invalid numeric token: {error}')
        return None
    if any(math.isfinite(v) and _bits(v) is None for v in values):
        reasons.append(f'{label}: finite numeric token outside f32 representation')
        return None
    # Trace decimals encode f32 values, not extra decimal precision.
    return [struct.unpack('<f', _bits(v))[0] if _bits(v) is not None else v for v in values]


def _initial():
    return [0.] * 68 + [0.] * 16 + [1.] * 16 + [0.] * 16 + [1.] * 16 + [0.] * 86


def _independent(label, v, row, previous, anchors, stats, reasons):
    if v is None:
        return 0, 0
    stats[f'{label}_independent_frames'] += 1
    width, mask, bypass = max(1, row[7]), row[75], row[76]
    groups = (width + 3) // 4
    for wave in range(4):
        expected = 1 if bypass else width if mask & (1 << wave) else 0
        stats[f'{label}_independent_counts'] += 1
        if v[wave] != expected:
            reasons.append(f'{label}: count[{wave}]={v[wave]!r}, expected {expected}')
        for lane in range(16):
            value = v[4 + wave * 16 + lane]
            if lane < expected and not math.isfinite(value):
                reasons.append(f'{label}: nonfinite active voltage[{wave}][{lane}]')
            if (bypass or (label == 'rust' and lane >= expected)) and not _exact(value, 0.):
                reasons.append(f'{label}: expected positive-zero padding/bypass[{wave}][{lane}]')
            stats[f'{label}_voltage_policy_fields'] += 1
    for i in range(68, len(v)):
        if not math.isfinite(v[i]):
            reasons.append(f'{label}: nonfinite meaningful state field {i}')
    for lane in range(16):
        if not 0 <= v[68+lane] < 1:
            reasons.append(f'{label}: phase[{lane}] outside [0,1): {v[68+lane]!r}')
        for start, name in [(84, 'direction'), (116, 'square_latch')]:
            if v[start+lane] not in (-1., 1.):
                reasons.append(f'{label}: {name}[{lane}] not ±1')
        if previous is not None and lane // 4 >= groups:
            indices = [start+lane for start in (68,84,100,116)]
            indices += [132+wave*16+lane for wave in range(4)]
            if any(not _exact(v[i], previous[i]) for i in indices):
                reasons.append(f'{label}: inactive quartet state changed at lane {lane}')
    for wave in range(4):
        for group in range(4):
            i = 196 + wave*4 + group
            advances = not bypass and group < groups and bool(mask & (1 << wave))
            if previous is not None:
                expected = (previous[i] + int(advances)) % 32
                if v[i] != expected:
                    reasons.append(f'{label}: cursor[{wave}][{group}]={v[i]!r}, expected {expected}')
            if not v[i].is_integer() or not 0 <= v[i] < 32:
                reasons.append(f'{label}: invalid cursor[{wave}][{group}]')
        if previous is not None and not mask & (1 << wave):
            indices = list(range(132+wave*16,148+wave*16))
            if wave == 3:
                indices += list(range(116,132))
            if any(not _exact(v[i], previous[i]) for i in indices):
                reasons.append(f'{label}: disconnected wave {wave} filter/latch changed')
    if previous is not None:
        expected_divider = (previous[212] + int(not bypass)) % 16
        if v[212] != expected_divider:
            reasons.append(f'{label}: divider={v[212]!r}, expected {expected_divider}')
        if bypass and any(not _exact(v[i], previous[i]) for i in range(68,218)):
            reasons.append(f'{label}: bypass mutated compact runtime state')
    if not v[212].is_integer() or not 0 <= v[212] < 16:
        reasons.append(f'{label}: invalid divider')
    lamps = v[213:218]
    for i, value in enumerate(lamps):
        low, high = (-1.00001,1.00001) if i < 2 else (0.,1.)
        if not low <= value <= high:
            reasons.append(f'{label}: light[{i}]={value!r} outside [{low},{high}]')
    if not bypass and v[212] == 0:
        wanted = [row[5],row[6]]
        if any(not _exact(x,y) for x,y in zip(lamps[3:],wanted)):
            reasons.append(f'{label}: tick mode lights differ from receiving switches')
        if width > 1 and any(not _exact(x,y) for x,y in zip(lamps[:3],[0.,0.,1.])):
            reasons.append(f'{label}: poly tick lights differ from [0,0,1]')
        elif width == 1 and not _exact(lamps[2],0.):
            reasons.append(f'{label}: mono blue light is not positive zero')
    elif previous is not None and any(not _exact(v[i],previous[i]) for i in range(213,218)):
        reasons.append(f'{label}: lights changed between ticks or during bypass')
    checked = fields = 0
    for anchor in anchors:
        checked += 1
        for key, expected in anchor.items():
            if key == 'frame':
                continue
            if key == 'phase_zero':
                pairs = [(68+lane,0.) for lane in expected]
            elif key == 'direction_lanes':
                pairs = [(84+int(lane),value) for lane,value in expected.items()]
            elif key == 'tails_zero':
                if len(v) != 2266:
                    reasons.append(f'{label}: tail anchor lacks full checkpoint')
                pairs = [(i,0.) for i in range(218,len(v))] if expected else []
            else:
                raise ValueError(f'Unknown frozen anchor {key}')
            fields += len(pairs)
            failures = [(i,v[i],wanted) for i,wanted in pairs if not _exact(v[i],wanted)]
            if failures:
                reasons.append(f'{label}: anchor {key} failed in {len(failures)} fields; first={failures[0]}')
    return checked, fields


def _compare(left, right, row, gates, stats, maxima, reasons):
    if left is None or right is None:
        return
    groups = [('counts',range(4),0.),('exact_runtime',range(68,132),0.),
              ('filter_state',range(132,196),gates['normalized_state_absolute']),
              ('exact_cursors_divider_lights',range(196,218),0.)]
    if row[77]:
        groups.append(('correction_tails',range(218,2266),gates['normalized_state_absolute']))
    width = 1 if row[76] else max(1,row[7])
    output_indices = [4+wave*16+lane for wave in range(4)
                      if row[76] or row[75] & (1<<wave) for lane in range(width)]
    groups.append(('output_volts',output_indices,gates['output_absolute_volts']))
    for name,indices,tolerance in groups:
        failures=[]
        for i in indices:
            stats[name] += 1
            finite=math.isfinite(left[i]) and math.isfinite(right[i])
            error=abs(left[i]-right[i]) if finite else math.inf
            if tolerance:
                if finite:
                    maxima[name]=max(maxima.get(name,0.),error)
                matches=finite and error<=tolerance
            else:
                matches=finite and _exact(left[i],right[i])
            if not matches:
                failures.append((i,left[i],right[i],error))
        if failures:
            reasons.append(f'{name}: {len(failures)} mismatches, first(field,SDK,Rust,error)={failures[0]}, tolerance={tolerance}')


def validate(rate,a,phase,input_path,sdk_trace_path_or_None,rust_trace_path):
    """Validate every frozen input/trace row, bounded-memory, retaining failures."""
    phase = 'reference' if phase == 'normal' else phase
    if phase not in ('reference','safety'):
        raise ValueError('phase must be reference or safety')
    if (phase == 'reference') != (sdk_trace_path_or_None is not None):
        raise ValueError('SDK oracle is required only for reference, forbidden for safety')
    schedules=(safety_cases if phase=='safety' else cases)(rate,a)
    anchors={item['name']:{anchor['frame']:anchor for anchor in item['anchors']} for item in schedules}
    stats,trace_rows=Counter(),Counter()
    report={'passed':False,'phase':phase,'rate':rate,'frames':0,'failed_frames':0,
            'protocol_failures':0,'first_failures':[],'max_errors':{},
            'independent_anchors_checked':0,'independent_anchor_fields':0,'checkpoints':0,
            'oracle_claim':phase=='reference'}
    previous={'sdk':None,'rust':None}
    def record(name,frame,index,reasons,protocol=False):
        if not reasons:
            return
        report['failed_frames'] += 1
        if protocol:
            report['protocol_failures'] += 1
        if len(report['first_failures']) < 16:
            report['first_failures'].append({'case':name,'frame':frame,'row':index,'reasons':reasons[:32],
                                             'reason_count':len(reasons)})
    with ExitStack() as stack:
        streams={'input':stack.enter_context(open(input_path)),
                 'rust':stack.enter_context(open(rust_trace_path))}
        if sdk_trace_path_or_None is not None:
            streams['sdk']=stack.enter_context(open(sdk_trace_path_or_None))
        iterator=(safety_rows if phase=='safety' else rows)(rate,a)
        for index,(name,frame,row) in enumerate(iterator):
            reasons=[]
            actual=_read(streams['input'],78,'input',reasons,trace_rows)
            if actual is not None:
                mismatches=[i for i,(x,y) in enumerate(zip(actual,row))
                            if not (math.isnan(x) and math.isnan(y)) and not _exact(x,y)]
                if mismatches:
                    reasons.append(f'input differs from frozen row at fields {mismatches}')
            size=218+2048*row[77]
            values={label:_read(stream,size,label,reasons,trace_rows)
                    for label,stream in streams.items() if label!='input'}
            protocol=bool(reasons)
            if row[0]==1:
                previous={'sdk':_initial(),'rust':_initial()}
            for label,v in values.items():
                checked,fields=_independent(label,v,row,previous[label],
                    [anchors[name][frame]] if frame in anchors[name] else [],stats,reasons)
                report['independent_anchors_checked']+=checked
                report['independent_anchor_fields']+=fields
                previous[label]=v[:218] if v is not None else None
            if phase=='reference':
                _compare(values['sdk'],values['rust'],row,a['numeric_gate'],stats,report['max_errors'],reasons)
            report['frames']+=1
            report['checkpoints']+=row[77]
            record(name,frame,index,reasons,protocol)
        for label,stream in streams.items():
            extras=sum(1 for _ in stream)
            trace_rows[label]+=extras
            if extras:
                record('<EOF>',None,report['frames'],[f'{label}: {extras} unexpected extra rows'],True)
    report['trace_rows']=dict(trace_rows)
    report['compared_field_counts']=dict(stats)
    report['passed']=report['failed_frames']==0
    return report


def validate_kernel(a,sdk_path,rust_path):
    """Actual prepared chronological 512 impulse-minus1 +512 ramp coefficients."""
    report={'passed':False,'fields':0,'failed_fields':0,'first_failures':[],
            'max_error':0.,'independent_anchors_checked':0}
    values={}
    for label,path in [('sdk',sdk_path),('rust',rust_path)]:
        with open(path) as stream:
            tokens=stream.read().split()
        try:
            parsed=[float(t) for t in tokens]
            values[label]=[struct.unpack('<f',_bits(v))[0] if _bits(v) is not None else v for v in parsed]
        except ValueError:
            values[label]=[]
        if len(values[label])!=1024:
            report['first_failures'].append(f'{label}: expected1024 numeric coefficients, got{len(values[label])}')
            report['failed_fields']+=1
        else:
            for i,wanted in [(0,-1.),(512,0.)]:
                report['independent_anchors_checked']+=1
                if not _exact(values[label][i],wanted):
                    report['first_failures'].append(f'{label}: independent kernel endpoint[{i}] != {wanted}')
                    report['failed_fields']+=1
    if all(len(v)==1024 for v in values.values()):
        for i,(left,right) in enumerate(zip(values['sdk'],values['rust'])):
            report['fields']+=1
            finite=math.isfinite(left) and math.isfinite(right) and _bits(left) is not None and _bits(right) is not None
            error=abs(left-right) if finite else math.inf
            if finite:
                report['max_error']=max(report['max_error'],error)
            if not finite or error>a['numeric_gate']['kernel_absolute']:
                report['failed_fields']+=1
                if len(report['first_failures'])<16:
                    report['first_failures'].append({'field':i,'sdk':left,'rust':right,'error':error})
    report['passed']=report['failed_fields']==0
    return report
