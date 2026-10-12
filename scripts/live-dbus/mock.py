#!/usr/bin/env python3
"""Mock D-Bus services for the live D-Bus comparison (scripts/live-dbus-test.sh).

Runs on the private session bus and claims:
  org.freedesktop.Notifications     Notify/CloseNotification/GetCapabilities/GetServerInformation
  org.freedesktop.systemd1          Manager.StartTransientUnit
  org.freedesktop.portal.Desktop    GlobalShortcuts, Settings, OpenURI (+ Request objects)
  test.ghostty.Mock                 control: Activate(s), SetColorScheme(u), Quit()

Every received call is appended to $MOCK_LOG as one JSON line:
  {"iface": ..., "method": ..., "args": ...}
The portal pieces are mocks (the real xdg-desktop-portal needs a running
backend and a desktop session); they implement just what Ghostty calls.
"""
import json
import os
import signal
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

LOG = os.environ.get("MOCK_LOG", "/dev/stderr")
COLOR_SCHEME = int(os.environ.get("MOCK_COLOR_SCHEME", "0"))  # 0 none, 1 dark, 2 light
BIND_REPLY = os.environ.get("MOCK_BIND", "ok")

XML = """
<node>
 <interface name='org.freedesktop.Notifications'>
  <method name='Notify'>
   <arg type='s' direction='in'/><arg type='u' direction='in'/><arg type='s' direction='in'/>
   <arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='as' direction='in'/>
   <arg type='a{sv}' direction='in'/><arg type='i' direction='in'/><arg type='u' direction='out'/>
  </method>
  <method name='CloseNotification'><arg type='u' direction='in'/></method>
  <method name='GetCapabilities'><arg type='as' direction='out'/></method>
  <method name='GetServerInformation'>
   <arg type='s' direction='out'/><arg type='s' direction='out'/><arg type='s' direction='out'/><arg type='s' direction='out'/>
  </method>
  <signal name='ActionInvoked'><arg type='u'/><arg type='s'/></signal>
  <signal name='NotificationClosed'><arg type='u'/><arg type='u'/></signal>
 </interface>
 <interface name='org.freedesktop.systemd1.Manager'>
  <method name='StartTransientUnit'>
   <arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='a(sv)' direction='in'/>
   <arg type='a(sa(sv))' direction='in'/><arg type='o' direction='out'/>
  </method>
 </interface>
 <interface name='org.freedesktop.portal.GlobalShortcuts'>
  <method name='CreateSession'><arg type='a{sv}' direction='in'/><arg type='o' direction='out'/></method>
  <method name='BindShortcuts'>
   <arg type='o' direction='in'/><arg type='a(sa{sv})' direction='in'/><arg type='s' direction='in'/>
   <arg type='a{sv}' direction='in'/><arg type='o' direction='out'/>
  </method>
  <signal name='Activated'><arg type='o'/><arg type='s'/><arg type='t'/><arg type='a{sv}'/></signal>
 </interface>
 <interface name='org.freedesktop.portal.Session'><method name='Close'/></interface>
 <interface name='org.freedesktop.portal.Settings'>
  <method name='ReadAll'><arg type='as' direction='in'/><arg type='a{sa{sv}}' direction='out'/></method>
  <method name='Read'><arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='v' direction='out'/></method>
  <signal name='SettingChanged'><arg type='s'/><arg type='s'/><arg type='v'/></signal>
  <property name='version' type='u' access='read'/>
 </interface>
 <interface name='org.freedesktop.portal.OpenURI'>
  <method name='OpenURI'>
   <arg type='s' direction='in'/><arg type='s' direction='in'/><arg type='a{sv}' direction='in'/><arg type='o' direction='out'/>
  </method>
  <method name='OpenFile'>
   <arg type='s' direction='in'/><arg type='h' direction='in'/><arg type='a{sv}' direction='in'/><arg type='o' direction='out'/>
  </method>
  <property name='version' type='u' access='read'/>
 </interface>
 <interface name='org.freedesktop.portal.Request'><method name='Close'/>
  <signal name='Response'><arg type='u'/><arg type='a{sv}'/></signal></interface>
</node>
"""

CTRL_XML = """
<node><interface name='test.ghostty.Mock'>
 <method name='Activate'><arg type='s' direction='in'/></method>
 <method name='SetColorScheme'><arg type='u' direction='in'/></method>
 <method name='Invoke'><arg type='u' direction='in'/><arg type='s' direction='in'/></method>
 <method name='SetFail'><arg type='b' direction='in'/></method>
 <method name='Quit'/>
</interface></node>
"""

nid = [0]
state = {"session": None, "scheme": COLOR_SCHEME}
loop = GLib.MainLoop()
conns = {}


def log(iface, method, args):
    def conv(o):
        if isinstance(o, (bytes, bytearray)):
            return list(o)
        return str(o)

    with open(LOG, "a") as f:
        f.write(json.dumps({"iface": iface, "method": method, "args": args}, default=conv, sort_keys=True) + "\n")


def unpack(v):
    return v.unpack() if v is not None else None


def request_path(sender, options):
    tok = options.get("handle_token", "tok")
    return "/org/freedesktop/portal/desktop/request/" + sender[1:].replace(".", "_") + "/" + tok


def reply_request(conn, sender, options, results):
    """Emit Request.Response after the method reply was sent."""
    path = request_path(sender, options)

    def emit():
        conn.emit_signal(
            sender, path, "org.freedesktop.portal.Request", "Response",
            GLib.Variant("(ua{sv})", (0, results)),
        )
        return False

    GLib.timeout_add(100, emit)
    return path


def settings_all():
    return {"org.freedesktop.appearance": {"color-scheme": GLib.Variant("u", state["scheme"])}}


def on_call(conn, sender, path, iface, method, params, inv):
    args = unpack(params)
    if iface == "org.freedesktop.Notifications":
        if method == "Notify":
            log(iface, method, args)
            nid[0] += 1
            inv.return_value(GLib.Variant("(u)", (nid[0],)))
        elif method == "GetCapabilities":
            inv.return_value(GLib.Variant("(as)", (["actions", "body", "body-markup", "icon-static"],)))
        elif method == "GetServerInformation":
            inv.return_value(GLib.Variant("(ssss)", ("mock", "ghostty-test", "1.0", "1.2")))
        else:
            log(iface, method, args)
            inv.return_value(None)
    elif iface == "org.freedesktop.systemd1.Manager":
        log(iface, method, args)
        if state.get("fail"):
            inv.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "mock failure")
            return
        inv.return_value(GLib.Variant("(o)", ("/org/freedesktop/systemd1/job/1",)))
    elif iface == "org.freedesktop.portal.GlobalShortcuts":
        log(iface, method, args)
        if method == "CreateSession":
            opts = args[0]
            sess = "/org/freedesktop/portal/desktop/session/" + sender[1:].replace(".", "_") + "/" + opts.get("session_handle_token", "s")
            state["session"] = sess
            state["sender"] = sender
            sess_info = Gio.DBusNodeInfo.new_for_xml(XML).lookup_interface("org.freedesktop.portal.Session")
            conn.register_object(sess, sess_info, on_call, None, None)
            rp = reply_request(conn, sender, opts, {"session_handle": GLib.Variant("s", sess)})
            inv.return_value(GLib.Variant("(o)", (rp,)))
        else:
            rp = reply_request(conn, sender, args[3], {"shortcuts": GLib.Variant("a(sa{sv})", [])})
            inv.return_value(GLib.Variant("(o)", (rp,)))
    elif iface == "org.freedesktop.portal.Session":
        log(iface, method, {"path": path})
        inv.return_value(None)
    elif iface == "org.freedesktop.portal.Settings":
        log(iface, method, args)
        if method == "ReadAll":
            inv.return_value(GLib.Variant("(a{sa{sv}})", (settings_all(),)))
        else:
            inv.return_value(GLib.Variant("(v)", (GLib.Variant("v", GLib.Variant("u", state["scheme"])),)))
    elif iface == "org.freedesktop.portal.OpenURI":
        log(iface, method, args)
        rp = reply_request(conn, sender, args[-1], {})
        inv.return_value(GLib.Variant("(o)", (rp,)))
    elif iface == "org.freedesktop.portal.Request":
        inv.return_value(None)
    elif iface == "test.ghostty.Mock":
        if method == "Activate":
            conn.emit_signal(
                None, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.GlobalShortcuts", "Activated",
                GLib.Variant("(osta{sv})", (state["session"], args[0], 0, {})),
            )
        elif method == "SetColorScheme":
            state["scheme"] = args[0]
            conn.emit_signal(
                None, "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings", "SettingChanged",
                GLib.Variant("(ssv)", ("org.freedesktop.appearance", "color-scheme", GLib.Variant("u", args[0]))),
            )
        elif method == "Invoke":
            conn.emit_signal(
                None, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked",
                GLib.Variant("(us)", (args[0], args[1])),
            )
        elif method == "SetFail":
            state["fail"] = args[0]
        elif method == "Quit":
            loop.quit()
        inv.return_value(None)


def on_prop(conn, sender, path, iface, name):
    return GLib.Variant("u", 1 if iface == "org.freedesktop.portal.Settings" else 3)


def on_bus(conn, name):
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    paths = {
        "org.freedesktop.Notifications": ["/org/freedesktop/Notifications"],
        "org.freedesktop.systemd1.Manager": ["/org/freedesktop/systemd1"],
        "org.freedesktop.portal.GlobalShortcuts": ["/org/freedesktop/portal/desktop"],
        "org.freedesktop.portal.Settings": ["/org/freedesktop/portal/desktop"],
        "org.freedesktop.portal.OpenURI": ["/org/freedesktop/portal/desktop"],
        "org.freedesktop.portal.Session": [],
    }
    for ifc in node.interfaces:
        for p in paths.get(ifc.name, []):
            conn.register_object(p, ifc, on_call, on_prop, None)
    ctrl = Gio.DBusNodeInfo.new_for_xml(CTRL_XML)
    conn.register_object("/test/ghostty/Mock", ctrl.interfaces[0], on_call, None, None)
    conns["c"] = conn


def acquired(conn, name):
    if name == "test.ghostty.Mock":
        sys.stdout.write("READY\n")
        sys.stdout.flush()


# Own all names on one connection; announce READY once the last is acquired.
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
on_bus(bus, None)
names = os.environ.get("MOCK_NAMES", "notifications,systemd,portal").split(",")
want = []
if "notifications" in names:
    want.append("org.freedesktop.Notifications")
if "systemd" in names:
    want.append("org.freedesktop.systemd1")
if "portal" in names:
    want.append("org.freedesktop.portal.Desktop")
want.append("test.ghostty.Mock")
for n in want:
    Gio.bus_own_name_on_connection(bus, n, Gio.BusNameOwnerFlags.NONE, acquired, None)
signal.signal(signal.SIGTERM, lambda *a: loop.quit())
GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, lambda *a: loop.quit())
loop.run()
