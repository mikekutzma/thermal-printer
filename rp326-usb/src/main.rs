use clap::{ArgGroup, Parser};
use rp326_usb::escpos::{DitherMode, Packet};
use rp326_usb::printer::Printer;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

/// Print to a Rongta RP326 thermal printer over USB.
///
///   rp326-usb Hello world
///   cat receipt.txt | rp326-usb
///   rp326-usb --image photo.jpg
#[derive(Parser)]
#[command(group(ArgGroup::new("input").args(["words", "image"])))]
struct Args {
    /// Text to print, joined with spaces. Reads stdin if omitted.
    words: Vec<String>,

    /// Image file to print (JPEG, PNG, BMP, etc.).
    #[arg(long, short)]
    image: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let payload = if let Some(path) = args.image {
        let img = image::open(&path)
            .map_err(|e| anyhow::anyhow!("Failed to open image '{}': {e}", path.display()))?;

        Packet::new()
            .initialize()
            .image(img, DitherMode::FloydSteinberg)
            .feed(4)
            .cut()
            .into_bytes()
    } else {
        let content = if !args.words.is_empty() {
            args.words.join(" ")
        } else if !io::stdin().is_terminal() {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        } else {
            anyhow::bail!("No input provided. Pass text as arguments, --image <path>, or pipe from stdin.");
        };

        Packet::new()
            .initialize()
            .text(&content)
            .feed(4)
            .cut()
            .into_bytes()
    };

    let printer = Printer::open()?;
    printer.write(&payload)?;
    println!("Printed {} bytes.", payload.len());
    Ok(())
}
