//! Drive the KiCad Routing Tools CLI (`py_router/route.py`) and reload the
//! open board. This process does not parse `.kicad_pcb`; the upstream plugin
//! writes the file. Undo for this step is gone (disk reload, not BeginCommit).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::kicad::Kicad;

const PLUGIN_ID: &str = "com.github.drandyhaas.kicadroutingtools";
const CLI_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Serialize)]
pub struct AutorouteResult {
    pub ok: bool,
    pub nets: Vec<String>,
    pub warnings: Vec<String>,
    pub routed: Vec<String>,
    pub failed: Vec<String>,
    pub elapsed_s: f64,
    pub track_count: usize,
    pub via_count: usize,
    pub reloaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_tail: Option<String>,
}

#[derive(Debug)]
pub struct PreparedNets {
    pub nets: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn prepare_nets(raw: &[String]) -> Result<PreparedNets, String> {
    let mut nets = Vec::new();
    for n in raw {
        let t = n.trim();
        if t.is_empty() {
            continue;
        }
        if t == "*" {
            return Err("autoroute_nets refuses '*' — name the nets to route".into());
        }
        nets.push(t.to_string());
    }
    if nets.is_empty() {
        return Err("autoroute_nets needs nets: [\"EN\", \"BOOT\", …] — never all nets".into());
    }
    for n in &nets {
        if blocked_power(n) {
            return Err(format!(
                "refusing {n}: pour GND (and VSS) as a copper zone, do not autoroute it"
            ));
        }
    }
    let mut warnings = Vec::new();
    let has_dn = nets.iter().any(|n| n.eq_ignore_ascii_case("USB_DN"));
    let has_dp = nets.iter().any(|n| n.eq_ignore_ascii_case("USB_DP"));
    if has_dn ^ has_dp {
        return Err(
            "USB_DN and USB_DP are a pair — pass both, or route them by hand. \
             v1 does not call route_diff.py"
                .into(),
        );
    }
    if has_dn && has_dp {
        warnings.push(
            "USB_DN/USB_DP will be routed as two single-ended nets, not a matched pair"
                .into(),
        );
    }
    Ok(PreparedNets { nets, warnings })
}

fn blocked_power(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    u == "GND" || u == "VSS" || u.starts_with("GND")
}

pub fn find_plugin_root() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("KICAD_ROUTING_TOOLS_ROOT") {
        let path = PathBuf::from(p);
        if path.join("py_router/route.py").is_file() {
            return Ok(path);
        }
        return Err(format!(
            "KICAD_ROUTING_TOOLS_ROOT has no py_router/route.py: {}",
            path.display()
        ));
    }
    let home = dirs_home()?;
    let dest = home
        .join(".local/share/kicad/10.0/3rdparty/plugins")
        .join(PLUGIN_ID);
    if dest.join("py_router/route.py").is_file() {
        return Ok(dest);
    }
    Err(
        "KiCad Routing Tools not installed for this user. Run kicad-routing-tools-setup \
         (from the companion .deb), then restart KiCad 10 via kicad-10."
            .into(),
    )
}

pub fn find_wheels_dir(plugin: &Path) -> Result<PathBuf, String> {
    if let Some(third) = plugin.parent().and_then(|p| p.parent()) {
        let wheels = third.join("python");
        if wheels.join("numpy").is_dir() {
            return Ok(wheels);
        }
    }
    let home = dirs_home()?;
    let wheels = home.join(".local/share/kicad/10.0/3rdparty/python");
    if wheels.join("numpy").is_dir() {
        return Ok(wheels);
    }
    Err(
        "numpy/scipy/shapely missing under ~/.local/share/kicad/10.0/3rdparty/python. \
         Run kicad-routing-tools-setup (do not pip from inside KiCad)."
            .into(),
    )
}

pub fn find_appimage_python() -> Result<(PathBuf, PathBuf), String> {
    let mut mounts = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/tmp") {
        for e in rd.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n.starts_with(".mount_kicad") && !n.ends_with(".pid") {
                mounts.push(e.path());
            }
        }
    }
    mounts.sort();
    for mount in mounts.into_iter().rev() {
        let py = mount.join("bin/python3.11");
        let home = mount.join("shared");
        if py.is_file() && home.is_dir() {
            return Ok((py, home));
        }
    }
    Err(
        "KiCad 10 AppImage Python 3.11 not found under /tmp/.mount_kicad*. \
         Start KiCad with kicad-10 (not the .AppImage, not system 9)."
            .into(),
    )
}

fn dirs_home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set".into())
}

pub async fn autoroute_nets(k: &Kicad, raw_nets: &[String]) -> Result<AutorouteResult, String> {
    let prepared = prepare_nets(raw_nets)?;
    let plugin = find_plugin_root()?;
    let wheels = find_wheels_dir(&plugin)?;
    let (python, python_home) = find_appimage_python()?;
    let board = k.board_file_path().await?;
    if !board.is_file() {
        return Err(format!("board file not on disk: {}", board.display()));
    }

    k.save().await?;
    let before_tracks = k.tracks().await?.len();
    let before_vias = k.vias().await?.len();

    let started = Instant::now();
    let (stdout, stderr) = run_route_cli(
        &python,
        &python_home,
        &plugin,
        &wheels,
        &board,
        &prepared.nets,
    )
    .await?;
    let elapsed_s = started.elapsed().as_secs_f64();
    let (routed, failed) = parse_cli_summary(&stdout);

    let reloaded = reload_from_disk(k, before_tracks).await?;
    let after = k.summary().await?;
    let log_tail = tail(&format!("{stdout}\n{stderr}"), 2500);

    let mut note = None;
    if !reloaded {
        note = Some(
            "CLI wrote the file but KiCad did not reload. File → Revert (or close and reopen the board)."
                .into(),
        );
    } else if after.track_count == before_tracks && after.via_count == before_vias && failed.is_empty()
    {
        note = Some(
            "Reload ran but track/via counts are unchanged — the CLI may have written no new copper."
                .into(),
        );
    }

    Ok(AutorouteResult {
        ok: failed.is_empty(),
        nets: prepared.nets,
        warnings: prepared.warnings,
        routed,
        failed,
        elapsed_s: (elapsed_s * 100.0).round() / 100.0,
        track_count: after.track_count,
        via_count: after.via_count,
        reloaded,
        note,
        log_tail: Some(log_tail),
    })
}

async fn run_route_cli(
    python: &Path,
    python_home: &Path,
    plugin: &Path,
    wheels: &Path,
    board: &Path,
    nets: &[String],
) -> Result<(String, String), String> {
    let python = python.to_path_buf();
    let python_home = python_home.to_path_buf();
    let plugin = plugin.to_path_buf();
    let wheels = wheels.to_path_buf();
    let board = board.to_path_buf();
    let nets = nets.to_vec();
    let out = tokio::task::spawn_blocking(move || run_route_cli_blocking(&python, &python_home, &plugin, &wheels, &board, &nets))
        .await
        .map_err(|e| format!("autoroute worker join: {e}"))??;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        return Err(format!(
            "route.py exited {}: {}",
            out.status,
            tail(&format!("{stdout}\n{stderr}"), 2000)
        ));
    }
    Ok((stdout, stderr))
}

fn run_route_cli_blocking(
    python: &Path,
    python_home: &Path,
    plugin: &Path,
    wheels: &Path,
    board: &Path,
    nets: &[String],
) -> Result<std::process::Output, String> {
    let mut cmd = Command::new(python);
    cmd.env("PYTHONHOME", python_home)
        .env_remove("PYTHONPATH")
        .arg("-c")
        .arg(wrapper_source())
        .arg(wheels)
        .arg(plugin)
        .arg(plugin.join("py_router/route.py"))
        .arg(board)
        .arg("--overwrite")
        .arg("--nets");
    for n in nets {
        cmd.arg(n);
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start AppImage python: {e}"))?;
    let start = Instant::now();
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("route.py wait: {e}"))?
        {
            Some(_) => {
                return child
                    .wait_with_output()
                    .map_err(|e| format!("route.py output: {e}"));
            }
            None if start.elapsed() > CLI_TIMEOUT => {
                let _ = child.kill();
                return Err("autoroute_nets timed out after 300s".into());
            }
            None => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn wrapper_source() -> &'static str {
    r#"
import runpy, sys
wheels, plugin, route_py = sys.argv[1], sys.argv[2], sys.argv[3]
board = sys.argv[4]
cli = sys.argv[5:]
for p in (wheels, plugin, plugin + "/py_router", plugin + "/rust_router"):
    if p not in sys.path:
        sys.path.insert(0, p)
sys.argv = [route_py, board, *cli]
runpy.run_path(route_py, run_name="__main__")
"#
}

fn parse_cli_summary(stdout: &str) -> (Vec<String>, Vec<String>) {
    for line in stdout.lines().rev() {
        let Some(rest) = line.strip_prefix("JSON_SUMMARY: ") else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) else {
            continue;
        };
        let routed = string_list(&v, "routed_single");
        let failed = string_list(&v, "failed_single");
        return (routed, failed);
    }
    (Vec::new(), Vec::new())
}

fn string_list(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.trim().to_string()
    } else {
        s[s.len() - max..].trim().to_string()
    }
}

/// Reload the open editor from disk. After Save the board is clean, so
/// RevertDocument is often a no-op; a dummy track dirties it first.
async fn reload_from_disk(k: &Kicad, tracks_before_cli: usize) -> Result<bool, String> {
    let _ = k.revert_document().await;
    let _ = k.refresh().await;
    if k.tracks().await?.len() != tracks_before_cli {
        return Ok(true);
    }

    let dummy = crate::copper::track_any(0.0, 0.0, 0.4, 0.0, Some(0.2), 3, "GND")?;
    let session = k.begin_commit().await?;
    match k.create_items(vec![dummy]).await {
        Ok(_) => {
            let _ = k
                .end_commit(session, "kicad-mcp dirty so RevertDocument reloads")
                .await;
        }
        Err(e) => {
            let _ = k.drop_commit(session).await;
            return Err(format!("could not dirty the board for reload: {e}"));
        }
    }
    k.revert_document().await?;
    let _ = k.run_action("pcbnew.Refresh").await;
    let _ = k.refresh().await;
    Ok(k.tracks().await?.len() != tracks_before_cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_empty_and_star() {
        assert!(prepare_nets(&[]).is_err());
        assert!(prepare_nets(&["".into()]).is_err());
        assert!(prepare_nets(&["*".into()]).is_err());
    }

    #[test]
    fn refuses_gnd() {
        assert!(prepare_nets(&["GND".into()]).is_err());
        assert!(prepare_nets(&["gnd".into()]).is_err());
        assert!(prepare_nets(&["GNDA".into()]).is_err());
        assert!(prepare_nets(&["VSS".into()]).is_err());
    }

    #[test]
    fn allows_5v_when_named() {
        let p = prepare_nets(&["5V".into(), "EN".into()]).unwrap();
        assert_eq!(p.nets, ["5V", "EN"]);
    }

    #[test]
    fn usb_must_be_both_or_neither() {
        assert!(prepare_nets(&["USB_DN".into()]).is_err());
        assert!(prepare_nets(&["USB_DP".into()]).is_err());
        let p = prepare_nets(&["USB_DN".into(), "USB_DP".into()]).unwrap();
        assert_eq!(p.warnings.len(), 1);
    }
}
