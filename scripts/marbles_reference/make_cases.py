#!/usr/bin/env python3
"""Author source inputs only; does not execute or calculate expected DSP."""
import copy,json
from pathlib import Path
HERE=Path(__file__).resolve().parent

def base(name):
 return {'id':name,'mode':0,'seed':1,'sample_rate':32000,'blocks':2400,'load_y':False,
 't':{'model':0,'range':2,'rate':48,'bias':.5,'jitter':.2,'pw':.35,'pwstd':.2,'deja':0,'length':8,'external':False},
 'x':{'control':0,'range':2,'register':False,'value':.5,'spread':.5,'bias':.5,'steps':.5,'deja':0,'scale':0,'length':8,'p':1,'q':1},
 'y':{'control':0,'range':2,'register':False,'value':0,'spread':.5,'bias':.5,'steps':.25,'deja':0,'scale':0,'length':1,'p':1,'q':16},
 'clock_source':0,'t_clock':{'period':128,'high':40,'start':25},'x_clock':{'period':96,'high':32,'start':35},'events':[]}
cases=[]
for model in range(7):
 c=base('t_model_'+str(model));c['t']['model']=model
 c['events']=[{'block':600,'t':{'deja':.5,'length':5}}, {'block':1200,'t':{'deja':1,'length':16}}, {'block':1800,'t':{'deja':.2,'bias':.85,'jitter':.7,'pwstd':.8},'reset_t':True}];cases.append(c)
for source in range(5):
 c=base('xy_clock_'+str(source));c['load_y']=True;c['clock_source']=source;c['x']['steps']=.9;c['y']['steps']=.9;c['y']['q']=2
 c['t']['external']=source%2==0
 c['events']=[{'block':500,'x':{'deja':.5,'length':7},'t':{'range':0,'rate':-24}}, {'block':1000,'x':{'deja':1,'steps':.05,'control':1},'y':{'steps':.05}}, {'block':1500,'x':{'control':2,'range':0,'bias':.9},'reset_x':True}, {'block':1900,'t':{'external':False,'range':2,'rate':48},'clock_source':0}];cases.append(c)
for scale in range(6):
 c=base('weighted_scale_'+str(scale));c['load_y']=True;c['clock_source']=2;c['x'].update(steps=1,scale=scale,spread=.8);c['y'].update(steps=1,scale=scale,q=1)
 c['events']=[{'block':600,'x':{'steps':.6}},{'block':1200,'x':{'steps':.85,'range':1}},{'block':1800,'x':{'steps':1,'bias':.8,'range':2}}];cases.append(c)
for control in range(3):
 c=base('register_reacquisition_'+str(control));c['load_y']=True;c['clock_source']=4;c['x'].update(register=True,value=.17,control=control,steps=.75)
 c['events']=[{'block':240,'x':{'value':.72}},{'block':241,'x':{'value':.86}},{'block':242,'x':{'value':.91}},{'block':700,'clock_source':0}, {'block':1100,'clock_source':2}, {'block':1500,'x':{'register':False,'deja':.5}}, {'block':1800,'x':{'register':True,'value':.3},'clock_source':4}];cases.append(c)
c=base('parameter_endpoints_and_reset');c['load_y']=True
c['events']=[{'block':400,'t':{'bias':0,'jitter':0,'pw':0,'pwstd':0},'x':{'spread':0,'bias':0,'steps':0}}, {'block':800,'t':{'bias':1,'jitter':1,'pw':1,'pwstd':1},'x':{'spread':1,'bias':1,'steps':1}}, {'block':1200,'t':{'model':1,'rate':-48,'range':0},'reset_t':True},{'block':1600,'t':{'model':6,'rate':48,'range':2,'deja':.5},'x':{'spread':.5,'bias':.5,'steps':.5},'reset_x':True}];cases.append(c)
c=base('seed_zero_defined');c['seed']=0;c['t']['deja']=.5;c['x']['deja']=.5;cases.append(c)
c=base('seed_high_defined');c['seed']=0xdeadbeef;c['t']['model']=6;cases.append(c)
# Stateful sequence APIs and pointer-relative Clone; no floating endpoint UB.
ops=[]
for i in range(240):
 op=0 if i<80 else (4 if i==80 else 5 if i==120 else 3 if i==160 else 2 if i==200 else 6 if i%9==0 else 7 if i%7==0 else 0)
 ops.append([op,[1,3,5,7,16][i//48], [0,.5,1][i//80],(i%11)/12, (1 if op==2 else 0xbeca55e5)])
cases.append({'id':'sequence_rewrite_clone_replay','mode':3,'seed':37,'operations':ops})
notes=[]
for i in range(80):
 voltage=[-.006,.0833,.3333,.5833,.9167][i%5]+(i%3-1)
 notes.extend([[0,voltage],[1,voltage+.002],[2,0]])
notes.extend([[3,0],[0,0],[2,0],[0,.5],[2,0]])
cases.append({'id':'note_filter_and_weighted_recorder','mode':4,'seed':1,'operations':notes})
doc={'schema':1,'cases':cases,'negative_cases':[{'id':'undefined_float_word_endpoint','mode':1,'seed':1},{'id':'undefined_y_unloaded_scale','mode':2,'seed':1},{'id':'undefined_terminal_interpolation','mode':5,'seed':1}]}
if __name__=='__main__':
 (HERE/'cases.json').write_text(json.dumps(doc,indent=2)+'\n');print(json.dumps({'cases':len(cases),'full_kernel_frames':sum(c.get('blocks',0)*5 for c in cases),'other_operations':sum(len(c.get('operations',[])) for c in cases)}))
