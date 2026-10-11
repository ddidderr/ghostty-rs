//! tmux layout string parsing (port of `tmux/layout.zig`).

/// A tmux layout.
///
/// A tree structure; children are owned by their parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Width of the node.
    pub width: usize,
    /// Height of the node.
    pub height: usize,
    /// X offset from the top-left corner of the window.
    pub x: usize,
    /// Y offset from the top-left corner of the window.
    pub y: usize,
    /// The content of this node, either a pane (leaf) or more nodes
    /// (split) horizontally or vertically.
    pub content: Content,
}

/// Content of a layout node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Leaf pane with its ID.
    Pane(usize),
    /// Left-right split.
    Horizontal(Vec<Layout>),
    /// Top-bottom split.
    Vertical(Vec<Layout>),
}

/// Errors from layout parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The layout string is malformed.
    #[error("syntax error")]
    SyntaxError,
    /// The checksum prefix did not match.
    #[error("checksum mismatch")]
    ChecksumMismatch,
}

fn parse_usize(s: &[u8]) -> Result<usize, ParseError> {
    // std.fmt.parseInt(usize, s, 10): optional '+', digits only.
    let s = s.strip_prefix(b"+").unwrap_or(s);
    if s.is_empty() {
        return Err(ParseError::SyntaxError);
    }
    let mut v: usize = 0;
    for &c in s {
        if !c.is_ascii_digit() {
            return Err(ParseError::SyntaxError);
        }
        v = v
            .checked_mul(10)
            .and_then(|v| v.checked_add(usize::from(c - b'0')))
            .ok_or(ParseError::SyntaxError)?;
    }
    Ok(v)
}

fn index_of_any(s: &[u8], set: &[u8]) -> Option<usize> {
    s.iter().position(|c| set.contains(c))
}

impl Layout {
    /// Parse a layout string that includes a 4-character checksum prefix
    /// (`XXXX,layout_string`).
    pub fn parse_with_checksum(str: &[u8]) -> Result<Layout, ParseError> {
        if str.len() < 5 || str[4] != b',' {
            return Err(ParseError::SyntaxError);
        }
        let checksum = Checksum::calculate(&str[5..]);
        if !str.starts_with(&checksum.as_string()) {
            return Err(ParseError::ChecksumMismatch);
        }
        Self::parse(&str[5..])
    }

    /// Parse a layout string into a Layout structure.
    ///
    /// - `WxH,X,Y,ID` leaf pane
    /// - `WxH,X,Y{...}` horizontal split
    /// - `WxH,X,Y[...]` vertical split
    pub fn parse(str: &[u8]) -> Result<Layout, ParseError> {
        let mut offset = 0;
        let root = Self::parse_next(str, &mut offset)?;
        if offset != str.len() {
            return Err(ParseError::SyntaxError);
        }
        Ok(root)
    }

    fn parse_next(str: &[u8], offset: &mut usize) -> Result<Layout, ParseError> {
        let err = ParseError::SyntaxError;
        let idx = str[*offset..].iter().position(|&c| c == b'x').ok_or(err)?;
        let width = parse_usize(&str[*offset..*offset + idx]);
        *offset += idx + 1;
        let width = width?;

        let idx = str[*offset..].iter().position(|&c| c == b',').ok_or(err)?;
        let height = parse_usize(&str[*offset..*offset + idx]);
        *offset += idx + 1;
        let height = height?;

        let idx = str[*offset..].iter().position(|&c| c == b',').ok_or(err)?;
        let x = parse_usize(&str[*offset..*offset + idx]);
        *offset += idx + 1;
        let x = x?;

        let idx = index_of_any(&str[*offset..], b",{[").ok_or(err)?;
        let y = parse_usize(&str[*offset..*offset + idx]);
        *offset += idx;
        let y = y?;

        let content = match str[*offset] {
            b',' => {
                *offset += 1;
                let idx = index_of_any(&str[*offset..], b",}]").unwrap_or(str.len() - *offset);
                let pane_id = parse_usize(&str[*offset..*offset + idx]);
                *offset += idx;
                Content::Pane(pane_id?)
            }
            opening @ (b'{' | b'[') => {
                let mut nodes = Vec::new();
                *offset += 1;
                loop {
                    nodes.push(Self::parse_next(str, offset)?);
                    if *offset >= str.len() {
                        return Err(err);
                    }
                    if str[*offset] == b',' {
                        *offset += 1;
                        continue;
                    }
                    let close = if opening == b'{' { b'}' } else { b']' };
                    if str[*offset] != close {
                        return Err(err);
                    }
                    *offset += 1;
                    break if opening == b'{' {
                        Content::Horizontal(nodes)
                    } else {
                        Content::Vertical(nodes)
                    };
                }
            }
            _ => unreachable!("index_of_any guarantees one of the delimiters"),
        };

        Ok(Layout {
            width,
            height,
            x,
            y,
            content,
        })
    }
}

/// Checksum of a tmux layout string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checksum(pub u16);

impl Checksum {
    /// Calculate the checksum of a tmux layout string: rotate right by 1
    /// bit (with wraparound) and add each byte.
    pub fn calculate(str: &[u8]) -> Checksum {
        let mut result: u16 = 0;
        for &c in str {
            result = result.rotate_right(1);
            result = result.wrapping_add(u16::from(c));
        }
        Checksum(result)
    }

    /// 4-character zero-padded lowercase hex string.
    pub fn as_string(self) -> [u8; 4] {
        let charset = b"0123456789abcdef";
        let v = self.0;
        [
            charset[usize::from((v >> 12) & 0xf)],
            charset[usize::from((v >> 8) & 0xf)],
            charset[usize::from((v >> 4) & 0xf)],
            charset[usize::from(v & 0xf)],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Layout, ParseError> {
        Layout::parse(s.as_bytes())
    }
    fn pane(l: &Layout) -> usize {
        match l.content {
            Content::Pane(p) => p,
            _ => panic!("not a pane"),
        }
    }
    fn horiz(l: &Layout) -> &[Layout] {
        match &l.content {
            Content::Horizontal(c) => c,
            _ => panic!("not horizontal"),
        }
    }
    fn vert(l: &Layout) -> &[Layout] {
        match &l.content {
            Content::Vertical(c) => c,
            _ => panic!("not vertical"),
        }
    }
    fn dims(l: &Layout) -> (usize, usize, usize, usize) {
        (l.width, l.height, l.x, l.y)
    }
    fn cs(s: &str) -> String {
        String::from_utf8(Checksum::calculate(s.as_bytes()).as_string().to_vec()).unwrap()
    }

    #[test]
    fn simple_single_pane() {
        let l = p("80x24,0,0,42").unwrap();
        assert_eq!(dims(&l), (80, 24, 0, 0));
        assert_eq!(pane(&l), 42);
    }

    #[test]
    fn single_pane_with_offset() {
        let l = p("40x12,10,5,7").unwrap();
        assert_eq!(dims(&l), (40, 12, 10, 5));
        assert_eq!(pane(&l), 7);
    }

    #[test]
    fn single_pane_large_values() {
        let l = p("1920x1080,100,200,999").unwrap();
        assert_eq!(dims(&l), (1920, 1080, 100, 200));
        assert_eq!(pane(&l), 999);
    }

    #[test]
    fn horizontal_split_two_panes() {
        let l = p("80x24,0,0{40x24,0,0,1,40x24,40,0,2}").unwrap();
        assert_eq!(dims(&l), (80, 24, 0, 0));
        let c = horiz(&l);
        assert_eq!(c.len(), 2);
        assert_eq!(dims(&c[0]), (40, 24, 0, 0));
        assert_eq!(pane(&c[0]), 1);
        assert_eq!(dims(&c[1]), (40, 24, 40, 0));
        assert_eq!(pane(&c[1]), 2);
    }

    #[test]
    fn vertical_split_two_panes() {
        let l = p("80x24,0,0[80x12,0,0,1,80x12,0,12,2]").unwrap();
        assert_eq!(dims(&l), (80, 24, 0, 0));
        let c = vert(&l);
        assert_eq!(c.len(), 2);
        assert_eq!(dims(&c[0]), (80, 12, 0, 0));
        assert_eq!(pane(&c[0]), 1);
        assert_eq!(dims(&c[1]), (80, 12, 0, 12));
        assert_eq!(pane(&c[1]), 2);
    }

    #[test]
    fn horizontal_split_three_panes() {
        let l = p("120x24,0,0{40x24,0,0,1,40x24,40,0,2,40x24,80,0,3}").unwrap();
        assert_eq!((l.width, l.height), (120, 24));
        let c = horiz(&l);
        assert_eq!(c.len(), 3);
        assert_eq!(pane(&c[0]), 1);
        assert_eq!(pane(&c[1]), 2);
        assert_eq!(pane(&c[2]), 3);
    }

    #[test]
    fn nested_horizontal_in_vertical() {
        let l = p("80x24,0,0[80x12,0,0,1,80x12,0,12{40x12,0,12,2,40x12,40,12,3}]").unwrap();
        let v = vert(&l);
        assert_eq!(v.len(), 2);
        assert_eq!(pane(&v[0]), 1);
        let h = horiz(&v[1]);
        assert_eq!(h.len(), 2);
        assert_eq!(pane(&h[0]), 2);
        assert_eq!(pane(&h[1]), 3);
    }

    #[test]
    fn nested_vertical_in_horizontal() {
        let l = p("80x24,0,0{40x24,0,0,1,40x24,40,0[40x12,40,0,2,40x12,40,12,3]}").unwrap();
        let h = horiz(&l);
        assert_eq!(h.len(), 2);
        assert_eq!(pane(&h[0]), 1);
        let v = vert(&h[1]);
        assert_eq!(v.len(), 2);
        assert_eq!(pane(&v[0]), 2);
        assert_eq!(pane(&v[1]), 3);
    }

    #[test]
    fn deeply_nested_layout() {
        let l = p("80x24,0,0{40x24,0,0[40x12,0,0,1,40x12,0,12,2],40x24,40,0,3}").unwrap();
        let h = horiz(&l);
        assert_eq!(h.len(), 2);
        let v = vert(&h[0]);
        assert_eq!(v.len(), 2);
        assert_eq!(pane(&v[0]), 1);
        assert_eq!(pane(&v[1]), 2);
        assert_eq!(pane(&h[1]), 3);
    }

    fn syntax(s: &str) {
        assert_eq!(p(s), Err(ParseError::SyntaxError), "{s}");
    }

    #[test]
    fn syntax_error_empty_string() {
        syntax("");
    }
    #[test]
    fn syntax_error_missing_width() {
        syntax("x24,0,0,1");
    }
    #[test]
    fn syntax_error_missing_height() {
        syntax("80x,0,0,1");
    }
    #[test]
    fn syntax_error_missing_x() {
        syntax("80x24,,0,1");
    }
    #[test]
    fn syntax_error_missing_y() {
        syntax("80x24,0,,1");
    }
    #[test]
    fn syntax_error_missing_pane_id() {
        syntax("80x24,0,0,");
    }
    #[test]
    fn syntax_error_non_numeric_width() {
        syntax("abcx24,0,0,1");
    }
    #[test]
    fn syntax_error_non_numeric_pane_id() {
        syntax("80x24,0,0,abc");
    }
    #[test]
    fn syntax_error_unclosed_horizontal_bracket() {
        syntax("80x24,0,0{40x24,0,0,1");
    }
    #[test]
    fn syntax_error_unclosed_vertical_bracket() {
        syntax("80x24,0,0[40x24,0,0,1");
    }
    #[test]
    fn syntax_error_mismatched_brackets() {
        syntax("80x24,0,0{40x24,0,0,1]");
        syntax("80x24,0,0[40x24,0,0,1}");
    }
    #[test]
    fn syntax_error_trailing_data() {
        syntax("80x24,0,0,1extra");
    }
    #[test]
    fn syntax_error_no_x_separator() {
        syntax("8024,0,0,1");
    }
    #[test]
    fn syntax_error_no_content_delimiter() {
        syntax("80x24,0,0");
    }

    #[test]
    fn parse_with_checksum_valid() {
        let l = Layout::parse_with_checksum(b"f8f9,80x24,0,0{40x24,0,0,1,40x24,40,0,2}").unwrap();
        assert_eq!((l.width, l.height), (80, 24));
    }
    #[test]
    fn parse_with_checksum_mismatch() {
        assert_eq!(
            Layout::parse_with_checksum(b"0000,80x24,0,0{40x24,0,0,1,40x24,40,0,2}"),
            Err(ParseError::ChecksumMismatch)
        );
    }
    #[test]
    fn parse_with_checksum_too_short() {
        assert_eq!(
            Layout::parse_with_checksum(b"bb62"),
            Err(ParseError::SyntaxError)
        );
        assert_eq!(
            Layout::parse_with_checksum(b""),
            Err(ParseError::SyntaxError)
        );
    }
    #[test]
    fn parse_with_checksum_missing_comma() {
        assert_eq!(
            Layout::parse_with_checksum(b"bb62x159x48,0,0"),
            Err(ParseError::SyntaxError)
        );
    }

    #[test]
    fn checksum_empty_string() {
        assert_eq!(Checksum::calculate(b"").0, 0);
        assert_eq!(cs(""), "0000");
    }
    #[test]
    fn checksum_single_character() {
        assert_eq!(Checksum::calculate(b"A").0, 65);
        assert_eq!(cs("A"), "0041");
    }
    #[test]
    fn checksum_two_characters() {
        assert_eq!(Checksum::calculate(b"AB").0, 32866);
        assert_eq!(cs("AB"), "8062");
    }
    #[test]
    fn checksum_simple_layout() {
        assert_eq!(cs("80x24,0,0,42"), "d962");
    }
    #[test]
    fn checksum_horizontal_split_layout() {
        assert_eq!(cs("80x24,0,0{40x24,0,0,1,40x24,40,0,2}"), "f8f9");
    }
    #[test]
    fn checksum_as_string_zero_padding() {
        assert_eq!(&Checksum(0x000f).as_string(), b"000f");
    }
    #[test]
    fn checksum_as_string_all_digits() {
        assert_eq!(&Checksum(0x1234).as_string(), b"1234");
    }
    #[test]
    fn checksum_as_string_with_letters() {
        assert_eq!(&Checksum(0xabcd).as_string(), b"abcd");
    }
    #[test]
    fn checksum_as_string_max_value() {
        assert_eq!(&Checksum(0xffff).as_string(), b"ffff");
    }
    #[test]
    fn checksum_wraparound() {
        let c = Checksum::calculate(&[0xff; 8]);
        assert_eq!(&c.as_string(), b"03fc");
    }
    #[test]
    fn checksum_deterministic() {
        let s = b"159x48,0,0{79x48,0,0,79x48,80,0}";
        assert_eq!(Checksum::calculate(s), Checksum::calculate(s));
    }
    #[test]
    fn checksum_different_inputs_different_outputs() {
        assert_ne!(
            Checksum::calculate(b"80x24,0,0,1"),
            Checksum::calculate(b"80x24,0,0,2")
        );
    }
    #[test]
    fn checksum_known_tmux_layout_bb62() {
        assert_eq!(cs("159x48,0,0{79x48,0,0,79x48,80,0}"), "bb62");
    }
}
