//! Source-level adaptation of combined image samplers for naga.
//!
//! glslang (Vulkan GLSL) accepts `sampler2D` as a function parameter type and
//! lets shaders pass the combined `iChannel0` around. naga's GLSL frontend
//! has no combined sampler *type*, only separate `texture2D` + `sampler`
//! objects (and a `sampler2D(texture, sampler)` constructor expression that
//! is valid only directly inside texture calls).
//!
//! This module rewrites user code so such helpers work:
//!
//! * `T f(.., [qual] sampler2D s, ..)`  ->  `T f(.., texture2D s_tex, sampler s_smp, ..)`
//! * uses of `s` in the function body  ->  `sampler2D(s_tex, s_smp)`
//! * `f(.., s, ..)` / `f(.., iChannel0, ..)` call arguments in sampler
//!   positions  ->  `s_tex, s_smp` / `iChannel0_tex, iChannel0_smp`
//! * legacy `texture2D(` calls  ->  `texture(` (glslang gets this from the
//!   prefix macro `#define texture2D texture`, which would otherwise clash
//!   with the `texture2D` type).
//!
//! Limitations (documented, reported as compile errors by naga): a sampler
//! argument must be a plain identifier (no ternaries or struct members),
//! samplers cannot be stored in local variables/arrays/struct fields, and
//! overloads of one function name must agree on which parameters are
//! samplers.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Whitespace, comments and preprocessor lines (not significant).
    Trivia,
    Ident,
    Punct,
    Other,
}

struct Tok<'a> {
    kind: Kind,
    text: &'a str,
}

fn lex(src: &str) -> Vec<Tok<'_>> {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < b.len() {
        let start = i;
        let c = b[i];
        let kind;
        if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                if b[i] == b'\n' {
                    line_start = true;
                }
                i += 1;
            }
            // line_start must reflect "only whitespace since newline".
            toks.push(Tok {
                kind: Kind::Trivia,
                text: &src[start..i],
            });
            continue;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            kind = Kind::Trivia;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
            kind = Kind::Trivia;
        } else if c == b'#' && line_start {
            // Preprocessor line (with backslash continuations).
            while i < b.len() && b[i] != b'\n' {
                if b[i] == b'\\' && b.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                i += 1;
            }
            kind = Kind::Trivia;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            kind = Kind::Ident;
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            kind = Kind::Other;
        } else {
            // Any single (possibly multi-byte) character.
            i += 1;
            while i < b.len() && (b[i] & 0xC0) == 0x80 {
                i += 1;
            }
            kind = Kind::Punct;
        }
        line_start = false;
        toks.push(Tok {
            kind,
            text: &src[start..i],
        });
    }
    toks
}

fn is_sig(t: &Tok<'_>) -> bool {
    t.kind != Kind::Trivia
}

/// Index of the token matching the opener at `sig[open]` (indices into the
/// significant-token list), or None.
fn matching(toks: &[Tok<'_>], sig: &[usize], open: usize) -> Option<usize> {
    let o = toks[sig[open]].text;
    let c = match o {
        "(" => ")",
        "{" => "}",
        "[" => "]",
        _ => return None,
    };
    let mut depth = 0usize;
    for (k, &ti) in sig.iter().enumerate().skip(open) {
        let t = toks[ti].text;
        if t == o {
            depth += 1;
        } else if t == c {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// Split the significant range `(from..to)` at top-level commas, returning
/// `(start, end)` ranges (exclusive end) in significant-token indices.
fn split_args(toks: &[Tok<'_>], sig: &[usize], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if from >= to {
        return out;
    }
    let mut depth = 0i32;
    let mut start = from;
    for k in from..to {
        match toks[sig[k]].text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "," if depth == 0 => {
                out.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push((start, to));
    out
}

/// A top-level function definition/prototype (indices are into the
/// significant-token list).
struct Def {
    name_k: usize,
    params: Vec<(usize, usize)>,
    body: Option<(usize, usize)>,
    samplers: Vec<Option<String>>,
}

struct FnInfo {
    /// Per parameter: the sampler name, if the parameter is a `sampler2D`.
    samplers: Vec<Option<String>>,
}

/// Rewrite the user's ShaderToy source. Returns the source unchanged if it
/// needs no adaptation other than `texture2D(` -> `texture(`.
pub fn rewrite(src: &str) -> String {
    let toks = lex(src);
    let sig: Vec<usize> = toks
        .iter()
        .enumerate()
        .filter(|(_, t)| is_sig(t))
        .map(|(i, _)| i)
        .collect();

    // Pass 1: find top-level functions with sampler2D parameters.
    let mut funcs: HashMap<&str, FnInfo> = HashMap::new();
    let mut defs: Vec<Def> = Vec::new();
    let mut k = 0;
    let mut brace = 0i32;
    while k < sig.len() {
        let t = &toks[sig[k]];
        match t.text {
            "{" => brace += 1,
            "}" => brace -= 1,
            _ => {}
        }
        if brace == 0
            && t.kind == Kind::Ident
            && k + 2 < sig.len()
            && toks[sig[k + 1]].kind == Kind::Ident
            && toks[sig[k + 2]].text == "("
        {
            let name_k = k + 1;
            if let Some(close) = matching(&toks, &sig, k + 2) {
                let next = sig.get(close + 1).map(|&i| toks[i].text);
                if next == Some("{") || next == Some(";") {
                    let params = split_args(&toks, &sig, k + 3, close);
                    let mut samplers = Vec::new();
                    let mut any = false;
                    for &(a, b) in &params {
                        let has = (a..b).any(|q| toks[sig[q]].text == "sampler2D");
                        if has {
                            let name = (a..b)
                                .rev()
                                .map(|q| &toks[sig[q]])
                                .find(|t| t.kind == Kind::Ident && t.text != "sampler2D")
                                .map(|t| t.text.to_owned());
                            any |= name.is_some();
                            samplers.push(name);
                        } else {
                            samplers.push(None);
                        }
                    }
                    let body = if next == Some("{") {
                        matching(&toks, &sig, close + 1).map(|e| (close + 1, e))
                    } else {
                        None
                    };
                    if any {
                        funcs
                            .entry(toks[sig[name_k]].text)
                            .or_insert_with(|| FnInfo {
                                samplers: samplers.clone(),
                            });
                    }
                    defs.push(Def {
                        name_k,
                        params,
                        body,
                        samplers,
                    });
                    // Skip past the signature (bodies are scanned for
                    // braces by the normal loop).
                    k = close + 1;
                    continue;
                }
            }
        }
        k += 1;
    }

    // Pass 2: collect replacements keyed by token index.
    let mut repl: HashMap<usize, String> = HashMap::new();
    let mut def_names: Vec<usize> = Vec::new();
    for d in &defs {
        def_names.push(d.name_k);
        for (pi, &(a, b)) in d.params.iter().enumerate() {
            if let Some(name) = d.samplers.get(pi).and_then(Option::as_ref) {
                repl.insert(sig[a], format!("texture2D {name}_tex, sampler {name}_smp"));
                // Blank the rest of the parameter, including interior trivia.
                for ti in (sig[a] + 1)..=sig[b - 1] {
                    repl.insert(ti, String::new());
                }
            }
        }
        // Uses of the sampler parameters in the body.
        if let Some((open, close)) = d.body {
            let names: Vec<&String> = d.samplers.iter().flatten().collect();
            for q in open + 1..close {
                let t = &toks[sig[q]];
                if t.kind == Kind::Ident && names.iter().any(|n| n.as_str() == t.text) {
                    repl.insert(sig[q], format!("sampler2D({0}_tex, {0}_smp)", t.text));
                }
            }
        }
    }

    // Call sites (and legacy texture2D).
    for k in 0..sig.len() {
        let t = &toks[sig[k]];
        if t.kind != Kind::Ident || sig.get(k + 1).map(|&i| toks[i].text) != Some("(") {
            continue;
        }
        if t.text == "texture2D" {
            repl.insert(sig[k], "texture".to_owned());
            continue;
        }
        if def_names.contains(&k) {
            continue;
        }
        let Some(info) = funcs.get(t.text) else {
            continue;
        };
        let Some(close) = matching(&toks, &sig, k + 1) else {
            continue;
        };
        let args = split_args(&toks, &sig, k + 2, close);
        for (ai, &(a, b)) in args.iter().enumerate() {
            if info.samplers.get(ai).is_some_and(Option::is_some)
                && b == a + 1
                && toks[sig[a]].kind == Kind::Ident
            {
                let n = toks[sig[a]].text;
                repl.insert(sig[a], format!("{n}_tex, {n}_smp"));
            }
        }
    }

    let mut out = String::with_capacity(src.len() + 64);
    for (i, t) in toks.iter().enumerate() {
        match repl.get(&i) {
            Some(r) => out.push_str(r),
            None => out.push_str(t.text),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_param_use_and_call() {
        let out = rewrite(
            "vec4 fetch(sampler2D tex, vec2 uv) { return texture(tex, uv); }\n\
             void mainImage(out vec4 c, in vec2 p) { c = fetch(iChannel0, p); }",
        );
        assert!(out.contains("vec4 fetch(texture2D tex_tex, sampler tex_smp, vec2 uv)"));
        assert!(out.contains("texture(sampler2D(tex_tex, tex_smp), uv)"));
        assert!(out.contains("fetch(iChannel0_tex, iChannel0_smp, p)"));
    }

    #[test]
    fn leaves_plain_code_and_rewrites_legacy_texture2d() {
        let src = "void mainImage(out vec4 c, in vec2 p) { c = texture2D(iChannel0, p); }";
        assert_eq!(
            rewrite(src),
            "void mainImage(out vec4 c, in vec2 p) { c = texture(iChannel0, p); }"
        );
    }

    #[test]
    fn ignores_comments_and_forwards_samplers() {
        let out = rewrite(
            "// sampler2D in comment\n\
             vec4 a(in sampler2D s) { return texture(s, vec2(0.0)); }\n\
             vec4 b(sampler2D t, float x) { return a(t) * x; }\n",
        );
        assert!(out.contains("// sampler2D in comment"));
        assert!(out.contains("vec4 a(texture2D s_tex, sampler s_smp)"));
        assert!(out.contains("return a(t_tex, t_smp) * x;"));
    }
}
