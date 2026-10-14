# SVF spectral baseline

This probe measures the **actual OpenPatch v2 engine** at runtime revision
`75e0e28` (SVF implementation introduced in `4b4bcd1`). It confirms a substantial
aliasing gap in the host-rate nonlinear drive. It is not a Rack comparison,
physical-hardware validation, or a passing DSP-quality certificate.

## Reproduce

From the repository root, with Rust and Python/NumPy installed:

```sh
cargo run --release --locked --manifest-path docs/benchmarks/svf-spectral/Cargo.toml -- native-test-output/svf-spectrum
python docs/benchmarks/svf-spectral/analyze.py --traces native-test-output/svf-spectrum --output native-test-output/svf-spectrum-summary.json
```

Use fresh output paths. The exporter and analyzer refuse to replace existing
trace/report files. The probe opens no window or audio device. Its separate Cargo
workspace and lockfile keep analysis dependencies independent of the application.

The graph is SINE → GAIN → SVF. Taps capture low/high outputs and the raw engine
output captures bandpass, before device clipping or WAV quantization. Parameters:

- 44.1, 48 and 96 kHz; 8192-sample coherent periods, bins 43/171/683/1501.
- 0.1/1/5 V peak input, drive 0/0.1/0.5/1, cutoff 12 kHz, resonance 0.2.
- Eight periods per case; export the last two and analyze the last. 144 cases.

The input frequencies are exactly representable in the engine's f32 parameter.
There is no FFT window. The analysis sums Parseval-weighted energy outside DC
and all legal in-band harmonic bins. This is an **alias-energy lower bound**:
aliases landing on legal harmonics are excluded. It does not measure total
harmonic distortion, modulation, or accuracy of the intended waveform.
Frequencies scale with sample rate; these are not matched-frequency comparisons
between rates.

## Recorded result

[baseline.json](baseline.json) records every case and trace SHA-256. The portable
probe reproduced all 144 traces byte for byte on the local ARM64 machine.
All cases passed the Parseval check; the last two periods were identical.
An injected off-harmonic calibration tone measured its expected RMS. The maximum
linear-control off-harmonic energy was −151.57 dB relative to 5 V peak.

Selected 48 kHz, 5 V peak cases, **off-harmonic RMS dB relative to 5 V peak**
(more negative is lower unwanted energy):

| Input frequency | Drive | Lowpass | Bandpass | Highpass |
| --- | ---: | ---: | ---: | ---: |
| 1001.95 Hz | 1.0 | −61.07 | −57.20 | −45.15 |
| 4001.95 Hz | 0.1 | −62.31 | −56.66 | −46.12 |
| 4001.95 Hz | 1.0 | −22.79 | −25.25 | −19.14 |
| 8794.92 Hz | 0.1 | −32.34 | −33.84 | −18.34 |
| 8794.92 Hz | 1.0 | −15.29 | −20.53 | −10.37 |

These results establish a defect to address, not an acceptable alias budget.
Next work must specify a target, add suitable antialiasing to the driven path,
and rerun this matrix alongside independent response/modulation tests and
matched-quality timing/latency measurements. A low result here alone cannot
prove the complete signal is correct.
