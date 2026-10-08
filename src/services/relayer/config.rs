use std::path::Path;

use crate::config::{get, ImageRef};

pub struct RelayerConfig {
    pub image: ImageRef,
    pub context: String,
    pub config: String,
    pub keys: String,
    pub api_addr: String,
    pub repo_url: String,
    pub branch: String,
}

impl RelayerConfig {
    pub fn from_env(root: &Path) -> Self {
        let branch = get("RELAYER_BRANCH", "main");

        Self {
            image: ImageRef {
                image: get("RELAYER_IMAGE", ""),
                tag: get("RELAYER_TAG", "latest"),
                registry: get("RELAYER_REGISTRY", ""),
            },
            context: get("RELAYER_CONTEXT", ""),
            config: root.join("relayer.yml").display().to_string(),
            keys: root.join("relayer-keys.json").display().to_string(),
            api_addr: format!("127.0.0.1:{}", get("RELAYER_API_PORT", "9001")),
            repo_url: get("RELAYER_REPO_URL", ""),
            branch: if branch.trim().is_empty() {
                "main".to_string()
            } else {
                branch
            },
        }
    }
}
