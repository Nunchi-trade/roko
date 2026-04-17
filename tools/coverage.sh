#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

cargo llvm-cov --html --output-dir target/llvm-cov --ignore-filename-regex='(tests/|target/|testdata/|benches/)' --ignore-run-fail

if [[ -f target/llvm-cov/html/index.html && ! -f target/llvm-cov/index.html ]]; then
  cp -R target/llvm-cov/html/. target/llvm-cov/
fi

if command -v open >/dev/null 2>&1; then
  open target/llvm-cov/index.html >/dev/null 2>&1 || true
fi
