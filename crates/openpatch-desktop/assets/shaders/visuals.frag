#version 330 core
in vec2 v_uv;
out vec4 frag_color;
uniform vec2 u_resolution;
uniform float u_time;
uniform float u_level;
uniform float u_wave[256];
uniform float u_spectrum[64];
uniform int u_mode; // 0 aurora, 1 spectrum, 2 waveform, 3 ferrofluid,
                    // 4 phosphor, 5 horizon, 6 pulsar, 7 vector, 8 ridges
uniform vec4 u_ferro; // smoothed low, middle, high and peak envelopes
// 128 x 48 single-channel history. Rows 0-31: compressed log-frequency
// spectrum, row 0 live and older rows above it. Rows 32-47: trigger-aligned,
// level-normalised waveform sweeps (about -1..1), row 32 live.
uniform sampler2D u_history;
// x spectrum scroll phase (0..1 toward the next row), y sweep scroll phase,
// z XY delay in samples (about a quarter period), w spectrum rows pushed.
uniform vec4 u_history_info;

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
// Studio visuals (modes 4-8): line art drawn from the signal itself, in the
// spirit of an oscilloscope, a vector display and stacked ridgelines. Thin
// anti-aliased strokes with a restrained phosphor halo on near-black, in the
// app's ivory, amber and teal. u_ferro (x low, y middle, z high, w peak pulse)
// is already smoothed on the UI thread and only nudges brightness and scale.
// Loops stay short (no raymarching) so a handheld iGPU keeps full frame rate.
// ---------------------------------------------------------------------------
const float TAU=6.28318530718;
const float PI=3.14159265359;
const int HISTORY_WIDTH=128;
const int SPECTRUM_ROWS=32;
const int SWEEP_ROWS=16;
const vec3 IVORY=vec3(0.95,0.91,0.82);
const vec3 AMBER=vec3(0.98,0.63,0.27);
const vec3 TEAL=vec3(0.24,0.72,0.68);
const vec3 INK=vec3(0.006,0.008,0.009);

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
float sq(float x) { return x*x; }
mat2 rot2(float a) { float c=cos(a); float s=sin(a); return mat2(c,-s,s,c); }
float waveAt(float x) {
    float f=clamp(x,0.0,1.0)*254.0;
    int i=int(f);
    return mix(u_wave[i],u_wave[i+1],fract(f));
}
float aspectRatio() { return u_resolution.x/max(u_resolution.y,1.0); }
float pixelSize() { return 1.0/max(u_resolution.y,1.0); }
float beatPulse() { return smoothstep(0.0,1.0,clamp(u_ferro.w,0.0,1.0)); }
vec3 finishColor(vec3 col, vec2 uv) {
    col=max(col,vec3(0.0));
    col=col/(1.0+col*0.85);
    col*=1.0-smoothstep(0.35,0.95,length((uv-0.5)*vec2(1.0,0.85)))*0.45;
    col=pow(col,vec3(0.92));
    col+=(hash12(uv*u_resolution+fract(u_time)*61.0)-0.5)/255.0;
    return col;
}

// One row of u_history over x in 0..1, Catmull-Rom between texels so near
// ridges stay round instead of polygonal. slope is d/dx.
float historyRow(int row, float x, out float slope) {
    float f=clamp(x,0.0,1.0)*float(HISTORY_WIDTH-1);
    int i=min(int(f),HISTORY_WIDTH-2);
    float t=f-float(i);
    float a=texelFetch(u_history,ivec2(max(i-1,0),row),0).r;
    float b=texelFetch(u_history,ivec2(i,row),0).r;
    float c=texelFetch(u_history,ivec2(i+1,row),0).r;
    float d=texelFetch(u_history,ivec2(min(i+2,HISTORY_WIDTH-1),row),0).r;
    float c1=0.5*(c-a);
    float c2=a-2.5*b+2.0*c-0.5*d;
    float c3=0.5*(d-a)+1.5*(b-c);
    slope=(c1+t*(2.0*c2+3.0*c3*t))*float(HISTORY_WIDTH-1);
    return b+t*(c1+t*(c2+t*c3));
}
// Spectrum history: row 0 is live, higher rows are older; x is log frequency.
float spectrumRow(int row, float x, out float slope) {
    return historyRow(clamp(row,0,SPECTRUM_ROWS-1),x,slope);
}
// Trigger-aligned, level-normalised waveform sweeps (about -1..1); 0 is live.
float sweepRow(int row, float x, out float slope) {
    return historyRow(SPECTRUM_ROWS+clamp(row,0,SWEEP_ROWS-1),x,slope);
}
// Stroke profile for a curve offset d away with screen-space slope s:
// x = crisp core (about one pixel), y = soft phosphor halo. The slope
// correction keeps steep flanks as thick as flat runs; it is capped (more so
// for the wide halo) so it cannot smear light far above a sharp peak.
vec2 stroke(float d, float s, float width) {
    float raw=abs(d)/pixelSize();
    float dist=raw*inversesqrt(1.0+min(s*s,9.0));
    float haloDist=raw*inversesqrt(1.0+min(s*s,2.0));
    return vec2(exp(-sq(dist/width)),exp(-sq(haloDist/(width*6.0)))*0.11+exp(-haloDist/(width*22.0))*0.018);
}

// 4 · Phosphor: a dual-trace oscilloscope. CH1 is the trigger-aligned
// waveform; its last sixteen sweeps linger as fading persistence, so a steady
// tone burns one bright line and a moving one smears like real phosphor. CH2
// below is the live spectrum with a short afterglow. Fast edges dim, as on an
// analog beam.
vec3 phosphorVisual(vec2 uv) {
    float aspect=aspectRatio();
    vec2 p=(uv-0.5)*vec2(aspect,1.0);
    float px=pixelSize();
    float pulse=beatPulse();
    vec3 col=INK+vec3(0.004,0.008,0.009)*(1.0-length(p));
    // Graticule: 10 x 8 divisions with a brighter, ticked centre cross.
    float halfW=aspect*0.5-0.05;
    float halfH=0.45;
    float centre=0.07;
    vec2 q=p-vec2(0.0,centre);
    vec2 division=vec2(halfW*0.2,halfH*0.25);
    vec2 g=q/division;
    vec2 gridDist=abs(fract(g+0.5)-0.5)*division/px;
    float inside=step(abs(p.x),halfW+px)*step(abs(p.y),0.5-0.03);
    float dotted=0.55+0.45*step(0.5,fract((p.x+p.y)/(px*4.0)));
    float grid=max(exp(-sq(gridDist.x/0.6)),exp(-sq(gridDist.y/0.6)))*dotted;
    vec2 axisDist=abs(q)/px;
    vec2 tick=abs(fract(g*5.0+0.5)-0.5)*division*0.2/px;
    float axes=exp(-sq(axisDist.x/0.6))+exp(-sq(axisDist.y/0.6));
    axes+=exp(-sq(tick.x/0.6))*step(axisDist.y,3.5)+exp(-sq(tick.y/0.6))*step(axisDist.x,3.5);
    col+=TEAL*(grid*0.045+min(axes,1.0)*0.07)*inside;

    float sx=(p.x+halfW)/(2.0*halfW);
    float edge=smoothstep(0.0,0.012,sx)*smoothstep(1.0,0.988,sx);
    // CH1 waveform with persistence.
    float amp=0.29;
    float phase=u_history_info.y;
    vec3 beam=vec3(0.0);
    for (int k=0;k<SWEEP_ROWS;k++) {
        float slope;
        float v=sweepRow(k,sx,slope);
        float d=p.y-(centre+v*amp);
        if (abs(d)>0.16) continue;
        float s=slope*amp/(2.0*halfW);
        vec2 line=stroke(d,s,k==0 ? 1.05 : 0.9);
        float age=k==0 ? 0.0 : float(k-1)+phase;
        float fade=k==0 ? 1.0 : 0.38*exp(-age*0.36);
        float speed=0.3+0.7*inversesqrt(1.0+s*s*0.06);
        vec3 tint=k==0 ? mix(IVORY,AMBER,0.22) : mix(AMBER,TEAL,smoothstep(0.0,5.0,age));
        beam+=tint*(line.x+line.y*(1.0+0.4*pulse))*fade*speed;
    }
    // CH2 spectrum, teal, with a three-row afterglow from the history.
    float base=centre-division.y*4.0;
    for (int k=0;k<4;k++) {
        float slope;
        float v=spectrumRow(k*2,sx,slope);
        float h=0.12;
        float d=p.y-(base+v*h);
        float s=slope*h/(2.0*halfW);
        vec2 line=stroke(d,s,0.9);
        float fade=k==0 ? 0.8 : 0.22*exp(-float(k-1)*0.7);
        beam+=TEAL*(line.x+line.y)*fade;
    }
    col+=beam*edge;
    return col;
}

// 5 · Horizon: a wireframe fly-over of the spectrum history. Every ridge is a
// past spectrum (newest in front, receding to the horizon), mirrored so the
// bass rises in the middle, with longitudinal grid lines on the faces and true
// front-to-back occlusion. The live waveform runs along the horizon.
vec3 horizonVisual(vec2 uv) {
    float aspect=aspectRatio();
    vec2 p=(uv-0.5)*vec2(aspect,1.0);
    float px=pixelSize();
    float pulse=beatPulse();
    float lowBand=u_ferro.x;
    // Slow camera drift: a slight bank and sway, never fast enough to notice.
    p=rot2(0.03*sin(u_time*0.071))*p;
    float sway=0.16*sin(u_time*0.043);
    float horizon=0.19;
    float camH=0.40;
    float spacing=0.19;
    float phase=u_history_info.x;
    float halfWidth=1.6;

    // Sky: near-black with a low warm band at the horizon.
    vec3 back=INK+vec3(0.030,0.022,0.016)*exp(-abs(p.y-horizon)*16.0);
    back+=vec3(0.004,0.010,0.012)*smoothstep(horizon,0.55,p.y);
    // The live sweep as a distant skyline, faded toward both screen edges.
    float sx=p.x/aspect+0.5;
    float slope;
    float v=sweepRow(0,sx,slope);
    float skyline=horizon+0.028+v*0.04;
    vec2 sky=stroke(p.y-skyline,slope*0.04/aspect,0.9);
    back+=mix(AMBER,IVORY,0.3)*(sky.x*0.75+sky.y)*smoothstep(0.5,0.15,abs(sx-0.5))*(0.8+0.3*pulse);
    back+=AMBER*0.12*exp(-sq((p.y-horizon)/px/1.2))*smoothstep(0.5,0.1,abs(sx-0.5));

    vec3 front=vec3(0.0);
    float yPrev=-1.0;
    float zPrev=0.0;
    for (int i=0;i<SPECTRUM_ROWS;i++) {
        float depth=float(i)+phase;
        float z=0.55+depth*spacing;
        float yBase=horizon-camH/z;
        // Rows that cannot reach this pixel cost no texture reads.
        if (p.y>yBase+0.62/z+0.03) { yPrev=yBase; zPrev=z; continue; }
        float X=p.x*z+sway;
        float u=abs(X)/halfWidth;
        float spectral=spectrumRow(i,u,slope);
        float envelope=smoothstep(1.0,0.62,u);
        float nearRow=exp(-depth*0.35);
        // A new front row rises from flat ground as it arrives, so neither its
        // stroke nor its silhouette pops in when the history scrolls.
        float emerge=smoothstep(0.0,1.0,depth);
        float h=spectral*envelope*0.36*emerge*(1.0+(0.10*lowBand+0.12*pulse)*nearRow);
        float y=yBase+h/z;
        float s=slope*envelope*0.36*emerge/halfWidth*sign(X);
        float k=depth/float(SPECTRUM_ROWS);
        // New rows ease in over a full row of travel, so pushes never flicker.
        float fade=smoothstep(0.0,1.0,depth)*smoothstep(float(SPECTRUM_ROWS),float(SPECTRUM_ROWS)-8.0,depth);
        // Rows closer than ~2 px on screen fade instead of aliasing.
        float gap=camH*spacing/(z*z)/px;
        fade*=smoothstep(0.8,3.0,gap);
        float fog=exp(-k*2.4);
        vec3 ink=mix(TEAL*0.75,mix(IVORY,AMBER,clamp(h*2.4,0.0,1.0)),exp(-k*3.2));
        vec2 line=stroke(p.y-y,s,mix(1.25,0.85,k));
        float lit=fade*fog*(1.0+0.25*pulse*nearRow);
        if (p.y<y) {
            // Under this ridge: the opaque face between it and the row in front.
            float zFace=i==0 ? camH/max(horizon-p.y,0.001) : mix(zPrev,z,clamp((p.y-yPrev)/max(y-yPrev,0.0001),0.0,1.0));
            float Xf=p.x*zFace+sway;
            float cell=abs(fract(Xf*7.0+0.5)-0.5)/7.0;
            float colGap=1.0/(7.0*zFace)/px;
            float column=exp(-sq(cell/zFace/px/0.8))*smoothstep(3.0,8.0,colGap);
            float faceFog=exp(-(zFace-0.55)/spacing/float(SPECTRUM_ROWS)*2.4);
            vec3 face=INK*0.7+TEAL*column*0.20*faceFog*smoothstep(halfWidth*1.6,halfWidth*0.6,abs(Xf));
            back=mix(back,face,i==0 ? 1.0 : fade)+ink*line.x*lit;
            break;
        }
        // Distant halos fade faster so peaks of the same note do not stack into rays.
        front+=ink*(line.x+line.y*sq(1.0-k))*lit;
        yPrev=y;
        zPrev=z;
    }
    return back+front;
}

// 6 · Pulsar: polar ridgelines. Each ring is a past spectrum, born at the
// centre and drifting outward; inner rings occlude the ones behind them. The
// angle is frequency (bass at the top, mirrored), and a small circular scope
// of the live waveform sits in the middle.
vec3 pulsarVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    float px=pixelSize();
    float pulse=beatPulse();
    p=rot2(0.05*sin(u_time*0.047))*p;
    p.y+=0.012;
    // A wide ellipse fills the slim dock; g converts radial offsets back to
    // screen distance so strokes keep one width all the way round.
    vec2 stretch=vec2(1.75,0.97);
    vec2 q=p/stretch;
    float r=max(length(q),0.0001);
    float g=max(length(q/stretch)/r,0.0001);
    float a=atan(q.x,q.y+1e-6);
    float u=abs(a)/PI;
    float phase=u_history_info.x;
    float scale=1.0+0.025*pulse;
    const int RINGS=20;
    float inner=0.085*scale;
    float spacing=0.0185*scale;
    float amp=0.075*scale;
    vec3 back=INK+vec3(0.004,0.008,0.009)*exp(-r*3.0);
    vec3 front=vec3(0.0);
    bool covered=false;
    float slope;
    for (int i=0;i<RINGS;i++) {
        float depth=float(i)+phase;
        float base=inner+depth*spacing;
        if (r>base+amp*1.35+0.02) continue;
        float spectral=spectrumRow(i,u,slope);
        // Rings are born as plain circles and grow their spectrum as they leave.
        float lift=amp*(1.0-0.35*depth/float(RINGS))*smoothstep(0.0,1.0,depth);
        float radius=base+spectral*lift;
        float s=slope*lift/PI/r;
        float k=depth/float(RINGS);
        float fade=smoothstep(0.0,1.0,depth)*smoothstep(float(RINGS),float(RINGS)-5.0,depth);
        vec3 ink=mix(mix(IVORY,AMBER,clamp(spectral*0.8,0.0,1.0)),TEAL*0.8,smoothstep(0.1,0.9,k));
        vec2 line=stroke((r-radius)/g,s,mix(1.15,0.85,k));
        float lit=fade*(1.0-0.55*k)*(1.0+0.25*pulse*exp(-depth*0.4));
        if (r<radius) {
            back=mix(back,INK*0.6,i==0 ? 1.0 : fade)+ink*line.x*lit;
            covered=i==0;
            break;
        }
        front+=ink*(line.x+line.y)*lit;
    }
    if (r<inner+amp) {
        // The centre scope: live waveform wrapped (mirrored) around a circle.
        float v=sweepRow(0,u,slope);
        float scope=inner*0.55+v*inner*0.22;
        vec2 line=stroke((r-scope)/g,slope*inner*0.22/PI/r,1.0);
        float glow=covered ? 1.0 : 0.0;
        back+=mix(AMBER,IVORY,0.25)*(line.x+line.y)*glow*(0.85+0.3*pulse);
    }
    return back+front;
}

// 7 · Vector: an XY vector display of the waveform against itself a quarter
// period later (x = s[n], y = s[n+d], depth = s[n+2d]), so tones draw clean
// loops and richer timbres knot into figures. As on a real vector display the
// beam is brightest where it moves slowest.
vec3 vectorProject(int n, int delay, float gain, mat2 yaw, mat2 pitch) {
    vec3 q=vec3(u_wave[n],u_wave[n+delay],u_wave[n+2*delay])*gain;
    q.xz=yaw*q.xz;
    q.yz=pitch*q.yz;
    return vec3(q.xy*(1.55/(2.5-q.z)),q.z);
}
vec3 vectorVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    float px=pixelSize();
    float pulse=beatPulse();
    float radius=length(p);
    vec3 col=INK+vec3(0.004,0.008,0.009)*exp(-radius*4.0);
    // Goniometer-style graticule: two circles, crosshair and diagonals.
    float ring=min(abs(radius-0.42),abs(radius-0.21))/px;
    float crosshair=min(abs(p.x),abs(p.y))/px;
    float diagonal=abs(abs(p.x)-abs(p.y))*0.7071/px;
    float graticule=exp(-sq(ring/0.6))*0.06+exp(-sq(crosshair/0.6))*0.04+exp(-sq(diagonal/0.6))*0.02;
    col+=TEAL*graticule*smoothstep(0.46,0.40,radius);

    // The figure can never leave this radius (|s| * gain <= 0.42 per axis),
    // so most of a wide dock skips the segment loop entirely.
    if (radius>0.66) return col;
    int delay=int(clamp(u_history_info.z,3.0,48.0)+0.5);
    float gain=clamp(0.42/(abs(u_level)+0.012),0.3,24.0);
    mat2 yaw=rot2(0.35*sin(u_time*0.09));
    mat2 pitch=rot2(0.30+0.12*sin(u_time*0.061));
    const int SEGMENTS=96;
    // About three periods: enough to close the figure, fine enough to stay round.
    float stride=float(min(255-2*delay,12*delay))/float(SEGMENTS);
    vec3 prev=vectorProject(0,delay,gain,yaw,pitch);
    float core=0.0;
    vec3 coreTint=IVORY;
    vec3 halo=vec3(0.0);
    for (int i=1;i<=SEGMENTS;i++) {
        vec3 cur=vectorProject(int(float(i)*stride),delay,gain,yaw,pitch);
        vec2 pa=p-prev.xy;
        vec2 ba=cur.xy-prev.xy;
        float h=clamp(dot(pa,ba)/max(dot(ba,ba),1e-8),0.0,1.0);
        float dist=length(pa-ba*h)/px;
        float near=clamp(0.5+mix(prev.z,cur.z,h)*0.6,0.0,1.0);
        float speed=clamp(0.016/(length(ba)+0.004),0.4,1.5);
        float age=float(i)/float(SEGMENTS);
        float strength=exp(-sq(dist/1.1))*speed*(0.45+0.55*near)*(0.55+0.45*age);
        if (strength>core) {
            core=strength;
            coreTint=mix(TEAL,IVORY,near*0.8+0.2);
        }
        halo+=mix(TEAL,AMBER,0.25*near)*(exp(-sq(dist/7.0))*0.03+exp(-dist/26.0)*0.003)*speed*(0.4+0.6*near);
        prev=cur;
    }
    // Silence parks the beam as a faint idle spot instead of a hot blob.
    float signal=0.12+0.88*smoothstep(0.002,0.02,abs(u_level));
    halo*=smoothstep(0.66,0.48,radius);
    col+=(coreTint*min(core,1.4)+min(halo,vec3(0.6))*(1.0+0.35*pulse))*signal;
    return col;
}

// 8 · Ridges: stacked ridgelines in the spirit of Unknown Pleasures, with true
// front-to-back occlusion. The rows drift slowly toward the horizon over a
// quiet terrain while the spectrum history ripples back through them: the
// front row is live and each row behind it shows the music a moment earlier.
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
vec3 ridgesVisual(vec2 uv) {
    vec2 p=(uv-0.5)*vec2(aspectRatio(),1.0);
    // Slow camera drift.
    p=rot2(0.015*sin(u_time*0.037))*p;
    p.x+=0.03*sin(u_time*0.052);
    float px=pixelSize();
    float midBand=u_ferro.y;
    float pulse=beatPulse();
    float speed=u_time*0.35;
    float scroll=fract(speed);
    float base=floor(speed);
    vec3 back=mix(vec3(0.010,0.013,0.020),vec3(0.018,0.023,0.036),uv.y);
    float horizon=0.30;
    back+=vec3(0.30,0.24,0.20)*exp(-sq((p.y-horizon)*9.0))*0.05;
    vec3 front=vec3(0.0);
    const int ROWS=30;
    for (int i=0;i<ROWS;i++) {
        float row=float(i)+scroll;
        float k=row/float(ROWS);
        float persp=pow(max(k,0.0),0.75);
        float y0=mix(-0.46,horizon,persp);
        float shrink=mix(1.0,0.38,persp);
        if (p.y>y0+0.62*shrink+0.03) continue;
        float x=p.x/shrink;
        float seed=base-float(i);
        float envelope=exp(-x*x*2.6);
        float freq=clamp(abs(x)*0.85,0.0,1.0);
        // History position for this depth, between two pushed rows.
        float h=max(row*float(SPECTRUM_ROWS-1)/float(ROWS)-u_history_info.x,0.0);
        int j=int(h);
        float slopeA;
        float slopeB;
        float a=spectrumRow(j,freq,slopeA);
        float b=spectrumRow(j+1,freq,slopeB);
        float spectral=mix(a,b,fract(h));
        float slope=mix(slopeA,slopeB,fract(h));
        float terrain=fbm4(vec2(x*3.2,seed*0.37))*0.55+vnoise(vec2(x*9.0,seed*1.3))*0.15;
        float gainS=0.20*(0.7+0.6*midBand)*(1.0-k*0.45);
        float height=envelope*(terrain*0.20+spectral*gainS)*shrink;
        float frontRows=1.0-smoothstep(0.0,0.2,k);
        height*=(1.0+0.18*pulse*frontRows)*smoothstep(0.0,1.0,row);
        float line=y0+height;
        // Screen slope of the spectral term keeps steep flanks as crisp as runs.
        float s=envelope*slope*0.85*sign(x)*gainS*smoothstep(0.0,1.0,row);
        float fadeIn=smoothstep(0.0,1.0,row)*smoothstep(float(ROWS),float(ROWS)-3.0,row);
        float haze=mix(1.0,0.25,k);
        vec3 ink=mix(vec3(0.32,0.50,0.66),vec3(0.98,0.86,0.70),pow(max(1.0-k,0.0),1.6));
        ink=mix(ink,TEAL,0.18*midBand*k);
        vec2 stroke2=stroke(p.y-line,s,mix(1.15,0.9,k));
        float lift=(1.0+3.6*height)*(1.0+0.35*frontRows*pulse);
        float lit=haze*fadeIn*lift;
        if (p.y<line) {
            // The opaque ridge hides everything behind it, including the glow
            // of the rows in front; that clean cut is what makes the stack read.
            back=mix(back+front,vec3(0.006,0.008,0.014),fadeIn);
            back+=ink*(stroke2.x+stroke2.y*0.25+0.04*exp((p.y-line)*60.0))*lit;
            return back;
        }
        // Thin core, a restrained halo and a faint sheen above the line.
        front+=ink*(stroke2.x+stroke2.y*0.25+exp(-(p.y-line)*90.0)*0.08)*lit;
    }
    return back+front;
}

vec3 studioVisual(vec2 uv) {
    vec3 col;
    if (u_mode == 4) col=phosphorVisual(uv);
    else if (u_mode == 6) col=pulsarVisual(uv);
    else if (u_mode == 7) col=vectorVisual(uv);
    else if (u_mode == 8) col=ridgesVisual(uv);
    else col=horizonVisual(uv); // u_mode == 5 and any future id
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
