#![allow(clippy::unwrap_used)]

use super::set::{AppendChainError, EntryFormatter, Value};
use super::*;
use crate::key::{Key, KeyEvent};
use crate::key_mods::Mods;

fn uni(c: char) -> Trigger {
    Trigger {
        key: TriggerKey::Unicode(c as u32),
        ..Trigger::default()
    }
}

fn phys(k: Key) -> TriggerKey {
    TriggerKey::Physical(k)
}

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

fn bind(trigger: Trigger, action: Action) -> Binding {
    Binding {
        trigger,
        action,
        flags: Flags::default(),
    }
}

fn bind_flags(trigger: Trigger, action: Action, flags: Flags) -> Binding {
    Binding {
        trigger,
        action,
        flags,
    }
}

fn trig(mods: Mods, key: TriggerKey) -> Trigger {
    Trigger { key, mods }
}

fn ps(s: &str) -> Binding {
    parse_single(s).unwrap()
}

fn ps_err(s: &str) -> ParseSingleError {
    parse_single(s).unwrap_err()
}

const INVALID: ParseSingleError = ParseSingleError::Error(Error::InvalidFormat);

#[test]
fn flags_cval() {
    assert_eq!(Flags::default().cval(), 0b0001);
    assert_eq!(
        Flags {
            consumed: false,
            ..Flags::default()
        }
        .cval(),
        0b0000
    );
    assert_eq!(
        Flags {
            all: true,
            ..Flags::default()
        }
        .cval(),
        0b0011
    );
    assert_eq!(
        Flags {
            global: true,
            ..Flags::default()
        }
        .cval(),
        0b0101
    );
    assert_eq!(
        Flags {
            performable: true,
            ..Flags::default()
        }
        .cval(),
        0b1001
    );
    assert_eq!(
        Flags {
            consumed: true,
            all: true,
            global: true,
            performable: true
        }
        .cval(),
        0b1111
    );
}

#[test]
fn split_focus_direction_parse() {
    use SplitFocusDirection as D;
    assert_eq!(D::parse("previous"), Ok(D::Previous));
    assert_eq!(D::parse("next"), Ok(D::Next));
    assert_eq!(D::parse("up"), Ok(D::Up));
    assert_eq!(D::parse("left"), Ok(D::Left));
    assert_eq!(D::parse("down"), Ok(D::Down));
    assert_eq!(D::parse("right"), Ok(D::Right));
    assert_eq!(D::parse("top"), Ok(D::Up));
    assert_eq!(D::parse("bottom"), Ok(D::Down));
    assert_eq!(D::parse(""), Err(Error::InvalidFormat));
    assert_eq!(D::parse("green"), Err(Error::InvalidFormat));
}

#[test]
fn parse_triggers() {
    // single character
    assert_eq!(ps("a=ignore"), bind(uni('a'), Action::Ignore));

    // single modifier
    assert_eq!(
        ps("shift+a=ignore"),
        bind(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore
        )
    );
    assert_eq!(
        ps("ctrl+a=ignore"),
        bind(
            trig(ctrl(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore
        )
    );

    // multiple modifier
    assert_eq!(
        ps("shift+ctrl+a=ignore"),
        bind(
            trig(
                Mods {
                    shift: true,
                    ctrl: true,
                    ..Mods::default()
                },
                TriggerKey::Unicode('a' as u32)
            ),
            Action::Ignore
        )
    );

    // key can come before modifier
    assert_eq!(
        ps("a+shift=ignore"),
        bind(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore
        )
    );

    // physical keys
    assert_eq!(
        ps("shift+key_a=ignore"),
        bind(trig(shift(), phys(Key::KeyA)), Action::Ignore)
    );

    // unicode keys
    assert_eq!(
        ps("shift+ö=ignore"),
        bind(
            trig(shift(), TriggerKey::Unicode('ö' as u32)),
            Action::Ignore
        )
    );

    // unconsumed keys
    let unconsumed = Flags {
        consumed: false,
        ..Flags::default()
    };
    assert_eq!(
        ps("unconsumed:shift+a=ignore"),
        bind_flags(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore,
            unconsumed
        )
    );

    // unconsumed physical keys
    assert_eq!(
        ps("unconsumed:key_a+shift=ignore"),
        bind_flags(trig(shift(), phys(Key::KeyA)), Action::Ignore, unconsumed)
    );

    // performable keys
    assert_eq!(
        ps("performable:shift+a=ignore"),
        bind_flags(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore,
            Flags {
                performable: true,
                ..Flags::default()
            }
        )
    );

    // invalid key
    assert_eq!(ps_err("foo=ignore"), INVALID);

    // repeated control
    assert_eq!(ps_err("shift+shift+a=ignore"), INVALID);

    // multiple character
    assert_eq!(ps_err("a+b=ignore"), INVALID);
}

#[test]
fn parse_w3c_key_names() {
    // Exact match
    assert_eq!(
        ps("KeyA=ignore"),
        bind(trig(Mods::default(), phys(Key::KeyA)), Action::Ignore)
    );

    // Case-sensitive
    assert_eq!(ps_err("Keya=ignore"), INVALID);
}

#[test]
fn parse_catch_all() {
    assert_eq!(
        ps("catch_all=ignore"),
        bind(trig(Mods::default(), TriggerKey::CatchAll), Action::Ignore)
    );
    assert_eq!(
        ps("ctrl+catch_all=ignore"),
        bind(trig(ctrl(), TriggerKey::CatchAll), Action::Ignore)
    );
}

#[test]
fn parse_plus_sign() {
    assert_eq!(ps("+=ignore"), bind(uni('+'), Action::Ignore));

    // Modifier
    assert_eq!(
        ps("ctrl++=ignore"),
        bind(
            trig(ctrl(), TriggerKey::Unicode('+' as u32)),
            Action::Ignore
        )
    );

    assert_eq!(ps_err("++=ignore"), INVALID);
}

#[test]
fn parse_equals_sign() {
    assert_eq!(ps("==ignore"), bind(uni('='), Action::Ignore));

    // Modifier
    assert_eq!(
        ps("ctrl+==ignore"),
        bind(
            trig(ctrl(), TriggerKey::Unicode('=' as u32)),
            Action::Ignore
        )
    );

    assert_eq!(ps_err("=ignore"), INVALID);
}

#[test]
fn parse_text_action_equals_sign() {
    let b = ps("==text:=");
    assert_eq!(b.trigger, uni('='));
    assert_eq!(b.action, Action::Text("=".into()));

    let b = ps("==text:=hello");
    assert_eq!(b.trigger, uni('='));
    assert_eq!(b.action, Action::Text("=hello".into()));

    let b = ps("ctrl+==text:=hello");
    assert_eq!(b.trigger, trig(ctrl(), TriggerKey::Unicode('=' as u32)));
    assert_eq!(b.action, Action::Text("=hello".into()));

    let b = ps("=+ctrl=text:=hello");
    assert_eq!(b.trigger, trig(ctrl(), TriggerKey::Unicode('=' as u32)));
    assert_eq!(b.action, Action::Text("=hello".into()));
}

// For Ghostty 1.2+ we changed our key names to match the W3C and removed
// `physical:`. This tests the backwards compatibility with the old format.
#[test]
fn parse_backwards_compatibility_with_le_1_1_x() {
    // simple, for sanity
    assert_eq!(ps("zero=ignore"), bind(uni('0'), Action::Ignore));
    assert_eq!(
        ps("physical:zero=ignore"),
        bind(trig(Mods::default(), phys(Key::Digit0)), Action::Ignore)
    );

    // duplicates
    assert_eq!(ps_err("zero+one=ignore"), INVALID);

    // test our full map
    for (k, v) in BACKWARDS_COMPATIBLE_KEYS {
        assert_eq!(
            ps(&format!("{k}=ignore")),
            bind(trig(Mods::default(), *v), Action::Ignore),
            "{k}"
        );
    }
}

#[test]
fn parse_global_triggers() {
    let g = Flags {
        global: true,
        ..Flags::default()
    };
    assert_eq!(
        ps("global:shift+a=ignore"),
        bind_flags(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore,
            g
        )
    );
    assert_eq!(
        ps("global:key_a+shift=ignore"),
        bind_flags(trig(shift(), phys(Key::KeyA)), Action::Ignore, g)
    );
    assert_eq!(
        ps("unconsumed:global:a+shift=ignore"),
        bind_flags(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore,
            Flags {
                global: true,
                consumed: false,
                ..Flags::default()
            }
        )
    );

    // global sequences not allowed
    let mut p = Parser::new("global:a>b=ignore").unwrap();
    assert_eq!(p.next_elem(), Err(Error::InvalidFormat));
}

#[test]
fn parse_all_triggers() {
    let a = Flags {
        all: true,
        ..Flags::default()
    };
    assert_eq!(
        ps("all:shift+a=ignore"),
        bind_flags(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore,
            a
        )
    );
    assert_eq!(
        ps("all:key_a+shift=ignore"),
        bind_flags(trig(shift(), phys(Key::KeyA)), Action::Ignore, a)
    );
    assert_eq!(
        ps("unconsumed:all:a+shift=ignore"),
        bind_flags(
            trig(shift(), TriggerKey::Unicode('a' as u32)),
            Action::Ignore,
            Flags {
                all: true,
                consumed: false,
                ..Flags::default()
            }
        )
    );

    // all sequences not allowed
    let mut p = Parser::new("all:a>b=ignore").unwrap();
    assert_eq!(p.next_elem(), Err(Error::InvalidFormat));
}

#[test]
fn trigger_equal() {
    let t = |key, mods| Trigger { key, mods };
    let up = phys(Key::ArrowUp);
    let down = phys(Key::ArrowDown);

    // Equal physical keys
    assert!(t(up, ctrl()).equal(&t(up, ctrl())));
    // Different physical keys
    assert!(!t(up, ctrl()).equal(&t(down, ctrl())));
    // Different mods
    assert!(!t(up, ctrl()).equal(&t(up, shift())));
    // Equal unicode keys
    let ua = TriggerKey::Unicode('a' as u32);
    let ub = TriggerKey::Unicode('b' as u32);
    assert!(t(ua, Mods::default()).equal(&t(ua, Mods::default())));
    // Different unicode keys
    assert!(!t(ua, Mods::default()).equal(&t(ub, Mods::default())));
    // Different key types
    assert!(!t(ua, Mods::default()).equal(&t(phys(Key::KeyA), Mods::default())));
    // catch_all
    let ca = TriggerKey::CatchAll;
    assert!(t(ca, Mods::default()).equal(&t(ca, Mods::default())));
    // catch_all with different mods
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    assert!(!t(ca, Mods::default()).equal(&t(ca, alt)));
}

#[test]
fn parse_modifier_aliases() {
    let sup = Mods {
        super_: true,
        ..Mods::default()
    };
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    let a = TriggerKey::Unicode('a' as u32);
    assert_eq!(ps("cmd+a=ignore"), bind(trig(sup, a), Action::Ignore));
    assert_eq!(ps("command+a=ignore"), bind(trig(sup, a), Action::Ignore));
    assert_eq!(ps("opt+a=ignore"), bind(trig(alt, a), Action::Ignore));
    assert_eq!(ps("option+a=ignore"), bind(trig(alt, a), Action::Ignore));
    assert_eq!(
        ps("control+a=ignore"),
        bind(trig(ctrl(), a), Action::Ignore)
    );
}

#[test]
fn parse_action_invalid() {
    // invalid action
    assert_eq!(
        ps_err("a=nopenopenope"),
        ParseSingleError::Error(Error::InvalidAction)
    );
}

#[test]
fn parse_action_no_parameters() {
    assert_eq!(ps("a=ignore"), bind(uni('a'), Action::Ignore));
    assert_eq!(ps_err("a=ignore:A"), INVALID);
}

#[test]
fn parse_action_with_string() {
    assert_eq!(ps("a=csi:A").action, Action::Csi("A".into()));
    assert_eq!(ps("a=esc:A").action, Action::Esc("A".into()));
    assert_eq!(
        ps("a=set_surface_title:surface").action,
        Action::SetSurfaceTitle("surface".into())
    );
    assert_eq!(
        ps("a=set_tab_title:tab").action,
        Action::SetTabTitle("tab".into())
    );
}

#[test]
fn parse_action_with_enum() {
    assert_eq!(
        ps("a=new_split:right").action,
        Action::NewSplit(SplitDirection::Right)
    );
}

#[test]
fn parse_action_with_enum_with_default() {
    assert_eq!(
        ps("a=new_split").action,
        Action::NewSplit(SplitDirection::Auto)
    );
}

#[test]
fn parse_action_with_int() {
    assert_eq!(ps("a=jump_to_prompt:-1").action, Action::JumpToPrompt(-1));
    assert_eq!(ps("a=jump_to_prompt:10").action, Action::JumpToPrompt(10));
}

#[test]
fn parse_action_with_float() {
    assert_eq!(
        ps("a=scroll_page_fractional:-0.5").action,
        Action::ScrollPageFractional(-0.5)
    );
    assert_eq!(
        ps("a=scroll_page_fractional:+0.5").action,
        Action::ScrollPageFractional(0.5)
    );
}

#[test]
fn parse_action_with_a_tuple() {
    assert_eq!(
        ps("a=resize_split:up,10").action,
        Action::ResizeSplit(SplitResizeParameter(SplitResizeDirection::Up, 10))
    );

    // missing parameter
    assert_eq!(ps_err("a=resize_split:up"), INVALID);
    // too many
    assert_eq!(ps_err("a=resize_split:up,10,12"), INVALID);
    // invalid type
    assert_eq!(ps_err("a=resize_split:up,four"), INVALID);
}

#[test]
fn parse_chain() {
    // Valid
    {
        let mut p = Parser::new("chain=new_tab").unwrap();
        assert_eq!(p.next_elem(), Ok(Some(Elem::Chain(Action::NewTab))));
        assert_eq!(p.next_elem(), Ok(None));
    }

    // Chain can't have flags
    assert_eq!(
        Parser::new("global:chain=ignore").unwrap_err(),
        Error::InvalidFormat
    );

    // Chain can't be part of a sequence
    {
        let mut p = Parser::new("a>chain=ignore").unwrap();
        let _ = p.next_elem();
        assert_eq!(p.next_elem(), Err(Error::InvalidFormat));
    }
}

#[test]
fn sequence_iterator() {
    // single character
    {
        let mut it = SequenceIterator::new("a");
        assert_eq!(it.next_trigger(), Ok(Some(uni('a'))));
        assert_eq!(it.next_trigger(), Ok(None));
    }

    // multi character
    {
        let mut it = SequenceIterator::new("a>b");
        assert_eq!(it.next_trigger(), Ok(Some(uni('a'))));
        assert_eq!(it.next_trigger(), Ok(Some(uni('b'))));
        assert_eq!(it.next_trigger(), Ok(None));
    }

    // empty
    {
        let mut it = SequenceIterator::new("");
        assert_eq!(it.next_trigger(), Err(Error::InvalidFormat));
    }

    // empty starting sequence
    {
        let mut it = SequenceIterator::new(">a");
        assert_eq!(it.next_trigger(), Err(Error::InvalidFormat));
    }

    // empty ending sequence
    {
        let mut it = SequenceIterator::new("a>");
        assert_eq!(it.next_trigger(), Ok(Some(uni('a'))));
        assert_eq!(it.next_trigger(), Err(Error::InvalidFormat));
    }
}

#[test]
fn parse_sequences() {
    // single character
    {
        let mut p = Parser::new("ctrl+a=ignore").unwrap();
        assert_eq!(
            p.next_elem(),
            Ok(Some(Elem::Binding(bind(
                trig(ctrl(), TriggerKey::Unicode('a' as u32)),
                Action::Ignore
            ))))
        );
        assert_eq!(p.next_elem(), Ok(None));
    }

    // sequence
    {
        let mut p = Parser::new("a>b=ignore").unwrap();
        assert_eq!(p.next_elem(), Ok(Some(Elem::Leader(uni('a')))));
        assert_eq!(
            p.next_elem(),
            Ok(Some(Elem::Binding(bind(uni('b'), Action::Ignore))))
        );
        assert_eq!(p.next_elem(), Ok(None));
    }
}

fn leaf(s: &Set, t: Trigger) -> &set::Leaf {
    s.get(t).unwrap().as_leaf().unwrap()
}

#[test]
fn set_parse_and_put_typical_binding() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();

    // Creates forward mapping
    let l = leaf(&s, uni('a'));
    assert_eq!(l.action, Action::NewWindow);
    assert_eq!(l.flags, Flags::default());

    // Creates reverse mapping
    let trigger = s.get_trigger(&Action::NewWindow).unwrap();
    assert_eq!(trigger.key, TriggerKey::Unicode('a' as u32));

    // Sets up the chain parent properly
    assert!(s.chain_parent().is_some());
    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "a");
}

#[test]
fn set_parse_and_put_unconsumed_binding() {
    let mut s = Set::new();
    s.parse_and_put("unconsumed:a=new_window").unwrap();

    let l = leaf(&s, uni('a'));
    assert_eq!(l.action, Action::NewWindow);
    assert_eq!(
        l.flags,
        Flags {
            consumed: false,
            ..Flags::default()
        }
    );

    let trigger = s.get_trigger(&Action::NewWindow).unwrap();
    assert_eq!(trigger.key, TriggerKey::Unicode('a' as u32));

    assert!(s.chain_parent().is_some());
    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "a");
}

#[test]
fn set_parse_and_put_removed_binding() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("a=unbind").unwrap();

    assert!(s.get(uni('a')).is_none());
    assert!(s.get_trigger(&Action::NewWindow).is_none());
    assert!(s.chain_parent().is_none());
}

#[test]
fn set_put_sets_chain_parent() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);

    assert!(s.chain_parent().is_some());
    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "a");
    assert!(matches!(s.chain_parent_value(), Some(Value::Leaf(_))));
}

#[test]
fn set_put_flags_sets_chain_parent() {
    let mut s = Set::new();
    s.put_flags(
        uni('a'),
        Action::NewWindow,
        Flags {
            consumed: false,
            ..Flags::default()
        },
    );

    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "a");
    let v = s.chain_parent_value().unwrap().as_leaf().unwrap();
    assert!(!v.flags.consumed);
}

#[test]
fn set_sequence_sets_chain_parent_to_final_leaf() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();

    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "b");
    let v = s.chain_parent_value().unwrap().as_leaf().unwrap();
    assert_eq!(v.action, Action::NewWindow);
}

#[test]
fn set_multiple_leaves_under_leader_updates_chain_parent() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "b");

    s.parse_and_put("a>c=new_tab").unwrap();
    assert_eq!(s.chain_parent_trigger().unwrap().to_string(), "c");
    assert_eq!(
        s.chain_parent_value().unwrap().as_leaf().unwrap().action,
        Action::NewTab
    );
}

#[test]
fn set_sequence_unbind_clears_chain_parent() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    assert!(s.chain_parent().is_some());

    s.parse_and_put("a>b=unbind").unwrap();
    assert!(s.chain_parent().is_none());
}

#[test]
fn set_sequence_unbind_with_remaining_leaves_clears_chain_parent() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    s.parse_and_put("a>c=new_tab").unwrap();
    s.parse_and_put("a>b=unbind").unwrap();

    // After unbind, chain_parent should be cleared even though 'c' remains
    assert!(s.chain_parent().is_none());

    // But 'c' should still exist
    let inner = s.get(uni('a')).unwrap().as_leader().unwrap();
    assert!(inner.get(uni('c')).is_some());
}

#[test]
fn set_direct_remove_clears_chain_parent() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    assert!(s.chain_parent().is_some());

    s.remove(uni('a'));
    assert!(s.chain_parent().is_none());
}

#[test]
fn set_invalid_format_preserves_chain_parent() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    let before = s.chain_parent().map(<[Trigger]>::to_vec);

    // Try an invalid parse - should fail
    assert_eq!(
        s.parse_and_put("a=invalid_action_xyz"),
        Err(Error::InvalidAction)
    );

    // chain_parent should be unchanged
    assert!(s.chain_parent().is_some());
    assert_eq!(s.chain_parent().map(<[Trigger]>::to_vec), before);
}

#[test]
fn set_clone_produces_null_chain_parent() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    assert!(s.chain_parent().is_some());

    let cloned = s.clone_set();
    assert!(cloned.chain_parent().is_none());
    assert!(cloned.get(uni('a')).is_some());
}

#[test]
fn set_clone_with_leaf_chained() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();

    let entry = s.get(uni('a')).unwrap();
    assert_eq!(entry.as_leaf_chained().unwrap().actions.len(), 2);

    let cloned = s.clone_set();
    let ce = cloned.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(ce.actions.len(), 2);
    assert_eq!(ce.actions[0], Action::NewWindow);
    assert_eq!(ce.actions[1], Action::NewTab);
}

#[test]
fn set_clone_with_leaf_chained_containing_allocated_data() {
    let mut s = Set::new();
    s.parse_and_put("a=text:hello").unwrap();
    s.parse_and_put("chain=text:world").unwrap();

    assert!(s.get(uni('a')).unwrap().as_leaf_chained().is_some());

    let cloned = s.clone_set();
    let ce = cloned.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(ce.actions[0], Action::Text("hello".into()));
    assert_eq!(ce.actions[1], Action::Text("world".into()));

    // Truly cloned, not shared.
    let (Action::Text(orig), Action::Text(copy)) = (
        &s.get(uni('a')).unwrap().as_leaf_chained().unwrap().actions[0],
        &ce.actions[0],
    ) else {
        panic!("expected text");
    };
    assert_ne!(orig.as_ptr(), copy.as_ptr());
}

#[test]
fn set_parse_and_put_sequence() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    let inner = s.get(uni('a')).unwrap().as_leader().unwrap();
    let l = leaf(inner, uni('b'));
    assert_eq!(l.action, Action::NewWindow);
    assert_eq!(l.flags, Flags::default());
}

#[test]
fn set_parse_and_put_sequence_with_two_actions() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    s.parse_and_put("a>c=new_tab").unwrap();
    let inner = s.get(uni('a')).unwrap().as_leader().unwrap();
    let b = leaf(inner, uni('b'));
    assert_eq!(b.action, Action::NewWindow);
    assert_eq!(b.flags, Flags::default());
    let c = leaf(inner, uni('c'));
    assert_eq!(c.action, Action::NewTab);
    assert_eq!(c.flags, Flags::default());
}

#[test]
fn set_parse_and_put_overwrite_sequence() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_tab").unwrap();
    s.parse_and_put("a>b=new_window").unwrap();
    let inner = s.get(uni('a')).unwrap().as_leader().unwrap();
    let b = leaf(inner, uni('b'));
    assert_eq!(b.action, Action::NewWindow);
    assert_eq!(b.flags, Flags::default());
}

#[test]
fn set_parse_and_put_overwrite_leader() {
    let mut s = Set::new();
    s.parse_and_put("a=new_tab").unwrap();
    s.parse_and_put("a>b=new_window").unwrap();
    let inner = s.get(uni('a')).unwrap().as_leader().unwrap();
    let b = leaf(inner, uni('b'));
    assert_eq!(b.action, Action::NewWindow);
    assert_eq!(b.flags, Flags::default());
}

#[test]
fn set_parse_and_put_unbind_sequence_unbinds_leader() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    s.parse_and_put("a>b=unbind").unwrap();
    assert!(s.get(uni('a')).is_none());
}

#[test]
fn set_parse_and_put_unbind_sequence_unbinds_leader_if_not_set() {
    let mut s = Set::new();
    s.parse_and_put("a>b=unbind").unwrap();
    assert!(s.get(uni('a')).is_none());
}

#[test]
fn set_parse_and_put_unbind_sequence_restores_previous_leaf() {
    // Extra: a failed leader insert must restore the old leaf binding.
    let mut s = Set::new();
    s.parse_and_put("a=new_tab").unwrap();
    s.parse_and_put("a>b=unbind").unwrap();
    assert_eq!(leaf(&s, uni('a')).action, Action::NewTab);
    assert!(s.chain_parent().is_none());
}

#[test]
fn set_parse_and_put_sequence_preserves_reverse_mapping() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("ctrl+a>b=new_window").unwrap();

    let trigger = s.get_trigger(&Action::NewWindow).unwrap();
    assert_eq!(trigger.key, TriggerKey::Unicode('a' as u32));
}

#[test]
fn set_put_overwrites_sequence() {
    let mut s = Set::new();
    s.parse_and_put("ctrl+a>b=new_window").unwrap();
    s.put(
        trig(ctrl(), TriggerKey::Unicode('a' as u32)),
        Action::NewWindow,
    );

    let trigger = s.get_trigger(&Action::NewWindow).unwrap();
    assert_eq!(trigger.key, TriggerKey::Unicode('a' as u32));
}

#[test]
fn set_maintains_reverse_mapping() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );

    // should be most recent
    s.put(uni('b'), Action::NewWindow);
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('b' as u32)
    );

    // removal should replace
    s.remove(uni('b'));
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );
}

#[test]
fn set_performable_is_not_part_of_reverse_mappings() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );

    // trigger should be non-performable
    s.put_flags(
        uni('b'),
        Action::NewWindow,
        Flags {
            performable: true,
            ..Flags::default()
        },
    );
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );

    // removal of performable should do nothing
    s.remove(uni('b'));
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );
}

#[test]
fn set_overriding_a_mapping_updates_reverse() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );

    // should be most recent
    s.put(uni('a'), Action::NewTab);
    assert!(s.get_trigger(&Action::NewWindow).is_none());
}

#[test]
fn set_consumed_state() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    assert!(leaf(&s, uni('a')).flags.consumed);

    s.put_flags(
        uni('a'),
        Action::NewWindow,
        Flags {
            consumed: false,
            ..Flags::default()
        },
    );
    assert!(!leaf(&s, uni('a')).flags.consumed);

    s.put(uni('a'), Action::NewWindow);
    assert!(leaf(&s, uni('a')).flags.consumed);
}

#[test]
fn set_parse_and_put_chain() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();

    // Creates forward mapping as leaf_chained
    let chained = s.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(chained.actions.len(), 2);
    assert_eq!(chained.actions[0], Action::NewWindow);
    assert_eq!(chained.actions[1], Action::NewTab);

    // Does not create reverse mapping, because reverse mappings are only for
    // non-chained actions.
    assert!(s.get_trigger(&Action::NewWindow).is_none());
}

#[test]
fn set_parse_and_put_chain_without_parent_is_error() {
    let mut s = Set::new();
    assert_eq!(s.parse_and_put("chain=new_tab"), Err(Error::InvalidFormat));
}

#[test]
fn set_parse_and_put_chain_multiple_times() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();
    s.parse_and_put("chain=close_surface").unwrap();

    let chained = s.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(chained.actions.len(), 3);
    assert_eq!(chained.actions[0], Action::NewWindow);
    assert_eq!(chained.actions[1], Action::NewTab);
    assert_eq!(chained.actions[2], Action::CloseSurface);
}

#[test]
fn set_parse_and_put_chain_preserves_flags() {
    let mut s = Set::new();
    s.parse_and_put("unconsumed:a=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();

    let chained = s.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert!(!chained.flags.consumed);
    assert_eq!(chained.actions.len(), 2);
}

#[test]
fn set_parse_and_put_chain_after_unbind_is_error() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("a=unbind").unwrap();

    // Chain after unbind should fail because chain_parent is cleared
    assert_eq!(s.parse_and_put("chain=new_tab"), Err(Error::InvalidFormat));
}

#[test]
fn set_parse_and_put_chain_on_sequence() {
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();

    let inner = s.get(uni('a')).unwrap().as_leader().unwrap();
    let chained = inner.get(uni('b')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(chained.actions.len(), 2);
    assert_eq!(chained.actions[0], Action::NewWindow);
    assert_eq!(chained.actions[1], Action::NewTab);
}

#[test]
fn set_parse_and_put_chain_with_unbind_is_error() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();

    // chain=unbind is not valid
    assert_eq!(s.parse_and_put("chain=unbind"), Err(Error::InvalidFormat));

    // Original binding should still exist
    assert_eq!(leaf(&s, uni('a')).action, Action::NewWindow);
}

fn ev(key: Key, mods: Mods, utf8: &str, unshifted: u32) -> KeyEvent<'_> {
    KeyEvent {
        key,
        mods,
        utf8,
        unshifted_codepoint: unshifted,
        ..KeyEvent::default()
    }
}

fn ev_action(s: &Set, e: &KeyEvent<'_>) -> Option<Action> {
    s.get_event(e)
        .map(|(_, v)| v.as_leaf().unwrap().action.clone())
}

#[test]
fn set_get_event_physical() {
    let mut s = Set::new();
    s.parse_and_put("ctrl+quote=new_window").unwrap();

    // Physical matches on physical
    assert_eq!(
        ev_action(&s, &ev(Key::Quote, ctrl(), "", 0)),
        Some(Action::NewWindow)
    );

    // Physical does not match on UTF8/codepoint
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, ctrl(), "'", '\'' as u32)),
        None
    );
}

#[test]
fn set_get_event_codepoint() {
    let mut s = Set::new();
    s.parse_and_put("ctrl+'=new_window").unwrap();

    // Matches on codepoint
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, ctrl(), "", '\'' as u32)),
        Some(Action::NewWindow)
    );

    // Matches on UTF-8
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, ctrl(), "'", 0)),
        Some(Action::NewWindow)
    );

    // Doesn't match on physical
    assert_eq!(ev_action(&s, &ev(Key::KeyA, ctrl(), "", 0)), None);
}

#[test]
fn set_get_event_codepoint_case_folding() {
    let mut s = Set::new();
    s.parse_and_put("ctrl+A=new_window").unwrap();

    // Lowercase codepoint
    assert_eq!(
        ev_action(&s, &ev(Key::KeyJ, ctrl(), "", 'a' as u32)),
        Some(Action::NewWindow)
    );

    // Uppercase codepoint
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, ctrl(), "", 'A' as u32)),
        Some(Action::NewWindow)
    );

    // Negative case for sanity
    assert_eq!(ev_action(&s, &ev(Key::KeyJ, ctrl(), "", 0)), None);
}

#[test]
fn set_get_event_catch_all_fallback() {
    let mut s = Set::new();
    s.parse_and_put("catch_all=ignore").unwrap();

    // Matches unbound key without modifiers
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, Mods::default(), "", 0)),
        Some(Action::Ignore)
    );

    // Matches unbound key with modifiers (falls back to catch_all without mods)
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, ctrl(), "", 0)),
        Some(Action::Ignore)
    );

    // Specific binding takes precedence over catch_all
    s.parse_and_put("ctrl+b=new_window").unwrap();
    assert_eq!(
        ev_action(&s, &ev(Key::KeyB, ctrl(), "", 'b' as u32)),
        Some(Action::NewWindow)
    );
}

#[test]
fn set_get_event_catch_all_with_modifiers() {
    let mut s = Set::new();
    s.parse_and_put("ctrl+catch_all=close_surface").unwrap();
    s.parse_and_put("catch_all=ignore").unwrap();

    // Key with ctrl matches catch_all with ctrl modifier
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, ctrl(), "", 0)),
        Some(Action::CloseSurface)
    );

    // Key without mods matches catch_all without mods
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, Mods::default(), "", 0)),
        Some(Action::Ignore)
    );

    // Key with different mods falls back to catch_all without mods
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    assert_eq!(
        ev_action(&s, &ev(Key::KeyA, alt, "", 0)),
        Some(Action::Ignore)
    );
}

#[test]
fn action_clone() {
    let a = Action::Ignore;
    assert_eq!(a.clone(), Action::Ignore);

    let a = Action::Text("foo".into());
    assert_eq!(a.clone(), Action::Text("foo".into()));
}

#[test]
fn parse_increase_font_size() {
    assert_eq!(
        ps("a=increase_font_size:1.5").action,
        Action::IncreaseFontSize(1.5)
    );
}

#[test]
fn parse_decrease_font_size() {
    assert_eq!(
        ps("a=decrease_font_size:2.5").action,
        Action::DecreaseFontSize(2.5)
    );
}

#[test]
fn parse_reset_font_size() {
    assert_eq!(ps("a=reset_font_size").action, Action::ResetFontSize);
}

#[test]
fn parse_set_font_size() {
    assert_eq!(ps("a=set_font_size:13.5").action, Action::SetFontSize(13.5));
}

#[test]
fn parse_copy_to_clipboard_default() {
    assert_eq!(
        ps("a=copy_to_clipboard").action,
        Action::CopyToClipboard(CopyToClipboard::Mixed)
    );
}

#[test]
fn parse_copy_to_clipboard_explicit() {
    assert_eq!(
        ps("a=copy_to_clipboard:html").action,
        Action::CopyToClipboard(CopyToClipboard::Html)
    );
}

#[test]
fn parse_write_screen_file_no_format() {
    assert_eq!(
        ps("a=write_screen_file:copy").action,
        Action::WriteScreenFile(WriteScreen::COPY)
    );
}

#[test]
fn parse_write_screen_file_format() {
    assert_eq!(
        ps("a=write_screen_file:copy,html").action,
        Action::WriteScreenFile(WriteScreen {
            action: WriteScreenAction::Copy,
            emit: WriteScreenFormat::Html,
        })
    );
}

#[test]
fn parse_write_screen_file_format_as_string() {
    let b = ps("a=write_screen_file:copy,html");
    assert_eq!(b.action.to_string(), "write_screen_file:copy,html");
}

#[test]
fn parse_write_screen_file_invalid() {
    assert_eq!(ps_err("a=write_screen_file:"), INVALID);
    assert_eq!(ps_err("a=write_screen_file:,"), INVALID);
    assert_eq!(ps_err("a=write_screen_file:copy,"), INVALID);
    assert_eq!(ps_err("a=write_screen_file:copy,html,extra"), INVALID);
}

#[test]
fn action_format() {
    let a = Action::Text("👻".into());
    assert_eq!(a.to_string(), "text:\\xf0\\x9f\\x91\\xbb");
}

#[test]
fn action_format_set_title() {
    let a = Action::SetTabTitle("foo bar".into());
    assert_eq!(a.to_string(), "set_tab_title:foo bar");
}

#[test]
fn set_append_chain_with_no_parent_returns_error() {
    let mut s = Set::new();
    assert_eq!(
        s.append_chain(Action::NewTab),
        Err(AppendChainError::NoChainParent)
    );
}

#[test]
fn set_append_chain_after_put_converts_to_leaf_chained() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    s.append_chain(Action::NewTab).unwrap();

    let chained = s.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(chained.actions.len(), 2);
    assert_eq!(chained.actions[0], Action::NewWindow);
    assert_eq!(chained.actions[1], Action::NewTab);
}

#[test]
fn set_append_chain_after_put_flags_preserves_flags() {
    let mut s = Set::new();
    s.put_flags(
        uni('a'),
        Action::NewWindow,
        Flags {
            consumed: false,
            ..Flags::default()
        },
    );
    s.append_chain(Action::NewTab).unwrap();

    let chained = s.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert!(!chained.flags.consumed);
    assert_eq!(chained.actions.len(), 2);
    assert_eq!(chained.actions[0], Action::NewWindow);
    assert_eq!(chained.actions[1], Action::NewTab);
}

#[test]
fn set_append_chain_multiple_times() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    s.append_chain(Action::NewTab).unwrap();
    s.append_chain(Action::CloseSurface).unwrap();

    let chained = s.get(uni('a')).unwrap().as_leaf_chained().unwrap();
    assert_eq!(chained.actions.len(), 3);
    assert_eq!(chained.actions[0], Action::NewWindow);
    assert_eq!(chained.actions[1], Action::NewTab);
    assert_eq!(chained.actions[2], Action::CloseSurface);
}

#[test]
fn set_append_chain_removes_reverse_mapping() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    assert!(s.get_trigger(&Action::NewWindow).is_some());

    // Chaining should remove the reverse mapping
    s.append_chain(Action::NewTab).unwrap();
    assert!(s.get_trigger(&Action::NewWindow).is_none());
}

#[test]
fn set_append_chain_with_performable_does_not_affect_reverse_mapping() {
    let mut s = Set::new();
    s.put(uni('b'), Action::NewWindow);
    assert!(s.get_trigger(&Action::NewWindow).is_some());

    // Add a performable binding (not in reverse map) and chain it
    s.put_flags(
        uni('a'),
        Action::CloseSurface,
        Flags {
            performable: true,
            ..Flags::default()
        },
    );
    assert!(s.get_trigger(&Action::CloseSurface).is_none());

    // Chaining the performable binding should not crash or affect anything
    s.append_chain(Action::NewTab).unwrap();

    // The non-performable new_window binding should still be in reverse map
    assert!(s.get_trigger(&Action::NewWindow).is_some());
}

#[test]
fn set_append_chain_restores_next_valid_reverse_mapping() {
    let mut s = Set::new();
    s.put(uni('a'), Action::NewWindow);
    s.put(uni('b'), Action::NewWindow);

    // Reverse mapping should point to 'b' (most recent)
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('b' as u32)
    );

    // Chain an action to 'b', which should restore 'a' in the reverse map
    s.append_chain(Action::NewTab).unwrap();
    assert_eq!(
        s.get_trigger(&Action::NewWindow).unwrap().key,
        TriggerKey::Unicode('a' as u32)
    );
}

/// Stand-in for `formatterpkg.entryFormatter("keybind", ...)`.
struct KeybindFormatter(String);

impl EntryFormatter for KeybindFormatter {
    fn format_entry(&mut self, value: &str) {
        self.0.push_str("keybind = ");
        self.0.push_str(value);
        self.0.push('\n');
    }
}

fn format_set_entry(s: &Set, t: Trigger) -> String {
    let (k, v) = s.get_entry(t).unwrap();
    let mut buf = k.to_string();
    let mut f = KeybindFormatter(String::new());
    v.format_entries(&mut buf, &mut f);
    f.0
}

#[test]
fn set_format_entries_leaf_chained() {
    let mut s = Set::new();
    s.parse_and_put("a=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();

    assert!(s.get(uni('a')).unwrap().as_leaf_chained().is_some());
    assert_eq!(
        format_set_entry(&s, uni('a')),
        "keybind = a=new_window\nkeybind = chain=new_tab\n"
    );
}

#[test]
fn set_format_entries_leaf_chained_multiple_chains() {
    let mut s = Set::new();
    s.parse_and_put("ctrl+a=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();
    s.parse_and_put("chain=close_surface").unwrap();

    let t = trig(ctrl(), TriggerKey::Unicode('a' as u32));
    assert_eq!(
        s.get(t).unwrap().as_leaf_chained().unwrap().actions.len(),
        3
    );
    assert_eq!(
        format_set_entry(&s, t),
        "keybind = ctrl+a=new_window\nkeybind = chain=new_tab\nkeybind = chain=close_surface\n"
    );
}

#[test]
fn set_format_entries_leaf_chained_with_text_action() {
    let mut s = Set::new();
    s.parse_and_put("a=text:hello").unwrap();
    s.parse_and_put("chain=text:world").unwrap();

    assert_eq!(
        format_set_entry(&s, uni('a')),
        "keybind = a=text:hello\nkeybind = chain=text:world\n"
    );
}

#[test]
fn set_format_entries_sequence_shares_prefix() {
    // Extra: sequences share the prefix and siblings are not corrupted by
    // chained leaves.
    let mut s = Set::new();
    s.parse_and_put("a>b=new_window").unwrap();
    s.parse_and_put("chain=new_tab").unwrap();
    s.parse_and_put("a>c=close_surface").unwrap();
    assert_eq!(
        format_set_entry(&s, uni('a')),
        "keybind = a>b=new_window\nkeybind = chain=new_tab\nkeybind = a>c=close_surface\n"
    );
}

#[test]
fn trigger_hash_folds_case() {
    assert_eq!(uni('a').hash(), uni('A').hash());
    assert!(uni('a').folded_equal(&uni('A')));
    assert!(!uni('a').equal(&uni('A')));
}

#[test]
fn trigger_display_roundtrip() {
    for s in ["super+ctrl+alt+shift+a", "catch_all", "ctrl+arrow_up", "+"] {
        let t = Trigger::parse(s).unwrap();
        assert_eq!(t.to_string(), s);
    }
}

#[test]
fn action_docs_and_scope() {
    assert!(
        Action::Ignore
            .docs()
            .starts_with("Ignore this key combination.")
    );
    assert_eq!(Action::NewWindow.scope(), Scope::App);
    assert_eq!(Action::NewTab.scope(), Scope::Surface);
    assert!(Action::NewTab.scoped(Scope::App).is_none());
    assert!(Action::NewWindow.scoped(Scope::App).is_some());
}

#[test]
fn action_roundtrip_all_default_forms() {
    for s in [
        "csi:0m",
        "text:hello world",
        "increase_font_size:1.5",
        "scroll_to_row:5",
        "goto_tab:3",
        "move_tab:-1",
        "adjust_selection:left",
        "resize_split:up,10",
        "close_tab:other",
        "inspector:toggle",
        "crash:main",
        "activate_key_table:foo",
    ] {
        assert_eq!(Action::parse(s).unwrap().to_string(), s);
    }
}

#[test]
fn less_than_orders_by_mod_count() {
    let a = bind(
        trig(ctrl(), TriggerKey::Unicode('a' as u32)),
        Action::Ignore,
    );
    let b = bind(uni('a'), Action::Ignore);
    assert!(less_than(&a, &b));
    assert!(!less_than(&b, &a));
}
