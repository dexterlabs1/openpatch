# Mutable Instruments Braids DSP

MIT-licensed DSP by Emilie Gillet. Copyright and license notices are retained in each file.

Sources: https://github.com/pichenettes/eurorack at f4cfa7697095a28d292bff758beb51deb49b5a86 (braids) and https://github.com/pichenettes/stmlib at d18def816c51d1da0c108236928b2bbd25c17481.

Host adaptation: the stmlib random generator uses an instance field in each DigitalOscillator, rather than global storage. Seed forwarding methods are added to the firmware classes. This avoids both cross-engine data races and lazy thread-local allocation in the audio callback; the random recurrence and synthesis algorithms are unchanged. The C++ wrapper adds allocation, reset, state copy and a fixed 24-sample render boundary. Braids runs internally at 96 kHz; the host band-limits and resamples its output. This is the original firmware synthesis engine, not the complete hardware firmware or Rack plugin.
