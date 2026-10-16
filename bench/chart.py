#!/usr/bin/env python3
"""Draw the README's chart from a recorded benchmark result.

Two panels of three bars: tsrs's, bun check's and tsgo's check time and peak memory, each summed over the projects
of bench/projects.json, all three at the same thread count on the recorded machine (4, tsgo's default, when the result
has it). Shorter is better.
"""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET

import run


# Drawn top to bottom; bun check only when the result measured it on every project.
COMPILERS = ("tsrs", "bun", "tsgo")
NAMES = {"tsrs": "tsrs", "bun": "bun check", "tsgo": "tsgo"}
# The Bun comparison equal-thread modes (bench/run.py WIDE_CHECKER_MODES; bench/publish.py publishes their tables).
POINTS = run.WIDE_CHECKER_MODES
# The mode a result must have for the chart to be drawn (bench/publish.py). The chart uses the fewest threads every
# project was measured at: 4 since the 4-thread mode exists, 16 for older results.
MODE = POINTS[16]
PANELS = (("wall_s", "Check time", "faster"), ("peak_rss_bytes", "Peak memory", "less memory"))
# The README shows the light or dark chart to match the reader's GitHub theme.
THEMES = {
    "light": {"tsrs": "#087b5b", "bun": "#d99a1e", "tsgo": "#3178c6", "background": "#ffffff", "border": "#e5e7eb",
              "text": "#111827", "muted": "#6b7280", "faint": "#9ca3af", "track": "#f3f4f6"},
    "dark": {"tsrs": "#3fb98c", "bun": "#e3b341", "tsgo": "#58a6ff", "background": "#0d1117", "border": "#30363d",
             "text": "#e6edf3", "muted": "#9198a1", "faint": "#6e7681", "track": "#21262d"},
}
GIB = 2**30
WIDTH, HEIGHT = 1200, 500
FONT = "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"
# One panel: its left edge, the bars' start and full length, the first bar's center, the row pitch and bar height.
PANEL_X, BAR_X, BAR_LENGTH, ROW_Y, ROW_PITCH, BAR_HEIGHT = (48, 624), 128, 290, 228, 92, 30


def measured(cell: dict, compiler: str, metric: str) -> float | None:
    sample = cell.get(compiler) or {}
    value = sample.get(metric)
    return value if (sample.get("ok_runs") and isinstance(value, (int, float))
                     and math.isfinite(value) and value > 0) else None


def format_value(metric: str, value: float | None) -> str:
    if value is None:
        return "n/a"
    return run.fmt_s(value) if metric == "wall_s" else f"{value / GIB:.2f} GiB"


def total_text(metric: str, value: float) -> str:
    return f"{value:.1f} s" if metric == "wall_s" else f"{value / GIB:.1f} GiB"


def chart_mode(result: dict) -> str:
    """The fewest equal threads every project has a tsgo and tsrs measurement at."""
    for checkers in sorted(POINTS):
        mode = POINTS[checkers]
        if all(measured(p.get(mode) or {}, c, m) is not None for p in result["projects"].values()
               for c in ("tsgo", "tsrs") for m, *_ in PANELS):
            return mode
    raise ValueError(f"no benchmark results with every project measured in one of {', '.join(POINTS.values())}")


def element(parent, tag, **attrs):
    return ET.SubElement(parent, tag, {key.replace('_', '-'): str(value) for key, value in attrs.items()})


def text(parent, x, y, value, size, weight=400, fill=None, anchor=None):
    attrs = {"x": x, "y": y, "font_size": size, "font_weight": weight}
    if fill:
        attrs["fill"] = fill
    if anchor:
        attrs["text_anchor"] = anchor
    element(parent, "text", **attrs).text = value


def svg(result: dict, theme: str = "light") -> str:
    colors = THEMES[theme]
    if not any(MODE in project for project in result["projects"].values()):
        raise ValueError(f"no benchmark results for mode {MODE!r}")
    mode = chart_mode(result)
    checkers = next(c for c, m in POINTS.items() if m == mode)
    cells = {name: project[mode] for name, project in result["projects"].items()}
    compilers = [c for c in COMPILERS if all(measured(cell, c, m) is not None for cell in cells.values()
                                             for m, *_ in PANELS)]
    totals = {metric: {c: sum(measured(cell, c, metric) for cell in cells.values()) for c in compilers}
              for metric, *_ in PANELS}
    machine = next((cell["machine"] for cell in cells.values() if "machine" in cell), result["machine"])
    threads = f"--checkers {checkers}" + (f", bun check with --threads {checkers}" if "bun" in compilers else "")
    title = " vs ".join(NAMES[c] for c in compilers)
    setting = (f"Summed over {len(cells)} TypeScript project{'s' if len(cells) != 1 else ''} · {checkers} threads each · "
               f"{machine.get('cpus')}-vCPU {machine.get('platform', 'Linux').split()[0]} machine")

    root = ET.Element("svg", {"xmlns": "http://www.w3.org/2000/svg", "width": str(WIDTH), "height": str(HEIGHT),
                              "viewBox": f"0 0 {WIDTH} {HEIGHT}", "role": "img", "aria-labelledby": "title description"})
    ET.SubElement(root, "title", id="title").text = title
    details = []
    for name, cell in cells.items():
        parts = [f"{NAMES[c]} {format_value('wall_s', measured(cell, c, 'wall_s'))}, "
                 f"{format_value('peak_rss_bytes', measured(cell, c, 'peak_rss_bytes'))}" for c in compilers]
        details.append(f"{name}: {'; '.join(parts)}. ")
    summary = " ".join(
        f"{label}: " + ", ".join(f"{NAMES[c]} {total_text(metric, totals[metric][c])}" for c in compilers) + "."
        for metric, label, _ in PANELS)
    build = run.TSRS_BUILDS[result["tsrs"].get("build", "release")].replace("`", "")
    ET.SubElement(root, "desc", id="description").text = (
        f"{setting}. {summary} Shorter is better. Per project: " + "".join(details) +
        f"tsgo and tsrs with {threads}. {run.reps_text(result, [mode]).capitalize()}. tsrs uses {build}. "
        f"Machine: {machine['label'].replace('`', '')}. Measured {result['date']}; tsrs commit {result['tsrs']['commit']}. "
        f"TypeScript {result['tsgo']['version']}; TypeScript reference {(result.get('reference') or {}).get('commit', 'not recorded')}.")
    ET.SubElement(root, "metadata").text = (
        f"Generated by bench/chart.py; mode: {mode}; tsrs commit: {result['tsrs']['commit']}; "
        f"measured: {result['date']}; build: {result['tsrs'].get('build', 'release')}. "
        "Details: https://github.com/oxc-project/tsrs/blob/main/bench/RESULTS.md")

    element(root, "rect", x=0.5, y=0.5, width=WIDTH - 1, height=HEIGHT - 1, rx=20, fill=colors["background"],
            stroke=colors["border"])
    drawing = element(root, "g", font_family=FONT, fill=colors["text"])
    text(drawing, 48, 72, title, 36, 700)
    text(drawing, 48, 110, setting, 17, fill=colors["muted"])

    for (metric, label, word), left in zip(PANELS, PANEL_X):
        values = totals[metric]
        full = max(values.values())
        panel = element(drawing, "g", data_metric=metric)
        text(panel, left, 170, label, 20, 600)
        text(panel, left + BAR_X + BAR_LENGTH, 170, "shorter is better", 14, fill=colors["faint"], anchor="end")
        for row, compiler in enumerate(compilers):
            y = ROW_Y + row * ROW_PITCH
            top = y - BAR_HEIGHT / 2
            length = max(BAR_HEIGHT, BAR_LENGTH * values[compiler] / full)
            bar = element(panel, "g", data_compiler=compiler, data_total=f"{values[compiler]:.6g}")
            element(bar, "rect", x=left + BAR_X, y=top, width=BAR_LENGTH, height=BAR_HEIGHT, rx=BAR_HEIGHT / 2,
                    fill=colors["track"])
            element(bar, "rect", x=left + BAR_X, y=top, width=f"{length:.1f}", height=BAR_HEIGHT, rx=BAR_HEIGHT / 2,
                    fill=colors[compiler])
            text(bar, left + BAR_X - 14, y + 6, NAMES[compiler], 18, 600 if compiler == "tsrs" else 500, anchor="end")
            value_x = left + BAR_X + BAR_LENGTH + 18
            text(bar, value_x, y + 7, total_text(metric, values[compiler]), 19, 700, colors[compiler])
            if compiler != "tsgo":
                text(bar, value_x, y + 32, f"{values['tsgo'] / values[compiler]:.1f}x {word}", 14, 500,
                     colors["muted"])

    text(drawing, 48, HEIGHT - 28, f"tsgo and tsrs with {threads}. Wall time and peak RSS, "
         f"{run.reps_text(result, [mode])}. tsrs {result['tsrs']['commit'][:12]}, measured {result['date'][:10]}.",
         13, fill=colors["faint"])
    ET.indent(root, space="  ")
    return ET.tostring(root, encoding="unicode") + "\n"


# The per-project charts (bench/RESULTS.md): one row per project, a dot per compiler on a shared axis. Check time is
# linear, so the long checks read as long; peak memory is logarithmic, since bun check and tsrs trade wins there and
# would sit on top of each other on a linear axis.
DETAIL_WIDTH, DETAIL_TOP, DETAIL_PITCH, DETAIL_LEFT, DETAIL_RIGHT = 1000, 150, 34, 210, 720
DETAIL = {"wall_s": ("Check time", "faster", "linear"), "peak_rss_bytes": ("Peak memory", "less", "log")}


def nice_step(span: float) -> float:
    """A 1-2-5 step that gives at most 8 grid lines over `span`."""
    magnitude = 10 ** math.floor(math.log10(span / 8))
    return next(m * magnitude for m in (1, 2, 5, 10) if span / (m * magnitude) <= 8)


def per_project_svg(result: dict, metric: str, theme: str = "light") -> str:
    """One project per row, sorted by tsgo's check time: tsgo's, bun check's and tsrs's value as dots joined by a
    line, then how many times faster (or less memory) tsrs is than tsgo, and bun check's multiple on the rows where
    it beats tsrs."""
    colors = THEMES[theme]
    label, word, scale = DETAIL[metric]
    mode = chart_mode(result)
    checkers = next(c for c, m in POINTS.items() if m == mode)
    cells = {name: project[mode] for name, project in result["projects"].items()}
    compilers = [c for c in COMPILERS if all(measured(cell, c, m) is not None for cell in cells.values()
                                             for m, *_ in PANELS)]
    names = sorted(cells, key=lambda name: -measured(cells[name], "tsgo", "wall_s"))
    unit = 1 if metric == "wall_s" else GIB
    values = {name: {c: measured(cells[name], c, metric) / unit for c in compilers} for name in names}
    low = min(v for row in values.values() for v in row.values())
    high = max(v for row in values.values() for v in row.values())
    if scale == "linear":
        step = nice_step(high)
        end = math.ceil(high / step) * step
        ticks = [k * step for k in range(int(round(end / step)) + 1)]
        x_of = lambda v: DETAIL_LEFT + (DETAIL_RIGHT - DETAIL_LEFT) * v / end
    else:
        steps = [m * 10 ** e for e in range(math.floor(math.log10(low)), math.ceil(math.log10(high)) + 1)
                 for m in (1, 2, 5)]
        start = max(t for t in steps if t <= low)
        end = min((t for t in steps if t >= high), default=10 ** math.ceil(math.log10(high)))
        ticks = [t for t in steps if start <= t <= end]
        x_of = lambda v: DETAIL_LEFT + (DETAIL_RIGHT - DETAIL_LEFT) * math.log(v / start) / math.log(end / start)
    height = DETAIL_TOP + len(names) * DETAIL_PITCH + 70
    bottom = DETAIL_TOP + len(names) * DETAIL_PITCH - DETAIL_PITCH / 2
    machine = next((cell["machine"] for cell in cells.values() if "machine" in cell), result["machine"])
    threads = f"--checkers {checkers}" + (f"; bun check: --threads {checkers}" if "bun" in compilers else "")
    title = f"{label}: " + " vs ".join(NAMES[c] for c in compilers)

    root = ET.Element("svg", {"xmlns": "http://www.w3.org/2000/svg", "width": str(DETAIL_WIDTH), "height": str(height),
                              "viewBox": f"0 0 {DETAIL_WIDTH} {height}", "role": "img",
                              "aria-labelledby": "title description"})
    ET.SubElement(root, "title", id="title").text = title
    ET.SubElement(root, "desc", id="description").text = " ".join(
        f"{name}: " + ", ".join(f"{NAMES[c]} {format_value(metric, values[name][c] * unit)}" for c in compilers) + "."
        for name in names) + f" {checkers} threads each ({threads}). Smaller is better."
    ET.SubElement(root, "metadata").text = (
        f"Generated by bench/chart.py; metric: {metric}; mode: {mode}; tsrs commit: {result['tsrs']['commit']}; "
        f"measured: {result['date']}.")
    element(root, "rect", x=0.5, y=0.5, width=DETAIL_WIDTH - 1, height=height - 1, rx=20, fill=colors["background"],
            stroke=colors["border"])
    drawing = element(root, "g", font_family=FONT, fill=colors["text"])
    text(drawing, 40, 58, title, 28, 700)
    text(drawing, 40, 90, f"{checkers} threads each ({threads}) on a {machine.get('cpus')}-vCPU machine, sorted by "
         "tsgo's check time. Smaller is better.", 15, fill=colors["muted"])
    legend_x = DETAIL_WIDTH - 40
    for compiler in reversed(compilers):
        legend_x -= 9 * len(NAMES[compiler]) + 34
        element(drawing, "circle", cx=legend_x, cy=124, r=6, fill=colors[compiler])
        text(drawing, legend_x + 12, 129, NAMES[compiler], 14, 500, colors["muted"])

    grid = element(drawing, "g", data_scale=scale)
    for tick in ticks:
        x = x_of(tick)
        element(grid, "line", x1=f"{x:.1f}", y1=DETAIL_TOP - 18, x2=f"{x:.1f}", y2=bottom, stroke=colors["track"],
                stroke_width=1.5)
        text(grid, f"{x:.1f}", bottom + 22, f"{tick:g}", 13, fill=colors["faint"], anchor="middle")
    text(grid, (DETAIL_LEFT + DETAIL_RIGHT) / 2, bottom + 44,
         ("seconds" if metric == "wall_s" else "GiB") + (", log scale" if scale == "log" else ""), 13,
         fill=colors["muted"], anchor="middle")

    for row, name in enumerate(names):
        y = DETAIL_TOP + row * DETAIL_PITCH
        v = values[name]
        line = element(drawing, "g", data_project=name)
        text(line, DETAIL_LEFT - 18, y + 5, name, 14, 500, anchor="end")
        xs = [x_of(v[c]) for c in compilers]
        element(line, "line", x1=f"{min(xs):.1f}", y1=y, x2=f"{max(xs):.1f}", y2=y, stroke=colors["track"],
                stroke_width=5, stroke_linecap="round")
        for compiler in ("tsgo", "bun", "tsrs"):
            if compiler in compilers:
                element(line, "circle", data_compiler=compiler, cx=f"{x_of(v[compiler]):.1f}", cy=y, r=7,
                        fill=colors[compiler], stroke=colors["background"], stroke_width=1.5)
        labels = [("tsrs", f"{v['tsgo'] / v['tsrs']:.1f}x {word}")]
        if "bun" in compilers and v["bun"] < v["tsrs"]:
            labels.append(("bun", f"bun {v['tsgo'] / v['bun']:.1f}x {word}"))
        x = max(xs) + 16
        for compiler, label_text in labels:
            text(line, f"{x:.1f}", y + 5, label_text, 13, 600, colors[compiler])
            x += 7.4 * len(label_text) + 16

    text(drawing, 40, height - 20, f"Wall time and peak RSS, {run.reps_text(result, [mode])}. tsrs "
         f"{result['tsrs']['commit'][:12]}, measured {result['date'][:10]}.", 12, fill=colors["faint"])
    ET.indent(root, space="  ")
    return ET.tostring(root, encoding="unicode") + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("result", type=Path, help="recorded benchmark result (.json)")
    parser.add_argument("--output", type=Path, required=True, help="SVG destination")
    parser.add_argument("--theme", choices=THEMES, default="light", help="color scheme (default: light)")
    parser.add_argument("--per-project", choices=DETAIL, help="draw bench/RESULTS.md's per-project chart of this "
                        "metric instead of the README's summary")
    args = parser.parse_args()
    try:
        result = json.loads(args.result.read_text())
        image = per_project_svg(result, args.per_project, args.theme) if args.per_project else svg(result, args.theme)
    except ValueError as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(image)


if __name__ == "__main__":
    main()
