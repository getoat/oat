"""Run Oat inside a Harbor task container with stuck-process cleanup."""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--log", required=True)
    parser.add_argument("--stats-dir", required=True)
    parser.add_argument("--poll-interval-seconds", type=float, default=1.0)
    parser.add_argument("--post-finish-grace-seconds", type=float, default=15.0)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        parser.error("missing command to execute")
    return args


def latest_finished_stats(stats_dir: Path) -> bool:
    if not stats_dir.exists():
        return False

    paths = sorted(
        stats_dir.glob("*.json"),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    for path in paths:
        try:
            with path.open("r", encoding="utf-8") as handle:
                payload = json.load(handle)
        except (OSError, ValueError, json.JSONDecodeError):
            continue
        if payload.get("finished_at_unix_ms"):
            return True
    return False


def wait_for_exit(process: subprocess.Popen[bytes], timeout_seconds: float) -> int | None:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        return_code = process.poll()
        if return_code is not None:
            return return_code
        time.sleep(0.1)
    return process.poll()


def append_runner_message(log_path: Path, message: str) -> None:
    line = f"[oat-runner] {message}\n".encode("utf-8", errors="replace")
    with log_path.open("ab") as handle:
        handle.write(line)


def main() -> int:
    args = parse_args()
    log_path = Path(args.log)
    stats_dir = Path(args.stats_dir)
    log_path.parent.mkdir(parents=True, exist_ok=True)

    with log_path.open("wb") as log_handle:
        process = subprocess.Popen(
            args.command,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            env=os.environ.copy(),
        )

    finished_seen_at: float | None = None
    forced_completion = False

    while True:
        return_code = process.poll()
        if return_code is not None:
            if forced_completion:
                return 0
            return return_code

        if latest_finished_stats(stats_dir):
            if finished_seen_at is None:
                finished_seen_at = time.monotonic()
                append_runner_message(
                    log_path,
                    "detected finished Oat stats; waiting for process exit",
                )
            elif (
                time.monotonic() - finished_seen_at
                >= args.post_finish_grace_seconds
            ):
                forced_completion = True
                append_runner_message(
                    log_path,
                    "Oat remained alive after stats finalization; sending SIGTERM",
                )
                process.terminate()
                if wait_for_exit(process, 5.0) is None:
                    append_runner_message(
                        log_path,
                        "process did not exit after SIGTERM; sending SIGKILL",
                    )
                    process.kill()
                    wait_for_exit(process, 5.0)
                return 0

        time.sleep(args.poll_interval_seconds)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(128 + signal.SIGINT)
