#!/usr/bin/env python3
"""Visualize cascade router learning from roko's learning state files.

Usage:
    python3 04-visualize-learning.py [LEARN_DIR]
    python3 04-visualize-learning.py .roko/learn/

Reads:
    - cascade-router.json  — router state (stage, per-model stats)
    - efficiency.jsonl     — per-turn cost/quality/latency events

Outputs ASCII charts to stdout. If matplotlib is available, also saves PNGs.
"""

import json
import os
import sys
from collections import defaultdict
from pathlib import Path


def load_json(path: Path):
    """Load a JSON file, return None on failure."""
    try:
        with open(path) as f:
            return json.load(f)
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def load_jsonl(path: Path) -> list:
    """Load a JSONL file, return list of dicts."""
    records = []
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        records.append(json.loads(line))
                    except json.JSONDecodeError:
                        pass
    except FileNotFoundError:
        pass
    return records


def ascii_bar(label: str, value: float, max_value: float, width: int = 40) -> str:
    """Render a single ASCII bar."""
    if max_value <= 0:
        filled = 0
    else:
        filled = int(value / max_value * width)
    bar = "\u2588" * filled + "\u2591" * (width - filled)
    return f"  {label:<20} \u2502{bar}\u2502 {value:.0f}"


def ascii_bar_chart(title: str, data: dict[str, float]):
    """Print an ASCII bar chart."""
    if not data:
        print(f"\n  {title}: no data\n")
        return
    max_val = max(data.values()) if data.values() else 1
    print(f"\n  {title}")
    print(f"  {'\u2500' * 66}")
    for label, value in sorted(data.items(), key=lambda x: -x[1]):
        print(ascii_bar(label, value, max_val))
    print()


def ascii_line_chart(title: str, values: list[float], width: int = 60, height: int = 12):
    """Print a simple ASCII line chart."""
    if not values:
        print(f"\n  {title}: no data\n")
        return

    min_v = min(values)
    max_v = max(values)
    val_range = max_v - min_v if max_v > min_v else 1

    print(f"\n  {title}")
    print(f"  {'\u2500' * (width + 8)}")

    # Downsample if needed
    if len(values) > width:
        step = len(values) / width
        sampled = [values[int(i * step)] for i in range(width)]
    else:
        sampled = values

    # Build grid
    for row in range(height - 1, -1, -1):
        threshold = min_v + (row / (height - 1)) * val_range
        line_label = f"{threshold:6.2f} \u2502"
        chars = []
        for v in sampled:
            v_row = int((v - min_v) / val_range * (height - 1))
            if v_row == row:
                chars.append("\u25cf")
            elif v_row > row:
                chars.append("\u2502")
            else:
                chars.append(" ")
        print(f"  {line_label}{''.join(chars)}")
    print(f"  {'':>7}\u2514{'\u2500' * len(sampled)}")
    print(f"  {'':>8}1{' ' * (len(sampled) - 2)}{len(values)}")
    print()


def visualize_router(router_data: dict):
    """Visualize cascade router state."""
    if not router_data:
        print("  No cascade router data found.\n")
        return

    stage = router_data.get("stage", "unknown")
    observations = router_data.get("total_observations", 0)
    models = router_data.get("models", router_data.get("model_stats", {}))

    print(f"\n  \u250c\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2510")
    print(f"  \u2502  Cascade Router State                       \u2502")
    print(f"  \u251c\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2524")
    print(f"  \u2502  Stage:        {stage:<29}\u2502")
    print(f"  \u2502  Observations: {observations:<29}\u2502")
    print(f"  \u2514\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2500\u2518")

    if isinstance(models, dict):
        # Selection frequency
        selection_counts = {}
        pass_rates = {}
        avg_costs = {}

        for model, stats in models.items():
            if isinstance(stats, dict):
                selection_counts[model] = stats.get("selections", stats.get("count", 0))
                total = stats.get("total", stats.get("selections", 1))
                passed = stats.get("passed", stats.get("successes", 0))
                pass_rates[model] = (passed / total * 100) if total > 0 else 0
                avg_costs[model] = stats.get("avg_cost", stats.get("mean_cost", 0))

        ascii_bar_chart("Model Selection Frequency", selection_counts)
        ascii_bar_chart("Pass Rate (%)", pass_rates)
        if any(v > 0 for v in avg_costs.values()):
            ascii_bar_chart("Average Cost ($)", avg_costs)


def visualize_efficiency(events: list):
    """Visualize efficiency events over time."""
    if not events:
        print("  No efficiency events found.\n")
        return

    # Extract pass rates over time (windowed)
    window = 5
    pass_history = []
    for i, event in enumerate(events):
        passed = event.get("gate_passed", event.get("passed", False))
        start = max(0, i - window + 1)
        window_events = events[start:i + 1]
        window_passes = sum(1 for e in window_events if e.get("gate_passed", e.get("passed", False)))
        pass_history.append(window_passes / len(window_events) * 100)

    ascii_line_chart(f"Pass Rate Over Time (window={window})", pass_history)

    # Per-model cost breakdown
    model_costs = defaultdict(list)
    for event in events:
        model = event.get("model", event.get("model_key", "unknown"))
        cost = event.get("cost", event.get("total_cost", 0))
        if cost:
            model_costs[model].append(cost)

    if model_costs:
        avg_costs = {m: sum(c) / len(c) for m, c in model_costs.items()}
        ascii_bar_chart("Average Cost by Model ($)", avg_costs)

    # Summary stats
    total_cost = sum(e.get("cost", e.get("total_cost", 0)) or 0 for e in events)
    total_passed = sum(1 for e in events if e.get("gate_passed", e.get("passed", False)))
    print(f"  Summary: {len(events)} events, {total_passed} passed ({total_passed/len(events)*100:.1f}%), total cost ${total_cost:.4f}")
    print()


def try_matplotlib(router_data: dict, events: list, output_dir: Path):
    """Generate PNG charts if matplotlib is available."""
    try:
        import matplotlib
        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError:
        print("  [info] matplotlib not installed — skipping PNG generation")
        print("  [info] Install with: pip install matplotlib")
        return

    output_dir.mkdir(parents=True, exist_ok=True)

    # 1. Selection frequency pie chart
    if router_data:
        models = router_data.get("models", router_data.get("model_stats", {}))
        if isinstance(models, dict):
            labels = []
            sizes = []
            for model, stats in models.items():
                if isinstance(stats, dict):
                    count = stats.get("selections", stats.get("count", 0))
                    if count > 0:
                        labels.append(model)
                        sizes.append(count)
            if labels:
                fig, ax = plt.subplots(figsize=(8, 6))
                ax.pie(sizes, labels=labels, autopct="%1.1f%%", startangle=90)
                ax.set_title("Model Selection Frequency")
                fig.savefig(output_dir / "selection-frequency.png", dpi=150, bbox_inches="tight")
                plt.close(fig)
                print(f"  [ok] Saved {output_dir / 'selection-frequency.png'}")

    # 2. Pass rate trajectory
    if events:
        window = 5
        pass_rates = []
        for i in range(len(events)):
            start = max(0, i - window + 1)
            w = events[start:i + 1]
            passes = sum(1 for e in w if e.get("gate_passed", e.get("passed", False)))
            pass_rates.append(passes / len(w) * 100)

        fig, ax = plt.subplots(figsize=(10, 4))
        ax.plot(range(1, len(pass_rates) + 1), pass_rates, "b-", linewidth=1.5)
        ax.set_xlabel("Iteration")
        ax.set_ylabel("Pass Rate (%)")
        ax.set_title(f"Pass Rate Over Time (window={window})")
        ax.set_ylim(0, 105)
        ax.grid(True, alpha=0.3)
        fig.savefig(output_dir / "pass-rate.png", dpi=150, bbox_inches="tight")
        plt.close(fig)
        print(f"  [ok] Saved {output_dir / 'pass-rate.png'}")

    # 3. Cumulative cost
    if events:
        costs = [e.get("cost", e.get("total_cost", 0)) or 0 for e in events]
        cumulative = []
        total = 0
        for c in costs:
            total += c
            cumulative.append(total)

        fig, ax = plt.subplots(figsize=(10, 4))
        ax.plot(range(1, len(cumulative) + 1), cumulative, "r-", linewidth=1.5)
        ax.set_xlabel("Iteration")
        ax.set_ylabel("Cumulative Cost ($)")
        ax.set_title("Cumulative Cost Over Time")
        ax.grid(True, alpha=0.3)
        fig.savefig(output_dir / "cumulative-cost.png", dpi=150, bbox_inches="tight")
        plt.close(fig)
        print(f"  [ok] Saved {output_dir / 'cumulative-cost.png'}")


def main():
    learn_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(".roko/learn")

    if not learn_dir.exists():
        print(f"Error: directory not found: {learn_dir}")
        print(f"Usage: {sys.argv[0]} [LEARN_DIR]")
        sys.exit(1)

    print(f"  Learning data: {learn_dir.resolve()}")

    # Load data
    router_data = load_json(learn_dir / "cascade-router.json")
    efficiency_events = load_jsonl(learn_dir / "efficiency.jsonl")

    print(f"  Router state:  {'found' if router_data else 'not found'}")
    print(f"  Efficiency:    {len(efficiency_events)} events")

    # ASCII visualizations
    visualize_router(router_data)
    visualize_efficiency(efficiency_events)

    # Optional PNG output
    try_matplotlib(router_data, efficiency_events, learn_dir.parent / "results")


if __name__ == "__main__":
    main()
