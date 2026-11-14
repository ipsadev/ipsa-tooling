use crate::config::Config;

pub struct RelayConfig {
    pub api_url: String,
    pub relayer_config: String,
}

impl From<&Config> for RelayConfig {
    fn from(cfg: &Config) -> Self {
        Self {
            api_url: cfg.stellar.api_url.clone(),
            relayer_config: cfg.relayer.config.clone(),
        }
    }
}

impl RelayConfig {
    pub fn relay_url(&self) -> String {
        format!("{}/relay", self.api_url)
    }

    pub fn status_url(&self) -> String {
        format!("{}/relay/status", self.api_url)
    }
}
