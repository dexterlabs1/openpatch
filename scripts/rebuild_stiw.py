#!/usr/bin/env python3
"""Reconstruct the supplied STIW Rack score with live native DSP via MCP.

No network, baked audio, or source-plugin binaries are used. The manifest records
all source roles, 83 cable translations, substitutions, and audition changes.
"""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
import math
import os
import struct
from pathlib import Path
from test_mcp import MCPProcess, PROTOCOL_VERSION

SOURCE_MAP_PATH = Path(__file__).with_name('stiw_source_map.json')
SOURCE_MAP = json.loads(SOURCE_MAP_PATH.read_text(encoding='utf-8'))
NOTES_MAX_UTF8_BYTES = 8192
REFERENCE_COMMIT = '10dd0160c664770910e5584b7b00498cc48d9ddd'
REFERENCES = {
    kind: {'contract': contract,
           'source': f'https://raw.githubusercontent.com/VCVRack/Fundamental/{REFERENCE_COMMIT}/src/{filename}'}
    for kind, contract, filename in (
        ('stock_vca', 'docs/STOCK_VCA.md', 'VCA-1.cpp'),
        ('adsr_eg', 'docs/ADSR_EG.md', 'ADSR.cpp'),
        ('stock_vcf', 'docs/STOCK_VCF.md', 'VCF.cpp'))
}

def source_id(value):
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        raise ValueError('Source IDs must be decimal strings or integers')
    result = str(value)
    if not result.isdecimal():
        raise ValueError(f'Invalid source ID {result!r}')
    return result

def source_modules(source):
    """Validate curated identities/routes independently of JSON array order."""
    expected = SOURCE_MAP['modules']
    modules = source.get('modules', [])
    if len(modules) != len(expected):
        raise ValueError('Expected exactly 55 curated STIW source modules')
    byid = {}
    for module in modules:
        ident = source_id(module['id'])
        if ident in byid:
            raise ValueError(f'Duplicate source module ID {ident}')
        byid[ident] = module
    if set(byid) != {m['id'] for m in expected}:
        raise ValueError('Missing or extra source module identity')
    resolved = {}
    for role in expected:
        module = byid[role['id']]
        for field in ('plugin', 'model', 'version'):
            if module.get(field) != role[field]:
                raise ValueError(f"{role['alias']}: source {field} differs from curated identity")
        resolved[role['alias']] = module
    cables = source.get('cables', [])
    if len(cables) != SOURCE_MAP['cable_count']:
        raise ValueError('Expected exactly 83 curated STIW source cables')
    bycable = {}
    for cable in cables:
        ident = source_id(cable['id'])
        if ident in bycable:
            raise ValueError(f'Duplicate source cable ID {ident}')
        bycable[ident] = cable
    if set(bycable) != {c['id'] for c in SOURCE_MAP['cables']}:
        raise ValueError('Missing or extra source cable identity')
    for route in SOURCE_MAP['cables']:
        actual = bycable[route['id']]
        expected_from = source_id(resolved[route['from']]['id'])
        expected_to = source_id(resolved[route['to']]['id'])
        if (source_id(actual['outputModuleId']) != expected_from
                or source_id(actual['inputModuleId']) != expected_to
                or type(actual['outputId']) is not int or type(actual['inputId']) is not int
                or actual['outputId'] != route['output_id'] or actual['inputId'] != route['input_id']):
            raise ValueError(f"Source cable {route['id']} differs from curated route")
    return resolved, [bycable[c['id']] for c in SOURCE_MAP['cables']]

def source_notes(module):
    data = module.get('data', {})
    if not isinstance(data, dict):
        raise ValueError('Notes data must be an object')
    texts = [data['text']] if 'text' in data else []
    if 'content' in module:
        content = module['content']
        if not isinstance(content, dict) or not isinstance(content.get('text'), str):
            raise ValueError('Notes content must contain string text')
        texts.append(content['text'])
    if any(not isinstance(text, str) for text in texts):
        raise ValueError('Notes text must be a string')
    if texts and any(text != texts[0] for text in texts):
        raise ValueError('Conflicting Notes content and data text')
    text = texts[0] if texts else ''
    try:
        text_bytes = text.encode('utf-8')
    except UnicodeEncodeError as error:
        raise ValueError('Notes text must be valid UTF-8') from error
    if len(text_bytes) > NOTES_MAX_UTF8_BYTES:
        raise ValueError(f'Notes text exceeds {NOTES_MAX_UTF8_BYTES} UTF-8 bytes')
    return {'type': 'notes', 'text': text}

C4 = 261.625565

def stock_vcf_frequency_bounds():
    # Match stock_vcf.rs's pinned f32 C4 and rounding after each operation.
    # A double-precision bound would reject the native minimum cutoff.
    def f32(value):
        return struct.unpack('<f', struct.pack('<f', value))[0]
    c4 = f32(261.6256)
    return tuple(f32(f32(f32(math.log2(f32(hz / c4))) + 5) / 10)
                 for hz in (8.0, 22000.0))

def automation_domain(module, parameter_id):
    # Supported reconstruction targets, not an inference from lane order.
    pair = (module['plugin'], module['model'], parameter_id)
    if pair == ('Fundamental', 'VCA-1', 0) or pair == ('SurgeXTRack', 'SurgeXTOSCClassic', 50):
        return 0.0, 1.0
    if pair == ('Fundamental', 'VCF', 0):
        return stock_vcf_frequency_bounds()
    if pair == ('SurgeXTRack', 'SurgeXTVCF', 0):
        # Bounded native approximation range shared with the ladder option.
        return math.log2(20.0 / C4), math.log2(18000.0 / C4)
    raise ValueError(f'Unsupported automation target {pair}')

def validate_automation_points(points, bounds, alias):
    for point in points:
        value = point['value']
        if (isinstance(value, bool) or not isinstance(value, (int, float))
                or not math.isfinite(value) or not bounds[0] <= value <= bounds[1]):
            raise ValueError(f'{alias}: automation value must be finite source raw domain {bounds}')

def native_score(data):
    """Translate saved RackMCP pattern data without changing its value domains."""
    patterns = data['patterns']
    if len(patterns) != 1:
        raise ValueError('This reconstruction supports one saved pattern per sequencer')
    pattern = patterns[0]
    if pattern['name'] != data['selectedPattern']:
        raise ValueError('Selected source pattern is missing')
    curves = {'linear':'linear', 'hold':'hold', 'easeIn':'ease_in',
              'easeOut':'ease_out', 'easeInOut':'ease_in_out'}
    tracks = []
    for track in pattern['tracks']:
        steps = []
        for step in track['steps']:
            if step.get('accent', False):
                raise ValueError('This reconstruction does not translate accent outputs')
            steps.append({'step':step['step'], 'notes':step['notes'],
                          'velocity':step['vel'], 'gate':step['gate'],
                          'probability':step['prob'], 'ratchet':step['ratchet'],
                          'slide':step['slide']})
        tracks.append({'voices':track['voices'], 'glide_ms':track['glideMs'], 'steps':steps})
    lanes = []
    for lane in data['lanes']:
        if lane.get('mode', 'raw') != 'raw':
            raise ValueError('Expected raw-domain source automation')
        lanes.append({'length_bars':lane['lengthBars'], 'looped':lane['loop'],
                      'points':[{'bar':p['bar'], 'value':p['value'], 'curve':curves[p['curve']]}
                                for p in lane['points']]})
    return {'version':1, 'seed':data['seed'], 'length':pattern['length'],
            'steps_per_beat':pattern['stepsPerBeat'], 'tracks':tracks, 'lanes':lanes}

class Builder:
    def __init__(self, source, scores=None, circuit_filters=False):
        self.circuit_filters = circuit_filters
        self.modules, cables = source_modules(source)
        self.source = {**source, 'modules': list(self.modules.values()), 'cables': cables}
        self.byid = {source_id(m['id']): a for a, m in self.modules.items()}
        if scores is None:
            scores = [native_score(self.modules[alias]['data']) for alias in ('seqA', 'seqB')]
        if len(scores) != 2:
            raise ValueError('Expected two native score overrides, seqA then seqB')
        self.nodes, self.cables, self.positions = [], [], {}
        self.ins, self.outs, self.roles = {}, {}, {}
        self.scores = copy.deepcopy(scores)
        self.automation = {}
        for seq, score in zip(('seqA', 'seqB'), self.scores):
            lanes = self.modules[seq]['data']['lanes']
            if len(lanes) != len(score['lanes']):
                raise ValueError(f'{seq}: score lane count differs from source')
            for i, lane in enumerate(lanes):
                target_id = source_id(lane['moduleId'])
                if target_id not in self.byid:
                    raise ValueError(f'{seq}: unknown automation target {target_id}')
                alias = self.byid[target_id]
                if alias in self.automation:
                    raise ValueError(f'{alias}: duplicate automation target')
                if type(lane['paramId']) is not int or lane.get('mode', 'raw') != 'raw':
                    raise ValueError(f'{alias}: expected integer parameter and raw automation domain')
                bounds = automation_domain(self.modules[alias], lane['paramId'])
                validate_automation_points(lane['points'], bounds, alias)
                validate_automation_points(score['lanes'][i]['points'], bounds, alias)
                if self.modules[alias]['model'] == 'VCA-1' and self.modules[alias].get('bypassed', False):
                    raise ValueError(f'{alias}: bypassed automated VCA cannot use downstream level proxy')
                self.automation[alias] = (seq, i, lane['paramId'])
                if lane['paramId'] == 0 and self.modules[alias]['model'] in ('VCF', 'SurgeXTVCF'):
                    fundamental = self.modules[alias]['model'] == 'VCF'
                    for point in score['lanes'][i]['points']:
                        point['value'] = 10 * point['value'] - 5 if fundamental else point['value']
    def p(self, alias, n, default=0):
        params = [p for p in self.modules[alias].get('params', []) if p['id'] == n]
        if len(params) > 1:
            raise ValueError(f'{alias}: duplicate source parameter {n}')
        value = params[0]['value'] if params else default
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
            raise ValueError(f'{alias}: source parameter {n} must be finite numeric')
        return value
    def bounded_source_parameter(self, alias, n, default, low, high, integer=False):
        value = self.p(alias, n, default)
        if not low <= value <= high or (integer and value != int(value)):
            raise ValueError(f'{alias}: source parameter {n} outside supported domain {low}..{high}')
        return value

    def add(self, ident, kind, params=None, score=None, content=None, bypassed=None):
        node = {'id': ident, 'kind': kind, 'params': params or {}}
        if score is not None: node['score'] = score
        if content is not None: node['content'] = content
        if bypassed is not None:
            if not isinstance(bypassed, bool):
                raise ValueError(f'{ident}: source bypassed must be Boolean')
            node['bypassed'] = bypassed
        self.nodes.append(node)
        index = len(self.nodes)-1
        self.positions[ident] = [(index % 10)*300, (index // 10)*420]
        return ident
    def wire(self, out, inp, provenance=None):
        cable = {'id': f'c{len(self.cables)+1:04}', 'from': out[0], 'output': out[1], 'to': inp[0], 'input': inp[1]}
        self.cables.append(cable)
        return cable['id']
    def bind(self, alias, inputs, outputs, nodes, note):
        self.ins[alias], self.outs[alias] = inputs, outputs
        self.roles[alias] = {'source_id': str(self.modules[alias]['id']), 'source_plugin': self.modules[alias]['plugin'],
                             'source_model': self.modules[alias]['model'], 'source_version': self.modules[alias]['version'], 'source_bypassed': self.modules[alias].get('bypassed', False), 'source_content': self.modules[alias].get('content'), 'source_params': self.modules[alias].get('params', []), 'source_data': self.modules[alias].get('data', {}), 'native_ids': nodes, 'substitution': note}
    def automatic_gain(self, alias, outputs):
        if alias not in self.automation: return outputs
        result = {}
        for port, endpoint in outputs.items():
            name = self.add(f'{alias}_arrange{port}', 'gain', {'gain': 1})
            self.wire(endpoint, (name, 'signal'))
            seq, lane, _ = self.automation[alias]
            self.wire((seq+'_lanes', f'out{lane+1}'), (name, 'cv'))
            result[port] = (name, 'out')
        return result
    def create(self):
        for seq, score in zip(('seqA', 'seqB'), self.scores):
            self.add(seq, 'score_sequencer', {'tempo':128, 'swing':54.525020599365234 if seq=='seqA' else 50, 'running':1}, score)
            self.add(seq+'_lanes', 'poly_split')
            self.wire((seq,'automation'), (seq+'_lanes','signal'))
            names = [f'pitch{i}' for i in range(1,5)] + [f'gate{i}' for i in range(1,5)] + [f'velocity{i}' for i in range(1,5)]
            self.bind(seq, {0:(seq,'clock'),1:(seq,'reset')}, {**{i:(seq,n) for i,n in enumerate(names)},16:(seq,'clock'),17:(seq,'reset')}, [seq,seq+'_lanes'], 'Live polyphonic score and source-domain automation; saved pattern/lane lengths preserved with native timing')
        for alias,m in self.modules.items():
            kind=m['model']
            if alias in ('seqA','seqB','main_mix'): continue
            if kind == 'Bridge':
                self.bind(alias,{}, {}, [], 'Host bridge retained in manifest; native MCP controls the reconstruction')
            elif kind == 'Notes':
                self.add(alias, 'notes', content=source_notes(m), bypassed=m.get('bypassed'))
                self.bind(alias, {}, {}, [alias], 'Native Notes preserves source Unicode text; no DSP')
            elif kind=='AudioInterface2':
                self.add(alias,'audio_stereo',{'dc_block':1 if m.get('data',{}).get('dcFilter',False) else 0});self.bind(alias,{0:(alias,'left'),1:(alias,'right')},{},[alias],'Native stereo device sink with source10Hz DC-filter setting; source savedgain0 overridden for audition')
            elif kind=='Listen':
                ids=[self.add(alias+s,'gain',{'gain':1}) for s in ('L','R')]
                self.bind(alias,{i:(n,'signal') for i,n in enumerate(ids)},{i:(n,'out') for i,n in enumerate(ids)},ids,'Unity monitoring pass-through; native MCP rendering replaces source Listen capture')
            elif kind=='VCA-1':
                raw_level=self.bounded_source_parameter(alias,0,1,0,1)
                response=self.bounded_source_parameter(alias,1,1,0,1,integer=True)
                self.add(alias, 'stock_vca', {'level': 1 if alias in self.automation else raw_level,
                         'response': response}, bypassed=m.get('bypassed'))
                outputs=self.automatic_gain(alias,{0:(alias,'out')})
                note='Qualified stock VCA-1: ideal multiplication with source response switch and CV normalization'
                if alias in self.automation:
                    note += '; raw automated level multiplies output through a downstream gain proxy; the stock gain meter excludes that automated multiplier'
                self.bind(alias,{0:(alias,'cv'),1:(alias,'signal')},outputs,[alias],note)
            elif kind in ('Kickall','Clap','HiHat'):
                mode={'Kickall':0,'Clap':1,'HiHat':2}[kind]
                self.add(alias,'drum_voice',{'mode':mode,'pitch':48 if mode==0 else 1800 if mode==1 else 2000,'decay':.32 if mode==0 else .1 if mode==1 else .01,'level':1,'tone':.2 if mode==0 else .7 if mode==1 else .9,'drive':.25 if mode==0 else 0})
                self.bind(alias,{0:(alias,'trigger')},{0:(alias,'out')},[alias],'Original synthesized drum; source raw controls retained in manifest, not Kickall/XOX parity')
            elif kind=='SurgeXTWaveshaper':
                self.add(alias,'gain',{'gain':2.02});self.bind(alias,{0:(alias,'signal')},{0:(alias,'out')},[alias],'Drive moved into original kick primitive; source soft-shaper gain approximated')
            elif kind=='VCO':
                self.add(alias,'multi_vco',{'frequency':C4});self.bind(alias,{0:(alias,'pitch')},{i:(alias,n) for i,n in enumerate(('sine','triangle','saw','square'))},[alias],'Native four-wave oscillator; saw selected as source')
            elif kind=='SurgeXTOSCClassic':
                count=4 if alias=='lead_osc' else 2
                ids=[];mix=[]
                for i in range(count):
                    detune=(i-(count-1)/2)*(.12 if count==4 else .1)
                    ident=self.add(f'{alias}_{i+1}','multi_vco',{'frequency':C4*2**(detune/12), 'fm_amount': .01*(i-(count-1)/2) if alias=='pad_osc' else 0})
                    ids.append(ident)
                for side in ('L','R'):
                    ident=self.add(alias+side,'gain',{'gain':1/count});mix.append(ident)
                    for i,osc in enumerate(ids):
                        weight=self.add(f'{alias}_{side}{i+1}','gain',{'gain':1 if (i%2==0)==(side=='L') else .7})
                        self.wire((osc,'saw'),(weight,'signal'));self.wire((weight,'out'),(ident,'signal'))
                outputs={i:(n,'out') for i,n in enumerate(mix)}
                if alias in ('stab_osc','pad_osc'):
                    # Source Surge left-only jack averages its stereo oscillator.
                    mono=self.add(alias+'_mono','gain',{'gain':.5})
                    for stereo in mix: self.wire((stereo,'out'),(mono,'signal'))
                    outputs[0]=(mono,'out')
                    ids.append(mono)
                outputs=self.automatic_gain(alias,outputs)
                self.bind(alias,{0:[(n,'pitch') for n in ids if n != alias+'_mono'],2:[(n,'fm') for n in ids if n != alias+'_mono']},outputs,ids+mix,'Detuned native saw unison;4lead/2stab/2pad; stab/pad left-only output averages L/R as source; original Surge waveform/character differ; pad opposing tinyFM approximates detune modulation, lead modulation remainszero')
            elif kind=='VCF':
                automated=alias in self.automation
                frequency_bounds=automation_domain(m,0)
                params={name:self.bounded_source_parameter(alias,index,default,low,high)
                        for name,index,default,low,high in (
                    ('frequency',0,.5,*frequency_bounds),('resonance',2,0,0,1),('cutoff_cv_amount',3,0,-1,1),
                    ('resonance_cv_amount',5,0,-1,1),('drive',4,0,-1,1),('drive_cv_amount',6,0,-1,1))}
                inputs={i:(alias,name) for i,name in enumerate(('cutoff_cv','resonance_cv','drive_cv','signal'))}
                ids=[alias]
                if automated:
                    # Native cutoff pitch = 10*frequency-5 + CV*amount.
                    # The transformed raw lane and original depth-scaled CV sum.
                    depth=params['cutoff_cv_amount']
                    params.update(frequency=.5,cutoff_cv_amount=1)
                    cv=self.add(alias+'_mod','gain',{'gain':depth});ids.append(cv)
                    self.wire((cv,'out'),(alias,'cutoff_cv'))
                    inputs[0]=(cv,'signal')
                    seq,lane,_=self.automation[alias]
                    self.wire((seq+'_lanes',f'out{lane+1}'),(alias,'cutoff_cv'))
                self.add(alias,'stock_vcf',params,bypassed=m.get('bypassed'))
                self.bind(alias,inputs,{0:(alias,'lowpass'),1:(alias,'highpass')},ids,
                    'Qualified stock Fundamental VCF, raw six controls and both LP/HP outputs; automated raw frequency is 10*raw-5 V via CV plus independent original cutoff CV depth' if automated else
                    'Qualified stock Fundamental VCF, raw six controls, all four inputs and both LP/HP outputs')
            elif kind=='SurgeXTVCF':
                is_svf=alias=='riser_filter'
                sides=('L','R') if alias=='lead_filter' else ('',)
                ids=[];ins={};outs={}
                for i,side in enumerate(sides):
                    ident=alias+side;ids.append(ident)
                    cutoff=C4 if alias in self.automation else C4*2**self.p(alias,0)
                    use_svf=is_svf or not self.circuit_filters
                    self.add(ident,'svf_filter' if use_svf else 'ladder_vcf',{'cutoff':cutoff,'resonance':self.p(alias,1),'drive':.1,'cutoff_cv_amount':1} if use_svf else {'cutoff':cutoff,'resonance':self.p(alias,1),'drive':1})
                    ins[i]=(ident,'signal')
                    output=(ident,'band' if alias=='riser_filter' else 'high' if is_svf else 'low' if use_svf else 'out')
                    stages=[ident]
                    if not is_svf and not self.circuit_filters:
                        second=self.add(ident+'_pole34','svf_filter',{'cutoff':cutoff,'resonance':0,'drive':0,'cutoff_cv_amount':1})
                        ids.append(second);stages.append(second)
                        self.wire(output,(second,'signal'));output=(second,'low')
                    outs[i]=output
                    if alias in self.automation:
                        seq,lane,_=self.automation[alias]
                        for stage in stages:self.wire((seq+'_lanes',f'out{lane+1}'),(stage,'cutoff_cv'))
                # Original cutoff modulation is an independent additive octave signal.
                depth=self.p(alias,5)
                if depth:
                    cv=self.add(alias+'_mod','gain',{'gain':depth});ids.append(cv)
                    for ident in ids[:-1]: self.wire((cv,'out'),(ident,'cutoff_cv'))
                    ins[2]=(cv,'signal')
                self.bind(alias,ins,outs,ids,'Original native TPT bandpass approximation for Surge XT; not source-identical' if is_svf else 'Native nonlinear ladder lowpass approximation; raw cutoff automation converted affinely to octaves' if self.circuit_filters else 'Original four-pole lowpass approximation: two cascaded TPT SVFs, shared cutoff/CV, resonance and drive only in first stage; not source-identical')
            elif kind=='ADSR':
                if self.p(alias,8,0) != 0:
                    raise ValueError(f'{alias}: saved held ADSR PUSH is unsupported')
                stages=('attack','decay','sustain','release')
                params={name:self.bounded_source_parameter(alias,i,.5,0,1) for i,name in enumerate(stages)}
                params.update({name+'_amount':self.bounded_source_parameter(alias,i+4,0,-1,1) for i,name in enumerate(stages)})
                self.add(alias,'adsr_eg',params,bypassed=m.get('bypassed'))
                self.bind(alias,{i:(alias,name) for i,name in enumerate(('attack_cv','decay_cv','sustain_cv','release_cv','gate','retrigger'))},
                    {0:(alias,'out')},[alias],'Qualified stock digital ADSR, raw normalized A/D/S/R and signed CV depths; native gate/retrigger routes preserved')
            elif kind=='Bogaudio-Offset':
                self.add(alias,'attenuverter',{'gain':-.95,'offset':10});self.bind(alias,{2:(alias,'signal')},{0:(alias,'out')},[alias],'Exact source duck formula10−.95×envelope')
            elif kind=='VCMixer':
                self.add(alias,'vca_mix',{'mix_level':self.p(alias,0),'channel_exp':0,'mix_exp':0,**{f'level{i}':self.p(alias,i) for i in range(1,5)}})
                self.bind(alias,{i:(alias,f'in{i}') for i in range(1,5)},{0:(alias,'mix')},[alias],'Squared strip gains and linear master preserved')
            elif kind in ('SurgeXTDelay','Plateau','Bogaudio-Pressor'):
                if kind=='SurgeXTDelay': native='stereo_delay';params={'time_left':.234375,'time_right':.46875,'feedback':.15,'crossfeed':.5,'dry':.6,'wet':.4,'low_cut':300,'high_cut':8000,'mod_rate':4,'mod_depth':.399857}
                elif kind=='Plateau': native='stereo_reverb';params={'dry':1,'wet':self.p(alias,1),'predelay':self.p(alias,2),'size':self.p(alias,5),'decay':self.p(alias,7),'damping':8000}
                else: native='sidechain_compressor';params={'threshold_db':30*self.p(alias,0)-24,'ratio':4 if alias=='lead_duck' else 2.29169,'attack_ms':2,'release_ms':200,'knee_db':6,'makeup_db':0,'detector_ms':10}
                self.add(alias,native,params)
                left_sum=self.add(alias+'_monoL','poly_sum');right_sum=self.add(alias+'_monoR','poly_sum')
                self.wire((left_sum,'out'),(alias,'left'));self.wire((right_sum,'out'),(alias,'right'))
                self.bind(alias,{0:(left_sum,'signal'),(4 if kind=='Bogaudio-Pressor' else 1):(right_sum,'signal'),**({1:(alias,'sidechain')} if kind=='Bogaudio-Pressor' else {})},{(1 if kind=='Bogaudio-Pressor' else 0):(alias,'left'),(2 if kind=='Bogaudio-Pressor' else 1):(alias,'right')},[alias,left_sum,right_sum],'Original native stereo effect; polyphonic channels explicitly summed beforemono stereo DSP; source timing/mix/sidechain roles preserved, not sourceDSP parity')
            elif kind=='Noise':
                self.add(alias,'white_noise');self.bind(alias,{}, {0:(alias,'out')},[alias],'Native deterministic white noise')
            elif kind in ('LFO','LFO2'):
                self.add(alias,'multi_lfo',{'frequency':2**self.p(alias,2),'unipolar':self.p(alias,0),'invert':self.p(alias,1)})
                self.bind(alias,{}, {0:(alias,'sine')},[alias],'Native sine; LFO2 Basic.wav approximated' if kind=='LFO2' else 'Native sine at source frequency')
            else: raise ValueError(f'Unmapped source model {kind}')
        self.main_mix()
        translated=[]
        for c in self.source['cables']:
            a,b=self.byid[str(c['outputModuleId'])],self.byid[str(c['inputModuleId'])]
            out=self.outs[a].get(c['outputId']);inp=self.ins[b].get(c['inputId']);ids=[]
            if out is not None and inp is not None:
                for target in inp if isinstance(inp,list) else [inp]: ids.append(self.wire(out,target))
            translated.append({'source_id':str(c['id']),'from':a,'output_id':c['outputId'],'to':b,'input_id':c['inputId'],'native_cable_ids':ids,'gap':None if ids else 'Source cable could not be translated'})
        if any(not item['native_cable_ids'] for item in translated):
            raise ValueError('Unmapped source cable: ' + str([item for item in translated if not item['native_cable_ids']]))
        for alias, role in self.roles.items():
            assigned = [n['id'] for n in self.nodes if n['id'].startswith(alias) and not any(n['id'].startswith(other) for other in self.roles if other != alias and len(other) > len(alias))]
            role['native_ids'] = list(dict.fromkeys(role['native_ids'] + assigned))
            if alias in self.automation:
                seq, lane, param = self.automation[alias]
                role['automation'] = {'sequencer': seq, 'lane': lane, 'source_param': param, 'conversion': '10*raw-5 octaves relative C4' if self.modules[alias]['model']=='VCF' else 'raw octaves relative C4' if self.modules[alias]['model']=='SurgeXTVCF' else 'raw linear gain'}
        self.layout()
        return {'version':2,'nodes':self.nodes,'cables':self.cables,'output':'audio'},translated
    def layout(self):
        # Rack dimensions in HP,15px/HP; each strip occupies its own420px row.
        widths={'score_sequencer':40,'stereo_delay':28,'sidechain_compressor':28,
                'stereo_reverb':24,'drum_voice':24,'svf_filter':18,'vca_mix':26,
                'rescale':16,'audio_stereo':12,'multi_lfo':20,'multi_vco':20,
                'poly_split':20,'poly_sum':10,'white_noise':12,'attenuverter':12,
                'adsr':16,'adsr_eg':24,'stock_vcf':18,'stock_vca':12,'notes':16,'ladder_vcf':14,'gain':10}
        groups=[[],[],[],[],[],[],[]]
        for node in self.nodes:
            ident=node['id']
            group=(0 if ident.startswith('seq') else 1 if ident.startswith(('kick','clap','hat'))
                   else 2 if ident.startswith(('bass','duck')) else 3 if ident.startswith('lead')
                   else 4 if ident.startswith(('pad','stab','texture')) else 5 if ident.startswith(('riser','noise','atmos')) else 6)
            groups[group].append(node)
        row=0
        for group in groups:
            x=0
            for node in group:
                width=15*widths.get(node['kind'],12)
                if x+width>4200:x=0;row+=1
                self.positions[node['id']]=[x,row*420]
                x+=width+15
            row+=1
    def main_mix(self):
        ids=[];inputs={}
        for side in ('L','R'):
            self.add('main_mix'+side,'gain',{'gain':10**((66*self.p('main_mix',24)-60)/20)})
            self.add('audition'+side,'gain',{'gain':1});self.wire(('main_mix'+side,'out'),('audition'+side,'signal'))
            ids.extend(['main_mix'+side,'audition'+side])
        for ch in range(8):
            mono=self.add(f'mix_ch{ch+1}','poly_sum');inputs[3*ch+2]=(mono,'signal');ids.append(mono)
            gain=10**((66*self.p('main_mix',3*ch)-60)/20);pan=self.p('main_mix',3*ch+2)
            for side,factor in [('L',math.cos((pan+1)*math.pi/4)),('R',math.sin((pan+1)*math.pi/4))]:
                n=self.add(f'mix_ch{ch+1}{side}','gain',{'gain':gain*factor});ids.append(n)
                self.wire((mono,'out'),(n,'signal'));self.wire((n,'out'),('main_mix'+side,'signal'))
        self.bind('main_mix',inputs,{0:('auditionL','out'),1:('auditionR','out')},ids,'Source dB gains and sine/cosine equal-power pan preserved; polyphonic channels summed; source mixer slew/saturation differ')

def tool(client,name,args):
    response=client.request('tools/call',{'name':name,'arguments':args})
    if 'error' in response or response.get('result',{}).get('isError'):
        raise RuntimeError(f'{name}: {response}')
    return response['result']

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--source',type=Path,required=True)
    ap.add_argument('--score-a',type=Path,help='seqA native score JSON with unconverted source-raw automation values')
    ap.add_argument('--score-b',type=Path,help='seqB native score JSON with unconverted source-raw automation values')
    ap.add_argument('--circuit-filters',action='store_true',help='Use nonlinear ladder approximations only for Surge XT lowpasses; Fundamental VCFs always use qualified stock_vcf (higher CPU cost)')
    ap.add_argument('--output',type=Path,required=True);ap.add_argument('--binary',type=Path,default=Path(__file__).resolve().parents[1]/'target'/'release'/('openpatch.exe' if os.name == 'nt' else 'openpatch'))
    args=ap.parse_args();source=json.loads(args.source.read_text(encoding='utf-8'))
    if bool(args.score_a) != bool(args.score_b):ap.error('Provide both score overrides or neither')
    scores = ([json.loads(p.read_text(encoding='utf-8')) for p in (args.score_a,args.score_b)] if args.score_a
              else None)
    builder=Builder(source,scores,args.circuit_filters);patch,translations=builder.create();args.output.mkdir(parents=True,exist_ok=True)
    for filename in ('stiw-native.json','stiw-native.openpatch','substitution-manifest.json'):
        if (args.output/filename).exists():raise FileExistsError(f'Refusing to overwrite {args.output/filename}')
    manifest={'source_name':args.source.name,'source_sha256':hashlib.sha256(args.source.read_bytes()).hexdigest(),
              'identity_map':'scripts/stiw_source_map.json','curated_original_sha256':SOURCE_MAP['source_sha256'],
              'filter_mode':'qualified-fundamental-stock; surge-circuit-ladder' if args.circuit_filters else 'qualified-fundamental-stock; surge-four-pole-svf',
              'source_modules':55,'source_cables':83,'native_modules':len(patch['nodes']),'native_cables':len(patch['cables']),
              'roles':builder.roles,'cable_translations':translations,'qualified_stock_references':REFERENCES,
              'audition_changes':['Transport running from reset origin','Core saved zero device gain replaced by auditionL/R gain 1; arrangement mutes preserved'],
              'limitations':['Whole patch is a native rendition, not bit-identical source/plugin playback',
                  'Original drums and stereo effects are substitutes, not plugin-identical DSP',
                  'Detuned saw unison replaces Surge Classic; sine replaces LFO2 wavetable',
                  'Surge XT lowpasses use nonlinear ladder approximations' if args.circuit_filters else 'Surge XT lowpasses use cascaded TPT SVF approximations',
                  'Surge riser bandpass and stereo lead filter roles remain native approximations; Fundamental VCFs use stock_vcf',
                  'One sample of delay per native cable; automation proxies add cable/stage latency compared with direct source parameter automation',
                  'Automated VCA level uses a downstream gain multiplier; stock VCA gain telemetry excludes that multiplier',
                  'Mix8 slew/saturation and source sequencer interpolation/timing differ; see docs/SCORE_AND_STUDIO.md',
                  'Stock VCF deterministic module-local RNG differs from Rack process-global RNG; see docs/STOCK_VCF.md']}
    client=MCPProcess(args.binary.resolve(),args.output.resolve())
    try:
        initialized=client.request('initialize',{'protocolVersion':PROTOCOL_VERSION,'capabilities':{},'clientInfo':{'name':'stiw-rebuilder','version':'1'}})
        if 'error' in initialized:raise RuntimeError(f'MCP initialize: {initialized}')
        client.notify('notifications/initialized')
        tool(client,'replace_patch',{'patch':patch})
        tool(client,'save_patch',{'filename':'stiw-native.json'})
    finally: client.close()
    saved_patch=json.loads((args.output/'stiw-native.json').read_text(encoding='utf-8'))
    project={'format':'openpatch-project','version':1,'patch':saved_patch,'layout':{'version':1,'zoom':.55,'pan':[24,24],'positions':builder.positions}}
    (args.output/'stiw-native.openpatch').write_text(json.dumps(project,indent=2)+'\n',encoding='utf-8')
    (args.output/'substitution-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
    print(f"Saved live reconstruction: {len(patch['nodes'])} modules / {len(patch['cables'])} cables; all83 source cables accounted in manifest")
if __name__=='__main__': main()
