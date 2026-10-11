//! Key event encoding: legacy (xterm/fixterms) and Kitty keyboard protocol
//! (ZIG/src/input/key_encode.zig).

use std::io::{self, Write};

use ghostty_vt::kitty::key::Flags as KittyFlags;

use crate::{
    config::OptionAsAlt,
    function_keys::{self, CursorMode, KeypadMode, ModifyKeys},
    key::{self, Key, KeyEvent},
    kitty::{self as kitty_table, Entry as KittyEntry},
};

/// Options that affect key encoding behavior. This is a mix of behavior
/// from terminal state as well as application configuration.
///
/// TODO(integration): Zig has `Options.fromTerminal(*const Terminal)`; the
/// terminal crate should construct this from its modes (`alt_esc_prefix`,
/// `cursor_keys`, `keypad_keys`, `ignore_keypad_with_numlock`, flag
/// `modify_other_keys_2`, active screen `kitty_keyboard.current()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Options {
    /// Terminal DEC mode 1
    pub cursor_key_application: bool,

    /// Terminal DEC mode 66
    pub keypad_key_application: bool,

    /// Terminal DEC mode 1035
    pub ignore_keypad_with_numlock: bool,

    /// Terminal DEC mode 1036
    pub alt_esc_prefix: bool,

    /// xterm "modifyOtherKeys mode 2". Details here:
    /// https://invisible-island.net/xterm/modified-keys.html
    pub modify_other_keys_state_2: bool,

    /// Kitty keyboard protocol flags.
    pub kitty_flags: KittyFlags,

    /// Determines whether the "option" key on macOS is treated
    /// as "alt" or not. See the Ghostty `macos_option-as-alt` config
    /// docs for a more detailed description of why this is needed.
    pub macos_option_as_alt: OptionAsAlt,
}

impl Options {
    pub const DEFAULT: Options = Options {
        cursor_key_application: false,
        keypad_key_application: false,
        ignore_keypad_with_numlock: false,
        alt_esc_prefix: false,
        modify_other_keys_state_2: false,
        kitty_flags: KittyFlags::DISABLED,
        macos_option_as_alt: OptionAsAlt::False,
    };
}

/// Encode the key event to the writer in the proper format given
/// the options. For example, this will properly encode a key press
/// such as "ctrl+A" to Kitty format if Kitty encoding is enabled.
///
/// Not all key events will result in output. It is up to the caller
/// to use a writer that can track whether any output was written if
/// they care about that.
pub fn encode<W: Write>(writer: &mut W, event: &KeyEvent, opts: &Options) -> io::Result<()> {
    if opts.kitty_flags.int() != 0 {
        kitty(writer, event, opts)
    } else {
        legacy(writer, event, opts)
    }
}

fn is_enter_backspace_tab(k: Key) -> bool {
    matches!(k, Key::Enter | Key::Backspace | Key::Tab)
}

/// Perform Kitty keyboard protocol encoding of the key event.
fn kitty<W: Write>(writer: &mut W, event: &KeyEvent, opts: &Options) -> io::Result<()> {
    // This should never happen but we'll check anyway.
    if opts.kitty_flags.int() == 0 {
        return legacy(writer, event, opts);
    }

    // We only processed "press" events unless report events is active
    if event.action == key::Action::Release {
        if !opts.kitty_flags.report_events {
            return Ok(());
        }

        // Enter, backspace, and tab do not report release events unless "report
        // all" is set
        if !opts.kitty_flags.report_all && is_enter_backspace_tab(event.key) {
            return Ok(());
        }
    }

    let all_mods = event.mods;
    let effective_mods = event.effective_mods();
    let binding_mods = effective_mods.binding();

    // Find the entry for this key in the kitty table.
    let entry_: Option<KittyEntry> = 'entry: {
        // Functional or predefined keys
        for entry in kitty_table::entries() {
            if entry.key == event.key {
                break 'entry Some(*entry);
            }
        }

        // Otherwise, we use our unicode codepoint from UTF8. We
        // always use the unshifted value.
        if event.unshifted_codepoint > 0 {
            break 'entry Some(KittyEntry {
                key: event.key,
                code: event.unshifted_codepoint,
                final_: b'u',
                modifier: false,
            });
        }

        None
    };

    'preprocessing: {
        // When composing, the only keys sent are plain modifiers.
        if event.composing {
            if let Some(entry) = &entry_
                && entry.modifier
            {
                break 'preprocessing;
            }

            return Ok(());
        }

        // IME confirmation still sends an enter key so if we have enter
        // and UTF8 text we just send it directly since we assume that is
        // what's happening. See legacy()'s similar logic for more details
        // on how to verify this.
        if !event.utf8.is_empty() && matches!(event.key, Key::Enter | Key::Backspace) {
            // See legacy for why we handle this this way.
            if !is_control_utf8(event.utf8) {
                if event.key == Key::Backspace {
                    return Ok(());
                }
                return writer.write_all(event.utf8.as_bytes());
            }
        }

        // If we're reporting all then we always send CSI sequences.
        if !opts.kitty_flags.report_all {
            // Quote:
            // The only exceptions are the Enter, Tab and Backspace keys which
            // still generate the same bytes as in legacy mode this is to allow the
            // user to type and execute commands in the shell such as reset after a
            // program that sets this mode crashes without clearing it.
            //
            // Quote ("report all" mode):
            // Note that all keys are reported as escape codes, including Enter,
            // Tab, Backspace etc.
            if binding_mods.empty() {
                match event.key {
                    Key::Enter => return writer.write_all(b"\r"),
                    Key::Tab => return writer.write_all(b"\t"),
                    Key::Backspace => return writer.write_all(&[0x7F]),
                    _ => {}
                }
            }

            // Send plain-text non-modified text directly to the terminal.
            // We don't send release events because those are specially encoded.
            if !event.utf8.is_empty()
                && binding_mods.empty()
                && event.action != key::Action::Release
            {
                // We only do this for printable characters. We should
                // inspect the real unicode codepoint properties here but
                // the real world issue is usually control characters.
                if !event.utf8.chars().any(|c| is_control(u32::from(c))) {
                    return writer.write_all(event.utf8.as_bytes());
                }
            }
        }
    }

    let Some(entry) = entry_ else {
        // No entry found. If we have UTF-8 text this is a pure text event
        // (e.g. composed/IME text), so send it as-is so programs can
        // still receive it.
        if !event.utf8.is_empty() {
            return writer.write_all(event.utf8.as_bytes());
        }
        return Ok(());
    };

    // If this is just a modifier we require "report all" to send the sequence.
    if entry.modifier && !opts.kitty_flags.report_all {
        return Ok(());
    }

    let seq = {
        let mut seq = KittySequence {
            key: entry.code,
            final_: entry.final_,
            mods: KittyMods::from_input(event.action, event.key, all_mods),
            event: KittyEvent::None,
            alternates: [None, None],
            text: "",
        };

        if opts.kitty_flags.report_events {
            seq.event = match event.action {
                key::Action::Press => KittyEvent::Press,
                key::Action::Release => KittyEvent::Release,
                key::Action::Repeat => KittyEvent::Repeat,
            };
        }

        'alternates: {
            if !opts.kitty_flags.report_alternates {
                break 'alternates;
            }

            // Break early if this is a control key
            if is_control(seq.key) {
                break 'alternates;
            }

            let mut it = event.utf8.chars();

            // If we have a codepoint in our UTF-8 sequence, then we can
            // report the shifted version.
            if let Some(cp1) = it.next().map(u32::from) {
                // Set the first alternate (shifted version)
                if cp1 != seq.key && seq.mods.shift {
                    seq.alternates[0] = Some(cp1);
                }

                // We want to know if there are additional codepoints because
                // our logic below depends on the utf8 being a single codepoint.
                let has_cp2 = it.next().is_some();

                // Set the base layout key. We only report this if this codepoint
                // differs from our pressed key.
                if let Some(base) = event.key.codepoint()
                    && base != seq.key
                    && (cp1 != base && !has_cp2)
                {
                    seq.alternates[1] = Some(base);
                }
            } else {
                // No UTF-8 so we can't report a shifted key but we can still
                // report a base layout key.
                if let Some(base) = event.key.codepoint()
                    && base != seq.key
                {
                    seq.alternates[1] = Some(base);
                }
            }
        }

        if opts.kitty_flags.report_associated && seq.event != KittyEvent::Release {
            // Determine if the Alt modifier should be treated as an actual
            // modifier (in which case it prevents associated text) or as
            // the macOS Option key, which does not prevent associated text.
            // (Non-macOS: always true.)
            let alt_prevents_text = true;

            if !seq.mods.prevents_text(alt_prevents_text) {
                seq.text = event.utf8;
            }
        }

        seq
    };

    seq.encode(writer)
}

/// Perform legacy encoding of the key event. "Legacy" in this case
/// is referring to the behavior of traditional terminals, plus
/// xterm's `modifyOtherKeys`, plus Paul Evans's "fixterms" spec.
/// These together combine the legacy protocol because they're all
/// meant to be extensions that do not change any existing behavior
/// and therefore safe to combine.
fn legacy<W: Write>(writer: &mut W, event: &KeyEvent, opts: &Options) -> io::Result<()> {
    let all_mods = event.mods;
    let effective_mods = event.effective_mods();
    let binding_mods = effective_mods.binding();

    // Legacy encoding only does press/repeat
    if event.action != key::Action::Press && event.action != key::Action::Repeat {
        return Ok(());
    }

    // If we're in a dead key state then we never emit a sequence.
    if event.composing {
        return Ok(());
    }

    // If we match a PC style function key then that is our result.
    if let Some(sequence) = pc_style_function_key(
        event.key,
        all_mods,
        opts.cursor_key_application,
        opts.keypad_key_application,
        opts.ignore_keypad_with_numlock,
        opts.modify_other_keys_state_2,
    ) {
        // If we have UTF-8 text, then we never emit PC style function
        // keys. Many function keys (escape, enter, backspace) have
        // a specific meaning when dead keys are active and so we don't
        // want to send that to the terminal. Examples:
        //
        //   - Japanese: escape clears the dead key state
        //   - Korean: escape commits the dead key state
        //   - Korean: backspace should delete a single preedit char
        //
        let mut skip_pc_style = false;
        if !event.utf8.is_empty()
            && matches!(event.key, Key::Backspace | Key::Enter | Key::Escape)
            // We want to ignore control characters. This is because
            // some apprts (macOS) will send control characters as
            // UTF-8 encodings and we handle that manually.
            && !is_control_utf8(event.utf8)
        {
            // Backspace encodes nothing because we modified IME.
            // Enter/escape don't encode the PC-style encoding
            // because we want to encode committed text.
            if event.key == Key::Backspace {
                return Ok(());
            }
            skip_pc_style = true;
        }

        if !skip_pc_style {
            return writer.write_all(sequence.as_bytes());
        }
    }

    // If we match a control sequence, we output that directly. For
    // ctrlSeq we have to use all mods because we want it to only
    // match ctrl+<char>.
    if let Some(ch) = ctrl_seq(event.key, event.utf8, event.unshifted_codepoint, all_mods) {
        // C0 sequences support alt-as-esc prefixing.
        if binding_mods.alt {
            return writer.write_all(&[0x1B, ch]);
        }

        return writer.write_all(&[ch]);
    }

    // If we have no UTF8 text then the only possibility is the
    // alt-prefix handling of unshifted codepoints... so we process that.
    let utf8 = event.utf8;
    if utf8.is_empty() {
        if let Some(byte) = legacy_alt_prefix(event, binding_mods, all_mods, opts) {
            writer.write_all(&[0x1B, byte])?;
        }
        return Ok(());
    }

    // In modify other keys state 2, we send the CSI 27 sequence
    // for any char with a modifier. Ctrl sequences like Ctrl+a
    // are already handled above.
    if opts.modify_other_keys_state_2 {
        let mut it = utf8.chars();
        // We only do this if we have a single codepoint. There shouldn't
        // ever be a multi-codepoint sequence that triggers this.
        if let (Some(codepoint), None) = (it.next().map(u32::from), it.clone().next()) {
            // The mods we encode for this are just the binding mods (shift, ctrl,
            // super, alt unless it is actually option).
            let mods = event.mods.binding();

            // This copies xterm's `ModifyOtherKeys` function that returns
            // whether modify other keys should be encoded for the given
            // input.
            let should_modify = 'should_modify: {
                // xterm IsControlInput
                if (0x40..=0x7F).contains(&codepoint) {
                    break 'should_modify true;
                }

                // If we have anything other than shift pressed, encode.
                let mut mods_no_shift = mods;
                mods_no_shift.shift = false;
                if !mods_no_shift.empty() {
                    break 'should_modify true;
                }

                // We only have shift pressed. We only allow space.
                if codepoint == u32::from(' ') {
                    break 'should_modify true;
                }

                // This logic isn't complete but I don't fully understand
                // the rest so I'm going to wait until we can have a
                // reasonable test scenario.
                false
            };

            if should_modify {
                for (modset, code) in function_keys::MODIFIERS.iter().zip(2..) {
                    if !mods.equal(*modset) {
                        continue;
                    }
                    return write!(writer, "\x1B[27;{code};{codepoint}~");
                }
            }
        }
    }

    // Let's see if we should apply fixterms to this codepoint.
    // At this stage of key processing, we only need to apply fixterms
    // to unicode codepoints if we have ctrl set.
    if event.mods.ctrl {
        'csiu: {
            // Important: we want to use the original mods here, not the
            // effective mods. The fixterms spec states the shifted chars
            // should be sent uppercase but Kitty changes that behavior
            // so we'll send all the mods.
            let mut csi_u_mods = CsiUMods::from_input(event.mods);

            // Get our codepoint. If we have more than one codepoint this
            // can't be valid CSIu.
            let mut it = event.utf8.chars();
            let Some(first) = it.next() else {
                break 'csiu;
            };
            if it.next().is_some() {
                break 'csiu;
            }
            let mut ch = u32::from(first);

            // If our character is A to Z and we have shift set, then
            // we lowercase it. This is a Kitty-specific behavior that
            // we choose to follow and diverge from the fixterms spec.
            // This makes it easier for programs to detect shifted letters
            // for keybindings and is not just theoretical but used by
            // real programs.
            if (u32::from('A')..=u32::from('Z')).contains(&ch) && csi_u_mods.shift {
                // We want to rely on apprt to send us the correct
                // unshifted codepoint...
                ch += 32;
            }

            // If our unshifted codepoint is identical to the shifted
            // then we consider shift. Otherwise, we do not because the
            // shift key was used to obtain the character. This is specified
            // by fixterms.
            if event.unshifted_codepoint != ch {
                csi_u_mods.shift = false;
            }

            return write!(writer, "\x1B[{};{}u", ch, csi_u_mods.seq_int());
        }
    }

    // If we have alt-pressed and alt-esc-prefix is enabled, then
    // we need to prefix the utf8 sequence with an esc.
    if let Some(byte) = legacy_alt_prefix(event, binding_mods, all_mods, opts) {
        return writer.write_all(&[0x1B, byte]);
    }

    // (On macOS, command+keys do not encode text. On Linux we continue to
    // encode text because it is typical.)

    writer.write_all(utf8.as_bytes())
}

fn legacy_alt_prefix(
    event: &KeyEvent,
    binding_mods: key::Mods,
    _mods: key::Mods,
    opts: &Options,
) -> Option<u8> {
    // This only takes effect with alt pressed
    if !binding_mods.alt || !opts.alt_esc_prefix {
        return None;
    }

    // (macOS option-as-alt handling does not apply on Linux.)

    // Otherwise, we require utf8 to already have the byte represented.
    let utf8 = event.utf8.as_bytes();
    if utf8.len() == 1 {
        return Some(utf8[0]);
    }

    // If UTF8 isn't set, we will allow unshifted codepoints through.
    if event.unshifted_codepoint > 0
        && let Ok(byte) = u8::try_from(event.unshifted_codepoint)
    {
        return Some(byte);
    }

    // Else, we can't figure out the byte to alt-prefix so we
    // exit this handling.
    None
}

/// Determines whether the key should be encoded in the xterm
/// "PC-style Function Key" syntax (roughly). This is a hardcoded
/// table of keys and modifiers that result in a specific sequence.
fn pc_style_function_key(
    keyval: Key,
    mods: key::Mods,
    cursor_key_application: bool,
    keypad_key_application_req: bool,
    ignore_keypad_with_numlock: bool,
    modify_other_keys: bool, // True if state 2
) -> Option<&'static str> {
    // We only want binding-sensitive mods because lock keys
    // and directional modifiers (left/right) don't matter for
    // pc-style function keys.
    let mods_int = mods.binding().int();

    // Keypad application keymode isn't super straightforward.
    // On xterm, in VT220 mode, numlock alone is enough to trigger
    // application mode. But in more modern modes, numlock is
    // ignored by default via mode 1035 (default true). If mode
    // 1035 is on, we always are in numerical keypad mode. If
    // mode 1035 is off, we are in application mode if the
    // proper numlock state is pressed. The numlock state is implicitly
    // determined based on the keycode sent (i.e. 1 with numlock
    // on will be kp_end).
    //
    // If we're ignoring keypad then this is always false.
    // In other words, we're always in numerical keypad mode.
    let keypad_key_application = if ignore_keypad_with_numlock {
        false
    } else {
        keypad_key_application_req
    };

    for entry in function_keys::keys(keyval) {
        match entry.cursor {
            CursorMode::Any => {}
            CursorMode::Normal => {
                if cursor_key_application {
                    continue;
                }
            }
            CursorMode::Application => {
                if !cursor_key_application {
                    continue;
                }
            }
        }

        match entry.keypad {
            KeypadMode::Any => {}
            KeypadMode::Normal => {
                if keypad_key_application {
                    continue;
                }
            }
            KeypadMode::Application => {
                if !keypad_key_application {
                    continue;
                }
            }
        }

        match entry.modify_other_keys {
            ModifyKeys::Any => {}
            ModifyKeys::Set => {
                if modify_other_keys {
                    continue;
                }
            }
            ModifyKeys::SetOther => {
                if !modify_other_keys {
                    continue;
                }
            }
        }

        let entry_mods_int = entry.mods.int();
        if entry_mods_int == 0 {
            if mods_int != 0 && !entry.mods_empty_is_any {
                continue;
            }
            // mods are either empty, or empty means any so we allow it.
        } else if entry_mods_int != mods_int {
            // any set mods require an exact match
            continue;
        }

        return Some(entry.sequence.as_str());
    }

    None
}

/// Returns the C0 byte for the key event if it should be used.
/// This converts a key event into the expected terminal behavior
/// such as Ctrl+C turning into 0x03, amongst many other translations.
///
/// This will return None if the key event should not be converted
/// into a C0 byte. There are many cases for this and you should read
/// the source code to understand them.
fn ctrl_seq(logical_key: Key, utf8: &str, unshifted_codepoint: u32, mods: key::Mods) -> Option<u8> {
    let ctrl_only = key::Mods {
        ctrl: true,
        ..key::Mods::default()
    }
    .int();

    // If ctrl is not pressed then we never do anything.
    if !mods.ctrl {
        return None;
    }

    // We need to only get binding modifiers so we strip lock
    // keys, sides, etc.
    let mut unset_mods = mods.binding();

    // Remove alt from our modifiers because it does not impact whether
    // we are generating a ctrl sequence and we handle the ESC-prefix
    // logic separately.
    unset_mods.alt = false;

    let mut ch: u8 = 'ch: {
        // If we have exactly one UTF8 byte, we assume that is the
        // character we want to convert to a C0 byte.
        if utf8.len() == 1 {
            break 'ch utf8.as_bytes()[0];
        }

        // If we have a logical key that maps to a single byte
        // printable character, we use that. History to explain this:
        // this was added to support cyrillic keyboard layouts such
        // as Russian and Mongolian. These layouts have a `c` key that
        // maps to U+0441 (cyrillic small letter "c") but every
        // terminal I've tested encodes this as ctrl+c.
        if let Some(cp) = logical_key.codepoint()
            && let Ok(byte) = u8::try_from(cp)
        {
            // For this specific case, we only map to the key if
            // we have exactly ctrl pressed. This is because shift
            // would modify the key and we don't know how to do that
            // properly here (don't have the layout). And we want
            // to encode shift as CSIu.
            if unset_mods.int() != ctrl_only {
                return None;
            }
            break 'ch byte;
        }

        // Otherwise we don't have a character to convert that
        // we can reliably map to a C0 byte.
        return None;
    };

    // Remove shift if we have something outside of the US letter
    // range. This is so that characters such as `ctrl+shift+-`
    // generate the correct ctrl-seq (used by emacs).
    if unset_mods.shift && !ch.is_ascii_uppercase() {
        // Special case for fixterms awkward case as specified.
        if ch != b'@' {
            unset_mods.shift = false;
        }
    }

    // If the character is uppercase, we convert it to lowercase. We
    // rely on the unshifted codepoint to do this. This handles
    // the scenario where we have caps lock pressed. Note that
    // shifted characters are handled above, if we are just pressing
    // shift then the ctrl-only check will fail later and we won't
    // ctrl-seq encode.
    if ch.is_ascii_uppercase()
        && unshifted_codepoint > 0
        && let Ok(byte) = u8::try_from(unshifted_codepoint)
    {
        ch = byte;
    }

    // An additional note on caps lock and shift interaction.
    // If we have caps lock set and an ASCII letter is pressed,
    // we lowercase it (above). If we have only control pressed,
    // we process it as a ctrl seq. For example ctrl+M with caps
    // lock but no shift will encode as 0x0D.
    //
    // But, if you press ctrl+shift+m, this will not encode as a
    // ctrl-seq and falls through to CSIu encoding. This lets programs
    // detect the difference between ctrl+M and ctrl+shift+M. This
    // diverges from the fixterms "spec" and most terminals. This
    // only matches Kitty in behavior. But I believe this is a
    // justified divergence because it's a useful distinction.

    // After unsetting, we only continue if we have ONLY control set.
    if unset_mods.int() != ctrl_only {
        return None;
    }

    // From Kitty's key encoding logic. I tried to discern the exact
    // behavior across different terminals but it's not clear, so I'm
    // just going to repeat what Kitty does.
    match ch {
        b' ' => Some(0),
        b'/' => Some(31),
        b'0' => Some(48),
        b'1' => Some(49),
        b'2' => Some(0),
        b'3' => Some(27),
        b'4' => Some(28),
        b'5' => Some(29),
        b'6' => Some(30),
        b'7' => Some(31),
        b'8' => Some(127),
        b'9' => Some(57),
        b'?' => Some(127),
        b'@' => Some(0),
        b'\\' => Some(28),
        b']' => Some(29),
        b'^' => Some(30),
        b'_' => Some(31),
        b'a' => Some(1),
        b'b' => Some(2),
        b'c' => Some(3),
        b'd' => Some(4),
        b'e' => Some(5),
        b'f' => Some(6),
        b'g' => Some(7),
        b'h' => Some(8),
        b'j' => Some(10),
        b'k' => Some(11),
        b'l' => Some(12),
        b'n' => Some(14),
        b'o' => Some(15),
        b'p' => Some(16),
        b'q' => Some(17),
        b'r' => Some(18),
        b's' => Some(19),
        b't' => Some(20),
        b'u' => Some(21),
        b'v' => Some(22),
        b'w' => Some(23),
        b'x' => Some(24),
        b'y' => Some(25),
        b'z' => Some(26),
        b'~' => Some(30),

        // These are purposely NOT handled here because of the fixterms
        // specification: https://www.leonerd.org.uk/hacks/fixterms/
        // These are processed as CSI u.
        // 'i' => 0x09,
        // 'm' => 0x0D,
        // '[' => 0x1B,
        _ => None,
    }
}

/// Returns true if this is an ASCII control character, matches libc implementation.
fn is_control(cp: u32) -> bool {
    cp < 0x20 || cp == 0x7F
}

/// Returns true if this string is comprised of a single
/// control character. This returns false for multi-byte strings.
fn is_control_utf8(s: &str) -> bool {
    s.len() == 1 && is_control(u32::from(s.as_bytes()[0]))
}

/// This is the bitmask for fixterm CSI u modifiers.
#[derive(Debug, Clone, Copy, Default)]
struct CsiUMods {
    shift: bool,
    alt: bool,
    ctrl: bool,
}

impl CsiUMods {
    /// Convert an input mods value into the CSI u mods value.
    fn from_input(mods: key::Mods) -> CsiUMods {
        CsiUMods {
            shift: mods.shift,
            alt: mods.alt,
            ctrl: mods.ctrl,
        }
    }

    /// Returns the raw int value of this packed struct.
    fn int(self) -> u8 {
        u8::from(self.shift) | u8::from(self.alt) << 1 | u8::from(self.ctrl) << 2
    }

    /// Returns the integer value sent as part of the CSI u sequence.
    /// This adds 1 to the bitmask value as described in the spec.
    fn seq_int(self) -> u8 {
        self.int() + 1
    }
}

/// This is the bitfields for Kitty modifiers.
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
struct KittyMods {
    shift: bool,
    alt: bool,
    ctrl: bool,
    super_: bool,
    hyper: bool,
    meta: bool,
    caps_lock: bool,
    num_lock: bool,
}

impl KittyMods {
    /// Convert an input mods value into the CSI u mods value.
    fn from_input(_action: key::Action, _k: Key, mods: key::Mods) -> KittyMods {
        KittyMods {
            shift: mods.shift,
            alt: mods.alt,
            ctrl: mods.ctrl,
            super_: mods.super_,
            hyper: false,
            meta: false,
            caps_lock: mods.caps_lock,
            num_lock: mods.num_lock,
        }
    }

    /// Returns true if the modifiers prevent printable text.
    ///
    /// The alt_prevents_text parameter determines whether or not the Alt
    /// modifier prevents printable text. On Linux, this is always true. On
    /// macOS, this is only true if macos-option-as-alt is set.
    fn prevents_text(self, alt_prevents_text: bool) -> bool {
        (self.alt && alt_prevents_text) || self.ctrl || self.super_ || self.hyper || self.meta
    }

    /// Returns the raw int value of this packed struct.
    fn int(self) -> u16 {
        u16::from(self.shift)
            | u16::from(self.alt) << 1
            | u16::from(self.ctrl) << 2
            | u16::from(self.super_) << 3
            | u16::from(self.hyper) << 4
            | u16::from(self.meta) << 5
            | u16::from(self.caps_lock) << 6
            | u16::from(self.num_lock) << 7
    }

    /// Returns the integer value sent as part of the Kitty sequence.
    /// This adds 1 to the bitmask value as described in the spec.
    fn seq_int(self) -> u16 {
        self.int() + 1
    }
}

/// Values for the event code (see "event-type" in the KittySequence comment).
/// Note that Kitty omits the ":1" for the press event but other
/// terminals include it. We'll include it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KittyEvent {
    None = 0,
    Press = 1,
    Repeat = 2,
    Release = 3,
}

/// Represents a kitty key sequence and has helpers for encoding it.
/// The sequence from the Kitty specification:
///
/// CSI unicode-key-code:alternate-key-codes ; modifiers:event-type ; text-as-codepoints u
#[derive(Debug, Clone, Copy)]
struct KittySequence<'a> {
    key: u32,
    final_: u8,
    mods: KittyMods,
    event: KittyEvent,
    alternates: [Option<u32>; 2],
    text: &'a str,
}

impl KittySequence<'_> {
    fn encode<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        if self.final_ == b'u' || self.final_ == b'~' {
            return self.encode_full(writer);
        }
        self.encode_special(writer)
    }

    fn encode_full<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        // Key section
        write!(writer, "\x1B[{}", self.key)?;
        // Write our alternates
        if let Some(shifted) = self.alternates[0] {
            write!(writer, ":{shifted}")?;
        }
        if let Some(base) = self.alternates[1] {
            if self.alternates[0].is_none() {
                write!(writer, "::{base}")?;
            } else {
                write!(writer, ":{base}")?;
            }
        }

        // Mods and events section
        let mods = self.mods.seq_int();
        let mut emit_prior = false;
        if self.event != KittyEvent::None && self.event != KittyEvent::Press {
            write!(writer, ";{}:{}", mods, self.event as u8)?;
            emit_prior = true;
        } else if mods > 1 {
            write!(writer, ";{mods}")?;
            emit_prior = true;
        }

        // Text section
        if !self.text.is_empty() {
            let mut count = 0;
            for c in self.text.chars() {
                let cp = u32::from(c);
                // If the codepoint is non-printable ASCII character, skip.
                if is_control(cp) {
                    continue;
                }

                // We need to add our ";". We need to add two if we didn't emit
                // the modifier section. We only do this initially.
                if count == 0 {
                    if !emit_prior {
                        writer.write_all(b";")?;
                    }
                    writer.write_all(b";")?;
                } else {
                    writer.write_all(b":")?;
                }

                write!(writer, "{cp}")?;
                count += 1;
            }
        }

        writer.write_all(&[self.final_])
    }

    fn encode_special<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let mods = self.mods.seq_int();
        let f = self.final_ as char;
        if self.event != KittyEvent::None {
            return write!(writer, "\x1B[1;{}:{}{}", mods, self.event as u8, f);
        }

        if mods > 1 {
            return write!(writer, "\x1B[1;{mods}{f}");
        }

        write!(writer, "\x1B[{f}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::key_mods::{ModSide, Mods, Side};

    #[test]
    fn kitty_plain_text() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyA,
                    mods: Mods { ..Mods::default() },
                    utf8: "abcd",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"abcd");
        }
    }

    #[test]
    fn kitty_repeat_with_just_disambiguate() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyA,
                    action: key::Action::Repeat,
                    mods: Mods { ..Mods::default() },
                    utf8: "a",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"a");
        }
    }

    #[test]
    fn kitty_enter_backspace_tab() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Enter,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x0d");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Backspace,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x7f");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Tab,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x09");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::Enter,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::Backspace,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::Tab,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::Enter,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[13;1:3u");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::Backspace,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[127;1:3u");
        }
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::Tab,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[9;1:3u");
        }
    }

    #[test]
    fn kitty_shift_backspace_emits_csi_u() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Backspace,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[127;2u");
        }
    }

    #[test]
    fn kitty_shift_enter_emits_csi_u() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Enter,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[13;2u");
        }
    }

    #[test]
    fn kitty_shift_tab_emits_csi_u() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Tab,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[9;2u");
        }
    }

    #[test]
    fn kitty_enter_with_all_flags() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Enter,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_alternates: true,
                        report_all: true,
                        report_associated: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[13u");
        }
    }

    #[test]
    fn kitty_ctrl_with_all_flags() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::ControlLeft,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_alternates: true,
                        report_all: true,
                        report_associated: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[57442;5u");
        }
    }

    #[test]
    fn kitty_ctrl_release_with_ctrl_mod_set() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::ControlLeft,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_alternates: true,
                        report_all: true,
                        report_associated: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[57442;5:3u");
        }
    }

    #[test]
    fn kitty_delete() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Delete,
                    mods: Mods { ..Mods::default() },
                    utf8: "\u{7f}",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[3~");
        }
    }

    #[test]
    fn kitty_composing_with_no_modifier() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyA,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    composing: true,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
    }

    #[test]
    fn kitty_composing_with_modifier() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::ShiftLeft,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    composing: true,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[57441;2u");
        }
    }

    #[test]
    fn kitty_composed_text_with_report_all() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Unidentified,
                    mods: Mods { ..Mods::default() },
                    utf8: "\u{fb}",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_alternates: true,
                        report_all: true,
                        report_associated: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\xc3\xbb");
        }
    }

    #[test]
    fn kitty_shift_a_on_us_keyboard() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyA,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "A",
                    unshifted_codepoint: 97,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_alternates: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[97:65;2u");
        }
    }

    #[test]
    fn kitty_matching_unshifted_codepoint() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyA,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "A",
                    unshifted_codepoint: 65,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_alternates: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[65::97;2u");
        }
    }

    #[test]
    fn kitty_report_alternates_with_caps() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyJ,
                    mods: Mods {
                        caps_lock: true,
                        ..Mods::default()
                    },
                    utf8: "J",
                    unshifted_codepoint: 106,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[106;65;74u");
        }
    }

    #[test]
    fn kitty_report_alternates_colon_shift() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Semicolon,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: ":",
                    unshifted_codepoint: u32::from(';'),
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[59:58;2;58u");
        }
    }

    #[test]
    fn kitty_report_alternates_with_ru_layout() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Semicolon,
                    mods: Mods { ..Mods::default() },
                    utf8: "\u{447}",
                    unshifted_codepoint: 1095,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1095::59;;1095u");
        }
    }

    #[test]
    fn kitty_report_alternates_with_ru_layout_shifted() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Semicolon,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "\u{427}",
                    unshifted_codepoint: 1095,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1095:1063:59;2;1063u");
        }
    }

    #[test]
    fn kitty_report_alternates_with_ru_layout_caps_lock() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Semicolon,
                    mods: Mods {
                        caps_lock: true,
                        ..Mods::default()
                    },
                    utf8: "\u{427}",
                    unshifted_codepoint: 1095,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1095::59;65;1063u");
        }
    }

    #[test]
    fn kitty_report_alternates_with_hu_layout_release() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::BracketLeft,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    unshifted_codepoint: 337,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        report_events: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[337::91;5:3u");
        }
    }

    #[test]
    fn kitty_up_arrow_with_utf8() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::ArrowUp,
                    mods: Mods { ..Mods::default() },
                    utf8: "\u{1e}",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[A");
        }
    }

    #[test]
    fn kitty_shift_tab() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Tab,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_alternates: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[9;2u");
        }
    }

    #[test]
    fn kitty_left_shift() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::ShiftLeft,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_alternates: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
    }

    #[test]
    fn kitty_left_shift_with_report_all() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::ShiftLeft,
                    mods: Mods { ..Mods::default() },
                    utf8: "",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[57441u");
        }
    }

    #[test]
    fn kitty_report_associated_with_modifiers() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyJ,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "j",
                    unshifted_codepoint: 106,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[106;5u");
        }
    }

    #[test]
    fn kitty_report_associated() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::KeyJ,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "J",
                    unshifted_codepoint: 106,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[106:74;2;74u");
        }
    }

    #[test]
    fn kitty_report_associated_on_release() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    action: key::Action::Release,
                    key: Key::KeyJ,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "J",
                    unshifted_codepoint: 106,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_all: true,
                        report_alternates: true,
                        report_associated: true,
                        report_events: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[106:74;2:3u");
        }
    }

    #[test]
    fn kitty_alternates_omit_control_characters() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Delete,
                    mods: Mods { ..Mods::default() },
                    utf8: "\u{7f}",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_alternates: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[3~");
        }
    }

    #[test]
    fn kitty_enter_with_utf8_dead_key_state() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Enter,
                    utf8: "A",
                    unshifted_codepoint: 0x0D,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_alternates: true,
                        report_all: true,
                        ..KittyFlags::DISABLED
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"A");
        }
    }

    #[test]
    fn kitty_keypad_number() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Numpad1,
                    mods: Mods { ..Mods::default() },
                    utf8: "1",
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_alternates: true,
                        report_all: true,
                        report_associated: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[57400;;49u");
        }
    }

    #[test]
    fn kitty_backspace_with_utf8_dead_key_state() {
        {
            let mut w = Vec::new();
            kitty(
                &mut w,
                &KeyEvent {
                    key: Key::Backspace,
                    utf8: "A",
                    unshifted_codepoint: 0x0D,
                    ..KeyEvent::default()
                },
                &Options {
                    kitty_flags: KittyFlags {
                        disambiguate: true,
                        report_events: true,
                        report_alternates: true,
                        report_all: true,
                        report_associated: true,
                    },
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
    }

    #[test]
    fn legacy_backspace_with_utf8_dead_key_state() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Backspace,
                    utf8: "A",
                    unshifted_codepoint: 0x0D,
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"");
        }
    }

    #[test]
    fn legacy_enter_with_utf8_dead_key_state() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Enter,
                    utf8: "A",
                    unshifted_codepoint: 0x0D,
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"A");
        }
    }

    #[test]
    fn legacy_esc_with_utf8_dead_key_state() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Escape,
                    utf8: "A",
                    unshifted_codepoint: 0x0D,
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"A");
        }
    }

    #[test]
    fn legacy_ctrl_shift_minus_underscore_on_us() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Minus,
                    mods: Mods {
                        ctrl: true,
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "_",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1f");
        }
    }

    #[test]
    fn legacy_ctrl_alt_c() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyC,
                    mods: Mods {
                        ctrl: true,
                        alt: true,
                        ..Mods::default()
                    },
                    utf8: "c",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b\x03");
        }
    }

    #[test]
    fn legacy_alt_c() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyC,
                    utf8: "c",
                    mods: Mods {
                        alt: true,
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    alt_esc_prefix: true,
                    macos_option_as_alt: OptionAsAlt::True,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1bc");
        }
    }

    #[test]
    fn legacy_alt_e_only_unshifted() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyE,
                    unshifted_codepoint: u32::from('e'),
                    mods: Mods {
                        alt: true,
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    alt_esc_prefix: true,
                    macos_option_as_alt: OptionAsAlt::True,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1be");
        }
    }

    #[test]
    fn legacy_alt() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyF,
                    utf8: "\u{444}",
                    mods: Mods {
                        alt: true,
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    alt_esc_prefix: true,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\xd1\x84");
        }
    }

    #[test]
    fn legacy_ctrl_c() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyC,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "c",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x03");
        }
    }

    #[test]
    fn legacy_ctrl_space() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Space,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: " ",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x00");
        }
    }

    #[test]
    fn legacy_ctrl_shift_backspace() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Backspace,
                    mods: Mods {
                        ctrl: true,
                        shift: true,
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x08");
        }
    }

    #[test]
    fn legacy_ctrl_shift_char_with_modify_other_state_2() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyH,
                    mods: Mods {
                        ctrl: true,
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "H",
                    ..KeyEvent::default()
                },
                &Options {
                    modify_other_keys_state_2: true,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[27;6;72~");
        }
    }

    #[test]
    fn legacy_ctrl_shift_char_with_modify_other_state_2_and_consumed_mods() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyH,
                    mods: Mods {
                        ctrl: true,
                        shift: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "H",
                    ..KeyEvent::default()
                },
                &Options {
                    modify_other_keys_state_2: true,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[27;6;72~");
        }
    }

    #[test]
    fn legacy_alt_digit_with_modify_other_state_2() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Digit8,
                    mods: Mods {
                        alt: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    utf8: "8",
                    ..KeyEvent::default()
                },
                &Options {
                    modify_other_keys_state_2: true,
                    macos_option_as_alt: OptionAsAlt::True,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[27;3;56~");
        }
    }

    #[test]
    fn legacy_fixterm_awkward_letters() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyI,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "i",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[105;5u");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyM,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "m",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[109;5u");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::BracketLeft,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "[",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[91;5u");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Digit2,
                    mods: Mods {
                        ctrl: true,
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "@",
                    unshifted_codepoint: u32::from('2'),
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[64;5u");
        }
    }

    #[test]
    fn legacy_ctrl_shift_letter_ascii() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::KeyM,
                    mods: Mods {
                        ctrl: true,
                        shift: true,
                        ..Mods::default()
                    },
                    utf8: "M",
                    unshifted_codepoint: u32::from('m'),
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[109;6u");
        }
    }

    #[test]
    fn legacy_shift_function_key_should_use_all_mods() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::ArrowUp,
                    mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods {
                        shift: true,
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1;2A");
        }
    }

    #[test]
    fn legacy_keypad_enter() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::NumpadEnter,
                    mods: Mods { ..Mods::default() },
                    consumed_mods: Mods { ..Mods::default() },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x0d");
        }
    }

    #[test]
    fn legacy_keypad_1() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Numpad1,
                    mods: Mods { ..Mods::default() },
                    consumed_mods: Mods { ..Mods::default() },
                    utf8: "1",
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"1");
        }
    }

    #[test]
    fn legacy_keypad_1_with_application_keypad() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Numpad1,
                    mods: Mods { ..Mods::default() },
                    consumed_mods: Mods { ..Mods::default() },
                    utf8: "1",
                    ..KeyEvent::default()
                },
                &Options {
                    keypad_key_application: true,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1bOq");
        }
    }

    #[test]
    fn legacy_keypad_1_with_application_keypad_and_numlock() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Numpad1,
                    mods: Mods {
                        num_lock: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    utf8: "1",
                    ..KeyEvent::default()
                },
                &Options {
                    keypad_key_application: true,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1bOq");
        }
    }

    #[test]
    fn legacy_keypad_1_with_application_keypad_and_numlock_ignore() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Numpad1,
                    mods: Mods {
                        num_lock: false,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    utf8: "1",
                    ..KeyEvent::default()
                },
                &Options {
                    keypad_key_application: true,
                    ignore_keypad_with_numlock: true,
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"1");
        }
    }

    #[test]
    fn legacy_f1() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::F1,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1;5P");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::F2,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1;5Q");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::F3,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[13;5~");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::F4,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[1;5S");
        }
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::F5,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    consumed_mods: Mods { ..Mods::default() },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[15;5~");
        }
    }

    #[test]
    fn legacy_left_shift_tab() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Tab,
                    mods: Mods {
                        shift: true,
                        sides: Side {
                            shift: ModSide::Left,
                            ..Side::default()
                        },
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[Z");
        }
    }

    #[test]
    fn legacy_right_shift_tab() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Tab,
                    mods: Mods {
                        shift: true,
                        sides: Side {
                            shift: ModSide::Right,
                            ..Side::default()
                        },
                        ..Mods::default()
                    },
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x1b[Z");
        }
    }

    #[test]
    fn legacy_hu_layout_ctrl_sends_proper_codepoint() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::BracketLeft,
                    mods: Mods {
                        ctrl: true,
                        ..Mods::default()
                    },
                    utf8: "\u{151}",
                    unshifted_codepoint: 337,
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w[1..], b"[337;5u");
        }
    }

    #[test]
    fn legacy_backspace_with_del_utf8() {
        {
            let mut w = Vec::new();
            legacy(
                &mut w,
                &KeyEvent {
                    key: Key::Backspace,
                    utf8: "\u{7f}",
                    unshifted_codepoint: 0x08,
                    ..KeyEvent::default()
                },
                &Options {
                    ..Options::default()
                },
            )
            .unwrap();
            assert_eq!(&w, b"\x7f");
        }
    }
    fn enc(seq: KittySequence) -> Vec<u8> {
        let mut w = Vec::new();
        seq.encode(&mut w).unwrap();
        w
    }

    fn seq(key: u32, final_: u8) -> KittySequence<'static> {
        KittySequence {
            key,
            final_,
            mods: KittyMods::default(),
            event: KittyEvent::None,
            alternates: [None, None],
            text: "",
        }
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::default()
        }
    }

    #[test]
    fn csi_u_mods_modifier_sequence_values() {
        let c = |shift, alt, ctrl| CsiUMods { shift, alt, ctrl }.seq_int();
        assert_eq!(1, c(false, false, false));
        assert_eq!(2, c(true, false, false));
        assert_eq!(3, c(false, true, false));
        assert_eq!(5, c(false, false, true));
        assert_eq!(4, c(true, true, false));
        assert_eq!(6, c(true, false, true));
        assert_eq!(7, c(false, true, true));
        assert_eq!(8, c(true, true, true));
    }

    #[test]
    fn kitty_mods_modifier_sequence_values() {
        let c = |shift, alt, ctrl| {
            KittyMods {
                shift,
                alt,
                ctrl,
                ..KittyMods::default()
            }
            .seq_int()
        };
        assert_eq!(1, c(false, false, false));
        assert_eq!(2, c(true, false, false));
        assert_eq!(3, c(false, true, false));
        assert_eq!(5, c(false, false, true));
        assert_eq!(4, c(true, true, false));
        assert_eq!(6, c(true, false, true));
        assert_eq!(7, c(false, true, true));
        assert_eq!(8, c(true, true, true));
    }

    #[test]
    fn kitty_sequence_backspace() {
        assert_eq!(enc(seq(127, b'u')), b"\x1B[127u");
        assert_eq!(
            enc(KittySequence {
                event: KittyEvent::Release,
                ..seq(127, b'u')
            }),
            b"\x1B[127;1:3u"
        );
        assert_eq!(
            enc(KittySequence {
                mods: KittyMods {
                    shift: true,
                    ..KittyMods::default()
                },
                ..seq(127, b'u')
            }),
            b"\x1B[127;2u"
        );
    }

    #[test]
    fn kitty_sequence_text() {
        assert_eq!(
            enc(KittySequence {
                text: "A",
                ..seq(127, b'u')
            }),
            b"\x1B[127;;65u"
        );
        assert_eq!(
            enc(KittySequence {
                event: KittyEvent::Release,
                text: "A",
                ..seq(127, b'u')
            }),
            b"\x1B[127;1:3;65u"
        );
        assert_eq!(
            enc(KittySequence {
                mods: KittyMods {
                    shift: true,
                    ..KittyMods::default()
                },
                text: "A",
                ..seq(127, b'u')
            }),
            b"\x1B[127;2;65u"
        );
    }

    #[test]
    fn kitty_sequence_text_with_control_characters() {
        assert_eq!(
            enc(KittySequence {
                text: "\n",
                ..seq(127, b'u')
            }),
            b"\x1b[127u"
        );
        assert_eq!(
            enc(KittySequence {
                text: "A\n",
                ..seq(127, b'u')
            }),
            b"\x1b[127;;65u"
        );
    }

    #[test]
    fn kitty_sequence_special_no_mods() {
        assert_eq!(enc(seq(1, b'A')), b"\x1B[A");
    }

    #[test]
    fn kitty_sequence_special_mods_only() {
        assert_eq!(
            enc(KittySequence {
                mods: KittyMods {
                    shift: true,
                    ..KittyMods::default()
                },
                ..seq(1, b'A')
            }),
            b"\x1B[1;2A"
        );
    }

    #[test]
    fn kitty_sequence_special_mods_and_event() {
        assert_eq!(
            enc(KittySequence {
                event: KittyEvent::Release,
                mods: KittyMods {
                    shift: true,
                    ..KittyMods::default()
                },
                ..seq(1, b'A')
            }),
            b"\x1B[1;2:3A"
        );
    }

    fn right_ctrl() -> Mods {
        Mods {
            ctrl: true,
            sides: Side {
                ctrl: ModSide::Right,
                ..Side::default()
            },
            ..Mods::default()
        }
    }

    #[test]
    fn ctrlseq_normal_ctrl_c() {
        assert_eq!(
            ctrl_seq(Key::Unidentified, "c", 'c' as u32, ctrl()),
            Some(0x03)
        );
    }

    #[test]
    fn ctrlseq_normal_ctrl_c_right_control() {
        assert_eq!(
            ctrl_seq(Key::Unidentified, "c", 'c' as u32, right_ctrl()),
            Some(0x03)
        );
    }

    #[test]
    fn ctrlseq_alt_should_be_allowed() {
        let mods = Mods {
            alt: true,
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(
            ctrl_seq(Key::Unidentified, "c", 'c' as u32, mods),
            Some(0x03)
        );
    }

    #[test]
    fn ctrlseq_no_ctrl_does_nothing() {
        assert_eq!(
            ctrl_seq(Key::Unidentified, "c", 'c' as u32, Mods::default()),
            None
        );
    }

    #[test]
    fn ctrlseq_shifted_non_character() {
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        assert_eq!(
            ctrl_seq(Key::Unidentified, "_", '-' as u32, mods),
            Some(0x1F)
        );
    }

    #[test]
    fn ctrlseq_caps_ascii_letter() {
        let mods = Mods {
            ctrl: true,
            caps_lock: true,
            ..Mods::default()
        };
        assert_eq!(
            ctrl_seq(Key::Unidentified, "C", 'c' as u32, mods),
            Some(0x03)
        );
    }

    #[test]
    fn ctrlseq_shift_does_not_generate_ctrl_seq() {
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(ctrl_seq(Key::Unidentified, "C", 'c' as u32, shift), None);
        let both = Mods {
            shift: true,
            ctrl: true,
            ..Mods::default()
        };
        assert_eq!(ctrl_seq(Key::Unidentified, "C", 'c' as u32, both), None);
    }

    #[test]
    fn ctrlseq_russian_ctrl_c() {
        assert_eq!(ctrl_seq(Key::KeyC, "с", 0x0441, ctrl()), Some(0x03));
    }

    #[test]
    fn ctrlseq_russian_shifted_ctrl_c() {
        let mods = Mods {
            ctrl: true,
            shift: true,
            ..Mods::default()
        };
        assert_eq!(ctrl_seq(Key::KeyC, "с", 0x0441, mods), None);
    }

    #[test]
    fn ctrlseq_russian_alt_ctrl_c() {
        let mods = Mods {
            ctrl: true,
            alt: true,
            ..Mods::default()
        };
        assert_eq!(ctrl_seq(Key::KeyC, "с", 0x0441, mods), Some(0x03));
    }

    #[test]
    fn ctrlseq_right_ctrl_c() {
        assert_eq!(
            ctrl_seq(Key::KeyC, "с", 'c' as u32, right_ctrl()),
            Some(0x03)
        );
    }
}
