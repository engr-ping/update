// 自定义 HTTP 源（对应原 Go internal/source/custom.go）。
//
// feed 可以是单个发布对象或对象数组（新版本在前）。支持从
// download_url_template 填充缺失的资产 URL。
use crate::config::CustomConfig;
use crate::semver::compare;
use crate::source::{Release, Source};
use crate::transport::{Client, Error};
use std::io::Read;

const MAX_FEED_BYTES: usize = 32 << 20;

pub struct CustomSource {
    cfg: CustomConfig,
    client: Client,
}

impl CustomSource {
    pub fn new(cfg: CustomConfig, client: &Client) -> Self {
        CustomSource {
            cfg,
            client: client.clone(),
        }
    }

    fn fetch_feed(&self) -> Result<Vec<Release>, Error> {
        let resp = self.client.get(&self.cfg.versions_url)?;
        let mut limited = resp.into_reader().take((MAX_FEED_BYTES + 1) as u64);
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut limited, &mut data)
            .map_err(|e| Error::source(format!("read feed: {e}")))?;

        let data = trim_space(&data);
        if data.is_empty() {
            return Err(Error::source(format!(
                "empty feed from {}",
                self.cfg.versions_url
            )));
        }
        match data[0] {
            b'[' => serde_json::from_slice(data)
                .map_err(|e| Error::source(format!("decode feed list: {e}"))),
            b'{' => serde_json::from_slice::<Release>(data)
                .map(|one| vec![one])
                .map_err(|e| Error::source(format!("decode feed: {e}"))),
            _ => Err(Error::source(
                "feed must be a JSON object or array".to_string(),
            )),
        }
    }

    /// 用 download_url_template 填充没有显式 URL 的资产。
    fn apply_template(&self, r: &mut Release) {
        if self.cfg.download_url_template.is_empty() {
            return;
        }
        for a in r.assets.iter_mut() {
            if a.url.is_empty() {
                let url = self
                    .cfg
                    .download_url_template
                    .replace("{asset}", &a.name)
                    .replace("{version}", &r.version)
                    .replace("{tag_name}", &r.tag_name);
                a.url = url;
            }
        }
    }
}

/// 去掉首尾空白（Go bytes.TrimSpace 语义）。
fn trim_space(b: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = b.len();
    while start < end && (b[start] as char).is_whitespace() {
        start += 1;
    }
    while end > start && (b[end - 1] as char).is_whitespace() {
        end -= 1;
    }
    &b[start..end]
}

impl Source for CustomSource {
    /// feed 中版本最高的发布。
    fn latest(&self) -> Result<Release, Error> {
        let rels = self.fetch_feed()?;
        let mut best: Option<Release> = None;
        for r in rels {
            match &best {
                None => best = Some(r),
                Some(b) if compare(&r.version, &b.version) > 0 => best = Some(r),
                _ => {}
            }
        }
        let mut best = best.ok_or_else(|| {
            Error::source(format!(
                "feed from {} has no releases",
                self.cfg.versions_url
            ))
        })?;
        self.apply_template(&mut best);
        Ok(best)
    }

    /// 按 feed 顺序返回，最多 limit 条。
    fn list(&self, limit: usize) -> Result<Vec<Release>, Error> {
        let limit = if limit == 0 { 10 } else { limit };
        let mut rels = self.fetch_feed()?;
        rels.truncate(limit);
        for r in &mut rels {
            self.apply_template(r);
        }
        Ok(rels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, Config, CustomConfig, SourceConfig};
    use crate::source::new;
    use crate::testutil::{json_response, req_header, TestServer};
    use crate::transport::{Auth, Options};
    use std::collections::HashMap;

    fn client_from_config(cfg: &Config) -> Client {
        let headers = match &cfg.source.custom {
            Some(cu) => cu.headers.clone().unwrap_or_default(),
            None => HashMap::new(),
        };
        let auth = match &cfg.source.custom {
            Some(cu) => cu.auth.as_ref().map(|a| Auth {
                ty: a.ty.clone(),
                token: a.token.clone(),
                username: a.username.clone(),
            }),
            None => None,
        };
        Client::new(Options {
            auth,
            headers,
            ..Default::default()
        })
    }

    fn new_custom_config(srv_url: &str) -> Config {
        Config {
            source: SourceConfig {
                ty: "custom".to_string(),
                custom: Some(CustomConfig {
                    versions_url: format!("{srv_url}/feed.json"),
                    download_url_template: format!("{srv_url}/files/{{version}}/{{asset}}"),
                    auth: Some(AuthConfig {
                        ty: "bearer".to_string(),
                        token: "tok".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_custom_single_feed() {
        // 对应 Go internal/source/custom_test.go TestCustomSingleFeed
        let srv = TestServer::new(|req| {
            assert_eq!(req_header(req, "Authorization"), "Bearer tok");
            json_response(
                200,
                r#"{
                    "version": "2.1.0",
                    "published_at": "2024-03-01T00:00:00Z",
                    "assets": [{"name": "app.zip"}]
                }"#,
            )
        });
        let cfg = new_custom_config(&srv.url);
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rel = src.latest().unwrap();
        assert_eq!(rel.version, "2.1.0");
        // 模板应填充资产 URL
        assert_eq!(
            rel.assets[0].url,
            format!("{}/files/2.1.0/app.zip", srv.url)
        );
    }

    #[test]
    fn test_custom_array_feed_latest() {
        let srv = TestServer::new(|_| {
            json_response(
                200,
                r#"[
                {"version": "1.0.0"},
                {"version": "2.0.0"},
                {"version": "1.5.0"}
            ]"#,
            )
        });
        let cfg = new_custom_config(&srv.url);
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        let rel = src.latest().unwrap();
        assert_eq!(rel.version, "2.0.0");

        let rels = src.list(2).unwrap();
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].version, "1.0.0");
        assert_eq!(rels[1].version, "2.0.0");
    }

    #[test]
    fn test_custom_empty_feed() {
        let srv = TestServer::new(|_| json_response(200, "[]"));
        let cfg = new_custom_config(&srv.url);
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        assert!(src.latest().is_err());
    }

    #[test]
    fn test_custom_invalid_feed() {
        let srv = TestServer::new(|_| json_response(200, "not json"));
        let cfg = new_custom_config(&srv.url);
        let src = new(&cfg, &client_from_config(&cfg)).unwrap();
        assert!(src.latest().is_err());
    }
}
