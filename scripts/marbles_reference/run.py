#!/usr/bin/env python3
"""Root-run genuine Marbles reference. Never substitutes generated expected DSP."""
import argparse,copy,difflib,hashlib,json,math,os,re,shutil,struct,subprocess,sys
from pathlib import Path
from fetch import HERE,ROOT,DEFAULT_CACHE,fetch

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def run(command,directory,label,stdin=None,env=None):
 (directory/(label+'-command.json')).write_text(json.dumps(command,indent=2)+'\n')
 with (directory/(label+'.stdout')).open('wb') as out,(directory/(label+'.stderr')).open('wb') as err:
  return subprocess.run(command,input=stdin,stdout=out,stderr=err,env=env).returncode

def presets(attempt):
 source=(DEFAULT_CACHE/'marbles/settings.cc').read_text();start=source.index('const Scale preset_scales[6] = {');end=source.index('\n};',start)+3
 literal=source[start:end];header='// Mechanically extracted pinned settings.cc literal; source hash in manifest.\nnamespace marbles {\n'+literal+'\n}\n'
 (attempt/'preset_scales.h').write_text(header)
 clean=re.sub(r'//[^\n]*','',literal[literal.index('=')+1:].strip().rstrip(';'));clean=re.sub(r'(?<=[0-9])f\b','',clean);values=__import__('ast').literal_eval(clean.replace('{','[').replace('}',']'))
 assert len(values)==6
 out='// Mechanically extracted source settings.cc scalars; no DSP.\nfn preset_scales()->[kernel::quantizer::Scale;6] {\nuse kernel::quantizer::{Scale,Degree};\n[\n'
 for interval,count,degrees in values:
  assert count==len(degrees) and max(d[1] for d in degrees)==255
  bits=lambda v:struct.unpack('<I',struct.pack('<f',v))[0]
  ds=[f'Degree{{voltage:f32::from_bits({bits(v)}),weight:{w}}}' for v,w in degrees]+['Degree{voltage:0.0,weight:0}']*(16-count)
  out+=f'Scale{{base_interval:f32::from_bits({bits(interval)}),num_degrees:{count},degree:['+','.join(ds)+']},\n'
 out+=']\n}\n';(attempt/'preset_scales.rs').write_text(out)

def group(g):return [g[k] for k in ['control','range','register','value','spread','bias','steps','deja','scale','length','p','q']]
def flags(clock,frame):
 def high(f):return f>=clock['start'] and (f-clock['start'])%clock['period']<clock['high']
 h,p=high(frame),high(frame-1) if frame else False
 return int(h)|(2 if h and not p else 4 if p and not h else 0)
def encode(case):
 lines=[f"{case['mode']} {case['seed']}"]
 if case['mode'] in [1,2,5]:return ('\n'.join(lines)+'\n').encode()
 if case['mode'] in [3,4]:return ('\n'.join(lines+[str(len(case['operations']))]+[' '.join(map(str,op)) for op in case['operations']])+'\n').encode()
 c=copy.deepcopy(case);assert c['sample_rate']==32000 and 0<c['blocks']<=20000
 lines.append(f"32000 {c['blocks']} {int(c['load_y'])}");events={e['block']:e for e in c['events']}
 for block in range(c['blocks']):
  e=events.get(block,{})
  for key in ['t','x','y']:c[key].update(e.get(key,{}))
  for key in ['clock_source','t_clock','x_clock']:
   if key in e:c[key]=e[key]
  t=c['t'];v=[t[k] for k in ['model','range','rate','bias','jitter','pw','pwstd','deja','length','external']]+[e.get('reset_t',False),e.get('reset_x',False),c['clock_source']]+group(c['x'])+group(c['y'])
  v += [flags(c['t_clock'],block*5+i) for i in range(5)]+[flags(c['x_clock'],block*5+i) for i in range(5)]
  lines.append(' '.join(str(int(x)) if isinstance(x,bool) else str(x) for x in v))
 return ('\n'.join(lines)+'\n').encode()

def validate(document):
 assert document['schema']==1
 identifiers=set()
 def normalized(v):assert isinstance(v,(int,float)) and math.isfinite(v) and 0<=v<=1
 def config(c):
  t=c['t'];assert t['model'] in range(7) and t['range'] in range(3)
  assert math.isfinite(t['rate']) and -96<=t['rate']<=96
  for key in ['bias','jitter','pw','pwstd','deja']:normalized(t[key])
  assert type(t['length']) is int and 1<=t['length']<=16 and type(t['external']) is bool
  for key in ['x','y']:
   g=c[key];assert g['control'] in range(3) and g['range'] in range(3) and type(g['register']) is bool
   for name in ['value','spread','bias','steps','deja']:normalized(g[name])
   assert type(g['length']) is int and 1<=g['length']<=16 and g['scale'] in range(6)
   assert type(g['p']) is int and type(g['q']) is int and 1<=g['p']<=64 and 1<=g['q']<=64
  assert c['clock_source'] in range(5)
  for key in ['t_clock','x_clock']:
   g=c[key];assert all(type(g[n]) is int for n in ['period','high','start'])
   assert 1<=g['high']<g['period'] and g['start']>=1
 for c in document['cases']+document['negative_cases']:
  assert re.fullmatch('[a-z0-9_]+',c['id']) and c['id'] not in identifiers;identifiers.add(c['id'])
  assert type(c['seed']) is int and 0<=c['seed']<=0xffffffff
  if c in document['negative_cases']:assert c['mode'] in [1,2,5];continue
  assert c['mode'] in [0,3,4]
  if c['mode'] in [3,4]:
   assert 1<=len(c['operations'])<=10000
   for op in c['operations']:
    if c['mode']==3:
     assert len(op)==5 and type(op[0]) is int and 0<=op[0]<=7 and 1<=op[1]<=16
     normalized(op[2]);normalized(op[3]);assert type(op[4]) is int and 0<=op[4]<=0xffffffff
    else:assert len(op)==2 and op[0] in range(4) and math.isfinite(op[1]) and -5<=op[1]<=5
   continue
  assert c['sample_rate']==32000 and type(c['blocks']) is int and 1<=c['blocks']<=20000 and type(c['load_y']) is bool
  config(c);updated=copy.deepcopy(c);blocks=set()
  for event in c['events']:
   assert type(event['block']) is int and 0<=event['block']<c['blocks'] and event['block'] not in blocks;blocks.add(event['block'])
   assert set(event)<=set(['block','t','x','y','clock_source','t_clock','x_clock','reset_t','reset_x'])
   for key in ['reset_t','reset_x']:
    if key in event:assert type(event[key]) is bool
   for key in ['t','x','y']:updated[key].update(event.get(key,{}))
   for key in ['clock_source','t_clock','x_clock']:
    if key in event:updated[key]=event[key]
   config(updated)

def main():
 ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--cases',type=Path,default=HERE/'cases.json');ap.add_argument('--compile-only',action='store_true');ap.add_argument('--extensions-only',action='store_true');ap.add_argument('--validate-only',action='store_true');args=ap.parse_args()
 document=json.loads(args.cases.read_text());validate(document)
 if args.validate_only:print(json.dumps({'valid':True,'cases':len(document['cases']),'negative_cases':len(document['negative_cases']),'dsp_executed':False}));return 0
 manifest=fetch(DEFAULT_CACHE);base=ROOT/'native-test-output/marbles-reference';base.mkdir(exist_ok=True)
 attempt=next(base/f'attempt-{n:03d}' for n in range(1,1000) if not(base/f'attempt-{n:03d}').exists());attempt.mkdir();print(attempt,flush=True)
 for name in ['source_manifest.json','reference_probe.cpp','rust_probe.rs','run.py','fetch.py']:
  shutil.copyfile(HERE/name,attempt/name)
 shutil.copyfile(args.cases,attempt/'cases.json');presets(attempt)
 kernel=ROOT/'crates/openpatch-core/src/recall_random/kernel';shutil.copytree(kernel,attempt/'production-kernel')
 # Paths in the retained Rust probe resolve only to the retained production copy.
 p=attempt/'rust_probe.rs';p.write_text(p.read_text().replace('../../crates/openpatch-core/src/recall_random/kernel/mod.rs','production-kernel/mod.rs'))
 result={'passed':False,'scope':'bounded source kernels, explicit host setup and source-negative boundaries; not native application/hardware','production_equivalence':False,'stage':'compile','cases':0,'records':0,'case_results':[]}
 def save(): (attempt/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 save();compiler=shutil.which('clang++');rustc=shutil.which('rustc');assert compiler and rustc
 (attempt/'compiler.txt').write_bytes(subprocess.check_output([compiler,'--version']));(attempt/'rustc.txt').write_bytes(subprocess.check_output([rustc,'-vV']))
 sources=[str(DEFAULT_CACHE/x) for x in manifest['compiled_sources']]
 platform_flags=[]
 if sys.platform=='darwin':
  sdk=Path(subprocess.check_output(['xcrun','--show-sdk-path'],text=True).strip());platform_flags=['-isysroot',str(sdk),'-isystem',str(sdk/'usr/include/c++/v1')]
 common=[compiler,'-std=c++14','-DTEST','-O1','-g','-ffp-contract=off',*platform_flags,'-I'+str(DEFAULT_CACHE),'-I'+str(attempt),str(attempt/'reference_probe.cpp'),*sources]
 sanitizer=['-fsanitize=address,undefined,float-cast-overflow,bounds','-fno-sanitize-recover=all','-fno-omit-frame-pointer']
 if run(common+sanitizer+['-o',str(attempt/'original_sanitized')],attempt,'original-sanitizer-compile'):save();return 1
 # Audited host correction for a genuine source endpoint read, preserved by
 # original_sanitized above. Never overwrite originals or disable checks.
 original=(DEFAULT_CACHE/'stmlib/dsp/dsp.h').read_text()
 body='''inline float Interpolate(const float* table, float index, float size) {
  index *= size;
  MAKE_INTEGRAL_FRACTIONAL(index)
  float a = table[index_integral];
  float b = table[index_integral + 1];
  return a + (b - a) * index_fractional;
}'''
 corrected=body.replace('float b = table[index_integral + 1];','float b = index_fractional == 0.0f && index_integral == static_cast<int32_t>(size)\n      ? a : table[index_integral + 1];')
 assert original.count(body)==1
 overlay=attempt/'host-overlay/stmlib/dsp/dsp.h';overlay.parent.mkdir(parents=True);overlay.write_text(original.replace(body,corrected))
 (attempt/'host-overlay.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True),overlay.read_text().splitlines(True),fromfile='original/stmlib/dsp/dsp.h',tofile='native-terminal-guard/stmlib/dsp/dsp.h')))
 result['terminal_guard']={'original_sha256':sha(DEFAULT_CACHE/'stmlib/dsp/dsp.h'),'overlay_sha256':sha(overlay),'rule':'At exact interpolation terminal knot only, repeat a instead of reading the unavailable next sentinel; preserve all arithmetic and every other original file. Original sanitizer remains a separate required audit.'}
 guarded=common[:1]+['-I'+str(attempt/'host-overlay')]+common[1:]
 for label,extra,out in [('compile',[],attempt/'reference_probe'),('sanitizer-compile',sanitizer,attempt/'reference_probe_sanitized')]:
  if run(guarded+extra+['-o',str(out)],attempt,label):save();return 1
 if run([rustc,'--edition=2021','-C','opt-level=1','-C','overflow-checks=on',str(p),'-o',str(attempt/'rust_probe')],attempt,'rust-compile'):save();return 1
 if args.compile_only:result['stage']='compile_pass';save();print('compile PASS (no DSP run)',flush=True);return 0
 result['stage']='compare'
 for case in ([] if args.extensions_only else document['cases']):
  d=attempt/case['id'];d.mkdir();data=encode(case);(d/'input.txt').write_bytes(data)
  for label,binary in [('source','reference_probe'),('sanitizer','reference_probe_sanitized'),('rust','rust_probe')]:
   code=run([str(attempt/binary)],d,label,data,env={**os.environ,'ASAN_OPTIONS':'detect_leaks=0','UBSAN_OPTIONS':'halt_on_error=1'})
   if code:result.update(stage=label+'_failure',failed_case=case['id'],exit_code=code);save();print(case['id'],label,'FAIL',flush=True);return 1
  a=(d/'source.stdout').read_bytes()
  for label in ['sanitizer','rust']:
   b=(d/(label+'.stdout')).read_bytes()
   if a!=b:
    first=next((i for i,(x,y) in enumerate(zip(a.splitlines(),b.splitlines())) if x!=y),min(len(a.splitlines()),len(b.splitlines())))
    result.update(stage=label+'_mismatch',failed_case=case['id'],first_mismatch=first);save();print(case['id'],label,'MISMATCH line',first,flush=True);return 1
  original_code=run([str(attempt/'original_sanitized')],d,'original-sanitizer',data,env={**os.environ,'ASAN_OPTIONS':'detect_leaks=0','UBSAN_OPTIONS':'halt_on_error=1'})
  original_trace=(d/'original-sanitizer.stdout').read_bytes();original_lines=original_trace.splitlines()
  if original_code:
   diagnostic=(d/'original-sanitizer.stderr').read_text()
   if not ('global-buffer-overflow' in diagnostic and 'lut_raised_cosine' in diagnostic and 'Interpolate' in diagnostic):
    result.update(stage='unclassified_original_source_failure',failed_case=case['id']);save();return 1
   # ASan abort can leave a partial last line. Compare every complete record
   # from before the original undefined read; the failed full trace is kept.
   if original_lines and len(original_lines[-1].split())!=15:original_lines.pop()
   if original_lines!=a.splitlines()[:len(original_lines)]:result.update(stage='original_defined_prefix_mismatch',failed_case=case['id']);save();return 1
  elif original_trace!=a:result.update(stage='original_defined_trace_mismatch',failed_case=case['id']);save();return 1
  count=len(a.splitlines());result['case_results'].append({'id':case['id'],'records':count,'sha256':hashlib.sha256(a).hexdigest(),'native_y_scale_setup':bool(case.get('load_y')),'original_sanitizer_exit':original_code,'original_defined_records':len(original_lines),'original_terminal_read_negative':original_code!=0});result['cases']+=1;result['records']+=count;save();print(case['id'],count,'PASS',('original terminal negative retained' if original_code else 'original defined'),flush=True)
 result['negative_cases']=[]
 for case in document['negative_cases']:
  d=attempt/case['id'];d.mkdir();data=encode(case);(d/'input.txt').write_bytes(data)
  code=run([str(attempt/'original_sanitized')],d,'sanitizer',data,env={**os.environ,'ASAN_OPTIONS':'detect_leaks=0','UBSAN_OPTIONS':'halt_on_error=1'})
  result['negative_cases'].append({'id':case['id'],'sanitizer_exit':code});
  diagnostic=(d/'sanitizer.stderr').read_text()
  expected={1:('outside the range of representable values' in diagnostic and 'random_sequence.h' in diagnostic),2:('index 255 out of bounds' in diagnostic and 'quantizer.cc' in diagnostic),5:('global-buffer-overflow' in diagnostic and 'lut_raised_cosine' in diagnostic and 'Interpolate' in diagnostic)}[case['mode']]
  if code==0 or not expected:result.update(stage='negative_not_rejected_as_expected',failed_case=case['id']);save();return 1
  native=run([str(attempt/'rust_probe')],d,'native-extension',data)
  word=(((0xffffffff ^ 1)*1664525+1013904223)&0xffffffff)
  expected_value={1:word/4294967296.0,2:0.2,5:1.0}[case['mode']]
  expected_bits=struct.unpack('<I',struct.pack('<f',expected_value))[0]
  actual=(d/'native-extension.stdout').read_text().strip()
  if native or actual!=str(expected_bits):result.update(stage='native_extension_failure',failed_case=case['id'],expected_bits=expected_bits,actual=actual);save();return 1
  result['negative_cases'][-1]['native_extension_bits']=expected_bits
 result.update(passed=True,stage='pass');save();(attempt/'artifact-sha256.json').write_text(json.dumps({str(x.relative_to(attempt)):sha(x) for x in attempt.rglob('*') if x.is_file() and x.name!='artifact-sha256.json'},indent=2)+'\n');print(json.dumps({k:result[k] for k in ['passed','cases','records']}),flush=True);return 0
if __name__=='__main__':raise SystemExit(main())
