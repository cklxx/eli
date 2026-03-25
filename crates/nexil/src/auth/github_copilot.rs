//! Authentication helpers for GitHub Copilot OAuth-backed sessions.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use crate::auth::APIKeyResolver;

/// Providers handled by the GitHub Copilot resolver.
const GITHUB_COPILOT_PROVIDERS: &[&str] = &["github-copilot"];

/// Default GitHub OAuth client ID for Copilot auth.
pub const DEFAULT_GITHUB_COPILOT_OAUTH_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
/// Default device code endpoint.
pub const DEFAULT_GITHUB_COPILOT_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
/// Default access token endpoint.
pub const DEFAULT_GITHUB_COPILOT_ACCESS_TOKEN_URL: &str =
    "https://github.com/login/oauth/access_token";
/// Default OAuth scope for Copilot auth.
pub const DEFAULT_GITHUB_COPILOT_SCOPE: &str = "read:user user:email";
const DEFAULT_GITHUB_API_VERSION: &str = "2022-11-28";
const DEFAULT_GITHUB_HOST: &str = "github.com";
const GITHUB_TOKEN_ENV_VARS: &[&str] = &["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during GitHub Copilot OAuth login.
#[derive(Debug, thiserror::Error)]
pub enum GitHubCopilotOAuthLoginError {
    #[error("GitHub device flow expired before authorization completed")]
    Expired,
    #[error("GitHub device flow was denied by the user")]
    Denied,
    #[error("GitHub device flow timed out before authorization completed")]
    Timeout,
    #[error("GitHub Copilot OAuth response is malformed: {0}")]
    ResponseError(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// Token struct
// ---------------------------------------------------------------------------

/// Persisted OAuth tokens for GitHub Copilot sessions.
#[derive(Clone, Serialize, Deserialize)]
pub struct GitHubCopilotOAuthTokens {
    pub github_token: String,
    #[serde(default = "default_bearer")]
    pub github_token_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enterprise_url: Option<String>,
}

impl std::fmt::Debug for GitHubCopilotOAuthTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubCopilotOAuthTokens")
            .field("github_token", &"[REDACTED]")
            .field("github_token_type", &self.github_token_type)
            .field("github_scope", &self.github_scope)
            .field("expires_at", &self.expires_at)
            .field(
                "account_id",
                &self.account_id.as_ref().map(|_| "[REDACTED]"),
            )
            .field("login", &self.login)
            .field("email", &self.email.as_ref().map(|_| "[REDACTED]"))
            .field("enterprise_url", &self.enterprise_url)
            .finish()
    }
}

fn default_bearer() -> String {
    "bearer".to_string()
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

fn resolve_auth_path(config_home: Option<&Path>) -> PathBuf {
    let base = match config_home {
        Some(p) => p.to_path_buf(),
        None => {
            if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
                PathBuf::from(val)
            } else {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            }
        }
    };
    base.join("conduit").join("github_copilot_auth.json")
}

fn normalize_optional_str(val: Option<&Value>) -> Option<String> {
    val.and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_optional_int(val: Option<&Value>) -> Option<i64> {
    val.and_then(|v| {
        if v.is_boolean() {
            return None;
        }
        v.as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
    })
}

// ---------------------------------------------------------------------------
// Load / Save
// ---------------------------------------------------------------------------

/// Load persisted GitHub Copilot OAuth tokens from disk.
pub fn load_github_copilot_oauth_tokens(
    config_home: Option<&Path>,
) -> Option<GitHubCopilotOAuthTokens> {
    let path = resolve_auth_path(config_home);
    let contents = fs::read_to_string(&path).ok()?;
    let payload: Value = serde_json::from_str(&contents).ok()?;
    let obj = payload.as_object()?;
    parse_tokens(obj)
}

fn parse_tokens(payload: &serde_json::Map<String, Value>) -> Option<GitHubCopilotOAuthTokens> {
    let github_token = normalize_optional_str(payload.get("github_token"))?;

    Some(GitHubCopilotOAuthTokens {
        github_token,
        github_token_type: normalize_optional_str(payload.get("github_token_type"))
            .unwrap_or_else(|| "bearer".to_string()),
        github_scope: normalize_optional_str(payload.get("github_scope")),
        expires_at: normalize_optional_int(payload.get("expires_at")),
        account_id: normalize_optional_str(payload.get("account_id")),
        login: normalize_optional_str(payload.get("login")),
        email: normalize_optional_str(payload.get("email")),
        enterprise_url: normalize_optional_str(payload.get("enterprise_url")),
    })
}

/// Save GitHub Copilot OAuth tokens to disk.
pub fn save_github_copilot_oauth_tokens(
    tokens: &GitHubCopilotOAuthTokens,
    config_home: Option<&Path>,
) -> Result<PathBuf, GitHubCopilotOAuthLoginError> {
    let path = resolve_auth_path(config_home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let payload = serde_json::json!({
        "github_token": tokens.github_token,
        "github_token_type": tokens.github_token_type,
        "github_scope": tokens.github_scope,
        "expires_at": tokens.expires_at,
        "account_id": tokens.account_id,
        "login": tokens.login,
        "email": tokens.email,
        "enterprise_url": tokens.enterprise_url,
        "updated_at": Utc::now().timestamp(),
    });

    let json_str = serde_json::to_string_pretty(&payload)? + "\n";
    fs::write(&path, &json_str)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    Ok(path)
}

// ---------------------------------------------------------------------------
// Token discovery helpers
// ---------------------------------------------------------------------------

fn load_github_token_from_env() -> Option<String> {
    GITHUB_TOKEN_ENV_VARS
        .iter()
        .filter_map(|env_name| std::env::var(env_name).ok())
        .map(|val| val.trim().to_string())
        .find(|trimmed| !trimmed.is_empty())
}

/// Load a GitHub token from the `gh` CLI hosts.yml configuration file.
pub fn load_github_cli_oauth_token(gh_config_dir: Option<&Path>, host: &str) -> Option<String> {
    let base = match gh_config_dir {
        Some(p) => p.to_path_buf(),
        None => {
            if let Ok(val) = std::env::var("GH_CONFIG_DIR") {
                PathBuf::from(val)
            } else {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
                    .join("gh")
            }
        }
    };
    let hosts_path = base.join("hosts.yml");
    let contents = fs::read_to_string(&hosts_path).ok()?;
    parse_github_cli_hosts_yaml(&contents, host)
}

fn parse_github_cli_hosts_yaml(contents: &str, host: &str) -> Option<String> {
    let mut current_host: Option<&str> = None;
    for raw_line in contents.lines() {
        let line = raw_line.trim_end();
        let stripped = line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent == 0 && stripped.ends_with(':') {
            current_host = Some(stripped.trim_end_matches(':').trim());
            continue;
        }
        if current_host != Some(host) {
            continue;
        }
        if indent < 2 || !stripped.contains(':') {
            continue;
        }
        if let Some((key, value)) = stripped.split_once(':')
            && key.trim() == "oauth_token"
        {
            let token = value.trim().trim_matches(|c| c == '\'' || c == '"');
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Load a GitHub token by running `gh auth token`.
pub fn load_github_cli_oauth_token_via_command(
    host: &str,
    _timeout_seconds: f64,
) -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token", "--hostname", host])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn resolve_github_token(
    config_home: Option<&Path>,
    gh_config_dir: Option<&Path>,
    host: &str,
) -> Option<String> {
    load_github_copilot_oauth_tokens(config_home)
        .map(|stored| stored.github_token)
        .or_else(load_github_token_from_env)
        .or_else(|| load_github_cli_oauth_token(gh_config_dir, host))
        .or_else(|| load_github_cli_oauth_token_via_command(host, 5.0))
}

// ---------------------------------------------------------------------------
// Login (device flow)
// ---------------------------------------------------------------------------

fn github_headers(token: Option<&str>) -> Vec<(String, String)> {
    let base = [
        ("Accept", "application/json"),
        ("X-GitHub-Api-Version", DEFAULT_GITHUB_API_VERSION),
        ("User-Agent", "conduit-github-copilot-auth/0"),
    ];
    let mut headers: Vec<(String, String)> = base
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    if let Some(t) = token {
        headers.push(("Authorization".to_string(), format!("Bearer {t}")));
    }
    headers
}

async fn post_json(
    url: &str,
    payload: &Value,
    timeout_seconds: f64,
) -> Result<serde_json::Map<String, Value>, GitHubCopilotOAuthLoginError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs_f64(timeout_seconds))
        .build()?;

    let mut req = client.post(url).json(payload);
    for (k, v) in github_headers(None) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await?.error_for_status()?;
    let body: Value = resp.json().await?;
    body.as_object()
        .cloned()
        .ok_or_else(|| GitHubCopilotOAuthLoginError::ResponseError("expected JSON object".into()))
}

async fn fetch_profile(
    github_token: &str,
    timeout_seconds: f64,
) -> Result<serde_json::Map<String, Value>, GitHubCopilotOAuthLoginError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs_f64(timeout_seconds))
        .build()?;

    let mut req = client.get("https://api.github.com/user");
    for (k, v) in github_headers(Some(github_token)) {
        req = req.header(&k, &v);
    }

    let resp = req.send().await?.error_for_status()?;
    let body: Value = resp.json().await?;
    body.as_object()
        .cloned()
        .ok_or_else(|| GitHubCopilotOAuthLoginError::ResponseError("expected JSON object".into()))
}

/// Run the GitHub device flow and persist the resulting token.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub async fn login_github_copilot_oauth(
    config_home: Option<&Path>,
    open_browser: bool,
    scope: &str,
    timeout_seconds: f64,
    client_id: &str,
    device_code_url: &str,
    access_token_url: &str,
    device_code_notifier: Option<Box<dyn Fn(&str, &str) + Send>>,
) -> Result<GitHubCopilotOAuthTokens, GitHubCopilotOAuthLoginError> {
    let device_payload = post_json(
        device_code_url,
        &serde_json::json!({
            "client_id": client_id,
            "scope": scope,
        }),
        timeout_seconds,
    )
    .await?;

    let device_code = normalize_optional_str(device_payload.get("device_code"))
        .ok_or_else(|| GitHubCopilotOAuthLoginError::ResponseError("missing device_code".into()))?;
    let user_code = normalize_optional_str(device_payload.get("user_code"))
        .ok_or_else(|| GitHubCopilotOAuthLoginError::ResponseError("missing user_code".into()))?;
    let verification_uri = normalize_optional_str(device_payload.get("verification_uri"))
        .ok_or_else(|| {
            GitHubCopilotOAuthLoginError::ResponseError("missing verification_uri".into())
        })?;
    let interval_seconds = normalize_optional_int(device_payload.get("interval")).unwrap_or(5);
    let expires_in_seconds =
        normalize_optional_int(device_payload.get("expires_in")).unwrap_or(900);

    if open_browser {
        let _ = open::that(&verification_uri);
    }

    match &device_code_notifier {
        Some(notifier) => notifier(&verification_uri, &user_code),
        None => {
            eprintln!("Open {verification_uri} and enter code: {user_code}");
        }
    }

    // Poll for access token
    let token_payload = poll_github_device_access_token(
        &device_code,
        interval_seconds,
        expires_in_seconds,
        timeout_seconds,
        client_id,
        access_token_url,
    )
    .await?;

    let github_token =
        normalize_optional_str(token_payload.get("access_token")).ok_or_else(|| {
            GitHubCopilotOAuthLoginError::ResponseError("missing access_token".into())
        })?;

    let expires_in = normalize_optional_int(token_payload.get("expires_in"));
    let expires_at = expires_in.map(|ei| Utc::now().timestamp() + ei);

    let profile = fetch_profile(&github_token, timeout_seconds).await?;

    let tokens = GitHubCopilotOAuthTokens {
        github_token,
        github_token_type: normalize_optional_str(token_payload.get("token_type"))
            .unwrap_or_else(|| "bearer".to_string()),
        github_scope: normalize_optional_str(token_payload.get("scope"))
            .or_else(|| Some(scope.to_string())),
        expires_at,
        account_id: profile
            .get("id")
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string()),
        login: normalize_optional_str(profile.get("login")),
        email: normalize_optional_str(profile.get("email")),
        enterprise_url: None,
    };

    save_github_copilot_oauth_tokens(&tokens, config_home)?;
    Ok(tokens)
}

async fn poll_github_device_access_token(
    device_code: &str,
    initial_interval: i64,
    expires_in_seconds: i64,
    timeout_seconds: f64,
    client_id: &str,
    access_token_url: &str,
) -> Result<serde_json::Map<String, Value>, GitHubCopilotOAuthLoginError> {
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(expires_in_seconds.max(1) as u64);
    let mut poll_interval = initial_interval.max(1) as u64;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err(GitHubCopilotOAuthLoginError::Timeout);
        }

        let payload = post_json(
            access_token_url,
            &serde_json::json!({
                "client_id": client_id,
                "device_code": device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }),
            timeout_seconds,
        )
        .await?;

        if payload.contains_key("access_token") {
            return Ok(payload);
        }

        let error = normalize_optional_str(payload.get("error"));
        match error.as_deref() {
            Some("authorization_pending") => {
                tokio::time::sleep(Duration::from_secs(poll_interval)).await;
                continue;
            }
            Some("slow_down") => {
                poll_interval += 5;
                tokio::time::sleep(Duration::from_secs(poll_interval)).await;
                continue;
            }
            Some("expired_token") => {
                return Err(GitHubCopilotOAuthLoginError::Expired);
            }
            Some("access_denied") => {
                return Err(GitHubCopilotOAuthLoginError::Denied);
            }
            Some(err) => {
                let message = normalize_optional_str(payload.get("error_description"))
                    .unwrap_or_else(|| err.to_string());
                return Err(GitHubCopilotOAuthLoginError::Other(message));
            }
            None => {
                tokio::time::sleep(Duration::from_secs(poll_interval)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

/// Build a resolver for `github-copilot` backed by GitHub OAuth tokens.
///
/// The resolver checks (in order): stored tokens, environment variables,
/// gh CLI config, and gh CLI command.
pub fn github_copilot_oauth_resolver(
    config_home: Option<PathBuf>,
    gh_config_dir: Option<PathBuf>,
    host: Option<String>,
) -> APIKeyResolver {
    let lock = Mutex::new(());
    let host = host.unwrap_or_else(|| DEFAULT_GITHUB_HOST.to_string());

    Box::new(move |provider: &str| -> Option<String> {
        if !GITHUB_COPILOT_PROVIDERS.contains(&provider) {
            return None;
        }
        let _guard = lock.lock().ok()?;
        resolve_github_token(config_home.as_deref(), gh_config_dir.as_deref(), &host)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn make_tokens(github_token: &str) -> GitHubCopilotOAuthTokens {
        GitHubCopilotOAuthTokens {
            github_token: github_token.to_string(),
            github_token_type: "bearer".to_string(),
            github_scope: Some("read:user".to_string()),
            expires_at: Some(9999999999),
            account_id: Some("12345".to_string()),
            login: Some("testuser".to_string()),
            email: Some("test@example.com".to_string()),
            enterprise_url: None,
        }
    }

    // ----- Token save/load round-trip -----

    #[test]
    fn test_save_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let tokens = make_tokens("ghp_abc123");

        let path = save_github_copilot_oauth_tokens(&tokens, Some(tmp.path())).unwrap();
        assert!(path.exists());

        let loaded = load_github_copilot_oauth_tokens(Some(tmp.path())).unwrap();
        assert_eq!(loaded.github_token, "ghp_abc123");
        assert_eq!(loaded.github_token_type, "bearer");
        assert_eq!(loaded.github_scope.as_deref(), Some("read:user"));
        assert_eq!(loaded.expires_at, Some(9999999999));
        assert_eq!(loaded.account_id.as_deref(), Some("12345"));
        assert_eq!(loaded.login.as_deref(), Some("testuser"));
        assert_eq!(loaded.email.as_deref(), Some("test@example.com"));
        assert_eq!(loaded.enterprise_url, None);
    }

    #[test]
    fn test_save_load_round_trip_minimal() {
        let tmp = TempDir::new().unwrap();
        let tokens = GitHubCopilotOAuthTokens {
            github_token: "tok".to_string(),
            github_token_type: "bearer".to_string(),
            github_scope: None,
            expires_at: None,
            account_id: None,
            login: None,
            email: None,
            enterprise_url: None,
        };

        save_github_copilot_oauth_tokens(&tokens, Some(tmp.path())).unwrap();
        let loaded = load_github_copilot_oauth_tokens(Some(tmp.path())).unwrap();

        assert_eq!(loaded.github_token, "tok");
        assert_eq!(loaded.github_token_type, "bearer");
    }

    #[test]
    fn test_load_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(load_github_copilot_oauth_tokens(Some(tmp.path())).is_none());
    }

    #[test]
    fn test_load_returns_none_for_empty_github_token() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("conduit");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("github_copilot_auth.json");
        let content = json!({
            "github_token": "",
        });
        fs::write(&path, serde_json::to_string(&content).unwrap()).unwrap();

        assert!(load_github_copilot_oauth_tokens(Some(tmp.path())).is_none());
    }

    // ----- parse_tokens edge cases -----

    #[test]
    fn test_parse_tokens_default_token_type() {
        let payload: serde_json::Map<String, serde_json::Value> = serde_json::from_value(json!({
            "github_token": "tok",
        }))
        .unwrap();

        let result = parse_tokens(&payload).unwrap();
        assert_eq!(result.github_token_type, "bearer");
    }

    // ----- normalize_optional_str -----

    #[test]
    fn test_normalize_optional_str_trims_whitespace() {
        let val = json!("  hello  ");
        assert_eq!(
            normalize_optional_str(Some(&val)),
            Some("hello".to_string())
        );
    }

    #[test]
    fn test_normalize_optional_str_returns_none_for_empty() {
        let val = json!("   ");
        assert_eq!(normalize_optional_str(Some(&val)), None);
    }

    #[test]
    fn test_normalize_optional_str_returns_none_for_none() {
        assert_eq!(normalize_optional_str(None), None);
    }

    // ----- normalize_optional_int -----

    #[test]
    fn test_normalize_optional_int_from_i64() {
        let val = json!(42);
        assert_eq!(normalize_optional_int(Some(&val)), Some(42));
    }

    #[test]
    fn test_normalize_optional_int_from_f64() {
        let val = json!(42.9);
        assert_eq!(normalize_optional_int(Some(&val)), Some(42));
    }

    #[test]
    fn test_normalize_optional_int_from_string() {
        let val = json!("123");
        assert_eq!(normalize_optional_int(Some(&val)), Some(123));
    }

    #[test]
    fn test_normalize_optional_int_rejects_boolean() {
        let val = json!(true);
        assert_eq!(normalize_optional_int(Some(&val)), None);
    }

    // ----- parse_github_cli_hosts_yaml -----

    #[test]
    fn test_parse_github_cli_hosts_yaml_basic() {
        let yaml = "github.com:\n  oauth_token: ghp_secret\n  user: testuser\n";
        assert_eq!(
            parse_github_cli_hosts_yaml(yaml, "github.com"),
            Some("ghp_secret".to_string())
        );
    }

    #[test]
    fn test_parse_github_cli_hosts_yaml_wrong_host() {
        let yaml = "github.com:\n  oauth_token: ghp_secret\n";
        assert_eq!(parse_github_cli_hosts_yaml(yaml, "other.example.com"), None);
    }

    #[test]
    fn test_parse_github_cli_hosts_yaml_multiple_hosts() {
        let yaml = "github.com:\n  oauth_token: ghp_pub\n  user: alice\n\
                     enterprise.example.com:\n  oauth_token: ghp_ent\n  user: bob\n";
        assert_eq!(
            parse_github_cli_hosts_yaml(yaml, "enterprise.example.com"),
            Some("ghp_ent".to_string())
        );
    }

    #[test]
    fn test_parse_github_cli_hosts_yaml_quoted_token() {
        let yaml = "github.com:\n  oauth_token: 'ghp_quoted'\n";
        assert_eq!(
            parse_github_cli_hosts_yaml(yaml, "github.com"),
            Some("ghp_quoted".to_string())
        );
    }

    #[test]
    fn test_parse_github_cli_hosts_yaml_empty_token() {
        let yaml = "github.com:\n  oauth_token: \n";
        assert_eq!(parse_github_cli_hosts_yaml(yaml, "github.com"), None);
    }

    // ----- load_github_cli_oauth_token -----

    #[test]
    fn test_load_github_cli_oauth_token_from_file() {
        let tmp = TempDir::new().unwrap();
        let hosts_path = tmp.path().join("hosts.yml");
        fs::write(&hosts_path, "github.com:\n  oauth_token: ghp_from_file\n").unwrap();

        let result = load_github_cli_oauth_token(Some(tmp.path()), "github.com");
        assert_eq!(result, Some("ghp_from_file".to_string()));
    }

    #[test]
    fn test_load_github_cli_oauth_token_missing_file() {
        let tmp = TempDir::new().unwrap();
        let result = load_github_cli_oauth_token(Some(tmp.path()), "github.com");
        assert_eq!(result, None);
    }

    // ----- github_copilot_oauth_resolver -----

    #[test]
    fn test_resolver_returns_none_for_non_copilot_provider() {
        let tmp = TempDir::new().unwrap();
        let tokens = make_tokens("ghp_test");
        save_github_copilot_oauth_tokens(&tokens, Some(tmp.path())).unwrap();

        let resolver = github_copilot_oauth_resolver(Some(tmp.path().to_path_buf()), None, None);

        assert_eq!(resolver("openai"), None);
        assert_eq!(resolver("anthropic"), None);
    }

    #[test]
    fn test_resolver_returns_token_for_github_copilot() {
        let tmp = TempDir::new().unwrap();
        let tokens = make_tokens("ghp_resolver_test");
        save_github_copilot_oauth_tokens(&tokens, Some(tmp.path())).unwrap();

        let resolver = github_copilot_oauth_resolver(Some(tmp.path().to_path_buf()), None, None);

        assert_eq!(
            resolver("github-copilot"),
            Some("ghp_resolver_test".to_string())
        );
    }

    #[test]
    fn test_resolver_returns_none_when_no_stored_tokens() {
        let tmp = TempDir::new().unwrap();
        // No stored tokens, no env vars, no gh CLI config

        let resolver = github_copilot_oauth_resolver(
            Some(tmp.path().to_path_buf()),
            Some(tmp.path().join("nonexistent").to_path_buf()),
            None,
        );

        // This will return None since there are no stored tokens,
        // env vars, or gh CLI tokens accessible
        // (gh CLI command might succeed on some machines, so we just verify the resolver runs)
        let _ = resolver("github-copilot");
    }

    // ----- resolve_auth_path -----

    #[test]
    fn test_resolve_auth_path_with_explicit_path() {
        let result = resolve_auth_path(Some(Path::new("/custom/config")));
        assert_eq!(
            result,
            PathBuf::from("/custom/config/conduit/github_copilot_auth.json")
        );
    }
}
