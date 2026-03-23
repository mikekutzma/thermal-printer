mod auth;
mod client;
mod fonts;
mod painter;
mod render;

use std::path::PathBuf;

use clap::Parser;

/// Render a Markdown or HTML document to PNG and send it to a thermal-printer-server.
///
/// Examples:
///   thermal-printer-cli doc.md --server http://magicmirror.lan
///   thermal-printer-cli doc.html --server http://magicmirror.lan --output preview.png
///   thermal-printer-cli doc.md --render-only --output preview.png
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Path to the input file (.md or .html)
    input: PathBuf,

    /// Base URL of the thermal-printer-server (e.g. http://magicmirror.lan)
    #[arg(long)]
    server: Option<String>,

    /// Cloudflare Zero Trust service token as JSON:
    /// '{"client_id":"<id>","client_secret":"<secret>"}'
    #[arg(long)]
    api_key: Option<String>,

    /// Save the rendered PNG to this path
    #[arg(long)]
    output: Option<PathBuf>,

    /// Render to PNG only — do not send to the printer (requires --output)
    #[arg(long, requires = "output")]
    render_only: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if !args.render_only && args.server.is_none() {
        anyhow::bail!("--server is required unless --render-only is set");
    }

    let api_key: Option<auth::ApiKey> = match &args.api_key {
        Some(raw) => Some(
            serde_json::from_str(raw).map_err(|e| {
                anyhow::anyhow!(
                    "--api-key must be valid JSON \
                     ({{\"client_id\":\"...\",\"client_secret\":\"...\"}}): {e}"
                )
            })?,
        ),
        None => None,
    };

    // ── 1. Read input ─────────────────────────────────────────────────────────
    let source = std::fs::read_to_string(&args.input)
        .map_err(|e| anyhow::anyhow!("Cannot read '{}': {e}", args.input.display()))?;

    // ── 2. Normalise to HTML ──────────────────────────────────────────────────
    let format = render::InputFormat::detect(&args.input)?;
    let html = render::to_html(&source, format);

    // ── 3. Render to PNG ──────────────────────────────────────────────────────
    eprintln!("Rendering…");
    let png = painter::Renderer::new().render_html(&html)?;
    eprintln!("Rendered {} bytes of PNG", png.len());

    // ── 4. Optionally save to disk ────────────────────────────────────────────
    if let Some(out) = &args.output {
        std::fs::write(out, &png)
            .map_err(|e| anyhow::anyhow!("Cannot write '{}': {e}", out.display()))?;
        eprintln!("Saved to {}", out.display());
    }

    // ── 5. Optionally send to printer ─────────────────────────────────────────
    if !args.render_only {
        let server = args.server.as_deref().unwrap();
        eprintln!("Sending to {server}…");
        client::send(server, png, api_key.as_ref())?;
        eprintln!("Done.");
    }

    Ok(())
}
