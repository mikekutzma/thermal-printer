use std::path::Path;

use comrak::{markdown_to_html, Options};

pub enum InputFormat {
    Markdown,
    Html,
}

impl InputFormat {
    pub fn detect(path: &Path) -> anyhow::Result<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("md") | Some("markdown") => Ok(Self::Markdown),
            Some("html") | Some("htm") => Ok(Self::Html),
            other => anyhow::bail!(
                "Unsupported file extension: {}.  Use .md, .markdown, .html, or .htm",
                other.unwrap_or("(none)")
            ),
        }
    }
}

/// Convert the input file contents to an HTML string ready for rendering.
pub fn to_html(source: &str, format: InputFormat) -> String {
    match format {
        InputFormat::Html => source.to_string(),
        InputFormat::Markdown => {
            let mut opts = Options::default();
            opts.extension.strikethrough = true;
            opts.extension.table = true;
            opts.extension.autolink = true;
            opts.extension.tasklist = true;
            opts.render.unsafe_ = true; // allow raw HTML blocks inside .md
            markdown_to_html(source, &opts)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_heading() {
        let html = to_html("# Hello", InputFormat::Markdown);
        assert!(html.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn html_passthrough() {
        let src = "<p>hello</p>";
        assert_eq!(to_html(src, InputFormat::Html), src);
    }

    #[test]
    fn detect_md() {
        assert!(matches!(
            InputFormat::detect(Path::new("doc.md")).unwrap(),
            InputFormat::Markdown
        ));
    }

    #[test]
    fn detect_html() {
        assert!(matches!(
            InputFormat::detect(Path::new("doc.html")).unwrap(),
            InputFormat::Html
        ));
    }

    #[test]
    fn detect_unknown_errors() {
        assert!(InputFormat::detect(Path::new("doc.txt")).is_err());
    }
}
