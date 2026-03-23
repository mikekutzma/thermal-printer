use serde::Deserialize;

/// Cloudflare Zero Trust service token, parsed from the --api-key JSON flag.
#[derive(Deserialize)]
pub struct ApiKey {
    pub client_id: String,
    pub client_secret: String,
}
