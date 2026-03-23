mod app;

use app::UploadPage;
use axum::{
    extract::{DefaultBodyLimit, Multipart},
    response::{Html, Json},
    routing::{get, post},
    Router,
};
use leptos::prelude::*;
use rp326_usb::{
    escpos::{DitherMode, Packet},
    printer::Printer,
};
use serde::Serialize;
use std::time::Instant;
use tracing::{info, instrument, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thermal_printer_server=debug".into()),
        )
        .init();

    let app = Router::new()
        .route("/", get(index))
        .route("/print", post(print_handler))
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)); // 50 MB

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    info!("Listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}

async fn index() -> Html<String> {
    Html(format!(
        "<!DOCTYPE html>{}",
        view! { <UploadPage/> }.to_html()
    ))
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct PrintResponse {
    ok: bool,
    message: String,
}

impl PrintResponse {
    fn ok(message: impl Into<String>) -> Json<Self> {
        Json(Self { ok: true, message: message.into() })
    }
    fn err(message: impl Into<String>) -> Json<Self> {
        Json(Self { ok: false, message: message.into() })
    }
}

// ── Print payload ─────────────────────────────────────────────────────────────

enum PrintPayload {
    Image { data: bytes::Bytes, dither: DitherMode },
    Text(String),
}

// ── Handler ───────────────────────────────────────────────────────────────────

#[instrument(skip(multipart))]
async fn print_handler(mut multipart: Multipart) -> Json<PrintResponse> {
    let request_start = Instant::now();

    let result = parse_and_print(&mut multipart, request_start).await;

    info!(
        success = result.is_ok(),
        total_ms = request_start.elapsed().as_millis(),
        "request finished"
    );

    match result {
        Ok(msg) => PrintResponse::ok(msg),
        Err(e) => {
            warn!(error = %e, "print failed");
            PrintResponse::err(e.to_string())
        }
    }
}

async fn parse_and_print(
    multipart: &mut Multipart,
    request_start: Instant,
) -> anyhow::Result<String> {
    let mut payload: Option<PrintPayload> = None;
    let mut quality = "high".to_string();

    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("quality") => {
                quality = field.text().await?;
            }
            Some("file") => {
                if payload.is_some() {
                    anyhow::bail!("Received both 'file' and 'text' fields — send one or the other");
                }
                let data = field.bytes().await?;
                let dither = dither_from_quality(&quality);
                payload = Some(PrintPayload::Image { data, dither });
            }
            Some("text") => {
                if payload.is_some() {
                    anyhow::bail!("Received both 'file' and 'text' fields — send one or the other");
                }
                payload = Some(PrintPayload::Text(field.text().await?));
            }
            _ => {}
        }
    }

    let payload = payload.ok_or_else(|| {
        anyhow::anyhow!("No payload: send either a 'file' field (image) or a 'text' field")
    })?;

    // --- Stage 1: received ---
    match &payload {
        PrintPayload::Image { data, .. } => info!(
            size_bytes = data.len(),
            quality,
            elapsed_ms = request_start.elapsed().as_millis(),
            "image upload received"
        ),
        PrintPayload::Text(t) => info!(
            chars = t.len(),
            elapsed_ms = request_start.elapsed().as_millis(),
            "text upload received"
        ),
    }

    // --- Stage 2: build ESC/POS payload ---
    let t = Instant::now();
    let escpos = build_escpos(payload)?;
    info!(
        payload_bytes = escpos.len(),
        elapsed_ms = t.elapsed().as_millis(),
        "ESC/POS payload built"
    );

    // --- Stage 3: open printer + write ---
    let t = Instant::now();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let open_t = Instant::now();
        let printer = Printer::open()?;
        info!(elapsed_ms = open_t.elapsed().as_millis(), "printer opened");

        let write_t = Instant::now();
        printer.write(&escpos)?;
        info!(elapsed_ms = write_t.elapsed().as_millis(), "printer write done");

        Ok(())
    })
    .await??;

    info!(elapsed_ms = t.elapsed().as_millis(), "print complete");
    Ok("Sent to printer.".into())
}

// ── ESC/POS builder ───────────────────────────────────────────────────────────

fn build_escpos(payload: PrintPayload) -> anyhow::Result<Vec<u8>> {
    let packet = Packet::new().initialize();
    let packet = match payload {
        PrintPayload::Image { data, dither } => {
            let t = Instant::now();
            let img = image::load_from_memory(&data)
                .map_err(|e| anyhow::anyhow!("Failed to decode image: {e}"))?;
            info!(
                width = img.width(),
                height = img.height(),
                elapsed_ms = t.elapsed().as_millis(),
                "image decoded"
            );
            let t = Instant::now();
            let p = packet.image(img, dither);
            info!(elapsed_ms = t.elapsed().as_millis(), "image dithered + encoded");
            p
        }
        PrintPayload::Text(text) => packet.text(&text),
    };
    Ok(packet.feed(4).cut().into_bytes())
}

fn dither_from_quality(quality: &str) -> DitherMode {
    match quality {
        "normal" => DitherMode::Threshold,
        _ => DitherMode::FloydSteinberg,
    }
}
