//! The page a folder gets when it has no page of its own.
//!
//! The product's claim is that any folder is an app. That was true for a folder
//! with a page in it, and true for a folder holding exactly one thing - a lone
//! PDF or photo opens directly - and false for the most ordinary case anybody
//! would try: a folder of photos. Six images and no `index.html` matched none of
//! `choose_entry`'s rules, so the app appeared healthy in the library and 404'd
//! when somebody tapped it.
//!
//! So a folder with no entry gets a listing built here. Two shapes, chosen by
//! what is in the folder: a grid when it is all images, a list otherwise.
//!
//! **Nothing is written to disk.** This is rendered per request, exactly like
//! the live-view tag in `live.rs`, and the moment somebody drops a real
//! `index.html` in the folder it takes over and this is never reached again.
//! Rewriting somebody's folder to make our own routing work is the one thing
//! `authoring.rs` is careful never to do, and this keeps that promise.

use std::path::Path;

/// What the listing shows for one entry.
struct Item {
    /// The on-disk name, used for the href.
    name: String,
    /// A directory gets a trailing slash and no thumbnail.
    is_dir: bool,
    /// Renderable in an `<img>` by every browser we care about.
    is_image: bool,
    /// `None` for directories, which have no meaningful single size.
    bytes: Option<u64>,
}

/// Extensions a browser will draw in an `<img>`.
///
/// Deliberately not `mime_guess`: that answers "what is this", and the question
/// here is the narrower "will a phone render this inline", which excludes
/// formats like TIFF that are images and do not display.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "bmp", "ico",
];

fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Hidden files stay hidden, matching `kt-registry`'s rule and what the rest of
/// the server will actually serve. A listing that offered `.env` would be a
/// listing that invented a way to leak it.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn read_items(dir: &Path) -> Vec<Item> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut items: Vec<Item> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_hidden(&name) {
                return None;
            }
            // `app.json` is ours, not theirs. It is in every folder the daemon
            // has ever scanned and means nothing to a viewer.
            if name == "app.json" {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let is_dir = meta.is_dir();
            Some(Item {
                is_image: !is_dir && is_image(&name),
                bytes: if is_dir { None } else { Some(meta.len()) },
                is_dir,
                name,
            })
        })
        .collect();

    // Folders first, then by name, case-insensitively - the order a file
    // manager would use, because that is the order the owner arranged them in.
    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    items
}

/// `1.2 MB`. Approximate on purpose: the exact byte count of a holiday photo
/// is not information anybody wants.
fn size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("MB", 1024 * 1024), ("KB", 1024), ("B", 1)];
    for (unit, scale) in UNITS {
        if bytes >= scale {
            let value = bytes as f64 / scale as f64;
            return if value >= 10.0 || scale == 1 {
                format!("{} {unit}", value.round() as u64)
            } else {
                format!("{value:.1} {unit}")
            };
        }
    }
    "0 B".to_string()
}

/// Percent-encode a filename for use in an href.
///
/// Small and local rather than a dependency: the set that actually breaks a
/// relative URL is short, and `%` must go first or it would double-encode the
/// escapes added after it.
fn url_encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the folder at `dir`, titled `app_name`.
///
/// `subpath` is where this folder sits under the app root, `""` at the top; it
/// is shown so somebody two folders deep knows where they are, and it is
/// already-escaped display text only - every link here is relative.
pub fn render(app_name: &str, subpath: &str, dir: &Path) -> String {
    let items = read_items(dir);
    // A grid whenever every *file* is an image. Subfolders do not disqualify
    // it: an album arranged into `summer/` and `winter/` is still an album, and
    // demoting it to a text list because it is tidy would be perverse. They
    // render as folder tiles alongside the photos.
    let images = items.iter().filter(|i| i.is_image).count();
    let grid = images > 0 && items.iter().all(|i| i.is_dir || i.is_image);

    let body = if items.is_empty() {
        "<p class=\"empty\">This folder is empty.</p>".to_string()
    } else if grid {
        let cells: String = items
            .iter()
            .map(|item| {
                let href = url_encode(&item.name);
                let label = escape(&item.name);
                if item.is_dir {
                    format!(
                        "<a class=\"tile\" href=\"{href}/\">\
<span class=\"folder\" aria-hidden=\"true\"></span>\
<span class=\"cap\">{label}/</span></a>"
                    )
                } else {
                    format!(
                        "<a class=\"tile\" href=\"{href}\">\
<img src=\"{href}\" alt=\"{label}\" loading=\"lazy\">\
<span class=\"cap\">{label}</span></a>"
                    )
                }
            })
            .collect();
        format!("<div class=\"grid\">{cells}</div>")
    } else {
        let rows: String = items
            .iter()
            .map(|item| {
                let href = url_encode(&item.name);
                let label = escape(&item.name);
                let (slash, meta) = match item.bytes {
                    None => ("/", "folder".to_string()),
                    Some(bytes) => ("", size(bytes)),
                };
                format!(
                    "<a class=\"row\" href=\"{href}{slash}\">\
<span class=\"nm\">{label}{slash}</span><span class=\"mt\">{meta}</span></a>"
                )
            })
            .collect();
        format!("<div class=\"list\">{rows}</div>")
    };

    let heading = if subpath.is_empty() {
        escape(app_name)
    } else {
        format!(
            "{} <span class=\"sub\">/{}</span>",
            escape(app_name),
            escape(subpath)
        )
    };
    let count = match items.len() {
        1 => "1 item".to_string(),
        n => format!("{n} items"),
    };
    let up = if subpath.is_empty() {
        String::new()
    } else {
        "<a class=\"up\" href=\"../\">&larr; up</a>".to_string()
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>{STYLE}</style>
</head>
<body>
<header><div><h1>{heading}</h1><p class="count">{count}</p></div>{up}</header>
<main>{body}</main>
</body>
</html>"#,
        title = escape(app_name),
        STYLE = STYLE,
    )
}

/// Its own palette rather than `pages::STYLE`, which centres a small card in
/// the viewport - the wrong shape entirely for a page that is a wall of
/// thumbnails. The colours are the same tokens.
const STYLE: &str = r#"
:root{--paper:#fbfaf8;--raised:#f0ece4;--ink:#1a1a1a;--ink3:#6b6b6b;
--faint:#a8a091;--border:rgba(0,0,0,.08)}
@media (prefers-color-scheme:dark){:root{--paper:#211f19;--raised:#2b2822;
--ink:#f2ede3;--ink3:#b2ab9d;--faint:#6f6a5e;--border:rgba(255,255,255,.11)}}
*{box-sizing:border-box}
body{margin:0;background:var(--paper);color:var(--ink);
font:400 16px/1.5 system-ui,-apple-system,sans-serif;-webkit-font-smoothing:antialiased}
header{display:flex;align-items:flex-end;justify-content:space-between;gap:16px;
padding:28px 24px 18px;max-width:1100px;margin:0 auto}
h1{margin:0;font-size:22px;letter-spacing:-.02em}
h1 .sub{color:var(--faint);font-weight:400}
.count{margin:4px 0 0;font-size:13px;color:var(--faint)}
.up{color:var(--ink3);text-decoration:none;font-size:14px;white-space:nowrap}
.up:hover{color:var(--ink)}
main{max-width:1100px;margin:0 auto;padding:0 24px 40px}
.grid{display:grid;gap:14px;grid-template-columns:repeat(auto-fill,minmax(150px,1fr))}
.tile{display:block;text-decoration:none;color:inherit}
.tile img{width:100%;aspect-ratio:1;object-fit:cover;border-radius:10px;
background:var(--raised);border:1px solid var(--border);display:block}
.cap{display:block;margin-top:7px;font-size:12.5px;color:var(--ink3);
overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.tile:hover img{border-color:var(--faint)}
.folder{display:block;width:100%;aspect-ratio:1;border-radius:10px;
background:var(--raised);border:1px solid var(--border);position:relative}
.folder::after{content:"";position:absolute;inset:32% 26% 30%;border-radius:4px;
border:2px solid var(--faint)}
.list{border:1px solid var(--border);border-radius:12px;overflow:hidden}
.row{display:flex;justify-content:space-between;gap:16px;padding:12px 16px;
text-decoration:none;color:inherit;border-bottom:1px solid var(--border)}
.row:last-child{border-bottom:0}
.row:hover{background:var(--raised)}
.nm{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.mt{color:var(--faint);font-size:13px;white-space:nowrap}
.empty{color:var(--faint)}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "kt-listing-{}-{}-{tag}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("makes the dir");
            Self(path)
        }
        fn file(&self, name: &str, bytes: &[u8]) -> &Self {
            std::fs::write(self.0.join(name), bytes).expect("writes");
            self
        }
        fn dir(&self, name: &str) -> &Self {
            std::fs::create_dir_all(self.0.join(name)).expect("makes it");
            self
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_folder_of_photos_becomes_a_grid() {
        let dir = Dir::new("photos");
        dir.file("one.png", b"x")
            .file("TWO.JPG", b"x")
            .file("three.webp", b"x");

        let html = render("PNG", "", &dir.0);
        assert!(html.contains("class=\"grid\""), "all images means a grid");
        assert!(!html.contains("class=\"list\""));
        // Every one is both a thumbnail and a link to itself.
        assert!(html.contains("<img src=\"one.png\""));
        assert!(html.contains("href=\"TWO.JPG\""));
        assert_eq!(html.matches("class=\"tile\"").count(), 3);
        assert!(html.contains("3 items"));
    }

    #[test]
    fn an_album_arranged_into_subfolders_is_still_a_grid() {
        let dir = Dir::new("album");
        dir.file("one.png", b"x").dir("summer");

        let html = render("Album", "", &dir.0);
        assert!(
            html.contains("class=\"grid\""),
            "a tidy album is still an album"
        );
        assert!(
            html.contains("class=\"folder\""),
            "the subfolder gets a tile"
        );
        assert!(html.contains("href=\"summer/\""));
    }

    #[test]
    fn a_folder_of_only_subfolders_is_a_list_not_an_empty_grid() {
        let dir = Dir::new("dirs");
        dir.dir("a").dir("b");
        let html = render("Dirs", "", &dir.0);
        assert!(
            html.contains("class=\"list\""),
            "no images, nothing to grid"
        );
    }

    #[test]
    fn a_mixed_folder_becomes_a_list_with_sizes() {
        let dir = Dir::new("mixed");
        dir.file("notes.txt", &[0u8; 2048]).file("photo.png", b"x");

        let html = render("Mixed", "", &dir.0);
        assert!(
            html.contains("class=\"list\""),
            "not all images means a list"
        );
        assert!(!html.contains("class=\"grid\""));
        assert!(html.contains("2.0 KB"), "sizes are shown: {html}");
    }

    #[test]
    fn folders_sort_first_and_are_marked() {
        let dir = Dir::new("nested");
        dir.file("a-file.txt", b"x").dir("zzz-folder");

        let html = render("Nested", "", &dir.0);
        let folder_at = html.find("zzz-folder").expect("folder is listed");
        let file_at = html.find("a-file.txt").expect("file is listed");
        assert!(folder_at < file_at, "folders come first regardless of name");
        assert!(
            html.contains("href=\"zzz-folder/\""),
            "folders get a trailing slash"
        );
        assert!(html.contains("folder</span>"));
    }

    #[test]
    fn hidden_files_and_our_own_manifest_are_never_listed() {
        let dir = Dir::new("hidden");
        dir.file(".env", b"SECRET=1")
            .file("app.json", b"{}")
            .file("visible.png", b"x");

        let html = render("Hidden", "", &dir.0);
        assert!(
            !html.contains(".env"),
            "a listing must not invent a way to leak dotfiles"
        );
        assert!(!html.contains("app.json"), "app.json is ours, not theirs");
        assert!(html.contains("visible.png"));
        assert!(html.contains("1 item"), "and the count agrees: {html}");
    }

    #[test]
    fn names_are_escaped_and_urls_encoded() {
        let dir = Dir::new("nasty");
        // A filename that is also an injection attempt, and one with a space.
        dir.file("<script>.txt", b"x")
            .file("holiday snap.txt", b"x");

        let html = render("Nasty", "", &dir.0);
        assert!(!html.contains("<script>"), "the name must not become a tag");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("%3Cscript%3E.txt"), "and the href is encoded");
        assert!(html.contains("holiday%20snap.txt"));
    }

    #[test]
    fn the_app_name_is_escaped_too() {
        let dir = Dir::new("title");
        dir.file("a.txt", b"x");
        let html = render("<b>Bad</b>", "", &dir.0);
        assert!(!html.contains("<b>Bad</b>"));
        assert!(html.contains("&lt;b&gt;Bad&lt;/b&gt;"));
    }

    #[test]
    fn a_subfolder_says_where_it_is_and_offers_a_way_back() {
        let dir = Dir::new("sub");
        dir.file("a.txt", b"x");
        let html = render("Album", "summer/beach", &dir.0);
        assert!(html.contains("/summer/beach"));
        assert!(html.contains("href=\"../\""), "there is a way up");
        // The top level has nowhere to go up to.
        assert!(!render("Album", "", &dir.0).contains("href=\"../\""));
    }

    #[test]
    fn an_empty_folder_says_so_rather_than_rendering_nothing() {
        let dir = Dir::new("empty");
        let html = render("Empty", "", &dir.0);
        assert!(html.contains("This folder is empty"));
        assert!(!html.contains("class=\"grid\""));
    }

    #[test]
    fn image_detection_is_by_extension_and_case_insensitive() {
        assert!(is_image("a.PNG"));
        assert!(is_image("a.jpeg"));
        assert!(!is_image("a.tiff"), "a browser will not draw it inline");
        assert!(!is_image("png"), "an extensionless file is not an image");
        assert!(!is_image("a.txt"));
    }

    #[test]
    fn sizes_round_the_way_a_human_reads_them() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(2048), "2.0 KB");
        assert_eq!(size(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(size(1024 * 1024 * 12), "12 MB");
    }
}
