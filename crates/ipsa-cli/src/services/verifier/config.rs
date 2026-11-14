use crate::config::{get, ImageRef};

pub struct VerifierConfig {
    pub image: ImageRef,
}

impl VerifierConfig {
    pub fn from_env() -> Self {
        Self {
            image: ImageRef {
                image: get("VERIFIER_IMAGE", ""),
                tag: get("VERIFIER_TAG", "latest"),
                registry: get("VERIFIER_REGISTRY", ""),
            },
        }
    }
}
