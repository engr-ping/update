// 配置加载与校验（对应原 Go internal/config/config.go）。
//
// 凭据（token）从不保存在配置文件里，只写环境变量名（token_env /
// username_env），运行时从环境读取。
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 根配置文档。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub product: Product,
    #[serde(default)]
    pub source: SourceConfig,
}

/// 标识宿主应用及其当前版本。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Product {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub current_version: String,
    /// 正则，可选
    #[serde(
        default,
        rename = "asset_filter",
        skip_serializing_if = "String::is_empty"
    )]
    pub asset_filter: String,
}

/// 选择发布源。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SourceConfig {
    /// "github-tag" | "custom"
    #[serde(default)]
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHubConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomConfig>,
}

/// GitHub tag/release 源配置。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GitHubConfig {
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub repo: String,
    /// 保存 PAT 的环境变量名（Basic 认证时为密码）
    #[serde(
        default,
        rename = "token_env",
        skip_serializing_if = "String::is_empty"
    )]
    pub token_env: String,
    /// 启用 HTTP Basic 认证（内部 GitHub Enterprise 需要用户名 + token/密码）
    #[serde(
        default,
        rename = "username_env",
        skip_serializing_if = "String::is_empty"
    )]
    pub username_env: String,
    /// 默认 https://api.github.com（GitHub Enterprise 覆盖）
    #[serde(
        default,
        rename = "api_base_url",
        skip_serializing_if = "String::is_empty"
    )]
    pub api_base_url: String,
    /// true: releases+assets；false: 仅 tags
    #[serde(default, rename = "use_releases")]
    pub use_releases: bool,

    // 运行时解析出的凭据（不序列化）
    #[serde(skip)]
    pub token: String,
    #[serde(skip)]
    pub username: String,
}

/// 自定义 HTTP 源配置。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CustomConfig {
    #[serde(default, rename = "versions_url")]
    pub versions_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    /// 支持 {version} / {asset} 占位符
    #[serde(
        default,
        rename = "download_url_template",
        skip_serializing_if = "String::is_empty"
    )]
    pub download_url_template: String,
}

/// 自定义源的 HTTP 认证。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct AuthConfig {
    /// "bearer" | "basic"
    #[serde(default)]
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(
        default,
        rename = "token_env",
        skip_serializing_if = "String::is_empty"
    )]
    pub token_env: String,
    #[serde(
        default,
        rename = "username_env",
        skip_serializing_if = "String::is_empty"
    )]
    pub username_env: String,

    #[serde(skip)]
    pub token: String,
    #[serde(skip)]
    pub username: String,
}

pub const DEFAULT_GITHUB_API_BASE_URL: &str = "https://api.github.com";

impl SourceConfig {
    fn validate_and_resolve(
        &mut self,
        getenv: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(), String> {
        match self.ty.as_str() {
            "github-tag" => {
                let g = self.github.as_mut().ok_or_else(|| {
                    "config: source type \"github-tag\" requires a github section".to_string()
                })?;
                if g.owner.is_empty() || g.repo.is_empty() {
                    return Err("config: github.owner and github.repo are required".to_string());
                }
                if g.api_base_url.is_empty() {
                    g.api_base_url = DEFAULT_GITHUB_API_BASE_URL.to_string();
                }
                if !g.token_env.is_empty() {
                    g.token = getenv(&g.token_env).unwrap_or_default();
                }
                if !g.username_env.is_empty() {
                    g.username = getenv(&g.username_env).unwrap_or_default();
                }
            }
            "custom" => {
                let cu = self.custom.as_mut().ok_or_else(|| {
                    "config: source type \"custom\" requires a custom section".to_string()
                })?;
                if cu.versions_url.is_empty() {
                    return Err("config: custom.versions_url is required".to_string());
                }
                if let Some(a) = cu.auth.as_mut() {
                    match a.ty.as_str() {
                        "bearer" => {
                            if !a.token_env.is_empty() {
                                a.token = getenv(&a.token_env).unwrap_or_default();
                            }
                        }
                        "basic" => {
                            if !a.username_env.is_empty() {
                                a.username = getenv(&a.username_env).unwrap_or_default();
                            }
                            if !a.token_env.is_empty() {
                                a.token = getenv(&a.token_env).unwrap_or_default();
                            }
                        }
                        other => {
                            return Err(format!(
                                "config: unsupported auth type {other:?} (want \"bearer\" or \"basic\")"
                            ));
                        }
                    }
                }
            }
            other => {
                return Err(format!(
                    "config: unsupported source type {other:?} (want \"github-tag\" or \"custom\")"
                ));
            }
        }
        Ok(())
    }
}

/// 环境变量读取函数（测试可注入假环境）。
pub type GetEnv = Box<dyn Fn(&str) -> Option<String>>;

/// 从 path 读取配置，解析、应用默认值并解析环境变量凭据。
pub fn load(path: &str, getenv: Option<GetEnv>) -> Result<Config, String> {
    let data = std::fs::read(path).map_err(|e| format!("read config: {e}"))?;
    parse(&data, getenv)
}

fn env_getter(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

/// 解析配置字节，应用默认值并解析凭据。
pub fn parse(data: &[u8], getenv: Option<GetEnv>) -> Result<Config, String> {
    let mut cfg: Config = serde_json::from_slice(data).map_err(|e| format!("parse config: {e}"))?;
    let getter: GetEnv = getenv.unwrap_or_else(|| Box::new(env_getter));
    cfg.source.validate_and_resolve(getter.as_ref())?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_with(cfg: &str, env: &HashMap<String, String>) -> Result<Config, String> {
        let env = env.clone();
        parse(
            cfg.as_bytes(),
            Some(Box::new(move |k: &str| env.get(k).cloned())),
        )
    }

    #[test]
    fn test_parse_github_releases() {
        let cfg = parse_with(
            r#"{
                "product": {"name": "my-app", "current_version": "1.0.0"},
                "source": {
                    "type": "github-tag",
                    "github": {"owner": "acme", "repo": "my-app", "token_env": "GITHUB_TOKEN", "use_releases": true}
                }
            }"#,
            &HashMap::from([("GITHUB_TOKEN".to_string(), "ghp_secret".to_string())]),
        )
        .expect("parse");
        let g = cfg.source.github.unwrap();
        assert_eq!(g.owner, "acme");
        assert_eq!(g.api_base_url, DEFAULT_GITHUB_API_BASE_URL);
        assert_eq!(g.token, "ghp_secret");
        assert!(g.use_releases);
    }

    #[test]
    fn test_parse_custom_bearer() {
        let cfg = parse_with(
            r#"{
                "source": {
                    "type": "custom",
                    "custom": {
                        "versions_url": "https://updates.example.com/feed.json",
                        "headers": {"X-Client": "my-app"},
                        "auth": {"type": "bearer", "token_env": "UPDATE_TOKEN"}
                    }
                }
            }"#,
            &HashMap::from([("UPDATE_TOKEN".to_string(), "tok123".to_string())]),
        )
        .expect("parse");
        let cu = cfg.source.custom.unwrap();
        assert_eq!(cu.auth.unwrap().token, "tok123");
        assert_eq!(cu.headers.unwrap()["X-Client"], "my-app");
    }

    #[test]
    fn test_parse_basic() {
        let cfg = parse_with(
            r#"{
                "source": {
                    "type": "custom",
                    "custom": {
                        "versions_url": "https://u.example.com/feed.json",
                        "auth": {"type": "basic", "username_env": "U_USER", "token_env": "U_PASS"}
                    }
                }
            }"#,
            &HashMap::from([
                ("U_USER".to_string(), "bob".to_string()),
                ("U_PASS".to_string(), "pw".to_string()),
            ]),
        )
        .expect("parse");
        let a = cfg.source.custom.unwrap().auth.unwrap();
        assert_eq!(a.username, "bob");
        assert_eq!(a.token, "pw");
    }

    #[test]
    fn test_parse_github_basic_auth() {
        let cfg = parse_with(
            r#"{
                "product": {"name": "my-app", "current_version": "1.0.0"},
                "source": {
                    "type": "github-tag",
                    "github": {
                        "owner": "acme", "repo": "my-app",
                        "username_env": "GHE_USER", "token_env": "GHE_TOKEN",
                        "api_base_url": "https://github.internal.example.com/api/v3"
                    }
                }
            }"#,
            &HashMap::from([
                ("GHE_USER".to_string(), "bob".to_string()),
                ("GHE_TOKEN".to_string(), "s3cret".to_string()),
            ]),
        )
        .expect("parse");
        let g = cfg.source.github.unwrap();
        assert_eq!(g.username, "bob");
        assert_eq!(g.token, "s3cret");
        assert_eq!(g.api_base_url, "https://github.internal.example.com/api/v3");
    }

    #[test]
    fn test_parse_errors() {
        let cases: &[(&str, &str)] = &[
            ("missing type", r#"{"source": {}}"#),
            ("bad type", r#"{"source": {"type": "nope"}}"#),
            (
                "github missing section",
                r#"{"source": {"type": "github-tag"}}"#,
            ),
            (
                "github missing owner",
                r#"{"source": {"type": "github-tag", "github": {"repo": "r"}}}"#,
            ),
            (
                "custom missing section",
                r#"{"source": {"type": "custom"}}"#,
            ),
            (
                "custom missing url",
                r#"{"source": {"type": "custom", "custom": {}}}"#,
            ),
            (
                "bad auth type",
                r#"{"source": {"type": "custom", "custom": {"versions_url": "https://x/", "auth": {"type": "digest"}}}}"#,
            ),
            ("invalid json", r#"{not json"#),
        ];
        for (name, data) in cases {
            let err = parse(data.as_bytes(), None).err();
            assert!(err.is_some(), "{name}: expected error");
        }
    }

    #[test]
    fn test_load_from_file() {
        let dir = std::env::temp_dir().join(format!("update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{"source":{"type":"github-tag","github":{"owner":"o","repo":"r"}}}"#,
        )
        .unwrap();
        let cfg = load(path.to_str().unwrap(), None).expect("load");
        assert_eq!(cfg.source.github.unwrap().owner, "o");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
