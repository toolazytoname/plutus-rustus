use std::env;
use std::time::Duration;

use reqwest::blocking::Client;

use crate::config::{self, NotifyConfig, NotifyProvider};

pub struct Notifier {
    client: Client,
    provider: NotifyProvider,
    webhook_url: Option<String>,
    serverchan_key: Option<String>,
    bark_key: Option<String>,
    bark_server: String,
}

impl Notifier {
    pub fn from_config(cfg: &NotifyConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        let webhook_url = env::var(&cfg.webhook_url_env)
            .ok()
            .filter(|s| !s.is_empty());
        let serverchan_key = env::var("PLUTUS_SERVERCHAN_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let bark_key = env::var(&cfg.token_env)
            .ok()
            .or_else(|| env::var("PLUTUS_BARK_KEY").ok())
            .filter(|s| !s.is_empty());
        Self {
            client,
            provider: cfg.provider,
            webhook_url,
            serverchan_key,
            bark_key,
            bark_server: config::bark_server(cfg),
        }
    }

    pub fn enabled(&self) -> bool {
        match self.provider {
            NotifyProvider::Disabled => false,
            NotifyProvider::Bark => self.bark_key.is_some(),
            NotifyProvider::Webhook => self.webhook_url.is_some(),
            NotifyProvider::ServerChan => self.serverchan_key.is_some(),
        }
    }

    pub fn configured_without_secret(&self) -> &'static str {
        match self.provider {
            NotifyProvider::Disabled => "disabled",
            NotifyProvider::Bark => {
                if self.bark_key.is_some() {
                    "bark"
                } else {
                    "bark (missing PLUTUS_BARK_KEY)"
                }
            }
            NotifyProvider::Webhook => {
                if self.webhook_url.is_some() {
                    "webhook"
                } else {
                    "webhook (missing URL env)"
                }
            }
            NotifyProvider::ServerChan => {
                if self.serverchan_key.is_some() {
                    "serverchan"
                } else {
                    "serverchan (missing token env)"
                }
            }
        }
    }

    /// Status/heartbeat/hit alerts. `body` must not contain private keys.
    pub fn send(&self, title: &str, body: &str) {
        if !self.enabled() {
            return;
        }
        if let Err(error) = self.send_inner(title, body) {
            eprintln!("notify failed: {error}");
        }
    }

    pub fn send_result(&self, title: &str, body: &str) -> Result<(), String> {
        if !self.enabled() {
            return Err("notify is not enabled or credentials are missing".into());
        }
        self.send_inner(title, body)
    }

    fn send_inner(&self, title: &str, body: &str) -> Result<(), String> {
        match self.provider {
            NotifyProvider::Disabled => Ok(()),
            NotifyProvider::Bark => self.send_bark(title, body),
            NotifyProvider::Webhook => {
                let url = self.webhook_url.as_ref().ok_or("webhook URL missing")?;
                let response = self
                    .client
                    .post(url)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(format!("{title}\n{body}"))
                    .send()
                    .map_err(|_| "webhook request failed".to_owned())?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(format!("webhook returned {}", response.status()))
                }
            }
            NotifyProvider::ServerChan => {
                let key = self
                    .serverchan_key
                    .as_ref()
                    .ok_or("ServerChan token missing")?;
                let url = format!("https://sctapi.ftqq.com/{key}.send");
                let response = self
                    .client
                    .post(url)
                    .form(&[("text", title), ("desp", body)])
                    .send()
                    .map_err(|_| "ServerChan request failed".to_owned())?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(format!("ServerChan returned {}", response.status()))
                }
            }
        }
    }

    fn send_bark(&self, title: &str, body: &str) -> Result<(), String> {
        let key = self.bark_key.as_ref().ok_or("Bark key missing")?;
        let url = if key.starts_with("http://") || key.starts_with("https://") {
            key.trim_end_matches('/').to_owned()
        } else {
            format!("{}/{key}", self.bark_server.trim_end_matches('/'))
        };
        let response = self
            .client
            .post(url)
            .form(&[
                ("title", title),
                ("body", body),
                ("group", "plutus"),
                ("level", "active"),
            ])
            .send()
            .map_err(|_| "Bark request failed".to_owned())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("Bark returned {}", response.status()))
        }
    }
}
