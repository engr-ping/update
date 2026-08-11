// 源抽象与统一发布清单（对应原 Go internal/source/source.go）。
use crate::config::Config;
use crate::custom::CustomSource;
use crate::github::GitHubSource;
use crate::transport::{Client, Error};
use serde::{Deserialize, Serialize};

/// 所有源共享的统一发布模型（docs/design.md §6）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "tag_name", skip_serializing_if = "String::is_empty")]
    pub tag_name: String,
    #[serde(
        default,
        rename = "published_at",
        skip_serializing_if = "String::is_empty"
    )]
    pub published_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checksum: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// 发布所附产物。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha256: String,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// 发布源。
pub trait Source: Send + Sync {
    /// 最新发布。
    fn latest(&self) -> Result<Release, Error>;
    /// 历史版本，最新在前，最多 limit 条。
    fn list(&self, limit: usize) -> Result<Vec<Release>, Error>;
}

/// 按配置构建源。
pub fn new(cfg: &Config, client: &Client) -> Result<Box<dyn Source>, String> {
    match cfg.source.ty.as_str() {
        "github-tag" => {
            let g = cfg.source.github.clone().ok_or_else(|| {
                "unsupported source: github-tag requires a github section".to_string()
            })?;
            Ok(Box::new(GitHubSource::new(g, client)))
        }
        "custom" => {
            let cu = cfg.source.custom.clone().ok_or_else(|| {
                "unsupported source: custom requires a custom section".to_string()
            })?;
            Ok(Box::new(CustomSource::new(cu, client)))
        }
        other => Err(format!("unsupported source type {other:?}")),
    }
}
