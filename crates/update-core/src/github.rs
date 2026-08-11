// GitHub tag/release 源（对应原 Go internal/source/github.go）。
use crate::config::GitHubConfig;
use crate::r#match::clean_tag;
use crate::semver::compare;
use crate::source::{Asset, Release, Source};
use crate::transport::{Client, Error};
use serde::Deserialize;

/// GitHub releases API 对象的子集。
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    #[serde(rename = "tag_name")]
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "published_at")]
    published_at: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
    #[serde(default, rename = "zipball_url")]
    #[allow(dead_code)]
    zipball_url: String,
    #[serde(default, rename = "tarball_url")]
    tarball_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    #[allow(dead_code)]
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    #[serde(default, rename = "browser_download_url")]
    browser_download_url: String,
    #[serde(default)]
    size: i64,
}

#[derive(Debug, Deserialize)]
struct GitHubTag {
    name: String,
}

pub struct GitHubSource {
    cfg: GitHubConfig,
    client: Client,
}

impl GitHubSource {
    pub fn new(cfg: GitHubConfig, client: &Client) -> Self {
        // Client 需要解析后的凭据；source::new 传入的 client 已经带认证
        GitHubSource {
            cfg,
            client: client.clone(),
        }
    }

    fn base(&self) -> String {
        self.cfg.api_base_url.trim_end_matches('/').to_string()
    }

    fn tags_url(&self, per_page: usize) -> String {
        format!(
            "{}/repos/{}/{}/tags?per_page={}",
            self.base(),
            self.cfg.owner,
            self.cfg.repo,
            per_page
        )
    }

    fn latest_url(&self) -> String {
        if self.cfg.use_releases {
            format!(
                "{}/repos/{}/{}/releases/latest",
                self.base(),
                self.cfg.owner,
                self.cfg.repo
            )
        } else {
            self.tags_url(100)
        }
    }

    fn list_url(&self, limit: usize) -> String {
        if self.cfg.use_releases {
            format!(
                "{}/repos/{}/{}/releases?per_page={limit}",
                self.base(),
                self.cfg.owner,
                self.cfg.repo
            )
        } else {
            self.tags_url(limit)
        }
    }

    fn to_release(&self, r: &GitHubRelease) -> Release {
        let mut rel = Release {
            version: clean_tag(&r.tag_name),
            tag_name: r.tag_name.clone(),
            published_at: r.published_at.clone(),
            name: r.name.clone(),
            notes: r.body.clone(),
            checksum: String::new(),
            assets: Vec::new(),
        };
        for a in &r.assets {
            rel.assets.push(Asset {
                name: a.name.clone(),
                url: a.browser_download_url.clone(),
                size: a.size,
                sha256: String::new(),
            });
        }
        if rel.assets.is_empty() && !r.tarball_url.is_empty() {
            rel.assets.push(Asset {
                name: format!("{}-{}.tar.gz", self.cfg.repo, r.tag_name),
                url: r.tarball_url.clone(),
                size: 0,
                sha256: String::new(),
            });
        }
        rel
    }

    fn fetch_tags(&self, per_page: usize) -> Result<Vec<GitHubTag>, Error> {
        self.client.get_json(&self.tags_url(per_page))
    }

    fn latest_from_tags(&self) -> Result<Release, Error> {
        let mut tags = self.fetch_tags(100)?;
        if tags.is_empty() {
            return Err(Error::source(format!(
                "no tags found in {}/{}",
                self.cfg.owner, self.cfg.repo
            )));
        }
        tags.sort_by(|a, b| compare(&b.name, &a.name).cmp(&0));
        let tag = &tags[0].name;
        Ok(Release {
            version: clean_tag(tag),
            tag_name: tag.clone(),
            ..Default::default()
        })
    }
}

impl Source for GitHubSource {
    /// 最新发布；use_releases=true 时若 releases/latest 404（如仓库有 tag 无
    /// release），回退到 tags。
    fn latest(&self) -> Result<Release, Error> {
        if self.cfg.use_releases {
            let rel: GitHubRelease = match self.client.get_json(&self.latest_url()) {
                Ok(r) => r,
                Err(e) if e.is_status(404) => return self.latest_from_tags(),
                Err(e) => return Err(e),
            };
            if rel.draft {
                return Err(Error::source(format!(
                    "latest release {:?} is a draft",
                    rel.tag_name
                )));
            }
            Ok(self.to_release(&rel))
        } else {
            self.latest_from_tags()
        }
    }

    /// 按 API 返回顺序（新在前）列出发布/tag，过滤 draft。
    fn list(&self, limit: usize) -> Result<Vec<Release>, Error> {
        let limit = if limit == 0 { 10 } else { limit };
        if self.cfg.use_releases {
            let rels: Vec<GitHubRelease> = self.client.get_json(&self.list_url(limit))?;
            let mut out = Vec::with_capacity(rels.len());
            for r in rels {
                if r.draft {
                    continue;
                }
                out.push(self.to_release(&r));
            }
            Ok(out)
        } else {
            let tags: Vec<GitHubTag> = self.fetch_tags(limit)?;
            Ok(tags
                .iter()
                .map(|t| Release {
                    version: clean_tag(&t.name),
                    tag_name: t.name.clone(),
                    ..Default::default()
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, GitHubConfig, SourceConfig};
    use crate::source::new;
    use crate::testutil::{json_response, req_header, TestServer};
    use crate::transport::{Auth, Options};

    fn client_from_config(cfg: &Config) -> Client {
        let mut auth: Option<Auth> = None;
        if let Some(g) = &cfg.source.github {
            if !g.token.is_empty() || !g.username.is_empty() {
                if !g.username.is_empty() {
                    auth = Some(Auth {
                        ty: "basic".to_string(),
                        token: g.token.clone(),
                        username: g.username.clone(),
                    });
                } else {
                    auth = Some(Auth {
                        ty: "bearer".to_string(),
                        token: g.token.clone(),
                        username: String::new(),
                    });
                }
            }
        }
        Client::new(Options {
            auth,
            ..Default::default()
        })
    }

    fn new_github_config(srv_url: &str, token: &str) -> Config {
        Config {
            source: SourceConfig {
                ty: "github-tag".to_string(),
                github: Some(GitHubConfig {
                    owner: "acme".to_string(),
                    repo: "my-app".to_string(),
                    api_base_url: srv_url.to_string(),
                    use_releases: true,
                    token: token.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// 与 Go github_test.go 同构的 releases 模拟 API。
    fn github_releases_handler(
    ) -> impl Fn(&crate::httpd::Request) -> crate::httpd::Response + Send + Sync {
        |req| {
            let path = req.path.as_str();
            if path.ends_with("/releases/latest") {
                if req_header(req, "Authorization") != "Bearer ghp_test" {
                    return json_response(401, r#"{"message":"Bad credentials"}"#);
                }
                return json_response(
                    200,
                    r#"{
                        "tag_name": "v1.2.0",
                        "name": "v1.2.0",
                        "published_at": "2024-01-15T10:00:00Z",
                        "body": "bug fixes",
                        "assets": [
                            {"name": "app-linux-amd64.tar.gz", "browser_download_url": "/dl/app-linux-amd64.tar.gz", "size": 10},
                            {"name": "app-windows-amd64.zip", "browser_download_url": "/dl/app-windows-amd64.zip", "size": 20}
                        ]
                    }"#,
                );
            }
            if path.ends_with("/releases") {
                return json_response(
                    200,
                    r#"[
                        {"tag_name": "v1.2.0", "published_at": "2024-01-15T10:00:00Z", "assets": []},
                        {"tag_name": "v1.1.0", "published_at": "2024-01-01T10:00:00Z", "assets": []}
                    ]"#,
                );
            }
            json_response(404, r#"{"message":"Not Found"}"#)
        }
    }

    #[test]
    fn test_github_latest() {
        let srv = TestServer::new(github_releases_handler());
        let cfg = new_github_config(&srv.url, "ghp_test");
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rel = src.latest().unwrap();
        assert_eq!(rel.version, "1.2.0");
        assert_eq!(rel.assets.len(), 2);
        assert_eq!(rel.assets[0].name, "app-linux-amd64.tar.gz");
    }

    #[test]
    fn test_github_list() {
        let srv = TestServer::new(github_releases_handler());
        let cfg = new_github_config(&srv.url, "ghp_test");
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rels = src.list(10).unwrap();
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].version, "1.2.0");
    }

    #[test]
    fn test_github_unauthorized() {
        let srv = TestServer::new(github_releases_handler());
        let cfg = new_github_config(&srv.url, "wrong_token");
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let err = src.latest().unwrap_err();
        assert_eq!(err.status_code, Some(401), "err = {err}");
    }

    #[test]
    fn test_github_tags_only() {
        let srv = TestServer::new(|req| {
            if req.path.ends_with("/tags") {
                json_response(
                    200,
                    r#"[
                    {"name": "v1.0.0"},
                    {"name": "v1.2.0"},
                    {"name": "v1.1.0"}
                ]"#,
                )
            } else {
                json_response(404, r#"{"message":"Not Found"}"#)
            }
        });
        let cfg = Config {
            source: SourceConfig {
                ty: "github-tag".to_string(),
                github: Some(GitHubConfig {
                    owner: "acme".to_string(),
                    repo: "my-app".to_string(),
                    api_base_url: srv.url.clone(),
                    use_releases: false,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rel = src.latest().unwrap();
        assert_eq!(rel.version, "1.2.0");
    }

    #[test]
    fn test_github_fallback_to_tags_on_404() {
        let srv = TestServer::new(|req| {
            if req.path.ends_with("/tags") {
                json_response(200, r#"[{"name": "v2.0.0"}]"#)
            } else {
                json_response(404, r#"{"message":"Not Found"}"#)
            }
        });
        let cfg = new_github_config(&srv.url, "");
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rel = src.latest().unwrap();
        assert_eq!(rel.version, "2.0.0");
    }

    #[test]
    fn test_github_tarball_fallback_asset() {
        let srv = TestServer::new(|req| {
            if req.path.ends_with("/releases/latest") {
                json_response(
                    200,
                    r#"{"tag_name": "v3.0.0", "tarball_url": "https://example.com/tarball/v3.0.0", "assets": []}"#,
                )
            } else {
                json_response(404, r#"{"message":"Not Found"}"#)
            }
        });
        let cfg = new_github_config(&srv.url, "");
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rel = src.latest().unwrap();
        assert_eq!(rel.assets.len(), 1);
        assert_eq!(rel.assets[0].name, "my-app-v3.0.0.tar.gz");
        assert_eq!(rel.assets[0].url, "https://example.com/tarball/v3.0.0");
    }

    #[test]
    fn test_github_draft_rejected() {
        let srv = TestServer::new(|req| {
            if req.path.ends_with("/releases/latest") {
                json_response(
                    200,
                    r#"{"tag_name": "v9.9.9", "draft": true, "assets": []}"#,
                )
            } else {
                json_response(404, r#"{"message":"Not Found"}"#)
            }
        });
        let cfg = new_github_config(&srv.url, "");
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let err = src.latest().unwrap_err();
        assert!(err.message.contains("draft"), "err = {err}");
    }
}
