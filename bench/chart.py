#!/usr/bin/env python3
"""Draw the README's chart from a recorded benchmark result.

One line per project through its points at 4, 8 and 16 checker threads, both compilers at the same count on the
16-vCPU machine: up is how many times faster tsrs is than TypeScript 7, right is how many times less peak memory it
uses.
"""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import xml.etree.ElementTree as ET

import run


PROJECTS = (("vscode", "VS Code", "microsoft/vscode · src"),
            ("t3code-server", "T3 Code", "pingdotgg/t3code · apps/server"),
            ("mui-docs", "MUI docs", "mui/material-ui · docs"))
COMPILERS = ("tsgo", "tsrs")
# The points of each line: both compilers at the same checker count on the 16-vCPU machine (bench/run.py
# WIDE_CHECKER_MODES), so every point compares thread for thread. At their defaults tsrs would run 16 checker threads
# there and tsgo its fixed 4.
POINTS = run.WIDE_CHECKER_MODES
# The mode a result must have for the chart to be drawn (bench/publish.py); the other points are drawn when measured.
MODE = POINTS[16]
# The README shows the light or dark chart to match the reader's GitHub theme.
THEMES = {
    "light": {"series": ("#3178c6", "#087b5b", "#b35900"), "background": "#fafcfb", "border": "#dce5e1",
              "text": "#162d27", "muted": "#52655e", "grid": "#e3eae6", "axis": "#9fb0a9"},
    "dark": {"series": ("#58a6ff", "#3fb98c", "#f0883e"), "background": "#0d1117", "border": "#30363d",
             "text": "#e6edf3", "muted": "#9198a1", "grid": "#262c33", "axis": "#4b535d"},
}
GIB = 2**30
WIDTH, HEIGHT = 960, 700
# The plot area: x is the memory ratio, y the speedup, both from 1x (the same as TypeScript 7).
LEFT, RIGHT, TOP, BOTTOM = 96, 620, 136, 584
DOT = 7  # the radius of a point


def measured(cell: dict, compiler: str, metric: str) -> float | None:
    sample = cell.get(compiler) or {}
    value = sample.get(metric)
    return value if (sample.get("ok_runs") and isinstance(value, (int, float))
                     and math.isfinite(value) and value >= 0) else None


def ratio(cell: dict, metric: str) -> float | None:
    """TypeScript 7's value over tsrs's: how many times faster, or how many times less memory."""
    tsgo, tsrs = (measured(cell, compiler, metric) for compiler in COMPILERS)
    return tsgo / tsrs if tsgo and tsrs else None


def format_value(metric: str, value: float | None) -> str:
    if value is None:
        return "n/a"
    return run.fmt_s(value) if metric == "wall_s" else f"{value / GIB:.2f} GiB"


def axis_max(values: list[float], minimum: float) -> tuple[float, float]:
    """The axis end and tick step: a 1-2-5 step giving at most 8 ticks above 1x, the end a multiple of it."""
    top = max([minimum, *values])
    step = next(s for s in (1, 2, 5, 10, 20, 50, 100) if (top - 1) / s <= 8)
    return max(minimum, math.ceil(top * 1.08 / step) * step), step


def ticks(end: float, step: float) -> list[float]:
    """1x (the axis origin), then the multiples of `step` up to `end`, leaving out one too close to 1x to label."""
    return [1.0] + [k * step for k in range(1, int(end / step) + 1) if k * step - 1 >= 0.6 * step]


def svg(result: dict, theme: str = "light") -> str:
    colors = THEMES[theme]
    if not any(MODE in project for project in result["projects"].values()):
        raise ValueError(f"no benchmark results for mode {MODE!r}")
    lines = []  # (name, title, source, files, [(checkers, cell, speedup, memory)])
    for name, title, source in PROJECTS:
        project = result["projects"].get(name, {})
        points = []
        for checkers, mode in POINTS.items():
            cell = project.get(mode) or {}
            speedup, memory = ratio(cell, "wall_s"), ratio(cell, "peak_rss_bytes")
            if speedup is not None and memory is not None:
                points.append((checkers, cell, speedup, memory))
        files = next((f for _, cell, _, _ in reversed(points) if (f := measured(cell, "tsrs", "files")) is not None), None)
        lines.append((name, title, source, files, points))
    machine = next((cell.get("machine", result["machine"]) for *_, points in lines for _, cell, _, _ in points),
                   result["machine"])
    counts = sorted({checkers for *_, points in lines for checkers, *_ in points})
    if len(counts) > 1:
        setting = (f"Both compilers at the same number of checker threads ({', '.join(map(str, counts[:-1]))} and "
                   f"{counts[-1]}) on {machine.get('cpus')} vCPUs")
    else:
        setting = f"Both compilers at {counts[0] if counts else 'n/a'} checker threads on {machine.get('cpus')} vCPUs"
    x_max, x_step = axis_max([memory for *_, points in lines for *_, memory in points], 6)
    y_max, y_step = axis_max([speedup for *_, points in lines for _, _, speedup, _ in points], 10)
    x_of = lambda value: LEFT + (RIGHT - LEFT) * (value - 1) / (x_max - 1)
    y_of = lambda value: BOTTOM - (BOTTOM - TOP) * (value - 1) / (y_max - 1)

    root = ET.Element("svg", {"xmlns": "http://www.w3.org/2000/svg", "width": str(WIDTH), "height": str(HEIGHT),
                              "viewBox": f"0 0 {WIDTH} {HEIGHT}", "role": "img", "aria-labelledby": "title description"})
    ET.SubElement(root, "title", id="title").text = "Performance"
    details = []
    for name, title, source, files, points in lines:
        detail = f"{title} ({source}): {f'{files:,.0f}' if files is not None else 'n/a'} files loaded by tsrs. "
        for checkers, cell, speedup, memory in points:
            times = [format_value("wall_s", measured(cell, c, "wall_s")) for c in COMPILERS]
            peaks = [format_value("peak_rss_bytes", measured(cell, c, "peak_rss_bytes")) for c in COMPILERS]
            detail += (f"{checkers} checker threads: TypeScript 7 {times[0]}, {peaks[0]}; tsrs {times[1]}, {peaks[1]}; "
                       f"{speedup:.1f}x faster, {memory:.1f}x less memory. ")
        details.append(detail)
    build = run.TSRS_BUILDS[result["tsrs"].get("build", "release")].replace("`", "")
    modes = [POINTS[c] for c in counts]
    ET.SubElement(root, "desc", id="description").text = "".join(details) + (
        f"{setting}. Up is faster, right is less peak memory; both axes start at 1x, the same as TypeScript 7. "
        f"{run.reps_text(result, modes).capitalize()}. tsrs uses {build}. Machine: {machine['label'].replace('`', '')}. "
        f"File counts include libraries and dependencies. Measured {result['date']}; tsrs commit {result['tsrs']['commit']}. "
        f"TypeScript {result['tsgo']['version']}; TypeScript reference {(result.get('reference') or {}).get('commit', 'not recorded')}.")
    ET.SubElement(root, "metadata").text = (
        f"Generated by bench/chart.py; modes: {', '.join(modes)}; tsrs commit: {result['tsrs']['commit']}; "
        f"measured: {result['date']}; build: {result['tsrs'].get('build', 'release')}. "
        "Details: https://github.com/maschwenk/tsrs/blob/main/bench/RESULTS.md")

    def element(parent, tag, **attrs):
        return ET.SubElement(parent, tag, {key.replace('_', '-'): str(value) for key, value in attrs.items()})

    def text(parent, x, y, value, size, weight=None, fill=None, anchor=None, **attrs):
        attrs.update(x=x, y=y, font_size=size)
        if weight is not None:
            attrs['font_weight'] = weight
        if fill:
            attrs['fill'] = fill
        if anchor:
            attrs['text_anchor'] = anchor
        element(parent, "text", **attrs).text = value

    element(root, "rect", x=0.5, y=0.5, width=WIDTH - 1, height=HEIGHT - 1, rx=20, fill=colors["background"],
            stroke=colors["border"])
    drawing = element(root, "g", font_family="-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif", fill=colors["text"])
    text(drawing, 40, 55, "Performance", 34, 700)
    text(drawing, 40, 88, setting, 17, fill=colors["muted"])

    plot = element(drawing, "g", id="plot")
    for tick in ticks(x_max, x_step):
        x = x_of(tick)
        element(plot, "line", x1=f"{x:.1f}", y1=TOP, x2=f"{x:.1f}", y2=BOTTOM, stroke=colors["grid"] if tick > 1 else colors["axis"])
        text(plot, f"{x:.1f}", BOTTOM + 24, f"{tick:g}×", 15, fill=colors["muted"], anchor="middle")
    for tick in ticks(y_max, y_step):
        y = y_of(tick)
        element(plot, "line", x1=LEFT, y1=f"{y:.1f}", x2=RIGHT, y2=f"{y:.1f}", stroke=colors["grid"] if tick > 1 else colors["axis"])
        text(plot, LEFT - 10, f"{y + 5:.1f}", f"{tick:g}×", 15, fill=colors["muted"], anchor="end")
    text(plot, (LEFT + RIGHT) / 2, BOTTOM + 54, "Less peak memory than TypeScript 7 →", 17, 600, anchor="middle")
    text(plot, 34, (TOP + BOTTOM) / 2, "Faster than TypeScript 7 →", 17, 600, anchor="middle",
         transform=f"rotate(-90 34 {(TOP + BOTTOM) / 2})")

    legend_x = RIGHT + 48
    # Every point is a dot at its exact position, its thread count beside it on the first side where the label covers
    # no other dot or label (points of one project can nearly coincide: T3 Code is about as fast at 4 as at 8 threads).
    dots = [(x_of(memory), y_of(speedup)) for *_, points in lines for _, _, speedup, memory in points]
    taken: list[tuple[float, float, float, float]] = [(x - DOT, y - DOT, x + DOT, y + DOT) for x, y in dots]
    overlaps = lambda a, b: a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]
    for index, (name, title, source, files, points) in enumerate(lines):
        color = colors["series"][index % len(colors["series"])]
        series = element(plot, "g", id=name, data_project=name)
        if len(points) > 1:
            element(series, "polyline", fill="none", stroke=color, stroke_width=3, stroke_linejoin="round",
                    points=" ".join(f"{x_of(memory):.1f},{y_of(speedup):.1f}" for _, _, speedup, memory in points))
        for checkers, _, speedup, memory in points:
            x, y = x_of(memory), y_of(speedup)
            element(series, "circle", data_checkers=checkers, cx=f"{x:.1f}", cy=f"{y:.1f}", r=DOT, fill=color,
                    stroke=colors["background"], stroke_width=2)
            width = 9 * len(str(checkers))
            sides = [(x + DOT + 4, y + 5, "start"), (x - DOT - 4, y + 5, "end"), (x, y - DOT - 6, "middle"),
                     (x, y + DOT + 18, "middle")]
            box_of = lambda lx, ly, anchor: (lx - {"start": 0, "end": width, "middle": width / 2}[anchor], ly - 14,
                                             lx + {"start": width, "end": 0, "middle": width / 2}[anchor], ly + 2)
            lx, ly, anchor = next((side for side in sides if not any(overlaps(box_of(*side), t) for t in taken)), sides[0])
            taken.append(box_of(lx, ly, anchor))
            text(series, f"{lx:.1f}", f"{ly:.1f}", str(checkers), 15, 700, color, anchor)

        y = TOP + 8 + index * 118
        entry = element(drawing, "g", data_legend=name)
        element(entry, "rect", x=legend_x, y=y, width=16, height=16, rx=3, fill=color)
        text(entry, legend_x + 26, y + 15, title, 21, 700)
        text(entry, legend_x + 26, y + 39, source, 14, fill=colors["muted"])
        text(entry, legend_x + 26, y + 60, (f"{files:,.0f}" if files is not None else "n/a") + " files", 14, fill=colors["muted"])
        if points:
            checkers, _, speedup, memory = points[-1]
            text(entry, legend_x + 26, y + 82, f"{checkers} threads: {speedup:.1f}× faster,", 14, 600, color)
            text(entry, legend_x + 26, y + 100, f"{memory:.1f}× less memory", 14, 600, color)

    text(drawing, 40, HEIGHT - 28, "Each point: TypeScript 7 ÷ tsrs, wall time (up) and peak memory (right), with both at "
         "the thread count beside it. Up and to the right is better.", 14, fill=colors["muted"])
    ET.indent(root, space="  ")
    return ET.tostring(root, encoding="unicode") + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("result", type=Path, help="recorded benchmark result (.json)")
    parser.add_argument("--output", type=Path, required=True, help="SVG destination")
    parser.add_argument("--theme", choices=THEMES, default="light", help="color scheme (default: light)")
    args = parser.parse_args()
    try:
        image = svg(json.loads(args.result.read_text()), args.theme)
    except ValueError as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(image)


if __name__ == "__main__":
    main()
