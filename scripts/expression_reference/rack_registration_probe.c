/* Independent reconstruction of Rack Quantity note/function registration.
 * Runs pinned tinyexpr only; does not execute Rack, its UI, or audio engine.
 * Provenance and source hashes are recorded by verify_parameter_expressions.py.
 */
#include "tinyexpr.h"
#include <math.h>
#include <stdio.h>
#include <string.h>
#include <ctype.h>
static double gaintodb(double x){return log10(x)*20;}
static double dbtogain(double x){return pow(10,x/20);}
static double vtof(double x){return pow(2,x)*261.6256f;}
static double ftov(double x){return log2(x/261.6256f);}
static double vtobpm(double x){return pow(2,x)*120.f;}
static double bpmtov(double x){return log2(x/120.f);}
int main(int argc,char**argv){
 char names[500][32];double values[500];te_variable vars[500];int count=0;
 strcpy(names[count],"inf");values[count++]=INFINITY;
 const char* notes="cdefgab";int semis[]={0,2,4,5,7,9,11};
 for(int i=0;i<7;i++)for(int accidental=-1;accidental<=1;accidental++)for(int oct=-1;oct<=9;oct++){
  char name[20],octave[4]="";if(oct>=0)snprintf(octave,sizeof(octave),"%d",oct);
  snprintf(name,sizeof(name),"%c%s%s",notes[i],accidental<0?"b":accidental>0?"s":"",octave);
  double voltage=(oct<0?4:oct)-4+(semis[i]+accidental)/12.0;
  strcpy(names[count],name);values[count++]=440.0*exp2(voltage-9/12.0);
  snprintf(names[count],32,"%sv",name);values[count++]=voltage;
 }
 for(int i=0;i<count;i++)vars[i]=(te_variable){names[i],&values[i],TE_VARIABLE,0};
 const char* fnames[]={"log2","gaintodb","dbtogain","vtof","ftov","vtobpm","bpmtov"};
 double(*fns[])(double)={log2,gaintodb,dbtogain,vtof,ftov,vtobpm,bpmtov};
 for(int i=0;i<7;i++)vars[count++]=(te_variable){fnames[i],fns[i],TE_FUNCTION1|TE_FLAG_PURE,0};
 for(int i=1;i<argc;i++){char s[4096];snprintf(s,sizeof(s),"%s",argv[i]);for(char*c=s;*c;c++){*c=tolower(*c);if(*c=='#')*c='s';}int err=0;te_expr*e=te_compile(s,vars,count,&err);if(!e)printf("%s\tERROR %d\n",argv[i],err);else{printf("%s\t%.17g\n",argv[i],te_eval(e));te_free(e);}}
}
