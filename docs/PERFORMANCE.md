# Replay Performance

Measured on 2026-09-30 using release builds before and after the replay
optimization. The baseline is commit `daf5356`. Both binaries used the same
local 50-session dataset, trading date `2026-07-10`, expiration `2026-07-17`,
micro pricing, classic dealer convention, and a 180-day surface horizon.

Each run started a separate process with empty application caches. OS file
caches were not flushed. These are local HTTP measurements, including JSON
serialization and transfer, not browser frame rates or a general hardware
performance guarantee. See [Development](DEVELOPMENT.md#performance-work) for
the reproducible command and parameter overrides.

| Request group | Baseline median | Optimized median | Speedup |
| --- | ---: | ---: | ---: |
| SPY, five consecutive new minutes | 1063.9 ms | 157.1–220.8 ms | 4.8–6.8x |
| SPY, five repeated requests for the last minute | 67.4 ms | 22.2–23.9 ms | 2.8–3.0x |
| QQQ, five consecutive new minutes | 937.5 ms | 242.6 ms | 3.9x |
| QQQ, five repeated requests for the last minute | 93.0 ms | 26.0 ms | 3.6x |

SPY ranges show medians from two optimized runs; QQQ was measured once.
The new-minute group spans `09:31` through `09:35`; its first request includes
initial underlying/OI loading. All 26 response trees (including the three
component routes per symbol) matched the baseline exactly, covering chain
rows, surface fits, historical IV, provenance, and quality gates. Raw responses
and timing reports are local ignored artifacts because they contain licensed
observations.

The changes remove work while retaining the same model:

- Parquet first filters the timestamp column to the requested New York date
  and minute, then decodes only the required quote columns for matching rows.
  Row order and null handling remain covered by synthetic fixtures. This also
  excludes stray rows from other dates that the previous `HH:MM` comparison
  would have accepted. Required schemas are checked even for empty results;
  unrelated, filtered-out rows are not fully decoded or validated.
- Historical ATM IV shares the full chain's forward, carry, pricing, IV
  validity, rounding, and tie selection rules. It stops after finding the
  nearest usable ATM candidate instead of calculating unused Greeks, SVI, and
  dealer scenarios for every historical session.
- SVI objective evaluations reuse invariant weights and density sampling
  points, with unchanged model, iteration count, and summation order.
- The playback scheduler includes request time in each frame period. It
  still commits a complete snapshot before advancing, retains one active
  request, and resets the period when playback resumes. If work takes longer
  than the selected interval, actual playback remains slower than 30 frames/s.

Validation included 43 Rust tests, 13 deterministic playback tests, two
adapter tests, Clippy, production builds, and API smoke checks. In-browser
30x playback kept the main time and snapshot time equal as they advanced;
pause and consecutive seeks to frames 150 and 5 settled on the final `09:35`
snapshot. No live provider or order execution was used in this benchmark.
