#!/usr/bin/env python3
"""Draw the README benchmark chart from a recorded result, without running a benchmark."""
from __future__ import annotations

import argparse
from html import escape
import json
import math
from pathlib import Path
import textwrap

import run


TOOLS = (("tsgo", "tsgo", "#497bc6"), ("tsrs", "tsrs", "#138a7a"), ("bun", "bun check", "#b77915"))


def svg(result: dict, mode: str = "wide") -> str:
    """Compare each project's wall time and peak RSS; unavailable samples have no bar."""
    rows = sorted(((name, project[mode]) for name, project in result["projects"].items() if mode in project),
                  key=lambda item: run.speedup_sort_key({mode: item[1]}, mode))
    if not rows:
        raise ValueError(f"no benchmark results for mode {mode!r}")
    tools = TOOLS if any("bun" in cell for _, cell in rows) else TOOLS[:2]
    whole, odd = run.cell_machines(result)
    mixed = any(cell_mode == mode for _, cell_mode, _ in odd)
    machine = whole.get(mode, result["machine"])
    hardware = "Multiple machines; see the detailed results." if mixed else machine["label"].replace("`", "")
    if mode in ("default", "wide"):
        setting = "Default settings" + (f" on {machine['cpus']} vCPUs" if not mixed else "")
    else:
        setting = run.MODE_NAMES.get(mode, mode).replace("`", "")
    metadata = [setting + f" · {len(rows)} projects", *textwrap.wrap(hardware, width=140),
                run.reps_text(result, [mode]).capitalize() + "."]
    commit = result["tsrs"]["commit"][:12]
    build = run.TSRS_BUILDS[result["tsrs"].get("build", "release")].replace("`", "")
    versions = f"tsgo {result['tsgo']['version']}"
    if len(tools) == 3:
        versions += f" · Bun {(result.get('bun') or {}).get('version', 'version not recorded')}"
    footer = [f"{result['date']} · tsrs {commit} · {build}", versions,
              "Rows sorted by tsrs speedup over tsgo. Error counts and methodology are in the detailed results."]
    width, row_height = 1200, 66 if len(tools) == 3 else 50
    legend_y = 80 + len(metadata) * 22
    scale_y, header_y = legend_y + 35, legend_y + 78
    rows_y = header_y + 20
    footer_y = rows_y + len(rows) * row_height + 31
    height = footer_y + len(footer) * 22 + 14
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
           f'viewBox="0 0 {width} {height}" role="img" aria-labelledby="title description">',
           '<title id="title">Type-checking benchmark: wall time and peak memory</title>',
           '<desc id="description">' + escape(f"{setting}. {hardware} {run.reps_text(result, [mode])}. "
           "Bars scaled within each project; lower is better. Values are wall-clock seconds and peak resident "
           "memory. Failed or unavailable measurements are marked n/a and have no bar. "
           f"Measured {result['date']}, tsrs commit {commit}.") + '</desc>',
           '<style>text{font-family:system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;fill:#213547}'
           '.muted{fill:#526476}.number{font-variant-numeric:tabular-nums}</style>',
           f'<rect width="{width}" height="{height}" rx="16" fill="#fff"/>']

    def text(x, y, value, size=14, weight=400, css="", anchor="start"):
        out.append(f'<text x="{x}" y="{y}" font-size="{size}" font-weight="{weight}" '
                   f'class="{css}" text-anchor="{anchor}">{escape(str(value))}</text>')

    text(32, 46, "Type-checking performance", 28, 700)
    for i, line in enumerate(metadata):
        text(32, 76 + i * 22, line, 14, css="muted")
    for i, (_, name, color) in enumerate(tools):
        x = 32 + i * 150
        out.append(f'<rect x="{x}" y="{legend_y - 11}" width="13" height="13" rx="3" fill="{color}"/>')
        text(x + 21, legend_y, name, 15, 600)
    text(32, scale_y, "Bars scaled within each project; lower is better", 15, 600)
    text(32, header_y, "PROJECT", 12, 700, "muted")
    text(238, header_y, "WALL TIME", 12, 700, "muted")
    text(718, header_y, "PEAK MEMORY", 12, 700, "muted")
    for index, (name, cell) in enumerate(rows):
        y = rows_y + index * row_height
        if index % 2 == 0:
            out.append(f'<rect x="20" y="{y}" width="1160" height="{row_height}" rx="6" fill="#f4f7fa"/>')
        text(32, y + row_height / 2 + 5, name, 14, 600)
        for x, metric, formatter in ((238, "wall_s", run.fmt_s), (718, "peak_rss_bytes", run.fmt_mem)):
            values = []
            for tool, _, _ in tools:
                sample = cell.get(tool) or {}
                value = sample.get(metric)
                values.append(value if sample.get("ok_runs") and isinstance(value, (int, float))
                              and math.isfinite(value) and value >= 0 else None)
            maximum = max((v for v in values if v is not None), default=0)
            for i, ((_, tool_name, color), value) in enumerate(zip(tools, values)):
                bar_y = y + 9 + i * 17
                label = formatter(value)
                out.append('<g><title>' + escape(f"{name}: {tool_name}, {metric}: {label}") + '</title>')
                if value is not None and value > 0:
                    out.append(f'<rect x="{x}" y="{bar_y}" width="{340 * value / maximum:.2f}" '
                               f'height="11" rx="3" fill="{color}"/>')
                text(x + 428, bar_y + 11, label, 13, 600 if tool_name == "tsrs" else 400, "number", "end")
                out.append('</g>')
    out.append(f'<path d="M 694 {header_y - 12} V {rows_y + len(rows) * row_height}" stroke="#dce4eb"/>')
    for i, line in enumerate(footer):
        text(32, footer_y + i * 22, line, 12, css="muted")
    out.append('</svg>')
    return "\n".join(out) + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path, help="recorded benchmark result (.json)")
    parser.add_argument("--output", type=Path, required=True, help="SVG destination")
    parser.add_argument("--mode", default="wide", choices=run.MODE_FLAGS)
    args = parser.parse_args()
    try:
        image = svg(json.loads(args.result.read_text()), args.mode)
    except ValueError as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(image)


if __name__ == "__main__":
    main()
