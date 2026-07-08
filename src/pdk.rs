//! PDK collateral resolution — the engine-side bridge to the `pdk-store` adapter.
//!
//! Engines name a PDK (`--pdk sky130a`) and ask for a collateral key
//! (`extract_rules`, `drc_deck`, `lvs_deck`, `primitives_spice`, `lib`, …); this
//! shells to the installed `vyges-pdk-store resolve` and returns a concrete path.
//!
//! Vyges-supplied collateral (e.g. `extract_rules`) lives under **`$VYGES_PLUGIN`**.
//! When that is not set we fall back to `$VYGES_HOME/plugin`, then `~/.vyges/plugin`,
//! and pass the chosen root into the resolver's environment so the descriptor's
//! `$VYGES_PLUGIN/…` path expands. Failures return a detailed message (unknown PDK,
//! unexpanded variable, or a resolved path that does not exist) so the caller can
//! print it verbatim.

use std::path::Path;
use std::process::Command;

/// The plugin-collateral root and whether it was set explicitly (`VYGES_PLUGIN`)
/// vs. defaulted. Order: `$VYGES_PLUGIN` -> `$VYGES_HOME/plugin` -> `~/.vyges/plugin`.
fn plugin_root() -> Option<(String, bool)> {
    let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let Some(p) = nonempty("VYGES_PLUGIN") {
        return Some((p, true));
    }
    if let Some(h) = nonempty("VYGES_HOME") {
        return Some((format!("{h}/plugin"), false));
    }
    if let Some(home) = nonempty("HOME") {
        return Some((format!("{home}/.vyges/plugin"), false));
    }
    None
}

/// Locate the `vyges-pdk-store` binary: prefer the sibling next to the current
/// executable (installed together), else rely on PATH.
fn resolver_prog() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("vyges-pdk-store")))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vyges-pdk-store".into())
}

/// Resolve `key` for PDK `name` (optionally at `corner`) to a concrete path.
///
/// On failure returns a detailed message: the resolver's own text for an unknown
/// PDK / key, a note when the result still holds an unexpanded `$VAR`, or that the
/// resolved path does not exist (with the `VYGES_PLUGIN` root that was used). A
/// warning is emitted to stderr when a defaulted plugin root was actually consumed.
pub fn resolve(name: &str, key: &str, corner: Option<&str>) -> Result<String, String> {
    let root = plugin_root();
    let mut cmd = Command::new(resolver_prog());
    cmd.args(["resolve", name, key]);
    if let Some(c) = corner {
        cmd.args(["--corner", c]);
    }
    if let Some((ref p, _)) = root {
        cmd.env("VYGES_PLUGIN", p);
    }
    let out = cmd.output().map_err(|e| format!("vyges-pdk-store not runnable: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("error:").trim().to_string();
        return Err(if err.is_empty() { format!("could not resolve {key} for PDK {name:?}") } else { err });
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return Err(format!("pdk-store returned no path for {key} of PDK {name:?}"));
    }
    if path.contains('$') {
        return Err(format!(
            "{key} for PDK {name:?} resolved to {path} — an environment variable is unset. \
             Set it (e.g. VYGES_PLUGIN for Vyges collateral, PDK_ROOT for foundry collateral)."
        ));
    }
    if !Path::new(&path).exists() {
        let plug = root.as_ref().map(|(p, _)| p.as_str()).unwrap_or("(unset)");
        return Err(format!(
            "{key} for PDK {name:?} resolves to {path}, which does not exist \
             (VYGES_PLUGIN={plug}) — provision the collateral there or set VYGES_PLUGIN/VYGES_HOME."
        ));
    }
    // The path exists. If a *defaulted* plugin root supplied it, note that we guessed.
    if let Some((p, false)) = &root {
        if path.starts_with(p.as_str()) {
            eprintln!("warning: VYGES_PLUGIN not set; using default plugin root {p}");
        }
    }
    Ok(path)
}
