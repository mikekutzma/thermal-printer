use leptos::prelude::*;

// Client-side JS: resizes images to the printer's max width before upload,
// saving transfer time and server-side decode work.
const RESIZE_SCRIPT: &str = r#"
const PRINT_WIDTH = 576;

document.querySelector('form').addEventListener('submit', async (e) => {
    const fileInput = document.querySelector('input[name="file"]');
    const file = fileInput.files[0];
    if (!file || !file.type.startsWith('image/')) return;

    e.preventDefault();

    const img = new Image();
    img.onload = () => {
        URL.revokeObjectURL(img.src);

        const scale = Math.min(1, PRINT_WIDTH / img.naturalWidth);
        const canvas = document.createElement('canvas');
        canvas.width  = Math.round(img.naturalWidth  * scale);
        canvas.height = Math.round(img.naturalHeight * scale);
        canvas.getContext('2d').drawImage(img, 0, 0, canvas.width, canvas.height);

        canvas.toBlob(async (blob) => {
            const fd = new FormData(e.target);
            fd.set('file', blob, file.name);
            const res = await fetch('/print', { method: 'POST', body: fd });
            document.open();
            document.write(await res.text());
            document.close();
        }, 'image/jpeg', 0.9);
    };
    img.src = URL.createObjectURL(file);
});
"#;

#[component]
pub fn UploadPage() -> impl IntoView {
    view! {
        <html lang="en">
            <head>
                <meta charset="UTF-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1.0"/>
                <title>"Thermal Printer"</title>
            </head>
            <body>
                <h1>"Thermal Printer"</h1>
                <form method="post" action="/print" enctype="multipart/form-data">
                    <input type="file" name="file"/>
                    <br/><br/>

                    <details>
                        <summary>"Advanced options"</summary>
                        <fieldset style="margin-top: 0.5em; display: inline-block">
                            <legend>"Image quality"</legend>
                            <label>
                                <input type="radio" name="quality" value="high" checked/>
                                " High (Floyd-Steinberg dithering)"
                            </label>
                            <br/>
                            <label>
                                <input type="radio" name="quality" value="normal"/>
                                " Normal (threshold)"
                            </label>
                        </fieldset>
                    </details>

                    <br/><br/>
                    <button type="submit">"Print"</button>
                </form>
                <script inner_html=RESIZE_SCRIPT/>
            </body>
        </html>
    }
}

#[component]
pub fn ResultPage(#[prop(into)] message: String, success: bool) -> impl IntoView {
    view! {
        <html lang="en">
            <head>
                <meta charset="UTF-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1.0"/>
                <title>"Thermal Printer"</title>
            </head>
            <body>
                <h1>{if success { "Printed!" } else { "Error" }}</h1>
                <p>{message}</p>
                <a href="/">"← Print another"</a>
            </body>
        </html>
    }
}
