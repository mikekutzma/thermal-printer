use reqwest::blocking::{multipart, Client};

use crate::auth::ApiKey;

/// POST PNG bytes to `{server}/print` as multipart form data.
pub fn send(server: &str, png: Vec<u8>, api_key: Option<&ApiKey>) -> anyhow::Result<()> {
    let url = format!("{}/print", server.trim_end_matches('/'));

    let part = multipart::Part::bytes(png)
        .file_name("document.png")
        .mime_str("image/png")?;
    let form = multipart::Form::new().part("file", part);

    let mut req = Client::new().post(&url).multipart(form);

    if let Some(key) = api_key {
        req = req
            .header("CF-Access-Client-Id", &key.client_id)
            .header("CF-Access-Client-Secret", &key.client_secret);
    }

    let resp = req.send()?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        anyhow::bail!("Server returned {status}: {body}");
    }

    Ok(())
}
