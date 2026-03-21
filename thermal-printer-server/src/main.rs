mod app;

use app::{ResultPage, UploadPage};
use axum::{
    extract::{DefaultBodyLimit, Multipart},
    response::Html,
    routing::{get, post},
    Router,
};
use leptos::prelude::*;
use rp326_usb::{
    escpos::{DitherMode, Packet},
    printer::Printer,
};
use std::path::Path;
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
        .route("/print", post(print_file))
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

#[instrument(skip(multipart))]
async fn print_file(mut multipart: Multipart) -> Html<String> {
    let request_start = Instant::now();

    let result = async {
        // Collect all fields — quality may arrive before or after the file.
        let mut filename = None;
        let mut data = None;
        let mut quality = "high".to_string();

        while let Some(field) = multipart.next_field().await? {
            match field.name() {
                Some("quality") => quality = field.text().await?,
                Some("file") => {
                    filename = Some(field.file_name().unwrap_or("file").to_string());
                    data = Some(field.bytes().await?);
                }
                _ => {}
            }
        }

        let filename = filename.ok_or_else(|| anyhow::anyhow!("No file uploaded"))?;
        let data = data.ok_or_else(|| anyhow::anyhow!("No file data"))?;

        let dither = match quality.as_str() {
            "normal" => DitherMode::Threshold,
            _ => DitherMode::FloydSteinberg,
        };

        // --- Stage 1: receive upload ---
        info!(
            filename,
            size_bytes = data.len(),
            quality,
            elapsed_ms = request_start.elapsed().as_millis(),
            "upload received"
        );

        // --- Stage 2: build ESC/POS payload (decode + dither) ---
        let t = Instant::now();
        let payload = build_payload(&filename, &data, dither)?;
        info!(
            payload_bytes = payload.len(),
            elapsed_ms = t.elapsed().as_millis(),
            "payload built"
        );

        // --- Stage 3: open printer + write ---
        let t = Instant::now();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let open_t = Instant::now();
            let printer = Printer::open()?;
            info!(elapsed_ms = open_t.elapsed().as_millis(), "printer opened");

            let write_t = Instant::now();
            printer.write(&payload)?;
            info!(elapsed_ms = write_t.elapsed().as_millis(), "printer write done");

            Ok(())
        })
        .await??;
        info!(elapsed_ms = t.elapsed().as_millis(), "print complete");

        anyhow::Ok(filename)
    }
    .await;

    info!(
        success = result.is_ok(),
        total_ms = request_start.elapsed().as_millis(),
        "request finished"
    );

    Html(format!(
        "<!DOCTYPE html>{}",
        match result {
            Ok(filename) => {
                let msg = format!("\"{}\" sent to printer.", filename);
                view! { <ResultPage message=msg success=true/> }.to_html()
            }
            Err(e) => {
                let msg = e.to_string();
                warn!(error = %e, "print failed");
                view! { <ResultPage message=msg success=false/> }.to_html()
            }
        }
    ))
}

fn build_payload(filename: &str, data: &[u8], dither: DitherMode) -> anyhow::Result<Vec<u8>> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let packet = Packet::new().initialize();

    let packet = if is_image_ext(&ext) {
        let t = Instant::now();
        let img = image::load_from_memory(data)
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
    } else {
        let text = std::str::from_utf8(data)
            .map_err(|_| anyhow::anyhow!("File is not valid UTF-8 text"))?;
        packet.text(text)
    };

    Ok(packet.feed(4).cut().into_bytes())
}

fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tiff" | "tif")
}
