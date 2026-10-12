//! Differential-testing driver (Zig side). Usage:
//!   difftest_zig <cols> <rows> <max_scrollback> <input-file>
//! Feeds the file through the read-only stream and prints a canonical dump.
//! The Rust twin (tools/difftest/src/main.rs) must print byte-identical text.
const std = @import("std");
const vt = @import("ghostty-vt");

const Screen = vt.Screen;

fn color(w: *std.Io.Writer, c: vt.Style.Color) !void {
    switch (c) {
        .none => try w.writeAll("n"),
        .palette => |p| try w.print("p{d}", .{p}),
        .rgb => |r| try w.print("r{x:0>2}{x:0>2}{x:0>2}", .{ r.r, r.g, r.b }),
    }
}

fn styleStr(w: *std.Io.Writer, s: vt.Style) !void {
    try color(w, s.fg_color);
    try w.writeAll(",");
    try color(w, s.bg_color);
    try w.writeAll(",");
    try color(w, s.underline_color);
    const bits: u16 = @bitCast(s.flags);
    try w.print(",{x}", .{bits & 0x7ff});
}

fn charsetStr(w: *std.Io.Writer, c: Screen.CharsetState) !void {
    try w.print("gl={d} gr={d} ss={d} g={d},{d},{d},{d}", .{
        @intFromEnum(c.gl),
        @intFromEnum(c.gr),
        if (c.single_shift) |ss| @as(i32, @intFromEnum(ss)) else -1,
        @intFromEnum(c.charsets.g0),
        @intFromEnum(c.charsets.g1),
        @intFromEnum(c.charsets.g2),
        @intFromEnum(c.charsets.g3),
    });
}

fn dumpScreen(w: *std.Io.Writer, name: []const u8, s: *Screen) !void {
    try w.print("[screen {s}]\n", .{name});
    const c = s.cursor;
    try w.print("cursor {d} {d} wrap={d} prot={d} cstyle={d} style=", .{ c.x, c.y, @intFromBool(c.pending_wrap), @intFromBool(c.protected), @intFromEnum(c.cursor_style) });
    try styleStr(w, c.style);
    try w.print(" hl={d}\n", .{@intFromBool(c.hyperlink_id != 0)});
    if (s.saved_cursor) |sc| {
        try w.print("saved {d} {d} wrap={d} prot={d} origin={d} style=", .{ sc.x, sc.y, @intFromBool(sc.pending_wrap), @intFromBool(sc.protected), @intFromBool(sc.origin) });
        try styleStr(w, sc.style);
        try w.writeAll(" ");
        try charsetStr(w, sc.charset);
        try w.writeAll("\n");
    } else try w.writeAll("saved none\n");
    try w.writeAll("charset ");
    try charsetStr(w, s.charset);
    try w.writeAll("\n");
    try w.print("protected_mode {d}\n", .{@intFromEnum(s.protected_mode)});
    try w.print("kitty_kb {d}\n", .{@as(u5, @bitCast(s.kitty_keyboard.current()))});
    try w.print("rows {d}\n", .{s.pages.total_rows});
    var y: usize = 0;
    while (y < s.pages.total_rows) : (y += 1) {
        const cell = s.pages.getCell(.{ .screen = .{ .x = 0, .y = @intCast(y) } }).?;
        const row = cell.row;
        const pin = s.pages.pin(.{ .screen = .{ .x = 0, .y = @intCast(y) } }).?;
        const cells = pin.cells(.all);
        try w.print("row {d} w={d} wc={d} sp={d} kv={d} |", .{
            y,
            @intFromBool(row.wrap),
            @intFromBool(row.wrap_continuation),
            @intFromEnum(row.semantic_prompt),
            @intFromBool(row.kitty_virtual_placeholder),
        });
        for (cells, 0..) |*cl, x| {
            const st: vt.Style = if (cl.style_id == 0) .{} else pin.node.data.styles.get(pin.node.data.memory, cl.style_id).*;
            const plain = cl.content_tag == .codepoint and cl.content.codepoint == 0 and
                cl.wide == .narrow and !cl.protected and !cl.hyperlink and
                @intFromEnum(cl.semantic_content) == 0 and st.eql(.{});
            if (plain) continue;
            try w.print(" {d}:{d}:", .{ x, @intFromEnum(cl.content_tag) });
            switch (cl.content_tag) {
                .codepoint, .codepoint_grapheme => try w.print("{x}", .{cl.content.codepoint}),
                .bg_color_palette => try w.print("{d}", .{cl.content.color_palette}),
                .bg_color_rgb => try w.print("{x:0>2}{x:0>2}{x:0>2}", .{ cl.content.color_rgb.r, cl.content.color_rgb.g, cl.content.color_rgb.b }),
            }
            if (cl.content_tag == .codepoint_grapheme) {
                if (pin.node.data.lookupGrapheme(cl)) |g| {
                    for (g) |cp| try w.print("+{x}", .{cp});
                } else try w.writeAll("+?");
            }
            try w.print(":{d}:{s}{s}{d}:", .{
                @intFromEnum(cl.wide),
                if (cl.protected) "p" else "",
                if (cl.hyperlink) "h" else "",
                @intFromEnum(cl.semantic_content),
            });
            try styleStr(w, st);
            if (cl.hyperlink) {
                const page = &pin.node.data;
                if (page.lookupHyperlink(cl)) |hid| {
                    const e = page.hyperlink_set.get(page.memory, hid);
                    try w.print(" @{s}", .{e.uri.slice(page.memory)});
                    switch (e.id) {
                        .explicit => |ex| try w.print("@E:{s}", .{ex.slice(page.memory)}),
                        .implicit => |n| try w.print("@I:{d}", .{n}),
                    }
                } else try w.writeAll(" @?");
            }
        }
        try w.writeAll("\n");
    }
}

pub fn main() !void {
    var gpa: std.heap.DebugAllocator(.{}) = .init;
    defer _ = gpa.deinit();
    const alloc = gpa.allocator();

    const args = try std.process.argsAlloc(alloc);
    defer std.process.argsFree(alloc, args);
    if (args.len != 5) return error.Usage;
    const cols = try std.fmt.parseInt(u16, args[1], 10);
    const rows = try std.fmt.parseInt(u16, args[2], 10);
    const max_sb = try std.fmt.parseInt(usize, args[3], 10);
    const input = try std.fs.cwd().readFileAlloc(alloc, args[4], 1 << 30);
    defer alloc.free(input);

    var t: vt.Terminal = try .init(alloc, .{ .cols = cols, .rows = rows, .max_scrollback = max_sb });
    defer t.deinit(alloc);

    var s: vt.ReadonlyStream = .initAlloc(alloc, .init(&t));
    defer s.deinit();
    try s.nextSlice(input);

    var buf: [8192]u8 = undefined;
    var fw = std.fs.File.stdout().writer(&buf);
    const w = &fw.interface;

    try w.print("size {d} {d}\n", .{ t.cols, t.rows });
    try w.print("active {s}\n", .{@tagName(t.screens.active_key)});
    const sr = t.scrolling_region;
    try w.print("region t={d} b={d} l={d} r={d}\n", .{ sr.top, sr.bottom, sr.left, sr.right });
    try w.writeAll("modes");
    inline for (@typeInfo(vt.Mode).@"enum".fields) |f| {
        const m: vt.Mode = @enumFromInt(f.value);
        if (t.modes.get(m)) try w.print(" {s}", .{f.name});
    }
    try w.writeAll("\nsaved_modes");
    inline for (@typeInfo(vt.Mode).@"enum".fields) |f| {
        const bit = @field(t.modes.saved, f.name);
        if (bit) try w.print(" {s}", .{f.name});
    }
    try w.writeAll("\ntabstops");
    for (0..t.cols) |x| if (t.tabstops.get(x)) try w.print(" {d}", .{x});
    try w.print("\nflags srp={d} mok2={d} mev={d} mfmt={d} msc={d}\n", .{
        @intFromEnum(t.flags.shell_redraws_prompt),
        @intFromBool(t.flags.modify_other_keys_2),
        @intFromEnum(t.flags.mouse_event),
        @intFromEnum(t.flags.mouse_format),
        @intFromEnum(t.flags.mouse_shift_capture),
    });
    try w.print("status_display {d}\n", .{@intFromEnum(t.status_display)});
    try w.print("mouse_shape {d}\n", .{@intFromEnum(t.mouse_shape)});
    try w.print("pwd {s}\n", .{t.pwd.items});
    if (t.previous_char) |pc| try w.print("prev {x}\n", .{pc}) else try w.writeAll("prev none\n");
    inline for (.{ "foreground", "background", "cursor" }) |n| {
        const d = @field(t.colors, n);
        try w.print("color {s}", .{n});
        if (d.override) |o| try w.print(" o={x:0>2}{x:0>2}{x:0>2}", .{ o.r, o.g, o.b }) else try w.writeAll(" o=-");
        if (d.default) |o| try w.print(" d={x:0>2}{x:0>2}{x:0>2}", .{ o.r, o.g, o.b }) else try w.writeAll(" d=-");
        try w.writeAll("\n");
    }
    try w.writeAll("palette");
    for (t.colors.palette.current, 0..) |c, i| {
        const o = t.colors.palette.original[i];
        if (c.r != o.r or c.g != o.g or c.b != o.b)
            try w.print(" {d}={x:0>2}{x:0>2}{x:0>2}", .{ i, c.r, c.g, c.b });
    }
    try w.writeAll("\n");

    if (t.screens.get(.primary)) |sc| try dumpScreen(w, "primary", sc);
    if (t.screens.get(.alternate)) |sc| try dumpScreen(w, "alternate", sc);

    try w.writeAll("[plain]\n");
    const str = try t.plainString(alloc);
    defer alloc.free(str);
    try w.writeAll(str);
    try w.writeAll("\n[end]\n");
    try w.flush();
}
