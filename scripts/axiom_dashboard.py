#!/usr/bin/env python3
"""Create or update the Axiom dashboard for overdueprogress.

Loads AXIOM_PAT, AXIOM_ORG_ID, AXIOM_DATASET from a .env file (in the repo
root) and/or the ambient environment. Run from the repo root:

    python3 scripts/axiom_dashboard.py           # apply
    python3 scripts/axiom_dashboard.py --dry-run # print the body, don't send

Re-runs are idempotent: the dashboard uid and each chart id are stable, so
editing PANELS and re-running updates the existing dashboard in place.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
import uuid
from pathlib import Path

API_URL = "https://api.axiom.co/v2/dashboards"
DASHBOARD_UID = "overdue-progress"
DASHBOARD_NAME = "Overdue Progress"
GRID_COLS = 2
CHART_W = 6
CHART_H = 4


def load_dotenv(path: Path) -> None:
    """Minimal python-dotenv replacement: parses KEY=VALUE lines.

    Values already set in os.environ win (same as dotenv's default).
    Silently skips a missing file.
    """
    if not path.is_file():
        return
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in ('"', "'"):
            value = value[1:-1]
        os.environ.setdefault(key, value)


def apl(q: str) -> str:
    return "\n".join(line for line in q.strip().splitlines())


# Each panel becomes one timeseries chart. Order here = top-to-bottom,
# left-to-right layout order.
PANELS = [
    {
        "name": "Request rate",
        "apl": apl("""
            ['{dataset}']
            | where name == "request"
            | summarize count() by bin_auto(_time)
        """),
    },
    {
        "name": "Request rate by URI",
        "apl": apl("""
            ['{dataset}']
            | where name == "request"
            | summarize count() by bin_auto(_time), uri=tostring(['attributes.custom'].uri)
        """),
    },
    {
        "name": "Latency percentiles",
        "apl": apl("""
            ['{dataset}']
            | where name == "request"
            | summarize p50=percentile(duration, 50), p95=percentile(duration, 95), p99=percentile(duration, 99) by bin_auto(_time)
        """),
    },
    {
        "name": "p95 latency by URI",
        "apl": apl("""
            ['{dataset}']
            | where name == "request"
            | summarize p95=percentile(duration, 95) by bin_auto(_time), uri=tostring(['attributes.custom'].uri)
        """),
    },
    {
        "name": "Status code class",
        "apl": apl("""
            ['{dataset}']
            | where name == "request"
            | mv-expand events
            | where tostring(events.name) == "finished processing request"
            | extend status = toint(events.attributes.status)
            | extend class = strcat(tostring(status / 100), "xx")
            | summarize count() by bin_auto(_time), class
        """),
    },
    {
        "name": "Submissions stored",
        "apl": apl("""
            ['{dataset}']
            | mv-expand events
            | where tostring(events.name) == "submission stored"
            | summarize count() by bin_auto(_time)
        """),
    },
    {
        "name": "Submission funnel",
        "apl": apl("""
            ['{dataset}']
            | mv-expand events
            | extend ev = tostring(events.name)
            | where ev in ("submission received", "submission stored", "submission rejected: deadline passed", "submission rejected: validation", "submission rejected: turnstile challenge failed")
            | summarize count() by bin_auto(_time), ev
        """),
    },
    {
        "name": "Confirmation emails",
        "apl": apl("""
            ['{dataset}']
            | mv-expand events
            | extend ev = tostring(events.name)
            | where ev in ("confirmation email sent", "resend send failed (submission already saved)")
            | summarize count() by bin_auto(_time), ev
        """),
    },
    {
        "name": "Admin login attempts",
        "apl": apl("""
            ['{dataset}']
            | mv-expand events
            | extend ev = tostring(events.name)
            | where ev in ("login success", "login failed")
            | summarize count() by bin_auto(_time), ev
        """),
    },
    {
        "name": "Handler and alert failures",
        "apl": apl("""
            ['{dataset}']
            | mv-expand events
            | extend ev = tostring(events.name)
            | where ev in ("handler failed", "telegram notify failed", "template render failed", "template missing", "insert submission failed")
            | summarize count() by bin_auto(_time), ev
        """),
    },
]


def stable_chart_id(name: str) -> str:
    return str(uuid.uuid5(uuid.NAMESPACE_URL, f"overdueprogress-dashboard:{name}"))


def build_chart(panel: dict, dataset: str) -> dict:
    query = panel["apl"].format(dataset=dataset)
    return {
        "id": stable_chart_id(panel["name"]),
        "name": panel["name"],
        "datasetId": dataset,
        "type": "TimeSeries",
        "numSeries": 1,
        "overrideDashboardCompareAgainst": False,
        "overrideDashboardTimeRange": False,
        "query": {
            "apl": query,
            "queryOptions": {
                "against": "",
                "aggChartOpts": "[]",
                "containsTimeFilter": "false",
                "editorContent": query,
                "openIntervals": "shown",
                "quickRange": "1h",
                "timeSeriesView": "charts",
            },
        },
    }


def build_layout(charts: list[dict]) -> list[dict]:
    layout = []
    for i, chart in enumerate(charts):
        layout.append({
            "i": chart["id"],
            "x": (i % GRID_COLS) * CHART_W,
            "y": (i // GRID_COLS) * CHART_H,
            "w": CHART_W,
            "h": CHART_H,
            "minW": 3,
            "minH": 3,
            "moved": False,
            "static": False,
        })
    return layout


def build_dashboard(dataset: str) -> dict:
    charts = [build_chart(p, dataset) for p in PANELS]
    return {
        "name": DASHBOARD_NAME,
        "charts": charts,
        "layout": build_layout(charts),
        "datasets": [dataset],
        "refreshTime": 60,
        "schemaVersion": 2,
        "timeWindowStart": "qr-now-1h",
        "timeWindowEnd": "qr-now",
        "uid": DASHBOARD_UID,
    }


def upsert(dashboard: dict, pat: str, org_id: str) -> int:
    body = {
        "dashboard": dashboard,
        "uid": dashboard["uid"],
        "overwrite": True,
        "version": 0,
        "message": "applied via scripts/axiom_dashboard.py",
    }
    req = urllib.request.Request(
        API_URL,
        method="POST",
        data=json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {pat}",
            "x-axiom-org-id": org_id,
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req) as resp:
            payload = json.loads(resp.read().decode() or "{}")
            status = payload.get("status", "ok")
            print(f"HTTP {resp.status} — {status}")
            return 0
    except urllib.error.HTTPError as e:
        body = e.read().decode()
        print(f"HTTP {e.code} {e.reason}: {body}", file=sys.stderr)
        return 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="print the request body instead of sending")
    parser.add_argument("--env-file", default=".env", help="path to .env file (default: ./.env)")
    args = parser.parse_args()

    load_dotenv(Path(args.env_file))

    dataset = os.environ.get("AXIOM_DATASET", "overdueprogress")
    dashboard = build_dashboard(dataset)

    if args.dry_run:
        print(json.dumps(dashboard, indent=2))
        return 0

    pat = os.environ.get("AXIOM_PAT")
    org_id = os.environ.get("AXIOM_ORG_ID")
    if not pat or not org_id:
        print("error: AXIOM_PAT and AXIOM_ORG_ID must be set in the environment", file=sys.stderr)
        return 1

    return upsert(dashboard, pat, org_id)


if __name__ == "__main__":
    sys.exit(main())
