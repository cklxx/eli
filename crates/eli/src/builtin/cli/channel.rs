use std::fs;
use std::net::TcpListener;
use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ChannelAction;

const SIDECAR_ELI_URL: &str = "http://127.0.0.1:3100";
const WEIXIN_ALIAS: &str = "weixin";
const WEIXIN_CHANNEL_ID: &str = "openclaw-weixin";
const WEIXIN_PLUGIN: &str = "@tencent-weixin/openclaw-weixin";

#[cfg(feature = "gateway")]
use super::sidecar_support::{ensure_node_available, ensure_sidecar_deps, find_sidecar_dir};

#[derive(Debug, Deserialize)]
struct QrStartResponse {
    message: String,
    #[serde(rename = "qrDataUrl")]
    qr_data_url: Option<String>,
    #[serde(rename = "sessionKey")]
    session_key: String,
}

#[derive(Debug, Deserialize)]
struct QrWaitResponse {
    connected: bool,
    message: String,
    #[serde(rename = "accountId")]
    account_id: Option<String>,
}

pub(crate) async fn channel_command(action: ChannelAction) -> anyhow::Result<()> {
    #[cfg(not(feature = "gateway"))]
    {
        let _ = action;
        bail!("Channel commands require the `gateway` feature.");
    }
    #[cfg(feature = "gateway")]
    {
        match action {
            ChannelAction::Login { channel } => login_channel(channel).await,
        }
    }
}

#[cfg(feature = "gateway")]
async fn login_channel(channel: String) -> anyhow::Result<()> {
    let channel_id = normalize_channel_id(&channel)?;
    let sidecar_dir = require_sidecar_dir()?;
    require_node()?;
    require_sidecar_deps(&sidecar_dir)?;
    ensure_sidecar_config(&sidecar_dir, channel_id)?;
    let port = allocate_port()?;
    let _sidecar = spawn_sidecar(&sidecar_dir, port)?;
    wait_for_sidecar(port).await?;
    let client = reqwest::Client::new();
    let start = start_qr_login(&client, port, channel_id).await?;
    display_qr(&sidecar_dir, &start)?;
    report_login(wait_qr_login(&client, port, channel_id, &start.session_key).await?)
}

#[cfg(feature = "gateway")]
fn normalize_channel_id(channel: &str) -> anyhow::Result<&'static str> {
    match channel.trim().to_ascii_lowercase().as_str() {
        WEIXIN_ALIAS | "wechat" | WEIXIN_CHANNEL_ID => Ok(WEIXIN_CHANNEL_ID),
        other => bail!("Unsupported channel: {other}. Supported channels: weixin"),
    }
}

#[cfg(not(feature = "gateway"))]
fn normalize_channel_id(channel: &str) -> anyhow::Result<&'static str> {
    let _ = channel;
    bail!("Channel commands require the `gateway` feature.")
}

#[cfg(feature = "gateway")]
fn require_sidecar_dir() -> anyhow::Result<std::path::PathBuf> {
    find_sidecar_dir().context(
        "Sidecar directory not found. Run `eli` from the repo root or set `ELI_SIDECAR_DIR`.",
    )
}

#[cfg(feature = "gateway")]
fn require_node() -> anyhow::Result<()> {
    if ensure_node_available() {
        return Ok(());
    }
    bail!("`node` not found in PATH. Install Node.js to use channel login.")
}

#[cfg(feature = "gateway")]
fn require_sidecar_deps(sidecar_dir: &Path) -> anyhow::Result<()> {
    if ensure_sidecar_deps(sidecar_dir, true) {
        return Ok(());
    }
    bail!("`npm install` failed in {}", sidecar_dir.display())
}

#[cfg(feature = "gateway")]
fn ensure_sidecar_config(sidecar_dir: &Path, channel_id: &str) -> anyhow::Result<()> {
    let config_path = sidecar_dir.join("sidecar.json");
    let mut root = load_config_value(&config_path)?;
    let changed = ensure_plugin_entry(&mut root)? | ensure_channel_entry(&mut root, channel_id)?;
    if changed {
        write_config_value(&config_path, &root)?;
    }
    Ok(())
}

#[cfg(feature = "gateway")]
fn load_config_value(path: &Path) -> anyhow::Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in {}", path.display()))
}

#[cfg(feature = "gateway")]
fn write_config_value(path: &Path, value: &Value) -> anyhow::Result<()> {
    let rendered = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{rendered}\n"))
        .with_context(|| format!("Failed to write {}", path.display()))
}

#[cfg(feature = "gateway")]
fn ensure_plugin_entry(root: &mut Value) -> anyhow::Result<bool> {
    let plugins = root_object(root)?
        .entry("plugins")
        .or_insert_with(|| Value::Array(Vec::new()));
    let list = plugins
        .as_array_mut()
        .context("`plugins` must be an array in sidecar.json")?;
    if list
        .iter()
        .any(|value| value.as_str() == Some(WEIXIN_PLUGIN))
    {
        return Ok(false);
    }
    list.push(Value::String(WEIXIN_PLUGIN.to_owned()));
    Ok(true)
}

#[cfg(feature = "gateway")]
fn ensure_channel_entry(root: &mut Value, channel_id: &str) -> anyhow::Result<bool> {
    let channels = root_object(root)?
        .entry("channels")
        .or_insert_with(|| json!({}));
    let map = channels
        .as_object_mut()
        .context("`channels` must be an object in sidecar.json")?;
    let mut changed = insert_channel_alias(map, channel_id);
    changed |= ensure_accounts_field(
        map.get_mut(channel_id)
            .context("channel missing after insert")?,
    )?;
    Ok(changed)
}

#[cfg(feature = "gateway")]
fn insert_channel_alias(map: &mut serde_json::Map<String, Value>, channel_id: &str) -> bool {
    if map.contains_key(channel_id) {
        return false;
    }
    let value = map
        .get(WEIXIN_ALIAS)
        .cloned()
        .unwrap_or_else(default_channel_config);
    map.insert(channel_id.to_owned(), value);
    true
}

#[cfg(feature = "gateway")]
fn ensure_accounts_field(value: &mut Value) -> anyhow::Result<bool> {
    let object = value
        .as_object_mut()
        .context("channel config must be an object in sidecar.json")?;
    if object.get("accounts").is_some() {
        return Ok(false);
    }
    object.insert("accounts".to_owned(), json!({}));
    Ok(true)
}

#[cfg(feature = "gateway")]
fn default_channel_config() -> Value {
    json!({ "accounts": {} })
}

#[cfg(feature = "gateway")]
fn root_object(root: &mut Value) -> anyhow::Result<&mut serde_json::Map<String, Value>> {
    root.as_object_mut()
        .context("sidecar.json must contain a top-level object")
}

#[cfg(feature = "gateway")]
fn allocate_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

#[cfg(feature = "gateway")]
fn spawn_sidecar(sidecar_dir: &Path, port: u16) -> anyhow::Result<SidecarProcess> {
    let child = std::process::Command::new("node")
        .arg("start.cjs")
        .current_dir(sidecar_dir)
        .env("SIDECAR_PORT", port.to_string())
        .env("SIDECAR_ELI_URL", SIDECAR_ELI_URL)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(SidecarProcess(child))
}

#[cfg(feature = "gateway")]
struct SidecarProcess(std::process::Child);

#[cfg(feature = "gateway")]
impl Drop for SidecarProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(feature = "gateway")]
async fn wait_for_sidecar(port: u16) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let url = sidecar_url(port);
    for _ in 0..20 {
        if sidecar_ready(&client, &url).await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    bail!("sidecar not reachable at {url}")
}

#[cfg(feature = "gateway")]
fn sidecar_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

#[cfg(feature = "gateway")]
async fn sidecar_ready(client: &reqwest::Client, base_url: &str) -> bool {
    client
        .get(format!("{base_url}/health"))
        .send()
        .await
        .is_ok_and(|resp| resp.status().is_success())
}

#[cfg(feature = "gateway")]
async fn start_qr_login(
    client: &reqwest::Client,
    port: u16,
    channel_id: &str,
) -> anyhow::Result<QrStartResponse> {
    let response = client
        .post(format!("{}/setup/{channel_id}/start", sidecar_url(port)))
        .json(&json!({}))
        .send()
        .await?;
    parse_json(response).await
}

#[cfg(feature = "gateway")]
async fn wait_qr_login(
    client: &reqwest::Client,
    port: u16,
    channel_id: &str,
    session_key: &str,
) -> anyhow::Result<QrWaitResponse> {
    let response = client
        .post(format!("{}/setup/{channel_id}/wait", sidecar_url(port)))
        .json(&json!({ "sessionKey": session_key }))
        .send()
        .await?;
    parse_json(response).await
}

#[cfg(feature = "gateway")]
async fn parse_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!("sidecar request failed (HTTP {status}): {body}");
    }
    serde_json::from_str(&body).context("Invalid JSON from sidecar")
}

#[cfg(feature = "gateway")]
fn display_qr(sidecar_dir: &Path, start: &QrStartResponse) -> anyhow::Result<()> {
    let qr_url = start
        .qr_data_url
        .as_deref()
        .context(start.message.clone())?;
    println!("使用微信扫描以下二维码：\n");
    let _ = render_qr(sidecar_dir, qr_url);
    println!("如果二维码未能成功展示，请用浏览器打开以下链接扫码：\n{qr_url}\n");
    Ok(())
}

#[cfg(feature = "gateway")]
fn render_qr(sidecar_dir: &Path, qr_url: &str) -> anyhow::Result<()> {
    let script = "const qr=require('qrcode-terminal');qr.generate(process.argv[1],{small:true},q=>console.log(q));";
    std::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(qr_url)
        .current_dir(sidecar_dir)
        .status()
        .context("failed to render QR code in terminal")?;
    Ok(())
}

#[cfg(feature = "gateway")]
fn report_login(result: QrWaitResponse) -> anyhow::Result<()> {
    if !result.connected {
        bail!("微信登录失败: {}", result.message);
    }
    match result.account_id {
        Some(account_id) => println!("微信登录成功。account: {account_id}"),
        None => println!("微信登录成功。"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_weixin_aliases() {
        assert_eq!(normalize_channel_id("weixin").unwrap(), WEIXIN_CHANNEL_ID);
        assert_eq!(normalize_channel_id("wechat").unwrap(), WEIXIN_CHANNEL_ID);
    }

    #[test]
    fn ensure_sidecar_config_bootstraps_weixin() {
        let dir = tempfile::tempdir().unwrap();
        ensure_sidecar_config(dir.path(), WEIXIN_CHANNEL_ID).unwrap();
        let config = load_config_value(&dir.path().join("sidecar.json")).unwrap();
        assert!(
            config["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == WEIXIN_PLUGIN)
        );
        assert!(config["channels"][WEIXIN_CHANNEL_ID]["accounts"].is_object());
    }

    #[test]
    fn ensure_sidecar_config_preserves_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sidecar.json");
        let config =
            json!({"plugins":["@larksuite/openclaw-lark"],"channels":{"weixin":{"enabled":true}}});
        write_config_value(&path, &config).unwrap();
        ensure_sidecar_config(dir.path(), WEIXIN_CHANNEL_ID).unwrap();
        let updated = load_config_value(&path).unwrap();
        assert!(
            updated["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "@larksuite/openclaw-lark")
        );
        assert_eq!(updated["channels"]["weixin"]["enabled"], true);
        assert_eq!(updated["channels"][WEIXIN_CHANNEL_ID]["enabled"], true);
    }
}
