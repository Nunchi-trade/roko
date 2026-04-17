#!/usr/bin/env python3
"""Mechanically regenerate the MORI parity checklist against the current tree.

The verifier is intentionally heuristic. It prefers:
1. Exact current-file hits from `target:` code spans.
2. Crate-qualified symbol/module lookups from `target:` code spans.
3. Mori appendix path mappings.
4. Filtered fallback symbol grep from the checklist line.
"""

from __future__ import annotations

import argparse
import dataclasses
import functools
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable, Sequence


CHECKBOX_RE = re.compile(r"^(\s*-\s*\[[^\]]+\]\s*)(.*)$")
CODE_SPAN_RE = re.compile(r"`([^`]+)`")
APPENDIX_ROW_RE = re.compile(r"^\|(.+)\|(.+)\|(.+)\|$")
SYMBOL_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
MODULE_PATH_RE = re.compile(r"^(roko-[a-z0-9-]+)::([A-Za-z0-9_:*]+)$")
TITLE_RE = re.compile(r"\*\*(.*?)\*\*")
TITLE_SYMBOL_RE = re.compile(r"\b(?:[A-Z][A-Za-z0-9_]{3,}|[a-z]+_[a-z0-9_]{2,}(?:\(\))?)\b")
CURRENT_PATH_PREFIXES = ("crates/", "apps/", "tools/", "tests/")
IGNORE_TOKENS = {
    "*",
    "xhigh",
    "json",
    "mod",
}


@dataclasses.dataclass(frozen=True)
class VerificationResult:
    verified: bool
    evidence: str


@dataclasses.dataclass(frozen=True)
class SearchRoots:
    all_roots: tuple[Path, ...]
    repo_root: Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path.cwd())
    parser.add_argument("--checklist", type=Path, required=True)
    parser.add_argument("--appendix", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def code_spans(text: str) -> list[str]:
    return CODE_SPAN_RE.findall(text)


def normalize_token(token: str) -> str:
    return token.strip().strip(".,;:()[]")


def canonicalize_token(token: str) -> list[str]:
    token = normalize_token(token)
    candidates = [token]
    if "(" in token:
        candidates.append(token.split("(", 1)[0])
    if " " in token:
        candidates.append(token.split(" ", 1)[0])
    if "->" in token:
        candidates.append(token.split("->", 1)[0].strip())
    candidates.append(strip_call_syntax(token))
    return [candidate for candidate in dedupe_preserve_order(candidates) if candidate]


def strip_call_syntax(token: str) -> str:
    token = re.sub(r"\(.*\)$", "", token)
    token = re.sub(r"<.*>$", "", token)
    return token.strip()


def display_rel(path: Path, repo_root: Path) -> str:
    try:
        return path.relative_to(repo_root).as_posix()
    except ValueError:
        return path.as_posix()


def parse_appendix(path: Path) -> dict[str, tuple[str, ...]]:
    mapping: dict[str, tuple[str, ...]] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = APPENDIX_ROW_RE.match(line.strip())
        if not match:
            continue
        mori_col = match.group(1)
        roko_col = match.group(2)
        mori_refs = [token for token in code_spans(mori_col) if token.startswith("apps/mori/")]
        roko_refs = tuple(token for token in code_spans(roko_col) if token)
        if not mori_refs or not roko_refs:
            continue
        for mori_ref in mori_refs:
            mapping[mori_ref] = roko_refs
    return mapping


def find_existing_path(pattern: str, repo_root: Path) -> Path | None:
    candidate = repo_root / pattern
    if "*" in pattern:
        matches = sorted(repo_root.glob(pattern))
        return matches[0] if matches else None
    return candidate if candidate.exists() else None


def target_segment(line: str) -> str:
    if "target:" not in line:
        return ""
    return line.split("target:", 1)[1]


def target_tokens(line: str) -> list[str]:
    tokens: list[str] = []
    for token in code_spans(target_segment(line)):
        tokens.extend(canonicalize_token(token))
    return dedupe_preserve_order(tokens)


def mori_tokens(line: str) -> list[str]:
    return [normalize_token(token) for token in code_spans(line) if token.startswith("apps/mori/")]


def fallback_tokens(line: str) -> list[str]:
    tokens: list[str] = []
    for token in code_spans(line):
        for candidate in canonicalize_token(token):
            if (
                not candidate
                or candidate.startswith("apps/mori/")
                or candidate.startswith("mori-agents/")
                or candidate.endswith(".md")
                or candidate in IGNORE_TOKENS
            ):
                continue
            tokens.append(candidate)
    title_match = TITLE_RE.search(line)
    if title_match:
        for token in TITLE_SYMBOL_RE.findall(title_match.group(1)):
            normalized = strip_call_syntax(normalize_token(token))
            if normalized and normalized not in IGNORE_TOKENS:
                tokens.append(normalized)
    return tokens


def module_path_candidates(token: str) -> list[str]:
    sanitized = strip_call_syntax(token)
    match = MODULE_PATH_RE.match(sanitized)
    if not match:
        return []
    crate_name = match.group(1)
    module_segments = [segment for segment in match.group(2).split("::") if segment and segment != "*"]
    if not module_segments:
        return [f"crates/{crate_name}/src/lib.rs"]
    candidates: list[str] = []
    if len(module_segments) == 1 and (module_segments[0].islower() or "_" in module_segments[0]):
        candidates.append(f"crates/{crate_name}/src/{module_segments[0]}.rs")
        candidates.append(f"crates/{crate_name}/src/{module_segments[0]}/mod.rs")
    elif len(module_segments) > 1 and all(segment.islower() or "_" in segment for segment in module_segments[:-1]):
        module_only = module_segments[:-1]
        candidates.append(f"crates/{crate_name}/src/{'/'.join(module_only)}.rs")
        candidates.append(f"crates/{crate_name}/src/{'/'.join(module_only)}/mod.rs")
    else:
        candidates.append(f"crates/{crate_name}/src/lib.rs")
    return candidates


def module_symbol_candidates(token: str) -> tuple[list[str], list[str]]:
    sanitized = strip_call_syntax(token)
    match = MODULE_PATH_RE.match(sanitized)
    if not match:
        return ([], [])
    crate_name = match.group(1)
    parts = [segment for segment in match.group(2).split("::") if segment and segment != "*"]
    if not parts:
        return ([f"crates/{crate_name}"], [])
    search_roots = [f"crates/{crate_name}"]
    search_terms: list[str] = []
    for part in reversed(parts):
        if not is_generic_symbol(part):
            search_terms.append(part)
    search_terms.append("::".join(parts))
    return (search_roots, dedupe_preserve_order(search_terms))


def is_module_only_target(token: str) -> bool:
    sanitized = strip_call_syntax(token)
    match = MODULE_PATH_RE.match(sanitized)
    if not match:
        return False
    parts = [segment for segment in match.group(2).split("::") if segment and segment != "*"]
    if not parts:
        return True
    if len(parts) == 1:
        return True
    return parts[-1] == "*"


def is_generic_symbol(token: str) -> bool:
    return len(token) < 4 or token.lower() in IGNORE_TOKENS


def dedupe_preserve_order(items: Iterable[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for item in items:
        if item in seen:
            continue
        seen.add(item)
        ordered.append(item)
    return ordered


def run_rg(term: str, roots: Sequence[Path]) -> list[Path]:
    cmd = ["rg", "--files-with-matches", "--fixed-strings", "--no-heading", "-g", "!*.md", term]
    cmd.extend(str(root) for root in roots)
    result = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr.strip() or f"rg failed for {term!r}")
    if result.returncode == 1 or not result.stdout.strip():
        return []
    return sorted(Path(line.strip()) for line in result.stdout.splitlines() if line.strip())


@functools.lru_cache(maxsize=4096)
def cached_rg(term: str, roots_key: tuple[str, ...]) -> tuple[str, ...]:
    roots = [Path(root) for root in roots_key]
    return tuple(str(path) for path in run_rg(term, roots))


class Verifier:
    def __init__(self, repo_root: Path, appendix_map: dict[str, tuple[str, ...]]) -> None:
        self.repo_root = repo_root
        self.appendix_map = appendix_map
        self.search_roots = SearchRoots(
            all_roots=tuple(
                root
                for root in (
                    repo_root / "crates",
                    repo_root / "apps",
                    repo_root / "tools",
                    repo_root / "tests",
                )
                if root.exists()
            ),
            repo_root=repo_root,
        )

    def verify_line(self, line: str) -> VerificationResult:
        checks = (
            self.check_target_paths,
            self.check_target_modules,
            self.check_mori_mappings,
            self.check_fallback_symbols,
        )
        for check in checks:
            result = check(line)
            if result is not None:
                return result
        return VerificationResult(
            verified=False,
            evidence="verified: ❌ not found in code search roots (crates/, apps/, tools/, tests/)",
        )

    def check_target_paths(self, line: str) -> VerificationResult | None:
        for token in target_tokens(line):
            if token.startswith(CURRENT_PATH_PREFIXES):
                if path := find_existing_path(token, self.repo_root):
                    return VerificationResult(
                        verified=True,
                        evidence=f"verified: ✅ target path `{display_rel(path, self.repo_root)}` exists",
                    )
            if is_module_only_target(token):
                for candidate in module_path_candidates(token):
                    if path := find_existing_path(candidate, self.repo_root):
                        return VerificationResult(
                            verified=True,
                            evidence=f"verified: ✅ target module `{token}` maps to `{display_rel(path, self.repo_root)}`",
                        )
        return None

    def check_target_modules(self, line: str) -> VerificationResult | None:
        for token in target_tokens(line):
            roots, terms = module_symbol_candidates(token)
            if not terms:
                if SYMBOL_RE.match(token) and not is_generic_symbol(token):
                    roots = [display_rel(root, self.repo_root) for root in self.search_roots.all_roots]
                    terms = [token]
                else:
                    continue
            hit = self.search_terms(token, terms, roots)
            if hit is not None:
                return hit
        return None

    def check_mori_mappings(self, line: str) -> VerificationResult | None:
        for mori_token in mori_tokens(line):
            current_targets = self.appendix_map.get(mori_token, ())
            for current in current_targets:
                if path := find_existing_path(current, self.repo_root):
                    return VerificationResult(
                        verified=True,
                        evidence=(
                            f"verified: ✅ mori-ref `{mori_token}` maps to `{display_rel(path, self.repo_root)}`"
                        ),
                    )
        return None

    def check_fallback_symbols(self, line: str) -> VerificationResult | None:
        for token in fallback_tokens(line):
            if token.startswith(CURRENT_PATH_PREFIXES):
                if path := find_existing_path(token, self.repo_root):
                    return VerificationResult(
                        verified=True,
                        evidence=f"verified: ✅ fallback path `{display_rel(path, self.repo_root)}` exists",
                    )
                continue
            if token.startswith("roko-") and "::" in token:
                roots, terms = module_symbol_candidates(token)
                hit = self.search_terms(token, terms, roots)
                if hit is not None:
                    return hit
                continue
            if "/" in token and not token.endswith(".md"):
                hit = self.search_terms(token, [token], self.display_roots(self.search_roots.all_roots))
                if hit is not None:
                    return hit
                continue
            if SYMBOL_RE.match(token) and not is_generic_symbol(token):
                hit = self.search_terms(token, [token], self.display_roots(self.search_roots.all_roots))
                if hit is not None:
                    return hit
        return None

    def search_terms(
        self,
        display_token: str,
        terms: Sequence[str],
        relative_roots: Sequence[str],
    ) -> VerificationResult | None:
        roots = [self.repo_root / root for root in relative_roots]
        roots = [root for root in roots if root.exists()]
        if not roots:
            return None
        roots_key = tuple(str(root) for root in roots)
        for term in terms:
            if not term or is_generic_symbol(term):
                continue
            hits = cached_rg(term, roots_key)
            if not hits:
                continue
            first_hit = Path(hits[0])
            return VerificationResult(
                verified=True,
                evidence=(
                    f"verified: ✅ symbol `{term}` from `{display_token}` found in "
                    f"`{display_rel(first_hit, self.repo_root)}`"
                ),
            )
        return None

    def display_roots(self, roots: Sequence[Path]) -> list[str]:
        return [display_rel(root, self.repo_root) for root in roots]


def inject_summary(lines: list[str], total: int, verified: int, missing: int, percent: float) -> list[str]:
    summary = [
        "",
        "> Generated by `tools/mori-parity-check/verify.py`.",
        f"> Total items: {total}",
        f"> Verified ✅: {verified}",
        f"> Verified ❌: {missing}",
        f"> Verified completion: {percent:.1f}%",
        "",
    ]
    if lines and lines[0].startswith("# "):
        return [lines[0], *summary, *lines[1:]]
    return [*summary, *lines]


def main() -> int:
    args = parse_args()
    repo_root = args.repo_root.resolve()
    checklist_path = args.checklist.resolve()
    appendix_path = args.appendix.resolve()
    output_path = args.output.resolve()

    appendix_map = parse_appendix(appendix_path)
    verifier = Verifier(repo_root=repo_root, appendix_map=appendix_map)

    total = 0
    verified = 0
    rendered: list[str] = []

    for line in checklist_path.read_text(encoding="utf-8").splitlines():
        if not CHECKBOX_RE.match(line):
            rendered.append(line)
            continue
        result = verifier.verify_line(line)
        total += 1
        if result.verified:
            verified += 1
        rendered.append(f"{line} — {result.evidence}")

    missing = total - verified
    percent = (verified / total * 100.0) if total else 0.0
    final_lines = inject_summary(rendered, total=total, verified=verified, missing=missing, percent=percent)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(final_lines) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
