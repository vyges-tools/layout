//! vyges-layout CLI.
//!
//!   vyges-layout info    GDS [--json]
//!   vyges-layout boolean GDS --op and|or|not|xor --a L/D --b L/D --out L/D -o OUT.gds [--top C]
//!   vyges-layout flatten GDS --top CELL -o OUT.gds
//!   vyges-layout demo
//!
//! Common flags: -h/--help, -V/--version, -q/--quiet, -v/--verbose.
//! Exit codes: 0 ok · 1 runtime error · 2 usage.

use std::process::exit;

use vyges_layout::boolean::Op;
use vyges_layout::engine;
use vyges_layout::gds::Library;

const USAGE: &str = "\
vyges-layout — layout geometry kernel (GDSII read/write, boolean ops, flatten)

usage:
  vyges-layout info    GDS [--json]
  vyges-layout boolean GDS --op and|or|not|xor --a L/D --b L/D --out L/D -o OUT.gds [--top C]
  vyges-layout flatten GDS --top CELL -o OUT.gds
  vyges-layout demo

`L/D` is layer/datatype, e.g. 68/20. `boolean` operates on one cell's own shapes
(flatten first for hierarchy); v0 boolean is Manhattan (axis-aligned rectangles).

flags:
  --op OP        and | or | not (A−B) | xor
  --a L/D        first layer/datatype     --b L/D   second layer/datatype
  --out L/D      output layer/datatype    --top C   cell to operate on
  -o FILE        output GDS / report file
  --json         machine-readable output (info)
  -q/--quiet · -v/--verbose · -h/--help · -V/--version
  --bug-report · --feature-request · --sponsor · --star ⭐
";

const BUG_URL: &str =
    "https://github.com/vyges/community/issues/new?template=bug_report_template.yaml";
const FEATURE_URL: &str = "https://github.com/vyges/community/issues/new?labels=enhancement";
const SPONSOR_URL: &str = "https://github.com/sponsors/vyges-ip";
const STAR_URL: &str = "https://github.com/vyges-tools/layout";

fn link(label: &str, url: &str) {
    use std::io::IsTerminal;
    println!("{label}:\n  {url}");
    if std::io::stdout().is_terminal() {
        let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let _ = std::process::Command::new(opener).arg(url).status();
    }
}

struct Cli {
    pos: Vec<String>,
    opts: std::collections::HashMap<String, String>,
    flags: std::collections::HashSet<String>,
}

fn parse_cli(args: &[String]) -> Cli {
    let mut pos = Vec::new();
    let mut opts = std::collections::HashMap::new();
    let mut flags = std::collections::HashSet::new();
    let valued = ["--op", "--a", "--b", "--out", "--top", "-o"];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if valued.contains(&a.as_str()) {
            if let Some(v) = args.get(i + 1) {
                opts.insert(a.trim_start_matches('-').to_string(), v.clone());
                i += 1;
            }
        } else if a.starts_with('-') {
            flags.insert(a.trim_start_matches('-').to_string());
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    Cli { pos, opts, flags }
}

fn parse_ld(s: &str) -> Option<(i16, i16)> {
    let (l, d) = s.split_once('/').unwrap_or((s, "0"));
    Some((l.trim().parse().ok()?, d.trim().parse().ok()?))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = parse_cli(&args);
    let has = |f: &str| cli.flags.contains(f);

    if has("bug-report") {
        return link("Report a bug (central — vyges/community)", BUG_URL);
    }
    if has("feature-request") {
        return link("Request a feature (central — vyges/community)", FEATURE_URL);
    }
    if has("sponsor") {
        return link("Sponsor Vyges", SPONSOR_URL);
    }
    if has("star") {
        return link("Star vyges-layout on GitHub ⭐", STAR_URL);
    }
    if has("V") || has("version") {
        println!("vyges-layout {} ({})", vyges_layout::VERSION, env!("VYGES_GIT_SHA"));
        println!("{}", vyges_layout::COPYRIGHT);
        return;
    }
    let cmd = cli.pos.first().cloned().unwrap_or_default();
    if has("h") || has("help") || cmd.is_empty() {
        print!("{USAGE}");
        exit(if cmd.is_empty() && !has("help") && !has("h") { 2 } else { 0 });
    }
    let quiet = has("q") || has("quiet");

    match cmd.as_str() {
        "demo" => print!("{}", engine::demo()),
        "info" => {
            let Some(path) = cli.pos.get(1) else {
                eprintln!("usage: vyges-layout info GDS");
                exit(2);
            };
            let lib = match Library::load(path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            };
            let out = if has("json") { engine::info_json(&lib) } else { engine::info(&lib) };
            match cli.opts.get("o") {
                Some(p) => std::fs::write(p, &out).unwrap_or_else(|e| {
                    eprintln!("error: {p}: {e}");
                    exit(1);
                }),
                None => print!("{out}"),
            }
        }
        "boolean" => {
            let (Some(path), Some(opn), Some(a), Some(b), Some(out_ld), Some(o)) = (
                cli.pos.get(1),
                cli.opts.get("op"),
                cli.opts.get("a"),
                cli.opts.get("b"),
                cli.opts.get("out"),
                cli.opts.get("o"),
            ) else {
                eprintln!("usage: vyges-layout boolean GDS --op OP --a L/D --b L/D --out L/D -o OUT.gds");
                exit(2);
            };
            let (Some(op), Some(la), Some(lb), Some(lo)) =
                (Op::parse(opn), parse_ld(a), parse_ld(b), parse_ld(out_ld))
            else {
                eprintln!("error: bad --op or L/D");
                exit(2);
            };
            match engine::run_boolean(path, cli.opts.get("top").map(|s| s.as_str()), la, lb, lo, op, o) {
                Ok(s) => {
                    if !quiet {
                        println!(
                            "{:?}: A {} rect, B {} rect -> {} rect (area {} dbu²){}; wrote {}",
                            op,
                            s.a,
                            s.b,
                            s.out,
                            s.out_area,
                            if s.approx > 0 { format!(", {} bbox-approx", s.approx) } else { String::new() },
                            o
                        );
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        "flatten" => {
            let (Some(path), Some(top), Some(o)) = (cli.pos.get(1), cli.opts.get("top"), cli.opts.get("o"))
            else {
                eprintln!("usage: vyges-layout flatten GDS --top CELL -o OUT.gds");
                exit(2);
            };
            match engine::run_flatten(path, top, o) {
                Ok(n) => {
                    if !quiet {
                        println!("flattened {top} -> {n} element(s); wrote {o}");
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    exit(1);
                }
            }
        }
        other => {
            eprintln!("vyges-layout: unknown command {other:?}\n");
            print!("{USAGE}");
            exit(2);
        }
    }
}
