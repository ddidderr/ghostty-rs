//! Default URL/path regex (port of `config/url.zig`).
//!
//! The pattern uses Oniguruma (Ruby syntax) features such as variable-length
//! lookbehind; use the `onig` crate to compile it.

use const_format::concatcp;

const URL_SCHEMES: &str = r"https?://|mailto:|ftp://|file:|ssh:|git://|ssh://|tel:|magnet:|ipfs://|ipns://|gemini://|gopher://|news:";

const IPV6_URL_PATTERN: &str = r"(?:\[[:0-9a-fA-F]+(?:[:0-9a-fA-F]*)+\](?::[0-9]+)?)";

const SCHEME_URL_CHARS: &str = r"[\w\-.~:/?#@!$&*+,;=%]";

const PATH_CHARS: &str = r"[\w\-.~:\/?#@!$&*+;=%]";

const OPTIONAL_BRACKETED_WORD_SUFFIX: &str = r"(?:[\(\[]\w*[\)\]])?";

const NO_TRAILING_PUNCTUATION: &str = r"(?<![,.])";

const NO_TRAILING_COLON: &str = r"(?<!:)";

const TRAILING_SPACES_AT_EOL: &str = r"(?: +(?= *$))?";

const DOTTED_PATH_LOOKAHEAD: &str = r"(?=[\w\-.~:\/?#@!$&*+;=%]*\.)";

const NON_DOTTED_PATH_LOOKAHEAD: &str = r"(?![\w\-.~:\/?#@!$&*+;=%]*\.)";

const DOTTED_PATH_SPACE_SEGMENTS: &str =
    r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]*[\/.])*";

const ANY_PATH_SPACE_SEGMENTS: &str =
    r"(?:(?<!:) (?!\w+:\/\/)(?!\.{0,2}\/)(?!~\/)[\w\-.~:\/?#@!$&*+;=%]+)*";

const ROOTED_OR_RELATIVE_PATH_PREFIX: &str = r"(?:\.\.\/|\.\/|(?<!\w)~\/|(?:[\w][\w\-.]*\/)*(?<!\w)\$[A-Za-z_]\w*\/|\.[\w][\w\-.]*\/|(?<![\w~\/])\/(?!\/))";

const BARE_RELATIVE_PATH_PREFIX: &str = r"(?<!\$\d*)(?<!\w)[\w][\w\-.]*\/";

// Branch 1: URLs with explicit schemes (http, mailto, ftp, etc.).
const SCHEME_URL_BRANCH: &str = concatcp!(
    "(?:",
    URL_SCHEMES,
    ")",
    "(?:",
    IPV6_URL_PATTERN,
    "|",
    SCHEME_URL_CHARS,
    "+",
    OPTIONAL_BRACKETED_WORD_SUFFIX,
    ")+",
    NO_TRAILING_PUNCTUATION
);

// Branch 2: Absolute paths and dot-relative paths (/, ./, ../).
// A dotted segment is treated as file-like, while the undotted case stays
// broad to capture directory-like paths with spaces.
const ROOTED_OR_RELATIVE_PATH_BRANCH: &str = concatcp!(
    ROOTED_OR_RELATIVE_PATH_PREFIX,
    "(?:",
    DOTTED_PATH_LOOKAHEAD,
    PATH_CHARS,
    "+",
    DOTTED_PATH_SPACE_SEGMENTS,
    NO_TRAILING_COLON,
    TRAILING_SPACES_AT_EOL,
    "|",
    NON_DOTTED_PATH_LOOKAHEAD,
    PATH_CHARS,
    "+",
    ANY_PATH_SPACE_SEGMENTS,
    NO_TRAILING_COLON,
    TRAILING_SPACES_AT_EOL,
    ")"
);

// Branch 3: Bare relative paths such as src/config/url.zig.
const BARE_RELATIVE_PATH_BRANCH: &str = concatcp!(
    DOTTED_PATH_LOOKAHEAD,
    BARE_RELATIVE_PATH_PREFIX,
    PATH_CHARS,
    "+",
    NO_TRAILING_COLON,
    TRAILING_SPACES_AT_EOL
);

/// Default URL/path regex. This is used to detect URLs and file paths in
/// terminal output.
///
/// This is here in the config package because one day the matchers will be
/// configurable and this will be a default.
///
/// For scheme URLs, this regex is liberal in what it accepts after the scheme,
/// with exceptions for URLs ending with . or ). Although such URLs are
/// perfectly valid, it is common for text to contain URLs surrounded by
/// parentheses (such as in Markdown links) or at the end of sentences.
/// Therefore, this regex excludes them as follows:
///
/// 1. Do not match regexes ending with .
/// 2. Do not match regexes ending with ), except for ones which contain a (
///    without a subsequent )
///
/// Rule 2 means that that we handle the following two cases:
///
///   "https://en.wikipedia.org/wiki/Rust_(video_game)" (include parens)
///   "(https://example.com)" (do not include the parens)
///
/// There are many complicated cases where these heuristics break down, but
/// handling them well requires a non-regex approach.
pub const REGEX: &str = concatcp!(
    SCHEME_URL_BRANCH,
    "|",
    ROOTED_OR_RELATIVE_PATH_BRANCH,
    "|",
    BARE_RELATIVE_PATH_BRANCH
);

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use onig::Regex;

    #[test]
    fn url_regex() {
        let re = Regex::new(REGEX).unwrap();

        let cases: &[(&str, &str)] = &[
            ("hello https://example.com world", "https://example.com"),
            (
                "https://example.com/foo(bar) more",
                "https://example.com/foo(bar)",
            ),
            (
                "https://example.com/foo(bar)baz more",
                "https://example.com/foo(bar)baz",
            ),
            (
                "Link inside (https://example.com) parens",
                "https://example.com",
            ),
            (
                "Link period https://example.com. More text.",
                "https://example.com",
            ),
            (
                "Link trailing comma https://example.com, more text.",
                "https://example.com",
            ),
            (
                "Link in double quotes \"https://example.com\" and more",
                "https://example.com",
            ),
            (
                "Link in single quotes 'https://example.com' and more",
                "https://example.com",
            ),
            (
                "some file with https://google.com https://duckduckgo.com links.",
                "https://google.com",
            ),
            (
                "and links in it. links https://yahoo.com mailto:test@example.com ssh://1.2.3.4",
                "https://yahoo.com",
            ),
            (
                "also match http://example.com non-secure links",
                "http://example.com",
            ),
            (
                "match tel://+12123456789 phone numbers",
                "tel://+12123456789",
            ),
            (
                "match with query url https://example.com?query=1&other=2 and more text.",
                "https://example.com?query=1&other=2",
            ),
            (
                "url with dashes [mode 2027](https://github.com/contour-terminal/terminal-unicode-core) for better unicode support",
                "https://github.com/contour-terminal/terminal-unicode-core",
            ),
            ("dot.http://example.com", "http://example.com"),
            (
                "weird characters https://example.com/~user/?query=1&other=2#hash and more",
                "https://example.com/~user/?query=1&other=2#hash",
            ),
            (
                "square brackets https://example.com/[foo] and more",
                "https://example.com/[foo]",
            ),
            (
                "[13]:TooManyStatements: TempFile#assign_temp_file_to_entity has approx 7 statements [https://example.com/docs/Too-Many-Statements.md]",
                "https://example.com/docs/Too-Many-Statements.md",
            ),
            ("match ftp://example.com ftp links", "ftp://example.com"),
            ("match file://example.com file links", "file://example.com"),
            ("match ssh://example.com ssh links", "ssh://example.com"),
            ("match git://example.com git links", "git://example.com"),
            ("/tmp/test.txt http://www.google.com", "/tmp/test.txt"),
            ("match tel:+18005551234 tel links", "tel:+18005551234"),
            (
                "match magnet:?xt=urn:btih:1234567890 magnet links",
                "magnet:?xt=urn:btih:1234567890",
            ),
            (
                "match ipfs://QmSomeHashValue ipfs links",
                "ipfs://QmSomeHashValue",
            ),
            (
                "match ipns://QmSomeHashValue ipns links",
                "ipns://QmSomeHashValue",
            ),
            (
                "match gemini://example.com gemini links",
                "gemini://example.com",
            ),
            (
                "match gopher://example.com gopher links",
                "gopher://example.com",
            ),
            (
                "match news:comp.infosystems.www.servers.unix news links",
                "news:comp.infosystems.www.servers.unix",
            ),
            (
                "/Users/ghostty.user/code/example.py",
                "/Users/ghostty.user/code/example.py",
            ),
            (
                "/Users/ghostty.user/code/../example.py",
                "/Users/ghostty.user/code/../example.py",
            ),
            (
                "/Users/ghostty.user/code/../example.py hello world",
                "/Users/ghostty.user/code/../example.py",
            ),
            ("../example.py", "../example.py"),
            ("../example.py ", "../example.py "),
            ("first time ../example.py contributor ", "../example.py"),
            (
                "[link](/home/user/ghostty.user/example)",
                "/home/user/ghostty.user/example",
            ),
            (
                "Serving HTTP on :: port 8000 (http://[::]:8000/)",
                "http://[::]:8000/",
            ),
            (
                "IPv6 address https://[2001:db8::1]:8080/path",
                "https://[2001:db8::1]:8080/path",
            ),
            ("IPv6 localhost http://[::1]:3000", "http://[::1]:3000"),
            (
                "Complex IPv6 https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/",
                "https://[2001:db8:85a3:8d3:1319:8a2e:370:7348]:443/",
            ),
            (
                "IPv6 with path https://[2001:db8::1]/path/to/resource",
                "https://[2001:db8::1]/path/to/resource",
            ),
            (
                "IPv6 with query https://[2001:db8::1]:8080/api?param=value&other=123",
                "https://[2001:db8::1]:8080/api?param=value&other=123",
            ),
            (
                "IPv6 compressed http://[2001:db8::]:80/",
                "http://[2001:db8::]:80/",
            ),
            (
                "IPv6 multiple zeros http://[2001:0:0:0:0:0:0:1]",
                "http://[2001:0:0:0:0:0:0:1]",
            ),
            (
                "IPv6 link-local https://[fe80::1234:5678:9abc]",
                "https://[fe80::1234:5678:9abc]",
            ),
            (
                "IPv6 multicast http://[ff02::1]/stream",
                "http://[ff02::1]/stream",
            ),
            (
                "IPv6 in markdown [link](http://[2001:db8::1]/docs)",
                "http://[2001:db8::1]/docs",
            ),
            ("./spaces-end.   ", "./spaces-end.   "),
            ("./space middle", "./space middle"),
            ("../test folder/file.txt", "../test folder/file.txt"),
            ("/tmp/test folder/file.txt", "/tmp/test folder/file.txt"),
            ("/tmp/test  folder/file.txt", "/tmp/test"),
            (
                "diff --git a/src/font/shaper/harfbuzz.zig b/src/font/shaper/harfbuzz.zig",
                "a/src/font/shaper/harfbuzz.zig",
            ),
            ("/tmp/foo /tmp/bar", "/tmp/foo"),
            ("/tmp/foo.txt /tmp/bar.txt", "/tmp/foo.txt"),
            ("src/config/url.zig", "src/config/url.zig"),
            ("app/folder/file.rb:1", "app/folder/file.rb:1"),
            ("modified:   src/config/url.zig", "src/config/url.zig"),
            (
                "lib/ghostty/terminal.zig:42:10",
                "lib/ghostty/terminal.zig:42:10",
            ),
            ("some-pkg/src/file.txt more text", "some-pkg/src/file.txt"),
            ("src/foo.c,baz.txt", "src/foo.c"),
            ("~/foo/bar.txt", "~/foo/bar.txt"),
            ("open ~/Documents/notes.md please", "~/Documents/notes.md"),
            ("~/.config/ghostty/config", "~/.config/ghostty/config"),
            (
                "directory: ~/src/ghostty-org/ghostty",
                "~/src/ghostty-org/ghostty",
            ),
            ("$HOME/src/config/url.zig", "$HOME/src/config/url.zig"),
            (
                "project dir: $PWD/src/ghostty/main.zig",
                "$PWD/src/ghostty/main.zig",
            ),
            ("foo/$BAR/baz", "foo/$BAR/baz"),
            (".foo/bar/$VAR", ".foo/bar/$VAR"),
            (".config/ghostty/config", ".config/ghostty/config"),
            (
                "loaded from .local/share/ghostty/state.db now",
                ".local/share/ghostty/state.db",
            ),
            ("../some/where", "../some/where"),
            (
                "  - shared/src/foo/SomeItem.m:12, shared/src/",
                "shared/src/foo/SomeItem.m:12",
            ),
            ("foo.local/share", "foo.local/share"),
            ("2024/report.txt", "2024/report.txt"),
            ("./foo bar,baz", "./foo bar"),
            ("/tmp/foo bar,baz", "/tmp/foo bar"),
            (
                "./.config/ghostty: Needs upstream (main)",
                "./.config/ghostty",
            ),
            ("./Downloads: Operation not permitted", "./Downloads"),
        ];

        for (input, expect) in cases {
            let mut it = re.find_iter(input);
            let (s, e) = it
                .next()
                .unwrap_or_else(|| panic!("no match for {input:?}"));
            assert_eq!(&input[s..e], *expect, "input: {input:?}");
        }

        let no_match_cases: &[&str] = &[
            "input/output",
            "foo/bar",
            "$10/bar",
            "$10/$20",
            "$10/bar.txt",
            "foo/bar,baz.txt",
            "foo$BAR/baz.txt",
            "foo~/bar.txt",
            "// foo bar",
            "//foo",
        ];
        for input in no_match_cases {
            assert!(re.find(input).is_none(), "unexpected match for {input:?}");
        }
    }
}
