#!/usr/bin/env python3
"""Measure an isolated release binary and optionally compare every JSON result.

Market-data responses stay in the chosen output directory (use artifacts/).
No broker, order, or external API endpoints are called.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import socket
import statistics
import subprocess
import time
from urllib.parse import urlencode
from urllib.request import ProxyHandler, build_opener


HTTP = build_opener(ProxyHandler({}))


def request(base_url, path, params=None):
    url = base_url + path + ("?" + urlencode(params) if params else "")
    start = time.perf_counter()
    with HTTP.open(url, timeout=180) as response:
        raw = response.read()
    elapsed_ms = (time.perf_counter() - start) * 1000
    return json.loads(raw), elapsed_ms, len(raw)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--compare", type=Path, help="Previous output directory")
    parser.add_argument("--symbol", default="SPY")
    parser.add_argument("--date", required=True)
    parser.add_argument("--expiration", required=True)
    parser.add_argument("--minutes", default="09:31,09:32,09:33,09:34,09:35")
    parser.add_argument("--pricing-mode", default="micro", choices=["mid", "micro", "ask"])
    parser.add_argument("--dealer-model", default="classic", choices=["classic", "short_all", "long_all"])
    parser.add_argument("--max-dte", type=int, default=180)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "responses").mkdir()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    env = {
        "PATH": os.defpath,
        "RUST_LOG": "warn",
        "OPTION_WORKSTATION_HOST": "127.0.0.1",
        "OPTION_WORKSTATION_PORT": str(port),
        "OPTION_WORKSTATION_DATA_ROOT": str(args.data_root.resolve()),
        "OPTION_WORKSTATION_AUDIT_PATH": str((args.output / "audit.jsonl").resolve()),
        "OPTION_WORKSTATION_PAPER_ORDER_EXECUTION": "0",
    }
    params = dict(symbol=args.symbol, date=args.date, expiration=args.expiration,
                  pricing_mode=args.pricing_mode, dealer_model=args.dealer_model, max_dte=args.max_dte)
    minutes = args.minutes.split(",")
    cases = [("sequential", "/api/v1/replay/snapshot", minute) for minute in minutes]
    cases += [("repeat", "/api/v1/replay/snapshot", minutes[-1])] * 5
    cases += [(name, path, "12:53") for name, path in [
        ("chain", "/api/chain"), ("surface", "/api/surface"),
        ("volatility", "/api/volatility-context"),
    ]]
    previous = None
    if args.compare:
        previous = json.loads((args.compare / "report.json").read_text())
        if previous["params"] != params or previous["minutes"] != minutes:
            raise ValueError("Comparison requires identical parameters and minutes")
        expected_cases = [(sample["group"], sample["endpoint"], sample["minute"])
                          for sample in previous["samples"]]
        if expected_cases != cases:
            raise ValueError("Comparison requires an identical request sequence")
    samples = []
    mismatches = []
    with (args.output / "server.log").open("w") as log:
        process = subprocess.Popen([str(args.binary.resolve())], env=env,
                                   stdout=log, stderr=subprocess.STDOUT)
        base_url = f"http://127.0.0.1:{port}"
        try:
            deadline = time.monotonic() + 30
            while True:
                if process.poll() is not None:
                    raise RuntimeError("Server exited; see server.log")
                try:
                    request(base_url, "/api/health")
                    break
                except OSError:
                    if time.monotonic() > deadline:
                        raise
                    time.sleep(0.05)
            for index, (group, endpoint, minute) in enumerate(cases):
                value, elapsed_ms, size = request(base_url, endpoint, dict(params, minute=minute))
                payload = canonical(value)
                filename = f"responses/{index:02d}-{group}-{minute.replace(':', '')}.json"
                (args.output / filename).write_text(payload + "\n")
                sample = dict(group=group, endpoint=endpoint, minute=minute,
                              elapsed_ms=round(elapsed_ms, 3), bytes=size,
                              sha256=hashlib.sha256(payload.encode()).hexdigest(), response=filename)
                samples.append(sample)
                if previous:
                    reference = previous["samples"][index]
                    # Parse and compare the complete tree; formatting or object key order is irrelevant.
                    expected = json.loads((args.compare / reference["response"]).read_text())
                    if payload != canonical(expected):
                        mismatches.append(filename)
                print(f"{group:10s} {minute} {elapsed_ms:9.3f} ms", flush=True)
            rss_kb = int(subprocess.check_output(["ps", "-o", "rss=", "-p", str(process.pid)], text=True))
            groups = {}
            for group in dict.fromkeys(sample["group"] for sample in samples):
                timings = [sample["elapsed_ms"] for sample in samples if sample["group"] == group]
                groups[group] = dict(count=len(timings), median_ms=round(statistics.median(timings), 3),
                                     min_ms=min(timings), max_ms=max(timings))
            report = dict(params=params, minutes=minutes, samples=samples, groups=groups,
                          binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                          process_rss_kb=rss_kb, comparison_mismatches=mismatches,
                          compared_to=str(args.compare) if args.compare else None,
                          note="Fresh process/application caches; OS file cache is not flushed. HTTP timings include serialization and transfer.")
            (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(dict(groups=groups, process_rss_kb=rss_kb,
                                  comparison_mismatches=mismatches), indent=2))
        finally:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
    if mismatches:
        raise SystemExit("Response mismatch: " + ", ".join(mismatches))


if __name__ == "__main__":
    main()
