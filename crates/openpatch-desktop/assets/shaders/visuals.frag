#version 330 core
in vec2 v_uv;
out vec4 frag_color;
uniform vec2 u_resolution;
uniform float u_time;
uniform float u_level;
uniform float u_wave[256];
uniform float u_spectrum[64];
uniform int u_mode; // 0 aurora, 1 spectrum, 2 waveform, 3 ferrofluid,
                    // 4 nebula, 5 tunnel, 6 halos, 7 lissajous, 8 ridges, 9 borealis
uniform vec4 u_ferro; // smoothed low, middle, high and peak envelopes

// A bounded procedural surface inspired by ferrofluid in a magnetic field.
// Geometry, lighting and analysis stay on the visual thread / GPU.
// Front-on liquid sculpture. Surface displacement follows three smoothed audio
// bands and recent output peaks, with smooth liquid surface motion.
float ferroShape(vec3 p) {
    float radius=length(p);
    vec3 n=p/max(radius,0.0001);
    float turn=u_time*0.16;
    mat2 rotation=mat2(cos(turn),-sin(turn),sin(turn),cos(turn));
    n.xy=rotation*n.xy;
    n += vec3(sin(n.y*4.0+u_time*.7),sin(n.z*4.0-u_time*.5),cos(n.x*4.0+u_time*.6))*0.05*(0.2+u_ferro.y);
    n=normalize(n);
    vec3 cell=fract(n*(3.2+u_ferro.z*0.65)+vec3(0.5,0.5,0.0))-0.5;
    float tip=pow(max(1.0-length(cell)*1.7,0.0),2.8);
    // A continuous field keeps the liquid normals smooth between audio frames.
    float wave=sin(n.x*7.0+u_time*1.7)*sin(n.y*6.0-u_time*1.3)*u_ferro.y;
    float energy=clamp(u_ferro.x*0.65+u_ferro.w*0.35,0.0,1.0);
    float body=0.73+energy*0.22+sin(n.x*4.0+n.y*3.0+u_time*0.8)*0.045*u_ferro.y;
    float spikes=(0.10+energy*0.70+u_ferro.z*0.22)*tip;
    return radius-body-spikes-wave*0.065*(0.2+energy);
}
float ferroScene(vec3 p) { return ferroShape(p)*0.14; }
vec3 ferroNormal(vec3 p) {
    vec2 e=vec2(0.001,0.0);
    return normalize(vec3(ferroShape(p+e.xyy)-ferroShape(p-e.xyy),ferroShape(p+e.yxy)-ferroShape(p-e.yxy),ferroShape(p+e.yyx)-ferroShape(p-e.yyx)));
}
vec3 studioReflection(vec3 r) {
    // Broad studio softboxes, a narrow silver strip and quiet colored edge cards.
    vec3 c = vec3(0.025,0.033,0.042)*(0.45+max(r.y,0.0));
    float key = pow(max(dot(r,normalize(vec3(-0.45,0.85,0.35))),0.0),22.0);
    float fill = pow(max(dot(r,normalize(vec3(0.7,0.55,-0.35))),0.0),34.0);
    float strip = exp(-pow((r.x+0.18)/0.085,2.0))*smoothstep(0.0,0.5,r.y);
    c += vec3(1.0,0.91,0.77)*key*2.5 + vec3(0.48,0.7,0.8)*fill*1.7;
    c += vec3(0.83,0.9,0.94)*strip*0.8;
    c += vec3(0.50,0.63,0.67)*pow(max(dot(r,normalize(vec3(0.15,0.35,1.0))),0.0),6.0)*0.75;
    c += vec3(0.07,0.23,0.22)*pow(max(r.z,0.0),8.0)*0.5;
    return c;
}
vec3 ferrofluid(vec2 uv) {
    float aspect=u_resolution.x/max(u_resolution.y,1.0);
    vec2 screen=(uv-0.5)*vec2(aspect,1.0)*3.3;
    vec3 color=mix(vec3(0.009,0.015,0.02),vec3(0.035,0.047,0.053),uv.y);
    vec3 origin=vec3(screen,3.0),direction=vec3(0.0,0.0,-1.0);
    vec3 p=origin;float t=1.1;bool hit=false;
    if (length(screen)<1.95) {
        for (int i=0;i<120;i++) {
            p=origin+direction*t;
            float d=ferroScene(p);
            if(d<0.0008){hit=true;break;}
            t+=max(d,0.0005);
            if(t>4.9)break;
        }
    }
    if(hit){
        vec3 n=ferroNormal(p);float facing=max(dot(n,-direction),0.0);
        vec3 reflected=reflect(direction,n);
        float occlusion=clamp(0.6+ferroShape(p+n*0.08)*3.0,0.35,1.0);
        float fresnel=0.18+0.82*pow(1.0-facing,5.0);
        color=vec3(0.030,0.043,0.047)*(0.55+max(n.y,0.0)*0.5)*occlusion;
        color+=studioReflection(reflected)*(0.60+fresnel)*occlusion;
        color+=vec3(0.08,0.25,0.23)*pow(1.0-facing,3.0)*0.24;
        color=color/(1.0+color);
    }else{color*=1.0-exp(-dot(screen,screen)*0.7)*0.48;}
    return pow(max(color,vec3(0.0)),vec3(0.78));
}

// ---------------------------------------------------------------------------
// Studio visuals (modes 4-9). Analytic or low-iteration techniques only, so a
// handheld iGPU keeps full frame rate. Audio drives motion through u_ferro:
// x low band, y middle band, z high band, w recent peak pulse. Every band is
// already smoothed on the UI thread; beats only nudge slow envelopes here.
// ---------------------------------------------------------------------------
const float TAU=6.28318530718;

float hash12(vec2 p) {
    vec3 p3=fract(vec3(p.xyx)*0.1031);
    p3+=dot(p3,p3.yzx+33.33);
    return fract((p3.x+p3.y)*p3.z);
}
float vnoise(vec2 p) {
    vec2 i=floor(p);
    vec2 f=fract(p);
    vec2 u=f*f*(3.0-2.0*f);
    float a=hash12(i);
    float b=hash12(i+vec2(1.0,0.0));
    float c=hash12(i+vec2(0.0,1.0));
    float d=hash12(i+vec2(1.0,1.0));
    return mix(mix(a,b,u.x),mix(c,d,u.x),u.y);
}
float fbm4(vec2 p) {
    float sum=0.0;
    float amp=0.5;
    mat2 turn=mat2(0.8,-0.6,0.6,0.8);
    for (int i=0;i<4;i++) {
        sum+=amp*vnoise(p);
        p=turn*p*2.03+vec2(17.13,9.71);
        amp*=0.5;
    }
    return sum/0.9375;
}
float sq(float x) { return x*x; }
mat2 rot2(float a) { float c=cos(a); float s=sin(a); return mat2(c,-s,s,c); }
vec3 cosinePalette(float t, vec3 a, vec3 b, vec3 c, vec3 d) {
    return a+b*cos(TAU*(c*t+d));
}
// Linear-interpolated, gently compressed spectrum lookup over 0..1.
float spectrumAt(float x) {
    float f=clamp(x,0.0,1.0)*61.0+1.0;
    int i=int(f);
    float v=mix(u_spectrum[i],u_spectrum[i+1],fract(f));
    return sqrt(clamp(v,0.0,4.0));
}
// Frequency-weighted lookup: more of the screen is given to musical lows.
float musicalSpectrum(float x) {
    float s=spectrumAt(x*x);
    return clamp(s*(1.3+x*1.8),0.0,1.5);
}
float waveAt(float x) {
    float f=clamp(x,0.0,1.0)*254.0;
    int i=int(f);
    return mix(u_wave[i],u_wave[i+1],fract(f));
}
float aspectRatio() { return u_resolution.x/max(u_resolution.y,1.0); }
vec3 finishColor(vec3 col, vec2 uv) {
    col=max(col,vec3(0.0));
    col=col/(1.0+col*0.85);
    col*=1.0-smoothstep(0.35,0.95,length((uv-0.5)*vec2(1.0,0.85)))*0.45;
    col=pow(col,vec3(0.92));
    col+=(hash12(uv*u_resolution+fract(u_time)*61.0)-0.5)/255.0;
    return col;
}

// 4 · Nebula: two-layer domain-warped fbm. Lows widen the warp, mids drift the
// palette, highs lift fine filaments, peaks warm the core a little.
vec3 nebulaVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0)*2.4;
    float t=u_time*0.05;
    float lowBand=u_ferro.x; float midBand=u_ferro.y; float highBand=u_ferro.z;
    float pulse=smoothstep(0.0,1.0,u_ferro.w);
    p=rot2(t*0.35)*p;
    p*=1.0-0.06*lowBand;
    vec2 q=vec2(fbm4(p*1.2+vec2(0.0,t*1.7)),fbm4(p*1.2+vec2(5.2,1.3)-vec2(t*1.3,0.0)));
    float warp=1.5+1.3*lowBand;
    vec2 r=vec2(fbm4(p+q*warp+vec2(1.7,9.2)+t*1.2),fbm4(p+q*warp+vec2(8.3,2.8)-t*0.9));
    float f=fbm4(p+r*(1.7+0.6*midBand));
    // Mids slowly rotate the gas hue between teal and violet; no hard cuts.
    float drift=0.5+0.5*sin(u_time*0.023)+0.6*midBand;
    vec3 gasA=mix(vec3(0.02,0.48,0.58),vec3(0.36,0.16,0.62),clamp(drift*0.6,0.0,1.0));
    vec3 gasB=vec3(0.92,0.58,0.36);
    vec3 col=mix(vec3(0.015,0.025,0.085),gasA,smoothstep(0.15,0.62,f));
    col=mix(col,gasB,smoothstep(0.58,0.92,f)*0.75);
    col*=1.0-0.65*smoothstep(0.55,1.05,length(q));
    col=mix(col,vec3(0.55,0.80,0.85),clamp(r.x*r.x*0.9,0.0,1.0)*(0.35+0.40*highBand));
    float shape=f*f*f+0.6*f*f+0.5*f;
    col*=shape*(1.05+0.35*lowBand);
    float filaments=pow(clamp(1.0-abs(f-r.y)*3.0,0.0,1.0),8.0);
    col+=vec3(0.60,0.78,0.88)*filaments*(0.03+0.16*highBand)*f;
    vec2 c=(uv-0.5)*vec2(aspectRatio(),1.0);
    col+=vec3(0.95,0.68,0.48)*exp(-dot(c,c)*7.0)*(0.02+0.08*pulse+0.05*lowBand)*f;
    return col*0.8;
}

// 5 · Tunnel: analytic polar tunnel. The spectrum is wrapped around the wall
// (mirrored, so there is no seam) and streams away into a soft fog.
vec3 tunnelVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    p-=vec2(sin(u_time*0.19),cos(u_time*0.15))*0.05;
    float lowBand=u_ferro.x; float midBand=u_ferro.y; float highBand=u_ferro.z;
    float pulse=smoothstep(0.0,1.0,u_ferro.w);
    float radius=max(length(p),0.0005);
    float angle=atan(p.y,p.x)/TAU+0.5;
    float depth=0.32/radius;
    float travel=u_time*0.55+lowBand*0.35;
    float twist=angle+depth*0.03*sin(u_time*0.13)+u_time*0.008;
        float lanes=24.0;
    float lane=fract(twist*lanes);
    float laneId=mod(floor(twist*lanes),lanes);
    float zc=depth+travel;
    float ring=fract(zc*1.2);
    float ringId=floor(zc*1.2);
    // Thin lit seams at panel edges; perspective widens them toward the viewer.
    float seamW=0.025+0.04*radius;
    float seam=exp(-sq(min(lane,1.0-lane)/seamW));
    float ringLine=exp(-sq(min(ring,1.0-ring)/(seamW*1.3)));
    float laneFreq=abs((laneId+0.5)/lanes*2.0-1.0);
    float energy=musicalSpectrum(laneFreq);
    float panel=hash12(vec2(laneId,ringId));
    float lit=step(0.72,panel);
    float fog=exp(-depth*0.38);
    float hue=0.5+0.5*sin(u_time*0.027+depth*0.12);
    vec3 cool=vec3(0.10,0.48,0.62);
    vec3 warm=vec3(0.70,0.36,0.62);
    vec3 tint=mix(cool,warm,clamp(hue*0.6+laneFreq*0.3+midBand*0.3,0.0,1.0));
    vec3 col=vec3(0.010,0.013,0.022);
    col+=tint*0.08*fog*(0.5+0.5*panel);
    float glow=energy*(0.55+0.35*midBand);
    float inset=smoothstep(0.0,0.12,lane)*smoothstep(1.0,0.88,lane);
    col+=tint*glow*fog*inset*(0.35+0.65*smoothstep(0.0,0.5,ring)*smoothstep(1.0,0.5,ring))*1.3;
    col+=vec3(0.85,0.70,0.55)*lit*fog*(0.03+0.10*highBand)*smoothstep(0.1,0.4,ring)*smoothstep(0.9,0.6,ring);
    col+=mix(vec3(0.55,0.65,0.75),tint,0.5)*seam*fog*(0.06+0.22*glow);
    col+=vec3(0.80,0.85,0.90)*ringLine*fog*(0.05+0.12*pulse+0.08*lowBand);
    col*=smoothstep(0.0,0.12,radius);
    float core=exp(-radius*radius*40.0/(1.0+lowBand*1.5));
    col+=vec3(0.90,0.66,0.50)*core*(0.06+0.18*lowBand+0.06*pulse);
    return col;
}

// 6 · Halos: a three-layer parallax starfield with slow concentric halos.
// Each halo carries one band, so bass, mids and highs are visible apart.
vec3 haloStars(vec2 p, float highBand) {
    vec3 col=vec3(0.0);
    for (int i=0;i<3;i++) {
        float layer=fract(u_time*0.018+float(i)/3.0);
        float scale=mix(26.0,3.0,layer);
        float fade=smoothstep(0.0,0.35,layer)*smoothstep(1.0,0.75,layer);
        vec2 sp=p*scale+vec2(float(i)*13.7,float(i)*7.1);
        vec2 cell=floor(sp);
        vec2 local=fract(sp)-0.5;
        float h=hash12(cell);
        vec2 offset=vec2(hash12(cell+3.1),hash12(cell+7.7))-0.5;
        float d=length(local-offset*0.7);
        float star=smoothstep(0.06,0.0,d)*step(0.78,h);
        float twinkle=0.6+0.4*sin(u_time*(1.0+h*3.0)+h*40.0)*highBand;
        col+=mix(vec3(0.70,0.78,0.95),vec3(0.98,0.85,0.70),h)*star*fade*twinkle*0.55;
    }
    return col;
}
vec3 halosVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    float lowBand=u_ferro.x; float midBand=u_ferro.y; float highBand=u_ferro.z;
    float pulse=smoothstep(0.0,1.0,u_ferro.w);
    float radius=length(p);
    float angle=atan(p.y,p.x);
    vec3 col=vec3(0.008,0.010,0.020)+vec3(0.010,0.014,0.030)*(1.0-uv.y);
    col+=haloStars(p,highBand);
    float sym=abs(fract(angle/TAU+0.5)*2.0-1.0);
    float ripple=waveAt(sym)*0.035;
    for (int i=0;i<6;i++) {
        float fi=float(i);
        float phase=fract(u_time*0.06+fi/6.0);
        float band=i<2 ? lowBand : (i<4 ? midBand : highBand);
        float rr=phase*0.95+ripple*phase;
        float width=0.004+0.010*phase;
        float halo=exp(-sq((radius-rr)/width));
        float life=smoothstep(0.0,0.15,phase)*(1.0-phase);
        vec3 tint=cosinePalette(fi/6.0+0.08*sin(u_time*0.04),vec3(0.50,0.50,0.55),vec3(0.30,0.25,0.25),vec3(1.0,1.0,1.0),vec3(0.0,0.15,0.35));
        col+=tint*halo*life*(0.05+0.55*band);
    }
    float coreSize=0.010+0.030*lowBand+0.012*pulse;
    col+=vec3(0.95,0.78,0.60)*exp(-radius*radius/coreSize)*(0.25+0.30*lowBand);
    col+=vec3(0.45,0.60,0.80)*exp(-radius*5.5)*0.04*(1.0+midBand);
    return col;
}

// 7 · Lissajous: a 3D delay embedding of the live waveform (x = s[n],
// y = s[n+k], z = s[n+2k]) drawn as a depth-cued ribbon that slowly orbits.
float smoothWave(int i) {
    return u_wave[i]*0.5+(u_wave[i+1]+u_wave[i+2])*0.25;
}
vec3 lissajousPoint(int i, float gain) {
    return vec3(smoothWave(i),smoothWave(i+18),smoothWave(i+36))*gain;
}
vec2 lissajousProject(vec3 q, mat2 yaw, mat2 pitch, out float depth) {
    q.xz=yaw*q.xz;
    q.yz=pitch*q.yz;
    depth=q.z;
    return q.xy*(1.6/(2.6-q.z));
}
vec3 lissajousVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0)*1.25;
    float midBand=u_ferro.y; float highBand=u_ferro.z;
    float pulse=smoothstep(0.0,1.0,u_ferro.w);
    float gain=clamp(0.75/(abs(u_level)+0.03),0.8,16.0);
    mat2 yaw=rot2(u_time*0.21);
    mat2 pitch=rot2(0.45+0.18*sin(u_time*0.13));
    vec3 col=vec3(0.008,0.011,0.020)+vec3(0.012,0.016,0.028)*(1.0-length(p));
    // Faint floor ellipse grounds the object in space.
    float ground;
    vec2 g=lissajousProject(vec3(0.0,-0.75,0.0),yaw,pitch,ground);
    vec2 e=(p-g)/vec2(0.95,0.95*0.42);
    col+=vec3(0.20,0.30,0.35)*exp(-sq((length(e)-1.0)*28.0))*0.08;
    float d0;
    vec2 prev=lissajousProject(lissajousPoint(0,gain),yaw,pitch,d0);
    vec3 halos=vec3(0.0);
    vec3 ribbon=vec3(0.0);
    float best=0.0;
    for (int i=1;i<96;i++) {
        float d1;
        vec2 cur=lissajousProject(lissajousPoint(i*2,gain),yaw,pitch,d1);
        vec2 pa=p-prev;
        vec2 ba=cur-prev;
        float h=clamp(dot(pa,ba)/max(dot(ba,ba),0.000001),0.0,1.0);
        float dist=length(pa-ba*h);
        float z=mix(d0,d1,h);
        float near=clamp(0.5+z*0.45,0.0,1.0);
        float width=0.0045+0.0055*near;
        float core=exp(-dist*dist/(width*width));
        float halo=exp(-dist*dist/(width*width*30.0))*0.07;
        float age=float(i)/95.0;
        vec3 tint=cosinePalette(age*0.45+0.08*sin(u_time*0.05)+0.15*midBand,
            vec3(0.55,0.55,0.60),vec3(0.30,0.25,0.25),vec3(1.0,1.0,1.0),vec3(0.0,0.15,0.35));
        tint=mix(tint*vec3(0.55,0.70,0.90),tint,near);
        // Strongest segment wins for the core, so joints do not double up.
        float weight=(0.40+0.60*age);
        float strength=core*(0.45+0.75*near)*weight;
        if (strength>best) { best=strength; ribbon=tint*strength; }
        halos+=tint*halo*weight;
        prev=cur;
        d0=d1;
    }
    col+=(ribbon+halos*0.3)*(0.85+0.25*highBand+0.15*pulse);
    col+=vec3(0.90,0.75,0.60)*exp(-dot(p,p)*14.0)*0.025;
    return col;
}

// 8 · Ridges: stacked spectral ridgelines with true front-to-back occlusion.
// Rows stream toward the viewer; nearer rows answer the spectrum strongest.
vec3 ridgesVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    float lowBand=u_ferro.x; float midBand=u_ferro.y;
    float pulse=smoothstep(0.0,1.0,u_ferro.w);
    float speed=u_time*0.35;
    float scroll=fract(speed);
    float base=floor(speed);
    vec3 col=mix(vec3(0.010,0.013,0.022),vec3(0.020,0.026,0.042),uv.y);
    float horizon=0.30;
    col+=vec3(0.30,0.24,0.22)*exp(-sq((p.y-horizon)*9.0))*0.05;
    const int ROWS=30;
    for (int i=0;i<ROWS;i++) {
        float row=float(i)+scroll;
        float k=row/float(ROWS);
        float persp=pow(k,0.75);
        float y0=mix(-0.46,horizon,persp);
        float shrink=mix(1.0,0.38,persp);
        float x=p.x/shrink;
        float seed=base-float(i);
        float envelope=exp(-x*x*2.6);
        float freq=clamp(abs(x)*0.85,0.0,1.0);
        float spectral=musicalSpectrum(freq)*(1.0-k*0.75);
        float terrain=fbm4(vec2(x*3.2,seed*0.37))*0.55+vnoise(vec2(x*9.0,seed*1.3))*0.15;
        float height=envelope*(terrain*0.20+spectral*0.20*(0.7+0.6*midBand))*shrink;
        height*=1.0+0.25*lowBand*(1.0-k);
        float line=y0+height;
        float dist=p.y-line;
        float fadeIn=smoothstep(0.0,1.0,row)*smoothstep(float(ROWS),float(ROWS)-3.0,row);
        float haze=mix(1.0,0.25,k);
        float px=1.0/max(u_resolution.y,1.0);
        float stroke=exp(-sq(dist/(px*0.9+0.0016*(1.0-k))));
        float sheen=exp(-max(dist,0.0)*90.0)*0.10;
        vec3 ink=mix(vec3(0.32,0.50,0.66),vec3(0.98,0.86,0.70),pow(max(1.0-k,0.0),1.6));
        ink=mix(ink,vec3(0.62,0.48,0.80),0.20*midBand*k);
        float front=1.0-smoothstep(0.0,0.2,k);
        float lift=(1.0+0.6*height*6.0)*(1.0+0.35*front*pulse);
        col+=ink*(stroke+sheen)*haze*fadeIn*lift;
        if (dist<0.0) {
            col=mix(col,vec3(0.006,0.008,0.014),fadeIn);
            col+=ink*(stroke+0.04*exp(dist*60.0))*haze*fadeIn*lift;
            break;
        }
    }
    return col;
}

// 9 · Borealis: layered auroral curtains above a still, rippled lake.
vec3 borealisSky(vec2 p, float lowBand, float midBand, float highBand, float pulse) {
    vec3 col=mix(vec3(0.012,0.020,0.035),vec3(0.004,0.006,0.014),clamp(p.y*1.6+0.2,0.0,1.0));
    vec2 sp=p*90.0;
    vec2 cell=floor(sp);
    float h=hash12(cell);
    float star=step(0.985,h)*smoothstep(0.35,0.0,length(fract(sp)-0.5));
    col+=vec3(0.75,0.80,0.90)*star*0.35*smoothstep(-0.05,0.25,p.y);
    float t=u_time*0.05;
    for (int j=0;j<4;j++) {
        float fj=float(j);
        float depth=1.0-fj*0.2;
        float x=p.x*(0.9+fj*0.25)+fj*3.7;
        float fold=fbm4(vec2(x*0.9+t*(1.0+fj*0.3),fj*5.0+t*0.5));
        float bottom=-0.02+fj*0.07+(fold-0.5)*(0.40+0.25*midBand);
        float h2=p.y-bottom;
        float rays=vnoise(vec2((x+fold*1.4)*26.0,t*3.0+fj*9.0));
        rays=mix(0.75,rays*rays*1.7,0.35+0.35*highBand);
        float reach=0.13+0.12*lowBand+fj*0.03;
        float vertical=smoothstep(-0.004,0.015,h2)*exp(-max(h2,0.0)/reach);
        float lane=smoothstep(0.42,0.72,vnoise(vec2(x*1.3-t*0.8,fj*2.3)));
        vec3 green=vec3(0.10,0.80,0.52);
        vec3 crown=mix(vec3(0.42,0.22,0.70),vec3(0.70,0.30,0.50),fj/3.0);
        vec3 tint=mix(green,crown,smoothstep(0.02,reach*1.6,h2));
        float edge=exp(-sq(h2/0.025))*(0.35+0.25*pulse);
        col+=tint*vertical*rays*lane*depth*0.55*(0.60+0.45*lowBand+0.25*midBand);
        col+=green*edge*lane*depth*0.40;
    }
    return col;
}
vec3 borealisVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    float lowBand=u_ferro.x; float midBand=u_ferro.y; float highBand=u_ferro.z;
    float pulse=smoothstep(0.0,1.0,u_ferro.w);
    float horizon=-0.30;
    if (p.y>=horizon) {
        vec3 sky=borealisSky(p-vec2(0.0,horizon+0.08),lowBand,midBand,highBand,pulse);
        float shore=exp(-sq((p.y-horizon)*40.0));
        return sky*(1.0-shore*0.6);
    }
    float below=horizon-p.y;
    vec2 m=vec2(p.x+sin(below*90.0-u_time*1.2)*0.004*(1.0+lowBand),horizon+below*1.4);
    vec3 reflected=borealisSky(m-vec2(0.0,horizon+0.08),lowBand,midBand,highBand,pulse);
    return reflected*0.32*exp(-below*3.5)+vec3(0.004,0.006,0.010);
}

vec3 studioVisual(vec2 uv) {
    vec3 col;
    if (u_mode == 5) col=tunnelVisual(uv);
    else if (u_mode == 6) col=halosVisual(uv);
    else if (u_mode == 7) col=lissajousVisual(uv);
    else if (u_mode == 8) col=ridgesVisual(uv);
    else if (u_mode == 9) col=borealisVisual(uv);
    else col=nebulaVisual(uv); // u_mode == 4 and any future id
    return finishColor(col,uv);
}

void main() {
    if (u_mode == 3) { frag_color=vec4(ferrofluid(v_uv),1.0); return; }
    if (u_mode >= 4) { frag_color=vec4(studioVisual(v_uv),1.0); return; }
    vec2 uv = v_uv;
    vec2 p = (uv - 0.5) * vec2(u_resolution.x/u_resolution.y, 1.0);
    vec3 col = vec3(0.019, 0.028, 0.054);
    float grid = pow(max(0.0, cos(p.x*48.0)), 32.0)*0.012
               + pow(max(0.0, cos(p.y*48.0)), 32.0)*0.012;
    col += vec3(0.2,0.55,0.7)*grid;
    if (u_mode == 0) {
        for (int i=0; i<7; i++) {
            float f = float(i);
            float y = sin(p.x*(1.5+f*0.16)+u_time*(0.2+f*0.045)+f*0.45)*0.15;
            y += sin(p.x*3.0-u_time*0.3+f)*0.045;
            y += u_wave[int(clamp(uv.x*255.0,0.0,255.0))]*0.30;
            float d = abs(p.y-y+(f-3.0)*0.025);
            vec3 tint = mix(vec3(0.04,0.8,0.74),vec3(0.48,0.25,0.97),f/6.0);
            col += tint*(0.0018/(d+0.004)+exp(-d*18.0)*0.08)*(0.38+u_level*3.0);
        }
    } else if (u_mode == 1) {
        float bin = clamp(floor(uv.x*64.0),0.0,63.0);
        float amplitude = clamp(sqrt(u_spectrum[int(bin)])*2.3,0.0,0.9);
        float bar = step(0.12,fract(uv.x*64.0))*step(uv.y,amplitude+0.045)*step(0.045,uv.y);
        col += mix(vec3(0.08,0.85,0.75),vec3(0.55,0.3,0.98),uv.x)*bar*(0.45+uv.y);
    } else {
        float index = clamp(uv.x*254.0,0.0,254.0);
        float wave = mix(u_wave[int(index)],u_wave[int(index)+1],fract(index));
        float d = abs(uv.y-0.5-wave*1.3);
        col += vec3(0.12,0.9,0.75)*(0.003/(d+0.004)+exp(-d*80.0)*0.1);
    }
    col *= 1.0-smoothstep(0.25,0.85,length(uv-0.5))*0.5;
    frag_color = vec4(col,1.0);
}
