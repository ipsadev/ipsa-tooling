use crate::{config::Config, logger, probe, shared, tx::clients::config::ClientsConfig};

fn client_type(state: Option<&serde_json::Value>) -> String {
    state
        .and_then(|s| s.get("@type"))
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string()
}

pub async fn stellar_clients(cfg: &Config, http: &reqwest::Client, client_id: Option<&str>) {
    logger::banner("query clients — stellar router");

    let cc = ClientsConfig::from(cfg);

    if !probe::http_ok(http, &cc.api_health_url()).await {
        logger::warn("api unreachable — start it with `ipsa up`");

        return;
    }

    match probe::get_json(http, &cc.clients_url()).await {
        Some(value) => shared::print_clients(&value, client_id),
        _ => logger::warn("could not read /stellar/clients"),
    }
}

pub async fn cosmos_clients(cfg: &Config, http: &reqwest::Client, client_id: Option<&str>) {
    logger::banner("query clients — cosmos");

    let base = format!("{}/ibc/core/client/v1/client_states", cfg.cosmos.rest_url);
    let url = match client_id {
        Some(id) => format!("{base}/{id}"),
        None => base,
    };

    let Some(value) = probe::get_json(http, &url).await else {
        logger::warn(&format!("could not read {url}"));

        return;
    };

    if let Some(states) = value.get("client_states").and_then(|v| v.as_array()) {
        if states.is_empty() {
            logger::detail("no clients on cosmos");

            return;
        }

        for state in states {
            let id = state
                .get("client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            logger::ok(&format!("{id}: {}", client_type(state.get("client_state"))));
        }
    } else if let Some(state) = value.get("client_state") {
        logger::ok(&format!(
            "{}: {}",
            client_id.unwrap_or("?"),
            client_type(Some(state))
        ));
    } else {
        logger::warn("unexpected client_states response shape");
    }
}
