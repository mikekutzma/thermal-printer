/// HTML → PNG renderer.
///
/// Walks a simplified HTML DOM produced by comrak (or a raw .html file) and
/// paints each block element onto a 576-px-wide tiny-skia Pixmap using
/// cosmic-text for font shaping and layout.
///
/// Supported elements
/// ──────────────────
/// Block:  h1–h3, p, ul/ol (li), blockquote, pre/code, hr
/// Inline: strong/b, em/i, code, plain text
/// Images: <img src="…"> loaded from disk paths or data: URIs

use std::path::Path;

use cosmic_text::{
    Attrs, AttrsList, Buffer, BufferLine, Color as TColor, Family, FontSystem, Metrics, Shaping,
    Style, SwashCache, Weight,
};
use scraper::{ElementRef, Html, Selector};
use tiny_skia::{Paint, Pixmap, Rect, Transform};

use crate::fonts;

// ── Layout constants ──────────────────────────────────────────────────────────

const WIDTH: u32 = 576;
const PAD: f32 = 24.0;
const INNER: f32 = WIDTH as f32 - PAD * 2.0;

// Font sizes are in pixels. The printer is 203 DPI across 576 dots (80mm).
// 1pt ≈ 2.82px at 203 DPI, so target sizes in points are shown in comments.
const FONT_BODY: f32 = 32.0;  // ~11pt
const LINE_BODY: f32 = 44.0;
const FONT_H1: f32 = 58.0;   // ~21pt
const LINE_H1: f32 = 72.0;
const FONT_H2: f32 = 46.0;   // ~16pt
const LINE_H2: f32 = 58.0;
const FONT_H3: f32 = 38.0;   // ~13pt
const LINE_H3: f32 = 50.0;
const FONT_CODE: f32 = 28.0; // ~10pt
const LINE_CODE: f32 = 40.0;

const SPACE_BEFORE_HEADING: f32 = 24.0;
const SPACE_AFTER_BLOCK: f32 = 18.0;
const BLOCKQUOTE_INDENT: f32 = 32.0;
const BLOCKQUOTE_BAR: f32 = 6.0;
const LIST_INDENT: f32 = 36.0;

// ── Colour helpers ────────────────────────────────────────────────────────────

fn col(r: u8, g: u8, b: u8) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(r, g, b, 255)
}

fn tc(c: tiny_skia::Color) -> TColor {
    TColor::rgba(
        (c.red() * 255.0) as u8,
        (c.green() * 255.0) as u8,
        (c.blue() * 255.0) as u8,
        (c.alpha() * 255.0) as u8,
    )
}

// ── Inline span ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
}

impl Span {
    fn new(text: impl Into<String>, bold: bool, italic: bool, code: bool) -> Self {
        Self { text: text.into(), bold, italic, code }
    }
}

// ── Block types ───────────────────────────────────────────────────────────────

enum Block {
    Heading { level: u8, spans: Vec<Span> },
    Paragraph { spans: Vec<Span> },
    Code { text: String },
    Rule,
    ListItem { ordered: bool, index: usize, spans: Vec<Span>, indent: usize },
    Blockquote { spans: Vec<Span> },
    Image { src: String, alt: String },
    TableRow { cells: Vec<Vec<Span>>, header: bool },
}

// ── Inline text extraction ────────────────────────────────────────────────────

fn extract_spans(el: ElementRef, bold: bool, italic: bool, code: bool) -> Vec<Span> {
    let mut spans = Vec::new();
    for node in el.children() {
        if let Some(text) = node.value().as_text() {
            // Normalise whitespace: collapse runs of whitespace to a single space,
            // but preserve a leading/trailing space so inline elements stay separated.
            let raw: &str = text.as_ref();
            let leading = if raw.starts_with(|c: char| c.is_whitespace()) { " " } else { "" };
            let trailing = if raw.len() > 1 && raw.ends_with(|c: char| c.is_whitespace()) { " " } else { "" };
            let inner: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            if !inner.is_empty() {
                let t = format!("{leading}{inner}{trailing}");
                spans.push(Span::new(t, bold, italic, code));
            }
        } else if let Some(child) = ElementRef::wrap(node) {
            let tag = child.value().name();
            let (b, i, c) = match tag {
                "strong" | "b" => (true, italic, code),
                "em" | "i" => (bold, true, code),
                "code" => (bold, italic, true),
                _ => (bold, italic, code),
            };
            spans.extend(extract_spans(child, b, i, c));
        }
    }
    spans
}

// ── DOM → Block list ──────────────────────────────────────────────────────────

fn parse_blocks(html: &str) -> Vec<Block> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("body > *").unwrap();
    let mut blocks = Vec::new();
    for el in doc.select(&sel) {
        collect_element(el, &mut blocks, 0);
    }
    blocks
}

fn collect_element(el: ElementRef, blocks: &mut Vec<Block>, list_depth: usize) {
    let tag = el.value().name();
    match tag {
        "h1" | "h2" | "h3" => {
            let level = tag[1..].parse::<u8>().unwrap();
            let bold = level <= 2;
            blocks.push(Block::Heading { level, spans: extract_spans(el, bold, false, false) });
        }
        "p" => {
            let img_sel = Selector::parse("img").unwrap();
            if let Some(img) = el.select(&img_sel).next() {
                let src = img.value().attr("src").unwrap_or("").to_string();
                let alt = img.value().attr("alt").unwrap_or("").to_string();
                blocks.push(Block::Image { src, alt });
            } else {
                let spans = extract_spans(el, false, false, false);
                if !spans.is_empty() {
                    blocks.push(Block::Paragraph { spans });
                }
            }
        }
        "ul" | "ol" => {
            let is_ordered = tag == "ol";
            let li_sel = Selector::parse("li").unwrap();
            for (idx, li) in el.select(&li_sel).enumerate() {
                // only direct children — skip nested list items
                if li.parent().map(|p| p.id()) != Some(el.id()) {
                    continue;
                }
                blocks.push(Block::ListItem {
                    ordered: is_ordered,
                    index: idx + 1,
                    spans: extract_spans(li, false, false, false),
                    indent: list_depth,
                });
            }
        }
        "blockquote" => {
            let p_sel = Selector::parse("p").unwrap();
            for p in el.select(&p_sel) {
                blocks.push(Block::Blockquote { spans: extract_spans(p, false, true, false) });
            }
        }
        "pre" => {
            let code_sel = Selector::parse("code").unwrap();
            let text = if let Some(code) = el.select(&code_sel).next() {
                code.text().collect::<String>()
            } else {
                el.text().collect::<String>()
            };
            blocks.push(Block::Code { text });
        }
        "table" => {
            let tr_sel = Selector::parse("tr").unwrap();
            let th_sel = Selector::parse("th").unwrap();
            let td_sel = Selector::parse("td").unwrap();
            for tr in el.select(&tr_sel) {
                let is_header = tr.select(&th_sel).next().is_some();
                let cell_sel = if is_header { &th_sel } else { &td_sel };
                let cells: Vec<Vec<Span>> = tr
                    .select(cell_sel)
                    .map(|cell| extract_spans(cell, is_header, false, false))
                    .collect();
                if !cells.is_empty() {
                    blocks.push(Block::TableRow { cells, header: is_header });
                }
            }
        }
        "hr" => blocks.push(Block::Rule),
        "img" => {
            let src = el.value().attr("src").unwrap_or("").to_string();
            let alt = el.value().attr("alt").unwrap_or("").to_string();
            blocks.push(Block::Image { src, alt });
        }
        _ => {}
    }
}

// ── Row kinds (deferred draw list) ────────────────────────────────────────────

struct RenderedRow {
    y: f32,
    height: f32,
    kind: RowKind,
}

enum RowKind {
    Text { buffer: Buffer, x: f32, color: TColor },
    Rule,
    VertBar { x: f32, height: f32 },
    FilledRect { x: f32, w: f32, color: tiny_skia::Color },
    Image { pixmap: Pixmap },
}

// ── Renderer ──────────────────────────────────────────────────────────────────

pub struct Renderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    rows: Vec<RenderedRow>,
    cursor_y: f32,
}

impl Renderer {
    pub fn new() -> Self {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(fonts::NOTO_SANS_REGULAR.to_vec());
        db.load_font_data(fonts::NOTO_SANS_BOLD.to_vec());
        db.load_font_data(fonts::NOTO_SANS_ITALIC.to_vec());
        db.load_font_data(fonts::NOTO_SANS_BOLD_ITALIC.to_vec());
        let font_system = FontSystem::new_with_locale_and_db("en-US".into(), db);
        Self {
            font_system,
            swash_cache: SwashCache::new(),
            rows: Vec::new(),
            cursor_y: PAD,
        }
    }

    fn shape_text(
        &mut self,
        spans: &[Span],
        font_size: f32,
        line_height: f32,
        max_width: f32,
        color: TColor,
        x: f32,
    ) -> f32 {
        if spans.is_empty() {
            return 0.0;
        }
        let metrics = Metrics::new(font_size, line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(&mut self.font_system, Some(max_width), None);

        let mut full_text = String::new();
        let default_attrs = Attrs::new().family(Family::Name("Noto Sans")).color(color);
        let mut attrs_list = AttrsList::new(default_attrs);

        for span in spans {
            let start = full_text.len();
            full_text.push_str(&span.text);
            let end = full_text.len();
            let weight = if span.bold { Weight::BOLD } else { Weight::NORMAL };
            let style = if span.italic { Style::Italic } else { Style::Normal };
            let family = if span.code { Family::Monospace } else { Family::Name("Noto Sans") };
            attrs_list.add_span(
                start..end,
                Attrs::new().family(family).weight(weight).style(style).color(color),
            );
        }

        buffer.lines.clear();
        buffer.lines.push(BufferLine::new(
            full_text,
            cosmic_text::LineEnding::default(),
            attrs_list,
            Shaping::Advanced,
        ));
        buffer.shape_until_scroll(&mut self.font_system, false);

        let h = buffer
            .layout_runs()
            .last()
            .map(|r| r.line_y + line_height * 0.3)
            .unwrap_or(line_height);

        self.rows.push(RenderedRow {
            y: self.cursor_y,
            height: h,
            kind: RowKind::Text { buffer, x, color },
        });
        h
    }

    fn render_block(&mut self, block: &Block) {
        match block {
            Block::Heading { level, spans } => {
                self.cursor_y += SPACE_BEFORE_HEADING;
                let (fs, lh) = match level {
                    1 => (FONT_H1, LINE_H1),
                    2 => (FONT_H2, LINE_H2),
                    _ => (FONT_H3, LINE_H3),
                };
                let h = self.shape_text(spans, fs, lh, INNER, tc(col(0, 0, 0)), PAD);
                self.cursor_y += h + SPACE_AFTER_BLOCK;
            }
            Block::Paragraph { spans } => {
                let h = self.shape_text(spans, FONT_BODY, LINE_BODY, INNER, tc(col(0, 0, 0)), PAD);
                self.cursor_y += h + SPACE_AFTER_BLOCK;
            }
            Block::Code { text } => {
                let code_text = text.trim_end();
                let code_lines: Vec<&str> = code_text.lines().collect();
                let line_count = code_lines.len().max(1);
                let bg_h = line_count as f32 * LINE_CODE + PAD;

                // Background rect drawn first so text paints on top
                self.rows.push(RenderedRow {
                    y: self.cursor_y,
                    height: bg_h,
                    kind: RowKind::FilledRect { x: PAD, w: INNER, color: col(235, 235, 235) },
                });

                // One BufferLine per source line so \n isn't passed to cosmic-text
                let metrics = Metrics::new(FONT_CODE, LINE_CODE);
                let mut buffer = Buffer::new(&mut self.font_system, metrics);
                buffer.set_size(&mut self.font_system, Some(INNER - PAD * 1.5), None);
                buffer.lines.clear();
                let color = tc(col(0, 0, 0));
                for line in &code_lines {
                    buffer.lines.push(BufferLine::new(
                        line.to_string(),
                        cosmic_text::LineEnding::default(),
                        AttrsList::new(
                            Attrs::new().family(Family::Monospace).color(color),
                        ),
                        Shaping::Advanced,
                    ));
                }
                buffer.shape_until_scroll(&mut self.font_system, false);

                self.rows.push(RenderedRow {
                    y: self.cursor_y + PAD * 0.5,
                    height: bg_h - PAD * 0.5,
                    kind: RowKind::Text { buffer, x: PAD * 1.5, color },
                });

                self.cursor_y += bg_h + SPACE_AFTER_BLOCK;
            }
            Block::Rule => {
                self.cursor_y += 6.0;
                self.rows.push(RenderedRow {
                    y: self.cursor_y,
                    height: 1.0,
                    kind: RowKind::Rule,
                });
                self.cursor_y += 7.0;
            }
            Block::ListItem { ordered, index, spans, indent } => {
                let x_offset = PAD + *indent as f32 * LIST_INDENT;
                let bullet = if *ordered { format!("{}. ", index) } else { "• ".to_string() };
                let bullet_spans = vec![Span::new(bullet, false, false, false)];
                // bullet and text share the same cursor_y — render bullet, then text at same y
                let saved_y = self.cursor_y;
                self.shape_text(&bullet_spans, FONT_BODY, LINE_BODY, LIST_INDENT, tc(col(0, 0, 0)), x_offset);
                self.cursor_y = saved_y;
                let item_w = INNER - (x_offset - PAD) - LIST_INDENT;
                let h = self.shape_text(spans, FONT_BODY, LINE_BODY, item_w, tc(col(0, 0, 0)), x_offset + LIST_INDENT);
                self.cursor_y = saved_y + h + 4.0;
            }
            Block::Blockquote { spans } => {
                let saved_y = self.cursor_y;
                let text_x = PAD + BLOCKQUOTE_INDENT;
                let text_w = INNER - BLOCKQUOTE_INDENT;
                let h = self.shape_text(spans, FONT_BODY, LINE_BODY, text_w, tc(col(60, 60, 60)), text_x);
                // Draw vertical bar at saved_y
                self.rows.push(RenderedRow {
                    y: saved_y,
                    height: h,
                    kind: RowKind::VertBar { x: PAD, height: h },
                });
                self.cursor_y = saved_y + h + SPACE_AFTER_BLOCK;
            }
            Block::TableRow { cells, header } => {
                if cells.is_empty() {
                    return;
                }
                let col_w = INNER / cells.len() as f32;
                let saved_y = self.cursor_y;
                let mut row_h: f32 = LINE_BODY;

                for (i, cell) in cells.iter().enumerate() {
                    let x = PAD + i as f32 * col_w;
                    self.cursor_y = saved_y;
                    let h = self.shape_text(cell, FONT_BODY, LINE_BODY, col_w - 4.0, tc(col(0, 0, 0)), x);
                    row_h = row_h.max(h);
                }

                // Underline header row
                if *header {
                    self.cursor_y = saved_y + row_h + 4.0;
                    self.rows.push(RenderedRow {
                        y: self.cursor_y,
                        height: 2.0,
                        kind: RowKind::Rule,
                    });
                    self.cursor_y += 2.0;
                } else {
                    self.cursor_y = saved_y + row_h + 8.0;
                }
            }
            Block::Image { src, alt } => {
                if let Ok(img) = load_image(src) {
                    let orig_w = img.width();
                    let orig_h = img.height();
                    let (w, h) = if orig_w > WIDTH {
                        let scale = WIDTH as f32 / orig_w as f32;
                        (WIDTH, (orig_h as f32 * scale) as u32)
                    } else {
                        (orig_w, orig_h)
                    };
                    let scaled = image::imageops::resize(
                        &img.to_rgba8(),
                        w,
                        h,
                        image::imageops::FilterType::Triangle,
                    );
                    if let Some(mut pm) = Pixmap::new(w, h) {
                        for (x, y, pixel) in scaled.enumerate_pixels() {
                            let c = tiny_skia::ColorU8::from_rgba(
                                pixel[0], pixel[1], pixel[2], pixel[3],
                            );
                            pm.pixels_mut()[(y * w + x) as usize] = c.premultiply();
                        }
                        let row_h = h as f32;
                        self.rows.push(RenderedRow {
                            y: self.cursor_y,
                            height: row_h,
                            kind: RowKind::Image { pixmap: pm },
                        });
                        self.cursor_y += row_h + SPACE_AFTER_BLOCK;
                    }
                } else if !alt.is_empty() {
                    let spans = vec![Span::new(format!("[{alt}]"), false, true, false)];
                    let h = self.shape_text(&spans, FONT_BODY, LINE_BODY, INNER, tc(col(60, 60, 60)), PAD);
                    self.cursor_y += h + SPACE_AFTER_BLOCK;
                }
            }
        }
    }

    /// Render HTML to a PNG-encoded byte vec.
    pub fn render_html(mut self, html: &str) -> anyhow::Result<Vec<u8>> {
        let blocks = parse_blocks(html);
        for block in &blocks {
            self.render_block(block);
        }
        self.cursor_y += PAD;

        let height = (self.cursor_y.ceil() as u32).max(1);
        let mut pixmap = Pixmap::new(WIDTH, height)
            .ok_or_else(|| anyhow::anyhow!("Failed to allocate pixmap ({WIDTH}×{height})"))?;
        pixmap.fill(tiny_skia::Color::WHITE);

        for row in &self.rows {
            let y = row.y;
            match &row.kind {
                RowKind::Text { buffer, x, color } => {
                    let x = *x;
                    buffer.draw(
                        &mut self.font_system,
                        &mut self.swash_cache,
                        *color,
                        |px, py, w, h, colour| {
                            let fx = px as f32 + x;
                            let fy = py as f32 + y;
                            if let Some(rect) = Rect::from_xywh(fx, fy, w as f32, h as f32) {
                                let mut paint = Paint::default();
                                paint.set_color_rgba8(
                                    colour.r(),
                                    colour.g(),
                                    colour.b(),
                                    colour.a(),
                                );
                                pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                            }
                        },
                    );
                }
                RowKind::Rule => {
                    let mut paint = Paint::default();
                    paint.set_color(col(180, 180, 180));
                    if let Some(rect) = Rect::from_xywh(PAD, y, INNER, 1.0) {
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
                RowKind::VertBar { x, height } => {
                    let mut paint = Paint::default();
                    paint.set_color(col(180, 180, 180));
                    if let Some(rect) = Rect::from_xywh(*x, y, BLOCKQUOTE_BAR, *height) {
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
                RowKind::FilledRect { x, w, color } => {
                    let mut paint = Paint::default();
                    paint.set_color(*color);
                    if let Some(rect) = Rect::from_xywh(*x, y, *w, row.height) {
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
                RowKind::Image { pixmap: src } => {
                    pixmap.draw_pixmap(
                        0,
                        y as i32,
                        src.as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        Transform::identity(),
                        None,
                    );
                }
            }
        }

        Ok(pixmap.encode_png()?)
    }
}

// ── Image loading ─────────────────────────────────────────────────────────────

fn load_image(src: &str) -> anyhow::Result<image::DynamicImage> {
    if src.starts_with("data:") {
        let comma = src.find(',').ok_or_else(|| anyhow::anyhow!("Invalid data URI"))?;
        let encoded = &src[comma + 1..];
        let bytes = data_url_decode(encoded)?;
        let cursor = std::io::Cursor::new(bytes);
        Ok(image::ImageReader::new(cursor).with_guessed_format()?.decode()?)
    } else {
        Ok(image::open(Path::new(src))?)
    }
}

fn data_url_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    const T: [u8; 128] = *b"\
        \xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\
        \xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\
        \xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\x3e\xff\xff\xff\x3f\
        \x34\x35\x36\x37\x38\x39\x3a\x3b\x3c\x3d\xff\xff\xff\xff\xff\xff\
        \xff\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\
        \x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\xff\xff\xff\xff\xff\
        \xff\x1a\x1b\x1c\x1d\x1e\x1f\x20\x21\x22\x23\x24\x25\x26\x27\x28\
        \x29\x2a\x2b\x2c\x2d\x2e\x2f\x30\x31\x32\x33\xff\xff\xff\xff\xff";
    let s: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut i = 0;
    while i + 3 < s.len() {
        let (a, b, c, d) = (T[s[i] as usize], T[s[i+1] as usize], T[s[i+2] as usize], T[s[i+3] as usize]);
        out.push((a << 2) | (b >> 4));
        out.push((b << 4) | (c >> 2));
        out.push((c << 6) | d);
        i += 4;
    }
    Ok(out)
}
