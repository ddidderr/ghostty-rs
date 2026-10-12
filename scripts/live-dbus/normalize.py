#!/usr/bin/env python3
"""Turn a scenario output dir into a comparable text: facts + filtered bus
traffic + mock-received calls, with volatile values (unique names, pids,
cookies, tokens, paths) normalized."""
import json
import os
import pwd
import re
import sys

d = sys.argv[1]
out = []


def rd(name):
    p = os.path.join(d, name)
    return open(p).read() if os.path.exists(p) else ""


def norm(s):
    s = re.sub(r"app-ghostty-surface-transient-\d+", "app-ghostty-surface-transient-PID", s)
    s = re.sub(r'"PIDs", (\[\d+\]|\{"data": \[\d+\])', lambda m: re.sub(r"\d+", "PID", m.group(0)), s)
    s = re.sub(r"ghostty_[0-9a-f]{7}", "ghostty_TOKEN", s)
    s = re.sub(r"gtk[0-9]{6,}", "gtkTOKEN", s)
    s = re.sub(r"/request/[0-9_]+/", "/request/SENDER/", s)
    s = re.sub(r"/session/[0-9_]+/", "/session/SENDER/", s)
    s = s.replace(os.environ.get("LDB_RUN", "\0"), "$RUN")
    s = s.replace(os.environ.get("LDB_ROOT", "\0"), "$ROOT")
    s = s.replace(os.path.expanduser("~" + os.environ.get("USER", "")) if False else pwd.getpwuid(os.getuid()).pw_dir, "$HOME")
    return s


# Scope creation races with the end of a scenario; only the cgroup scenario checks it.
CGROUP = os.path.basename(os.path.dirname(d.rstrip("/"))) == "cgroup"

out.append("== facts ==")
for l in rd("facts.txt").splitlines():
    out.append(norm(l))

out.append("== mock received ==")
for l in rd("mock.jsonl").splitlines():
    j = json.loads(l)
    if j["method"] == "StartTransientUnit" and not CGROUP:
        continue
    out.append(norm(f"{j['iface']}.{j['method']} {json.dumps(j['args'], sort_keys=True)}"))

out.append("== bus traffic (filtered) ==")
names = {}


def label(n):
    if n is None:
        return "-"
    if n.startswith(":"):
        names.setdefault(n, f"conn{len(names)+1}")
        return names[n]
    return n


KEEP = re.compile(r"^(org\.gtk\.|org\.freedesktop\.(Notifications|systemd1|portal))")
for l in rd("bus.json").splitlines():
    try:
        m = json.loads(l)
    except ValueError:
        continue
    iface = m.get("interface") or ""
    if not KEEP.match(iface):
        continue
    if m.get("member") == "StartTransientUnit" and not CGROUP:
        continue
    payload = m.get("payload") or {}
    data = json.dumps(payload.get("data"), sort_keys=True)
    out.append(norm(f"{m['type']} {iface}.{m.get('member')} path={m.get('path')} -> {data}"))
print("\n".join(out))
