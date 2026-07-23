# Example Replay Data

The repository is source code only. Market-data files under `data/` are ignored
by Git and are intended for local demonstrations.

To materialize the local two-session example set from an existing ThetaData-style
replay archive:

```bash
./scripts/materialize-example-data.sh
OPTION_WORKSTATION_DATA_ROOT="$PWD/data/example" ./scripts/start.sh
```

Default source:

```text
~/MarketData/thetadata-options-2026-last-50d
```

Default copied universe:

```text
AAPL AMZN GOOGL META MSFT NVDA QQQ SPY TSLA
```

Default copied sessions:

```text
2026-07-09
2026-07-10
```

You can override the copied universe or sessions without editing the script:

```bash
OPTION_WORKSTATION_EXAMPLE_SYMBOLS="QQQ SPY NVDA" \
OPTION_WORKSTATION_EXAMPLE_DATES="2026-07-09 2026-07-10" \
./scripts/materialize-example-data.sh
```

The target can also be supplied explicitly:

```bash
./scripts/materialize-example-data.sh /path/to/source /path/to/target
```
