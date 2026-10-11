# ghostty-config: types_b.rs (Config.zig 8566-10250)

Ported: CopyOnSelect .. NotifyOnCommandFinishAction (all enums via config_enum!, packed structs via
config_packed!), RepeatableCommand, MouseScrollMultiplier, QuickTerminalSize (+Value, Dimensions),
BackgroundBlur, WindowDecoration, Theme, Duration, WindowPadding, AlphaBlending::is_linear.

Tests: 21 ported/added (all Zig tests in range except 2 skipped: "RepeatableCommand cval", "RepeatableCommand cval cleared" -- C API).

Deviations / notes:
- OSCColorReportFormat variants are `Bit8`/`Bit16` ("8-bit"/"16-bit"). Enum variants for `false`/`true`/`none` are `False`/`True`/`None`.
- WindowDecoration, Duration, WindowPadding have inherent `parse(Option<&str>)` (Zig 1-arg parseCLI) used by ConfigField::parse_cli.
- QuickTerminalSize is `{primary, secondary: Option<QuickTerminalSizeValue>}`; formats nothing when primary is None (as Zig).
- Skipped C-API: cval/C structs, getGObjectType, c_int discriminants.
- GtkSingleInstance::DEFAULT, MouseScrollMultiplier::DEFAULT mirror Zig `default` consts (Default::default for MouseScrollMultiplier is 1/3).

TODO(integration):
- `Command` is a local shim (action: ghostty_input::binding::Action, title, description) with `Command::parse_auto`; replace with ghostty_input::command::Command when it exists (input/command.rs was a stub).
- `default_commands()` returns an empty Vec; must return ghostty_input::command::defaults (used by RepeatableCommand::reset == Zig `init`).
- Requires ghostty_input::binding::Action: Display, PartialEq, Clone, parse().
