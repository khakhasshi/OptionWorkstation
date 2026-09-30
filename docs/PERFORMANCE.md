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

## Visualization performance

The visualization pass on 2026-09-30 compares the previous frontend at
`bf5958f` with the new workspace UI. Browser checks used the same local
release backend, SPY `2026-07-10`, expiration `2026-07-17`, micro pricing,
classic dealer convention, and a 1280 x 800 viewport.

| Observation | Before | After |
| --- | ---: | ---: |
| Mounted chart components in overview | 8 | 4 |
| Mounted chain data rows at 09:35, default filters | 150 | 0 in overview; 16 in trading |
| Inactive surface renderer in overview | Mounted | Absent |

These are DOM/work-count measurements, not FPS or latency speedup claims.
The 16-row count depends on viewport height and scroll position; the table
keeps the same 150 filtered strikes and mounts a bounded visible window with
six overscan rows per side. The data and the backend analytics are not sampled
or reduced by this change.

During a 30x replay in the trading workspace, the fully offscreen market chart
stayed at option version 2 while the partially visible exposure chart advanced
from version 3 to 66. The tape and snapshot both reached 10:38. Scrolling the
market chart back into view applied the latest snapshot once (version 3),
without replaying hidden intermediate renders. All chart consumers, including
live mode, use this scheduling path. Live-provider throughput was not measured.

The chart scheduler merges option/resize work into one pending animation
frame, retains the latest input while hidden, and defers renderer creation
until visible with usable dimensions. Callback bindings remain stable, and
unmount cancels pending work. Interactive zoom and 3D camera state survive
ordinary data updates; changing the chart's data context resets the view.

The volatility workspace starts with a 2D heatmap. ECharts GL remains a
separate on-demand bundle; it is still roughly 1 MB uncompressed when loaded.
3D displays all supplied observations instead of the former every-fourth-point
sample. Automatic bounds include the displayed data; the explicit fixed
0-150% option supports comparable scales but may clip outliers. Auto bounds
can change between frames, so use the fixed range for visual comparisons.

Validation: 43 Rust tests, 27 frontend tests, two adapter tests, Clippy,
production build, and API smoke checks passed. Interactive browser checks
covered high-speed snapshot consistency, deferred hidden-chart updates,
virtual-chain End/Home and repeated boundary keys, filter focus, adding a leg
and obtaining a risk preview, 2D/3D switching, camera preservation, and
responsive layouts. The browser smoke script was updated and syntax checked;
interactive checks were performed through the in-app browser instead of
executing that script. No provider connection or order execution was used.
