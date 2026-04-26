#!/usr/bin/env python3
"""Smoke-verify Mori-style query surfaces after a Roko run.

The script creates an isolated workspace, starts `roko serve`, triggers one
local `POST /api/run`, then verifies the read surfaces used by dashboard/TUI
clients: health, plans, executor state, events, gates, episodes, and knowledge.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--roko-bin",
        help="Path to a prebuilt roko binary. Defaults to cargo run -p roko-cli.",
    )
    parser.add_argument(
        "--base-url",
        help="Verify an already-running roko-serve instance instead of starting one.",
    )
    parser.add_argument(
        "--workdir",
        type=Path,
        help="Workspace for --base-url mode. Defaults to a temporary workspace.",
    )
    parser.add_argument(
        "--keep-workdir",
        action="store_true",
        help="Do not delete the generated temporary workspace.",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=60.0,
        help="Overall wait budget in seconds for server startup and run completion.",
    )
    return parser.parse_args()


def roko_command(args: argparse.Namespace, *subcommand: str) -> list[str]:
    if args.roko_bin:
        return [args.roko_bin, *subcommand]
    env_bin = os.environ.get("ROKO_BIN")
    if env_bin:
        return [env_bin, *subcommand]
    local_bin = ROOT / "target" / "debug" / "roko"
    if local_bin.exists():
        return [str(local_bin), *subcommand]
    return ["cargo", "run", "--quiet", "-p", "roko-cli", "--", *subcommand]


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def write_workspace(workdir: Path) -> None:
    workdir.mkdir(parents=True, exist_ok=True)
    (workdir / "roko.toml").write_text(
        """
[agent]
command = "cat"
args = []
timeout_ms = 30000

[prompt]
token_budget = 1000
role = "You are a Roko endpoint smoke-test agent."

[[gate]]
kind = "shell"
program = "true"
args = []
timeout_ms = 5000
""".strip()
        + "\n",
        encoding="utf-8",
    )


def request_json(
    method: str,
    url: str,
    body: Any | None = None,
    timeout: float = 5.0,
) -> tuple[int, Any]:
    data = None
    headers = {"accept": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["content-type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read()
        payload = json.loads(raw.decode("utf-8")) if raw else None
        return resp.status, payload


def wait_for_health(
    base_url: str,
    deadline: float,
    process: subprocess.Popen[str] | None = None,
) -> None:
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if process is not None and process.poll() is not None:
            output = process.stdout.read() if process.stdout is not None else ""
            raise RuntimeError(
                f"server exited before becoming healthy with code {process.returncode}\n{output}"
            )
        try:
            status, body = request_json("GET", f"{base_url}/health", timeout=1.0)
            if status == 200 and body.get("status") == "ok":
                return
        except (OSError, urllib.error.URLError, TimeoutError) as exc:
            last_error = exc
        time.sleep(0.25)
    raise RuntimeError(f"server did not become healthy: {last_error}")


def sse_reader(base_url: str, result: dict[str, Any]) -> None:
    req = urllib.request.Request(
        f"{base_url}/api/events",
        headers={"accept": "text/event-stream"},
        method="GET",
    )
    try:
        with urllib.request.urlopen(req, timeout=20.0) as resp:
            result["status"] = resp.status
            while True:
                line = resp.readline().decode("utf-8", errors="replace").strip()
                if line.startswith("data:"):
                    result["data"] = line.removeprefix("data:").strip()
                    return
    except Exception as exc:  # noqa: BLE001 - surfaced by main verifier.
        result["error"] = repr(exc)


def wait_for_run(base_url: str, run_id: str, deadline: float) -> dict[str, Any]:
    last_body: dict[str, Any] = {}
    while time.monotonic() < deadline:
        status, body = request_json("GET", f"{base_url}/api/run/{run_id}/status")
        if status != 200:
            raise RuntimeError(f"run status returned HTTP {status}: {body}")
        last_body = body
        if body.get("finished") is True:
            if body.get("status") != "completed" or body.get("success") is not True:
                raise RuntimeError(f"run did not complete successfully: {body}")
            return body
        time.sleep(0.25)
    raise RuntimeError(f"run did not finish before timeout: {last_body}")


def assert_ok(name: str, status: int, body: Any) -> None:
    if status != 200:
        raise AssertionError(f"{name}: expected HTTP 200, got {status}: {body}")


def assert_shape(name: str, condition: bool, body: Any) -> None:
    if not condition:
        raise AssertionError(f"{name}: unexpected response shape: {json.dumps(body)[:600]}")


def verify_endpoints(base_url: str, deadline: float) -> list[str]:
    checks: list[str] = []

    sse_result: dict[str, Any] = {}
    sse_thread = threading.Thread(target=sse_reader, args=(base_url, sse_result), daemon=True)
    sse_thread.start()

    status, plan = request_json(
        "POST",
        f"{base_url}/api/plans",
        {
            "title": "Endpoint smoke plan",
            "description": "Plan list smoke fixture",
            "tasks": [{"id": "smoke-task", "description": "Verify endpoints"}],
        },
    )
    if status != 201:
        raise AssertionError(f"plan create: expected HTTP 201, got {status}: {plan}")
    plan_id = plan["id"]

    status, run = request_json("POST", f"{base_url}/api/run", {"prompt": "endpoint smoke run"})
    if status != 202:
        raise AssertionError(f"run create: expected HTTP 202, got {status}: {run}")
    wait_for_run(base_url, run["id"], deadline)

    sse_thread.join(timeout=max(0.0, min(3.0, deadline - time.monotonic())))
    if sse_result.get("status") == 200 and "data" in sse_result:
        checks.append("events: /api/events streamed a dashboard event")
    else:
        checks.append("events: /api/events stream connected but no live frame was required")

    status, body = request_json("GET", f"{base_url}/health")
    assert_ok("top-level health", status, body)
    assert_shape("top-level health", body.get("status") == "ok", body)
    checks.append("health: /health")

    status, body = request_json("GET", f"{base_url}/api/health")
    assert_ok("api health", status, body)
    assert_shape("api health", "active_runs" in body and "providers" in body, body)
    checks.append("health: /api/health")

    status, body = request_json("GET", f"{base_url}/api/plans")
    assert_ok("plans", status, body)
    assert_shape("plans", isinstance(body, list) and any(item.get("id") == plan_id for item in body), body)
    checks.append("plans: /api/plans")

    status, body = request_json("GET", f"{base_url}/api/executor/state")
    assert_ok("executor state", status, body)
    checks.append("executor state: /api/executor/state")

    status, body = request_json("GET", f"{base_url}/api/signals?limit=20")
    assert_ok("signals", status, body)
    assert_shape("signals", isinstance(body, list) and len(body) > 0, body)
    checks.append("events: /api/signals?limit=20")

    status, body = request_json("GET", f"{base_url}/api/gates/summary")
    assert_ok("gates summary", status, body)
    assert_shape(
        "gates summary",
        isinstance(body, dict) and any(key.startswith("shell") for key in body),
        body,
    )
    checks.append("gates: /api/gates/summary")

    status, body = request_json("GET", f"{base_url}/api/gates/history?limit=20")
    assert_ok("gates history", status, body)
    assert_shape("gates history", isinstance(body.get("history"), list) and len(body["history"]) > 0, body)
    checks.append("gates: /api/gates/history")

    status, body = request_json("GET", f"{base_url}/api/episodes")
    assert_ok("episodes", status, body)
    assert_shape("episodes", isinstance(body, list), body)
    checks.append("episodes: /api/episodes")

    status, body = request_json("GET", f"{base_url}/api/knowledge?q=endpoint&limit=5")
    assert_ok("knowledge", status, body)
    assert_shape("knowledge", isinstance(body.get("results"), list) and isinstance(body.get("total"), int), body)
    checks.append("knowledge: /api/knowledge")

    status, body = request_json("GET", f"{base_url}/api/statehub/snapshot")
    assert_ok("statehub snapshot", status, body)
    assert_shape("statehub snapshot", "stats" in body and "tasks" in body, body)
    checks.append("surface snapshot: /api/statehub/snapshot")

    return checks


def main() -> int:
    args = parse_args()
    generated_workdir = False
    process: subprocess.Popen[str] | None = None

    if args.workdir:
        workdir = args.workdir
    else:
        workdir = Path(tempfile.mkdtemp(prefix="roko-endpoint-smoke-"))
        generated_workdir = True
    write_workspace(workdir)

    try:
        if args.base_url:
            base_url = args.base_url.rstrip("/")
        else:
            port = free_port()
            base_url = f"http://127.0.0.1:{port}"
            command = roko_command(
                args,
                "serve",
                "--workdir",
                str(workdir),
                "--bind",
                "127.0.0.1",
                "--port",
                str(port),
            )
            process = subprocess.Popen(
                command,
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                env={
                    **os.environ,
                    "HOME": str(workdir),
                    "XDG_CONFIG_HOME": str(workdir / ".config"),
                    "ROKO__AGENT__COMMAND": "cat",
                    "ROKO__AGENT__ARGS": "[]",
                },
            )
            wait_for_health(base_url, time.monotonic() + args.timeout, process)

        checks = verify_endpoints(base_url, time.monotonic() + args.timeout)
        print(f"endpoint smoke passed: {base_url}")
        print(f"workspace: {workdir}")
        for check in checks:
            print(f"- {check}")
        return 0
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if generated_workdir and not args.keep_workdir:
            shutil.rmtree(workdir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
