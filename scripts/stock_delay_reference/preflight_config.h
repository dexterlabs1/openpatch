/* SPDX-License-Identifier: MIT
 * Minimal platform configuration for a source-definedness preflight only.
 * No autotools/configure invocation or compiler feature probes are needed on
 * the supported C99 macOS/Linux hosts. Numerical oracle remains the SDK.
 */
#define HAVE_STDINT_H 1
#define HAVE_LRINT 1
#define HAVE_LRINTF 1
#define CPU_CLIPS_POSITIVE 0
#define CPU_CLIPS_NEGATIVE 0
#define PACKAGE "libsamplerate"
#define VERSION "0.1.9"
