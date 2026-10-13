# Parameter expressions and musical notes

Right-click a knob or double-click its displayed value. The old entry is selected
so you can replace it immediately. **Result** previews the evaluated value;
**Enter** or **Apply** commits it as one undo action. **Escape**, **Cancel**, or
clicking outside the menu discards the draft. Invalid or stale drafts cannot apply.

| Entry | Result |
|---|---|
| `220*2` or `A4` | 440 Hz in a frequency control |
| `C#4` or `Db4` | About 277.183 Hz |
| `C4v` or `Cv` | 0, using middle C as the 1 V/oct reference |
| `A4v` | 0.75 |
| `25+10` | 35 in a percentage control |
| `gaintodb(0.5)` | About −6.0206 in a dB control |
| `gaintodb(0)` or `-inf` | Silence in a dB gain control |
| `log2(8)` | 3 |

Expressions produce numbers in the **entry units shown by the editor**. A note
name produces a frequency even in a non-frequency control; a name ending in `v`
produces a voltage number. Neither chooses units automatically. Slew values remain
milliseconds per volt, clocked LFO frequency remains a multiplier, and stepped
controls still require a permitted step. Gain controls also accept a trailing
`dB` suffix. Each control retains its existing limits, including sample-rate limits.

## Syntax

Names are case-insensitive. Decimal/scientific numbers (`1.5`, `2e-3`) and
hexadecimal numbers (`0x10`, `0x1.8p2`) are supported, along with parentheses,
unary `+`/`-`, and binary `+`, `-`, `*`, `/`, `%`, `^`. Multiplication is explicit:
write `2*pi`, not `2pi`. `%` is remainder. Comma expressions return their last
value, for example `(1,2)` returns 2.

Powers associate to the right and bind more tightly than unary signs:
`2^3^2 = 512`, `-2^2 = -4`, `(-2)^2 = 4`, and `2^-2^2 = 0.0625`.
Parentheses are recommended for clarity.

Notes use A–G, optionally `#` or `s` for sharp, or `b` for flat. Explicit octaves
range from 0 through 9; omitting the octave selects 4. Append `v` to any note
alias for its voltage. `e` means the note E4; use `exp(1)` for Euler's number.
`pi` and `pi()` are available, as is `inf`.

| Functions | Meaning |
|---|---|
| `abs`, `ceil`, `floor`, `sqrt`, `exp` | Standard scalar math |
| `ln`, `log`, `log10`, `log2` | `ln` and `log` both use the natural logarithm |
| `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2(y,x)` | Trigonometry in radians |
| `sinh`, `cosh`, `tanh` | Hyperbolic functions |
| `pow(x,y)` | Raise x to the power y |
| `fac(n)`, `ncr(n,r)`, `npr(n,r)` | Factorial, combinations, permutations |
| `gaintodb(x)`, `dbtogain(x)` | Amplitude gain ↔ dB |
| `vtof(x)`, `ftov(x)` | Voltage ↔ frequency around Rack's rounded middle-C constant |
| `vtobpm(x)`, `bpmtov(x)` | Voltage ↔ tempo, with 0 V = 120 BPM |

Unary functions also accept a bare argument, such as `sin 2`. This form binds
to a signed primary value: `sin 2^2` means `(sin(2))^2`. Parenthesize the argument
to evaluate a complete expression, as in `sin(2^2)`.

## Validation and compatibility

The original Rust evaluator uses f64 arithmetic before checked conversion to the
control's f32 entry value. Infinity can occur within an expression, so
`exp(log(0))` and `dbtogain(-inf)` produce zero. A final nonfinite value is rejected
except negative infinity in a dB gain control. Invalid intermediate arithmetic
is rejected even if an enclosing operation or comma could hide it.

Evaluation is limited to 512 input bytes, 256 tokens, a parser recursion budget
of 32, 256 parsed operations, and at most 256 combination steps per call.
These bounds keep typing responsive; exceeding one leaves the patch unchanged.
Combinatoric functions retain the pinned evaluator's nonnegative truncation and
64-bit intermediate-overflow behavior, including `fac(21) = inf`.

Comparisons use tinyexpr commit `4e8cc0067a1e2378faae23eb2dfdd21e9e9907c2`,
Rack commit `8ea470f343058ade6f75f16d503a1eb330e1700c`, and an independently
reconstructed registration of Rack's note names and conversion functions.
This tests the isolated evaluator, not the complete Rack application or audio.
Finite values compare with relative tolerance `1e-12` and absolute tolerance
`1e-14`; bit-identical floating-point results are not promised.
The [recorded comparison](benchmarks/parameter-expression-comparison.json) checks
1,088 cases, with zero unexpected differences and three intentional corrections.

OpenPatch deliberately corrects the pinned evaluator's exponent grouping:
the reference produces −4 for `(-2)^2`, −1 for `(-1)^0`, and 16 for `2^-2^2`.
OpenPatch produces 4, 1, and 0.0625 respectively. Resource bounds and rejection of
invalid intermediate arithmetic are additional deliberate validation differences.
The native editor continues to reject out-of-range values rather than silently
clamping them.

Reproduce the comparison with Python 3, curl, Clang, and Rust installed:

```sh
python3 scripts/verify_parameter_expressions.py --output expression-comparison.json
```

The script downloads and verifies pinned reference sources into a temporary
directory. They are used only for verification and are not linked into OpenPatch.
