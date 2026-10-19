#!/usr/bin/env python3
"""Render the README's agent-session chart and table from one data file.

Reads docs/images/agent-sessions.json and writes
docs/images/agent-sessions-light.svg and docs/images/agent-sessions-dark.svg
(the README shows the one matching the reader's theme), then prints the
README's results table, so the chart and the table cannot disagree.

  python3 scripts/render-readme-charts.py
"""
import json
from pathlib import Path
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / 'docs/images/agent-sessions.json'
METRICS = (('time', 'Time'), ('tokens', 'Tokens'), ('toolCalls', 'Tool calls'))
THEMES = {
    'light': {'text': '#1f2328', 'muted': '#59636e', 'grid': '#d1d9e0',
              'bars': ('#0969da', '#8250df', '#1a7f37')},
    'dark': {'text': '#f0f6fc', 'muted': '#9198a1', 'grid': '#3d444d',
             'bars': ('#4493f8', '#ab7df8', '#3fb950')},
}
WIDTH, LABEL, RIGHT, BAR, GAP, GROUP_GAP, TOP = 760, 120, 60, 14, 4, 18, 64


def svg(tools, theme):
    colors = THEMES[theme]
    plot = WIDTH - LABEL - RIGHT
    group = len(METRICS) * BAR + (len(METRICS) - 1) * GAP
    height = TOP + len(tools) * (group + GROUP_GAP) + 34
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" '
           f'viewBox="0 0 {WIDTH} {height}" font-family="-apple-system, BlinkMacSystemFont, '
           f'Segoe UI, Helvetica, Arial, sans-serif" role="img">',
           f'<title>How much less each coding tool spends with Oko</title>',
           f'<text x="{LABEL}" y="22" font-size="15" font-weight="600" fill="{colors["text"]}">'
           f'How much less each coding tool spends with Oko</text>']
    # Legend
    x = LABEL
    for (key, label), color in zip(METRICS, colors['bars']):
        out.append(f'<rect x="{x}" y="36" width="12" height="12" rx="2" fill="{color}"/>')
        out.append(f'<text x="{x + 18}" y="46" font-size="12" fill="{colors["muted"]}">{label}</text>')
        x += 18 + 8 * len(label) + 22
    # Grid at 0, 25, 50, 75, 100 %
    bottom = height - 30
    for pct in (0, 25, 50, 75, 100):
        gx = LABEL + plot * pct / 100
        out.append(f'<line x1="{gx:.1f}" y1="{TOP - 6}" x2="{gx:.1f}" y2="{bottom}" '
                   f'stroke="{colors["grid"]}" stroke-width="1"/>')
        out.append(f'<text x="{gx:.1f}" y="{bottom + 16}" font-size="11" text-anchor="middle" '
                   f'fill="{colors["muted"]}">{pct}%</text>')
    y = TOP
    for tool in tools:
        out.append(f'<text x="{LABEL - 12}" y="{y + group / 2 + 4:.1f}" font-size="13" '
                   f'text-anchor="end" fill="{colors["text"]}">{escape(tool["name"])}</text>')
        for index, ((key, label), color) in enumerate(zip(METRICS, colors['bars'])):
            pct = tool[key][2]
            by = y + index * (BAR + GAP)
            out.append(f'<rect x="{LABEL}" y="{by}" width="{plot * pct / 100:.1f}" height="{BAR}" '
                       f'rx="2" fill="{color}"><title>{escape(tool["name"])}: {label} {pct}% less</title></rect>')
            out.append(f'<text x="{LABEL + plot * pct / 100 + 6:.1f}" y="{by + BAR - 3}" font-size="11" '
                       f'fill="{colors["text"]}">{pct}%</text>')
        y += group + GROUP_GAP
    out.append('</svg>')
    return '\n'.join(out) + '\n'


def number(value):
    return f'{value:,}' if isinstance(value, int) else f'{value:.1f}'


def table(tools):
    rows = ['| Coding tool | Time | Tokens | Tool calls | Tasks passed (without → with) |',
            '| --- | --- | --- | --- | --- |']
    for tool in tools:
        cells = [f"**{tool[key][2]}% less**<br><sub>{number(tool[key][0])} → {number(tool[key][1])}"
                 f"{' s' if key == 'time' else ''}</sub>" for key, _ in METRICS]
        rows.append(f"| {tool['name']} | " + ' | '.join(cells)
                    + f" | {tool['passed'][0]} → {tool['passed'][1]} of 27 |")
    return '\n'.join(rows)


def main():
    tools = json.loads(DATA.read_text())['tools']
    for theme in THEMES:
        (DATA.parent / f'agent-sessions-{theme}.svg').write_text(svg(tools, theme))
    print(table(tools))


if __name__ == '__main__':
    main()
