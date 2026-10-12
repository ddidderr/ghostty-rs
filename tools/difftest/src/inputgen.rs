//! Deterministic, escape-heavy random byte-stream generator.

/// splitmix64 PRNG (deterministic, dependency-free).
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn chance(&mut self, num: usize, den: usize) -> bool {
        self.below(den) < num
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

const WORDS: &[&str] = &[
    "hello",
    "world",
    "The quick brown fox",
    "x",
    "ab",
    "0123456789",
    "~~~",
    "   ",
    "foo bar baz",
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
];

/// Interesting non-ASCII text: wide, combining, emoji, ZWJ, VS, regional
/// indicators, Hangul jamo, Indic, zero-width, ambiguous.
const UNICODE: &[&str] = &[
    "é",
    "e\u{301}",
    "\u{301}",
    "中",
    "文字",
    "ｱ",
    "Ａ",
    "한",
    "ᄀ\u{1161}\u{11a8}",
    "😀",
    "👍🏽",
    "👨\u{200d}👩\u{200d}👧",
    "🇺🇸",
    "🇺",
    "\u{fe0f}",
    "\u{fe0e}",
    "❤\u{fe0f}",
    "❤\u{fe0e}",
    "☺",
    "\u{200b}",
    "\u{200d}",
    "\u{200c}",
    "\u{feff}",
    "\u{ad}",
    "ä\u{308}\u{323}",
    "क्षि",
    "ก็",
    "\u{1f3f4}\u{e0067}\u{e0062}\u{e007f}",
    "®",
    "©",
    "™",
    "\u{2764}",
    "\u{1f600}\u{fe0f}",
    "ß",
    "─",
    "│",
    "█",
    "\u{1b000}",
    "\u{10ffff}",
    "\u{e000}",
    "\u{ffff}",
    "\u{3000}",
    "〜",
    "\u{2060}",
    "\u{0600}",
    "\u{2028}",
    "\u{85}",
];

const MODES_DEC: &[u32] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 25, 40, 41, 45, 47, 66, 67, 69, 80, 95, 1000, 1002, 1003, 1004,
    1005, 1006, 1007, 1015, 1016, 1035, 1036, 1039, 1045, 1047, 1048, 1049, 2004, 2026, 2027, 2031,
    2048, 7727, 0, 9999, 65535, 12345,
];
const MODES_ANSI: &[u32] = &[2, 4, 12, 20, 0, 1, 3, 25, 99];

const CSI_FINALS: &[u8] = b"@ABCDEFGHIJKLMPSTXZ^`abcdefghijklmnpqrstuvwxyz{|}~";
const EDGE_NUMS: &[&str] = &[
    "",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "10",
    "24",
    "25",
    "80",
    "100",
    "255",
    "256",
    "999",
    "1000",
    "32767",
    "32768",
    "65534",
    "65535",
    "65536",
    "65537",
    "99999",
    "2147483647",
    "2147483648",
    "4294967295",
    "4294967296",
    "18446744073709551616",
    "-1",
];

fn num(r: &mut Rng, out: &mut Vec<u8>) {
    if r.chance(3, 5) {
        out.extend_from_slice(r.below(30).to_string().as_bytes());
    } else {
        out.extend_from_slice(r.pick(EDGE_NUMS).as_bytes());
    }
}

fn params(r: &mut Rng, out: &mut Vec<u8>, max: usize) {
    let n = r.below(max + 1);
    for i in 0..n {
        if i > 0 {
            out.push(if r.chance(1, 12) { b':' } else { b';' });
        }
        num(r, out);
    }
}

fn sgr(r: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[");
    let n = 1 + r.below(6);
    for i in 0..n {
        if i > 0 {
            out.push(b';');
        }
        match r.below(14) {
            0 => out.extend_from_slice(b"0"),
            1 => out.extend_from_slice(
                r.pick(&[
                    "1", "2", "3", "4", "5", "7", "8", "9", "21", "53", "22", "23", "24", "25",
                    "27", "28", "29", "55",
                ])
                .as_bytes(),
            ),
            2 => out.extend_from_slice(
                r.pick(&[
                    "30", "31", "32", "37", "39", "40", "41", "47", "49", "90", "97", "100", "107",
                    "59",
                ])
                .as_bytes(),
            ),
            3 => {
                out.extend_from_slice(b"38;2;");
                for k in 0..3 {
                    if k > 0 {
                        out.push(b';');
                    }
                    num(r, out);
                }
            }
            4 => {
                out.extend_from_slice(r.pick(&["48", "58", "38"]).as_bytes());
                out.extend_from_slice(b";5;");
                num(r, out);
            }
            5 => {
                out.extend_from_slice(
                    r.pick(&["4:0", "4:1", "4:2", "4:3", "4:4", "4:5", "4:9", "4:"])
                        .as_bytes(),
                );
            }
            6 => {
                out.extend_from_slice(b"38:2::");
                for k in 0..3 {
                    if k > 0 {
                        out.push(b':');
                    }
                    num(r, out);
                }
            }
            7 => {
                out.extend_from_slice(b"48:2:");
                for k in 0..4 {
                    if k > 0 {
                        out.push(b':');
                    }
                    num(r, out);
                }
            }
            8 => out.extend_from_slice(b"58:5:"),
            9 => num(r, out),
            10 => out.extend_from_slice(
                r.pick(&["38", "48", "58", "38;2", "38;5", "48;2;1", "58;2;1;2"])
                    .as_bytes(),
            ),
            _ => out.extend_from_slice(r.below(110).to_string().as_bytes()),
        }
    }
    out.push(b'm');
}

fn osc(r: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b]");
    let colors = [
        "rgb:ff/00/80",
        "#ff0000",
        "red",
        "rgb:1/2/3",
        "?",
        "#abc",
        "rgbi:0.5/0.5/0.5",
        "#12345",
        "blue;green",
        "",
    ];
    let c = |r: &mut Rng| *r.pick(&colors);
    match r.below(18) {
        0 => out.extend_from_slice(format!("0;title{}", r.below(100)).as_bytes()),
        1 => out.extend_from_slice(format!("2;{}", c(r)).as_bytes()),
        2 => out.extend_from_slice(format!("4;{};{}", r.below(260), c(r)).as_bytes()),
        3 => out.extend_from_slice(
            format!("4;{};{};{};{}", r.below(256), c(r), r.below(256), c(r)).as_bytes(),
        ),
        4 => out.extend_from_slice(
            format!(
                "{};{}",
                r.pick(&[10, 11, 12, 13, 14, 15, 16, 17, 18, 19]),
                c(r)
            )
            .as_bytes(),
        ),
        5 => out.extend_from_slice(
            r.pick(&["104", "110", "111", "112", "104;1;2", "104;300"])
                .as_bytes(),
        ),
        6 => out.extend_from_slice(format!("7;file://host/tmp/{}", r.below(10)).as_bytes()),
        7 => out.extend_from_slice(
            r.pick(&[
                "8;;http://example.com",
                "8;id=1;http://a.b",
                "8;;",
                "8;id=x:y=z;u",
            ])
            .as_bytes(),
        ),
        8 => out.extend_from_slice(
            r.pick(&[
                "133;A",
                "133;B",
                "133;C",
                "133;D;0",
                "133;A;k=s",
                "133;L",
                "133;P;k=c",
                "133;N",
            ])
            .as_bytes(),
        ),
        9 => out.extend_from_slice(r.pick(&["52;c;aGVsbG8=", "52;c;?", "52;;"]).as_bytes()),
        10 => out.extend_from_slice(
            r.pick(&[
                "9;4;1;50",
                "9;hello",
                "777;notify;a;b",
                "1337;File=x",
                "22;pointer",
                "66;s=2;x",
                "5522;x",
            ])
            .as_bytes(),
        ),
        11 => out.extend_from_slice(format!("{};", r.below(2000)).as_bytes()),
        12 => out.extend_from_slice(
            r.pick(&[
                "21;foreground=?",
                "21;foreground=red;background=#fff",
                "21;palette=1=?",
            ])
            .as_bytes(),
        ),
        _ => {
            let n = r.below(20);
            for _ in 0..n {
                out.push(32 + r.below(95) as u8);
            }
        }
    }
    match r.below(6) {
        0..=2 => out.push(0x07),
        3 | 4 => out.extend_from_slice(b"\x1b\\"),
        _ => {} // unterminated
    }
}

fn dcs(r: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1bP");
    out.extend_from_slice(
        r.pick(&[
            "$qm",
            "$q\"p",
            "$qr",
            "+q544e",
            "1000p",
            "1000p%begin 1 2 3\n",
            "p",
            "0;1|",
            "1;2;3q",
            "$q r",
            "$t",
            "=1s",
            "garbage",
        ])
        .as_bytes(),
    );
    let n = r.below(12);
    for _ in 0..n {
        out.push(32 + r.below(95) as u8);
    }
    if !r.chance(1, 8) {
        out.extend_from_slice(b"\x1b\\");
    }
}

fn apc(r: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b_");
    out.extend_from_slice(
        r.pick(&[
            "Ga=T,f=24,s=1,v=1;AAAA",
            "Ga=d",
            "Gi=1,a=q;AAAA",
            "Ga=p,i=1",
            "Gq=2,a=T,f=100;",
            "x",
            "Ga=T,t=s,i=3;/x",
        ])
        .as_bytes(),
    );
    out.extend_from_slice(b"\x1b\\");
}

fn csi(r: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[");
    match r.below(10) {
        0 => out.push(b'?'),
        1 => out.push(*r.pick(b">=<!")),
        _ => {}
    }
    params(r, out, 5);
    if r.chance(1, 10) {
        out.push(*r.pick(b" $\"!#'*+%,"));
    }
    let f = *r.pick(CSI_FINALS);
    out.push(f);
}

fn csi_common(r: &mut Rng, out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b[");
    let finals = b"HABCDEFGdJKLMP@XSTIZbg`aef";
    let f = *r.pick(finals);
    let n = r.below(3);
    for i in 0..n {
        if i > 0 {
            out.push(b';');
        }
        if r.chance(3, 4) {
            out.extend_from_slice(r.below(12).to_string().as_bytes());
        } else {
            num(r, out);
        }
    }
    out.push(f);
}

fn margins(r: &mut Rng, out: &mut Vec<u8>) {
    let fin = if r.chance(1, 3) { b's' } else { b'r' };
    if fin == b's' && r.chance(2, 3) {
        out.extend_from_slice(b"\x1b[?69h");
    }
    out.extend_from_slice(b"\x1b[");
    if r.chance(9, 10) {
        out.extend_from_slice(r.below(10).to_string().as_bytes());
        out.push(b';');
        out.extend_from_slice(r.below(14).to_string().as_bytes());
    }
    out.push(fin);
}

fn mode(r: &mut Rng, out: &mut Vec<u8>) {
    let set = r.chance(1, 2);
    if r.chance(3, 4) {
        out.extend_from_slice(b"\x1b[?");
        let n = 1 + r.below(3);
        for i in 0..n {
            if i > 0 {
                out.push(b';');
            }
            out.extend_from_slice(r.pick(MODES_DEC).to_string().as_bytes());
        }
        out.push(match r.below(8) {
            0 => b's',
            1 => b'r',
            _ => {
                if set {
                    b'h'
                } else {
                    b'l'
                }
            }
        });
    } else {
        out.extend_from_slice(b"\x1b[");
        out.extend_from_slice(r.pick(MODES_ANSI).to_string().as_bytes());
        out.push(if set { b'h' } else { b'l' });
    }
}

fn esc(r: &mut Rng, out: &mut Vec<u8>) {
    out.push(0x1b);
    match r.below(12) {
        0 => out.push(*r.pick(b"78DEMHc=>NOno|}~Z\\69VWFlm")),
        1 => {
            out.push(*r.pick(b"()*+-./"));
            out.push(*r.pick(b"B0AUK<>12456789=%7HCEQRYZ`afi"));
        }
        2 => {
            out.push(b'#');
            out.push(*r.pick(b"3456789"));
        }
        3 => {
            out.push(b'%');
            out.push(*r.pick(b"G@8"));
        }
        4 => {
            out.push(b' ');
            out.push(*r.pick(b"FGLMN"));
        }
        _ => out.push(32 + r.below(95) as u8),
    }
}

fn text(r: &mut Rng, out: &mut Vec<u8>) {
    match r.below(12) {
        0..=3 => out.extend_from_slice(r.pick(WORDS).as_bytes()),
        4..=7 => out.extend_from_slice(r.pick(UNICODE).as_bytes()),
        8 => {
            let n = 1 + r.below(30);
            let c = 33 + r.below(90) as u8;
            out.extend(std::iter::repeat_n(c, n));
        }
        9 => {
            // random unicode scalar
            let cp = match r.below(5) {
                0 => r.below(0x80) as u32,
                1 => 0x80 + r.below(0x780) as u32,
                2 => 0x800 + r.below(0xf000) as u32,
                _ => 0x10000 + r.below(0x10000) as u32,
            };
            if let Some(ch) = char::from_u32(cp) {
                let mut b = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
            }
        }
        10 => {
            // invalid / truncated utf8 and C1 bytes
            let n = 1 + r.below(3);
            for _ in 0..n {
                out.push(*r.pick(&[
                    0x80, 0x9b, 0x90, 0x9d, 0x9c, 0xc0, 0xc3, 0xe4, 0xe4, 0xb8, 0xf0, 0x9f, 0xf8,
                    0xfe, 0xff, 0xa9, 0x85, 0xed, 0xa0,
                ]));
            }
        }
        _ => {
            let n = 1 + r.below(8);
            for _ in 0..n {
                out.push(32 + r.below(95) as u8);
            }
        }
    }
}

fn token(r: &mut Rng, out: &mut Vec<u8>) {
    match r.below(100) {
        0..=29 => text(r, out),
        30..=39 => out.push(*r.pick(&[
            b'\r', b'\n', b'\n', b'\t', b'\t', 0x08, 0x0b, 0x0c, 0x0e, 0x0f, 0x07, 0x00, 0x05,
            0x7f, 0x1a, 0x18, 0x1f, 0x01, 0x11, 0x13,
        ])),
        40..=47 => sgr(r, out),
        48..=57 => csi_common(r, out),
        58..=64 => csi(r, out),
        65..=71 => mode(r, out),
        72..=75 => margins(r, out),
        76..=81 => esc(r, out),
        82..=85 => osc(r, out),
        86..=87 => dcs(r, out),
        88 => apc(r, out),
        89..=91 => {
            // tabs stuff
            out.extend_from_slice(
                r.pick(&[
                    "\x1b[3g",
                    "\x1b[0g",
                    "\x1bH",
                    "\t\t",
                    "\x1b[2I",
                    "\x1b[3Z",
                    "\x1b[?5W",
                    "\x1b[2 W",
                    "\x1b[0;5;9 W",
                ])
                .as_bytes(),
            );
        }
        92..=94 => {
            // fill screen to provoke scrolling
            let n = 1 + r.below(40);
            for _ in 0..n {
                out.extend_from_slice(b"line text\r\n");
            }
        }
        95 => out.extend_from_slice(
            r.pick(&[
                "\x1b[?1049h",
                "\x1b[?1049l",
                "\x1b[?47h",
                "\x1b[?47l",
                "\x1b[?1047h",
                "\x1b[?1048h",
                "\x1b[?1048l",
                "\x1bc",
                "\x1b[!p",
                "\x1b[2J",
                "\x1b[3J",
                "\x1b[H\x1b[J",
                "\x1b[?25l",
            ])
            .as_bytes(),
        ),
        96 => out.extend_from_slice(
            r.pick(&[
                "\x1b[1\"q",
                "\x1b[2\"q",
                "\x1b[0\"q",
                "\x1bV",
                "\x1bW",
                "\x1b[?1J",
                "\x1b[?2K",
                "\x1b[1;5r\x1b[?6h",
                "\x1b[0 q",
                "\x1b[5 q",
                "\x1b[>1u",
                "\x1b[=5;1u",
                "\x1b[<u",
                "\x1b[?u",
                "\x1b[>4;2m",
            ])
            .as_bytes(),
        ),
        97 if r.chance(1, 2) => {
            // hyperlink span with text, sometimes left open
            let uri = r.pick(&["http://a.example/x", "file:///tmp/y", "", "https://z"]);
            let id = r.pick(&["", "id=1", "id=abc", "foo=bar:id=q"]);
            out.extend_from_slice(format!("\x1b]8;{id};{uri}\x1b\\").as_bytes());
            text(r, out);
            if r.chance(3, 4) {
                out.extend_from_slice(b"\x1b]8;;\x1b\\");
            }
        }
        97 => out.extend_from_slice(
            b"\x1b[?7h\x1b[?7l\x1b[?45h\x1b[?1045h\x1b[?4h\x1b[4l\x1b[?2027h\x1b[?2027l",
        ),
        98 => {
            // grapheme clustering + margins + right-edge cursor
            out.extend_from_slice(b"\x1b[?2027h\x1b[?69h");
            out.extend_from_slice(
                format!("\x1b[{};{}s", 1 + r.below(5), 3 + r.below(12)).as_bytes(),
            );
            out.extend_from_slice(
                format!("\x1b[{};{}H", 1 + r.below(30), 1 + r.below(200)).as_bytes(),
            );
            text(r, out);
            text(r, out);
        }
        _ => {
            // move cursor to a random edge before writing
            out.extend_from_slice(format!("\x1b[{};{}H", r.below(60), r.below(120)).as_bytes());
        }
    }
}

/// Generate a byte stream of roughly `tokens` tokens.
pub fn generate(r: &mut Rng, tokens: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..tokens {
        token(r, &mut out);
    }
    // Occasionally mutate raw bytes to hit odd parser paths.
    if r.chance(1, 8) && !out.is_empty() {
        for _ in 0..=r.below(4) {
            let i = r.below(out.len());
            out[i] = r.below(256) as u8;
        }
    }
    out
}

/// Choose terminal geometry and scrollback for a case.
pub fn config(r: &mut Rng) -> (u16, u16, usize) {
    let sizes: &[(u16, u16)] = &[
        (1, 1),
        (2, 1),
        (1, 2),
        (2, 2),
        (3, 3),
        (5, 3),
        (8, 4),
        (10, 5),
        (16, 6),
        (20, 8),
        (40, 10),
        (80, 24),
        (132, 12),
        (7, 30),
        (255, 5),
        (300, 3),
    ];
    let (c, rr) = *r.pick(sizes);
    let sb = *r.pick(&[0usize, 1, 100, 10_000, 10_000, 100_000]);
    (c, rr, sb)
}
