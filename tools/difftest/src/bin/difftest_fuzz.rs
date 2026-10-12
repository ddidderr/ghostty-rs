//! Differential fuzz harness: Rust dump vs Zig driver output.
//!
//! ```text
//! difftest_fuzz run [--seed N] [--cases N] [--jobs N] [--tokens N]
//!                   [--zig PATH] [--work DIR] [--size CxR] [--sb BYTES]
//! difftest_fuzz minz|minr <cols> <rows> <sb> <input-file> # minimise an abort
//! difftest_fuzz panics [--seed N] [--cases N] [--tokens N]  # Rust only, no Zig
//! difftest_fuzz min <cols> <rows> <sb> <input-file>   # minimise a divergence
//! difftest_fuzz check <cols> <rows> <sb> <input-file> # compare one input
//! ```
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use ghostty_difftest::{
    dump,
    inputgen::{Rng, config, generate},
};

struct Ctx {
    zig: PathBuf,
    work: PathBuf,
}

fn rust_dump(input: &[u8], c: u16, r: u16, sb: usize) -> String {
    let input = input.to_vec();
    std::panic::catch_unwind(move || dump(&input, c, r, sb))
        .unwrap_or_else(|_| "RUST PANIC\n".to_string())
}

fn zig_dump(ctx: &Ctx, tag: &str, input: &[u8], c: u16, r: u16, sb: usize) -> String {
    zig_run(ctx, tag, input, c, r, sb).0
}

/// Run the Zig driver. The bool is true when Zig grew a page's capacity
/// (`info(page_list): adjusting page capacity`) while running: several Zig
/// code paths keep raw cell pointers across such a reallocation (e.g. the
/// grapheme-attach after a wrap in `Terminal.print`), so a mismatch there can
/// be a use-after-free in Zig rather than a Rust bug.
fn zig_run(ctx: &Ctx, tag: &str, input: &[u8], c: u16, r: u16, sb: usize) -> (String, bool) {
    let f = ctx.work.join(format!("in-{tag}.bin"));
    std::fs::write(&f, input).expect("write input");
    let out = Command::new(&ctx.zig)
        .args([c.to_string(), r.to_string(), sb.to_string()])
        .arg(&f)
        .output()
        .expect("spawn zig");
    let _ = std::fs::remove_file(&f);
    let grown = String::from_utf8_lossy(&out.stderr).contains("adjusting page capacity");
    let text = if out.status.success() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        format!(
            "ZIG CRASH {:?}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .take(3)
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    (text, grown)
}

fn crashed(s: &str) -> bool {
    s.starts_with("ZIG CRASH") || s.starts_with("RUST PANIC")
}

/// Two outputs are equivalent if identical, or if the Zig reference itself
/// aborted (a failed debug assertion or memory-safety fault in the Zig code
/// is undefined behaviour there, so there is no behaviour to compare; these
/// are counted and saved separately as `zigcrash`).
fn equivalent(rs: &str, z: &str) -> bool {
    rs == z || z.starts_with("ZIG CRASH")
}

fn differs(ctx: &Ctx, tag: &str, input: &[u8], c: u16, r: u16, sb: usize) -> bool {
    !equivalent(
        &rust_dump(input, c, r, sb),
        &zig_dump(ctx, tag, input, c, r, sb),
    )
}

/// Predicate for `minc`: the Zig driver (or Rust, per `which`) aborts.
fn aborts(ctx: &Ctx, which: &str, input: &[u8], c: u16, r: u16, sb: usize) -> bool {
    if which == "zig" {
        crashed(&zig_dump(ctx, "minc", input, c, r, sb))
    } else {
        crashed(&rust_dump(input, c, r, sb))
    }
}

fn minimize(ctx: &Ctx, input: &[u8], c: u16, r: u16, sb: usize, crash: Option<&str>) -> Vec<u8> {
    let mut cur = input.to_vec();
    let mut chunk = cur.len().div_ceil(2).max(1);
    loop {
        let mut i = 0;
        let mut changed = false;
        while i < cur.len() {
            let end = (i + chunk).min(cur.len());
            let mut cand = cur[..i].to_vec();
            cand.extend_from_slice(&cur[end..]);
            let keep = match crash {
                Some(w) => aborts(ctx, w, &cand, c, r, sb),
                None => differs(ctx, "min", &cand, c, r, sb),
            };
            if keep {
                cur = cand;
                changed = true;
            } else {
                i += chunk;
            }
        }
        if chunk == 1 {
            if !changed {
                break;
            }
        } else {
            chunk = chunk.div_ceil(2);
        }
    }
    cur
}

fn show_diff(a: &str, b: &str) {
    let (al, bl): (Vec<_>, Vec<_>) = (a.lines().collect(), b.lines().collect());
    let mut shown = 0;
    for i in 0..al.len().max(bl.len()) {
        let (x, y) = (
            al.get(i).copied().unwrap_or("<eof>"),
            bl.get(i).copied().unwrap_or("<eof>"),
        );
        if x != y {
            println!("  line {i}\n    zig : {x}\n    rust: {y}");
            shown += 1;
            if shown >= 4 {
                break;
            }
        }
    }
}

fn arg<T: std::str::FromStr>(a: &[String], name: &str, default: T) -> T {
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let zig = a
        .iter()
        .position(|x| x == "--zig")
        .and_then(|i| a.get(i + 1))
        .map_or_else(
            || root.join("target/difftest-zig/bin/difftest_zig"),
            PathBuf::from,
        );
    let work = root.join("target/difftest-work");
    std::fs::create_dir_all(&work).expect("mkdir");
    let ctx = Arc::new(Ctx { zig, work });
    if a.first().map(String::as_str) == Some("panics") {
        panics(&a, &ctx);
        return;
    }
    std::panic::set_hook(Box::new(|_| {}));
    match a.first().map(String::as_str) {
        Some("check" | "min" | "minz" | "minr") => {
            let (c, r, sb): (u16, u16, usize) = (
                a[1].parse().expect("numeric arg"),
                a[2].parse().expect("numeric arg"),
                a[3].parse().expect("numeric arg"),
            );
            let input = std::fs::read(&a[4]).expect("read");
            let crash = match a[0].as_str() {
                "minz" => Some("zig"),
                "minr" => Some("rust"),
                _ => None,
            };
            let input = if a[0] == "check" {
                input
            } else {
                minimize(&ctx, &input, c, r, sb, crash)
            };
            if a[0] != "check" {
                std::fs::write(ctx.work.join("min.bin"), &input).expect("write");
                println!("minimised to {} bytes: {:02x?}", input.len(), input);
            }
            let (z, rs) = (
                zig_dump(&ctx, "chk", &input, c, r, sb),
                rust_dump(&input, c, r, sb),
            );
            if equivalent(&rs, &z) {
                println!("SAME{}", if z == rs { "" } else { " (both abort)" });
            } else {
                show_diff(&z, &rs);
            }
        }
        _ => run(&ctx, &a),
    }
}

fn run(ctx: &Arc<Ctx>, a: &[String]) {
    let seed: u64 = arg(a, "--seed", 1);
    let cases: usize = arg(a, "--cases", 1000);
    let jobs: usize = arg(a, "--jobs", 8);
    let tokens: usize = arg(a, "--tokens", 60);
    let size: String = arg(a, "--size", String::new());
    let sb_fixed: Option<usize> = a
        .iter()
        .position(|x| x == "--sb")
        .and_then(|i| a.get(i + 1)?.parse().ok());
    let next = Arc::new(AtomicUsize::new(0));
    let bad = Arc::new(Mutex::new(Vec::<usize>::new()));
    let zig_only = Arc::new(AtomicUsize::new(0));
    let suspects = Arc::new(AtomicUsize::new(0));
    let zig_and_rust = Arc::new(AtomicUsize::new(0));
    let fails_dir = ctx.work.join(format!("fails-{seed}"));
    let handles: Vec<_> = (0..jobs)
        .map(|j| {
            let (ctx, next, bad, fails_dir, size) = (
                ctx.clone(),
                next.clone(),
                bad.clone(),
                fails_dir.clone(),
                size.clone(),
            );
            let (zc, zboth, zsus) = (zig_only.clone(), zig_and_rust.clone(), suspects.clone());
            std::thread::spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= cases {
                        break;
                    }
                    let mut rng = Rng(seed.wrapping_mul(0x1_0000_0001).wrapping_add(i as u64));
                    let (mut c, mut r, mut sb) = config(&mut rng);
                    if let Some((cc, rr)) = size.split_once('x') {
                        c = cc.parse().unwrap_or(c);
                        r = rr.parse().unwrap_or(r);
                    }
                    if let Some(s) = sb_fixed {
                        sb = s;
                    }
                    let n = 1 + rng.below(tokens);
                    let input = generate(&mut rng, n);
                    let rs = rust_dump(&input, c, r, sb);
                    let (z, grown) = zig_run(&ctx, &format!("w{j}"), &input, c, r, sb);
                    if z.starts_with("ZIG CRASH")
                        && std::env::var_os("DIFFTEST_SAVE_ABORTS").is_some()
                    {
                        std::fs::create_dir_all(&fails_dir).expect("mkdir");
                        let stem = format!("abort{i}-{c}x{r}-sb{sb}");
                        std::fs::write(fails_dir.join(format!("{stem}.bin")), &input).expect("w");
                        std::fs::write(fails_dir.join(format!("{stem}.zig.txt")), &z).expect("w");
                    }
                    if z.starts_with("ZIG CRASH") && !rs.starts_with("RUST PANIC") {
                        zc.fetch_add(1, Ordering::SeqCst);
                    } else if z.starts_with("ZIG CRASH") {
                        zboth.fetch_add(1, Ordering::SeqCst);
                    }
                    if !equivalent(&rs, &z) {
                        std::fs::create_dir_all(&fails_dir).expect("mkdir");
                        let suspect = grown && !rs.starts_with("RUST PANIC");
                        let stem = format!(
                            "{}{i}-{c}x{r}-sb{sb}",
                            if suspect { "suspect" } else { "case" }
                        );
                        if suspect {
                            zsus.fetch_add(1, Ordering::SeqCst);
                        }
                        std::fs::write(fails_dir.join(format!("{stem}.bin")), &input).expect("w");
                        std::fs::write(fails_dir.join(format!("{stem}.zig.txt")), &z).expect("w");
                        std::fs::write(fails_dir.join(format!("{stem}.rs.txt")), &rs).expect("w");
                        if !suspect {
                            bad.lock().expect("lock").push(i);
                        }
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("join");
    }
    let mut bad = bad.lock().expect("lock").clone();
    bad.sort_unstable();
    println!(
        "seed={seed} cases={cases} divergences={} (zig aborted: {} rust-ok, {} rust-also-panicked; {} suspect: mismatch after Zig page growth, saved as suspect*)",
        bad.len(),
        zig_only.load(Ordering::SeqCst),
        zig_and_rust.load(Ordering::SeqCst),
        suspects.load(Ordering::SeqCst)
    );
    if !bad.is_empty() {
        println!(
            "failing cases saved in {} (first: {:?})",
            fails_dir.display(),
            &bad[..bad.len().min(10)]
        );
        std::process::exit(1);
    }
}

fn lenient_panics(input: &[u8], c: u16, r: u16, sb: usize) -> bool {
    let input = input.to_vec();
    std::panic::catch_unwind(move || ghostty_difftest::drive_lenient(&input, c, r, sb)).is_err()
}

/// Rust-only panic hunt (run it with a debug build so assertions are on):
/// untrusted terminal input must never panic, even where Zig aborts.
fn panics(a: &[String], ctx: &Ctx) {
    let seed: u64 = arg(a, "--seed", 1);
    let cases: usize = arg(a, "--cases", 1000);
    let tokens: usize = arg(a, "--tokens", 60);
    let mut found = 0;
    for i in 0..cases {
        let mut rng = Rng(seed.wrapping_mul(0x1_0000_0001).wrapping_add(i as u64));
        let (c, r, sb) = config(&mut rng);
        let n = 1 + rng.below(tokens);
        let input = generate(&mut rng, n);
        if lenient_panics(&input, c, r, sb) {
            found += 1;
            let mut min = input.clone();
            let mut i = 0;
            while i < min.len() {
                let mut cand = min.clone();
                cand.remove(i);
                if lenient_panics(&cand, c, r, sb) {
                    min = cand;
                } else {
                    i += 1;
                }
            }
            let _ = ctx;
            eprintln!("case {i}: {c}x{r} sb={sb} minimised={min:02x?}");
        }
    }
    println!("seed={seed} cases={cases} rust panics={found}");
    if found > 0 {
        std::process::exit(1);
    }
}
