// OpenPatch harness only. Pinned Frames sources are compiled unchanged with TEST.
#include <iostream>
#include <string>
#include "frames/keyframer.h"
static frames::Keyframer kernel;  // Declared zero-initialized storage.
int main() {
  kernel.Init();  // Upstream TEST Init is a no-op.
  kernel.Clear();
  for (unsigned i = 0; i < 4; ++i) {
    kernel.mutable_settings(i)->easing_curve = frames::EASING_CURVE_LINEAR;
    kernel.mutable_settings(i)->response = 0;
    kernel.set_immediate(i, 0);
  }
  std::string op;
  unsigned record = 0;
  while (std::cin >> op) {
    unsigned a, b, c, d, e;
    if (op == "S") { std::cin >> a >> b >> c; if(a>=4 || b>=6 || c>=256) return 2;
      kernel.mutable_settings(a)->easing_curve = static_cast<frames::EasingCurve>(b);
      kernel.mutable_settings(a)->response = c;
    } else if (op == "I") { std::cin >> a >> b; if(a>=4 || b>65535) return 2; kernel.set_immediate(a, b);
    } else if (op == "C") { kernel.Clear();
    } else if (op == "A") { std::cin >> a >> b >> c >> d >> e; uint16_t values[4] = {uint16_t(b),uint16_t(c),uint16_t(d),uint16_t(e)};
      bool result = kernel.AddKeyframe(a,values);
      std::cout << "{\"record\":" << record++ << ",\"op\":\"A\",\"success\":" << result << ",\"keys\":" << kernel.num_keyframes() << "}\n";
    } else if (op == "R") { std::cin >> a;
      if(kernel.num_keyframes()==64 && a>kernel.keyframe(63).timestamp) { std::cerr << "capacity after-last read\n"; return 3; }
      bool result = kernel.RemoveKeyframe(a);
      std::cout << "{\"record\":" << record++ << ",\"op\":\"R\",\"success\":" << result << ",\"keys\":" << kernel.num_keyframes() << "}\n";
    } else if (op == "E") { std::cin >> a >> b;
      if(kernel.num_keyframes()==64 && a>kernel.keyframe(63).timestamp) { std::cerr << "capacity after-last read\n"; return 3; }
      // Exact nonfirst timestamps have scale 65536, causing a curve-table
      // overread for curves >= 2. Reject before calling unmodified source.
      for (unsigned k=1;k<kernel.num_keyframes();++k) if(a==kernel.keyframe(k).timestamp)
        for(unsigned ch=0;ch<4;++ch) if(kernel.mutable_settings(ch)->easing_curve>=frames::EASING_CURVE_IN_QUARTIC) { std::cerr << "curve endpoint table read\n"; return 3; }
      // Conservative defined-domain bound for the signed final multiply:
      // bounce unsigned table interpolation can enlarge shaped_scale to <196608.
      for(unsigned ch=0;ch<4;++ch) if(kernel.mutable_settings(ch)->easing_curve==frames::EASING_CURVE_BOUNCE)
        for(unsigned k=1;k<kernel.num_keyframes();++k) {
          int delta=int(kernel.keyframe(k).values[ch])-int(kernel.keyframe(k-1).values[ch]);
          if(delta>20000 || delta< -20000) { std::cerr << "bounce signed overflow domain\n"; return 3; }
        }
      kernel.Evaluate(a);
      std::cout << "{\"record\":" << record++ << ",\"op\":\"E\",\"timestamp\":" << a << ",\"levels\":[";
      for(unsigned i=0;i<4;++i) std::cout << (i?",":"") << kernel.level(i);
      std::cout << "],\"dac_code\":["; for(unsigned i=0;i<4;++i) std::cout << (i?",":"") << kernel.dac_code(i);
      std::cout << "],\"color\":["; for(unsigned i=0;i<3;++i) std::cout << (i?",":"") << unsigned(kernel.color(i));
      std::cout << "],\"position\":" << kernel.position() << ",\"nearest_keyframe\":" << kernel.nearest_keyframe()
                << ",\"find_nearest\":" << kernel.FindNearestKeyframe(a,b) << "}\n";
    } else if (op == "D") { std::cin >> a >> b;
      std::cout << "{\"record\":" << record++ << ",\"op\":\"D\",\"gain\":" << a << ",\"response\":" << b << ",\"dac_code\":" << kernel.ConvertToDacCode(a,b) << "}\n";
    } else if (op == "V") { std::cin >> a >> b >> c;
      if(c && kernel.mutable_settings(a)->easing_curve==frames::EASING_CURVE_BOUNCE) { std::cerr << "bounce signed overflow domain\n"; return 3; }
      std::cout << "{\"record\":" << record++ << ",\"op\":\"V\",\"value\":" << kernel.SampleAnimation(a,b,c!=0) << "}\n";
    } else return 2;
    if(!std::cin) return 2;
  }
}
