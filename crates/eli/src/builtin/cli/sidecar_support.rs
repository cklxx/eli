#[cfg(feature = "gateway")]
use std::path::{Path, PathBuf};

#[cfg(feature = "gateway")]
use serde_json::Value;

#[cfg(feature = "gateway")]
pub(crate) fn find_sidecar_dir() -> Option<PathBuf> {
    [
        std::env::var("ELI_SIDECAR_DIR").ok().map(PathBuf::from),
        installed_sidecar_dir(),
        std::env::current_dir().ok().map(|dir| dir.join("sidecar")),
    ]
    .into_iter()
    .flatten()
    .find(|path| sidecar_exists(path))
}

#[cfg(feature = "gateway")]
pub(crate) fn ensure_node_available() -> bool {
    let status = std::process::Command::new("node")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.is_ok()
}

#[cfg(feature = "gateway")]
pub(crate) fn ensure_sidecar_deps(sidecar_dir: &Path, refresh: bool) -> bool {
    if !refresh && sidecar_dir.join("node_modules").exists() {
        return true;
    }
    if refresh && deps_are_current(sidecar_dir).unwrap_or(false) {
        return true;
    }
    run_npm_install(sidecar_dir)
}

#[cfg(feature = "gateway")]
fn installed_sidecar_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.join("sidecar")))
}

#[cfg(feature = "gateway")]
fn sidecar_exists(path: &Path) -> bool {
    path.join("start.cjs").exists()
}

#[cfg(feature = "gateway")]
fn run_npm_install(sidecar_dir: &Path) -> bool {
    std::process::Command::new("npm")
        .arg("install")
        .current_dir(sidecar_dir)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(feature = "gateway")]
fn deps_are_current(sidecar_dir: &Path) -> Option<bool> {
    let openclaw = expected_dep_version(sidecar_dir, "openclaw")?;
    let weixin = expected_dep_version(sidecar_dir, "@tencent-weixin/openclaw-weixin")?;
    Some(
        installed_dep_version(sidecar_dir, "openclaw")? == openclaw
            && installed_dep_version(sidecar_dir, "@tencent-weixin/openclaw-weixin")? == weixin,
    )
}

#[cfg(feature = "gateway")]
fn expected_dep_version(sidecar_dir: &Path, package: &str) -> Option<String> {
    let package_json = read_package_json(&sidecar_dir.join("package.json"))?;
    package_json["dependencies"][package]
        .as_str()
        .map(str::to_owned)
}

#[cfg(feature = "gateway")]
fn installed_dep_version(sidecar_dir: &Path, package: &str) -> Option<String> {
    let path = package_json_path(&sidecar_dir.join("node_modules"), package);
    read_package_json(&path)?["version"]
        .as_str()
        .map(str::to_owned)
}

#[cfg(feature = "gateway")]
fn read_package_json(path: &Path) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[cfg(feature = "gateway")]
fn package_json_path(base_dir: &Path, package: &str) -> PathBuf {
    package
        .split('/')
        .fold(base_dir.to_path_buf(), |path, part| path.join(part))
        .join("package.json")
}
