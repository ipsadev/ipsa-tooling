use crate::config::{get, ImageRef};

pub struct GatewayConfig {
    pub image: ImageRef,
}

impl GatewayConfig {
    pub fn from_env() -> Self {
        Self {
            image: ImageRef {
                image: get("GATEWAY_IMAGE", ""),
                tag: get("GATEWAY_TAG", "latest"),
                registry: get("GATEWAY_REGISTRY", ""),
            },
        }
    }
}
