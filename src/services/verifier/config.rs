use crate::config::{get, ImageRef};

pub struct VerifierConfig {
    pub image: ImageRef,
    pub repo_url: String,
    pub branch: String,
}

impl VerifierConfig {
    pub fn from_env() -> Self {
        let branch = get("VERIFIER_BRANCH", "main");

        Self {
            image: ImageRef {
                image: get("VERIFIER_IMAGE", "amandagonsalvesx/ipsa-scp-verifier"),
                tag: get("VERIFIER_TAG", "latest"),
                registry: get("VERIFIER_REGISTRY", ""),
            },
            repo_url: get("VERIFIER_REPO_URL", ""),
            branch: if branch.trim().is_empty() {
                "main".to_string()
            } else {
                branch
            },
        }
    }
}
