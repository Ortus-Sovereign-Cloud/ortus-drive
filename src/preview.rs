//! Previews: small pictures of images, and the text of documents (for the
//! preview and for search). Both are made once per version and kept in
//! the file interface beside the contents.

use std::io::Read;
use std::sync::Arc;

use ortus_core::api::{answer, ApiError, Method, Route};
use ortus_core::auth::Access;
use ortus_core::instance::Instance;
use ortus_core::storage::BoxFuture;
use rust_web_server::header::Header;
use rust_web_server::prelude::*;
use serde_json::{json, Value};

use crate::api::{drive_error, reach, signed};
use crate::store::{self, DriveError, Item, Role};

/// Files larger than this get no preview text.
const MAX_SOURCE: i64 = 50 * 1024 * 1024;
/// Text kept per file, in characters.
const MAX_TEXT: usize = 200_000;
/// Thumbnails fit in this square.
const THUMB: u32 = 320;

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Text in the elements of an XML document whose local name is in `names`
/// (paragraph-like elements end with a line break).
fn xml_text(xml: &str, names: &[&str], breaks: &[&str]) -> String {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut depth_in = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                let local: String = name.as_ref().to_string();
                if names.contains(&local.as_str()) {
                    depth_in += 1;
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name();
                let local: String = name.as_ref().to_string();
                if names.contains(&local.as_str()) {
                    depth_in = depth_in.saturating_sub(1);
                }
                if breaks.contains(&local.as_str()) {
                    out.push('\n');
                }
            }
            Ok(Event::Text(t)) if depth_in > 0 => out.push_str(t.as_ref()),
            Ok(Event::GeneralRef(r)) if depth_in > 0 => out.push_str(match r.as_ref() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" => "'",
                _ => "",
            }),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if out.len() > MAX_TEXT * 4 {
            break;
        }
    }
    out
}

/// Entries of a zip, by name, read up to a size.
fn zip_entries(bytes: &[u8], wanted: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = archive
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .filter(|n| wanted(n))
        .collect();
    names.sort_by_key(|n| (n.len(), n.clone()));
    let mut out = Vec::new();
    for name in names.into_iter().take(500) {
        if let Ok(file) = archive.by_name(&name) {
            let mut text = String::new();
            // A small entry may claim to unpack into a huge one: read a bounded amount.
            if file
                .take(20 * 1024 * 1024)
                .read_to_string(&mut text)
                .is_ok()
            {
                out.push(text);
            }
        }
    }
    out
}

/// The text of a document, when it has one we can read.
pub fn text_of(name: &str, mime: &str, bytes: &[u8]) -> Option<String> {
    let ext = extension(name);
    let text = match ext.as_str() {
        "txt" | "md" | "csv" | "tsv" | "json" | "log" | "ics" | "vcf" | "eml" | "html" | "htm"
        | "xml" => String::from_utf8_lossy(bytes).into_owned(),
        "docx" => zip_entries(bytes, |n| n == "word/document.xml")
            .iter()
            .map(|x| xml_text(x, &["t"], &["p"]))
            .collect::<Vec<_>>()
            .join("\n"),
        "pptx" => zip_entries(bytes, |n| {
            n.starts_with("ppt/slides/slide") && n.ends_with(".xml")
        })
        .iter()
        .map(|x| xml_text(x, &["t"], &["p"]))
        .collect::<Vec<_>>()
        .join("\n"),
        "xlsx" => zip_entries(bytes, |n| n == "xl/sharedStrings.xml")
            .iter()
            .map(|x| xml_text(x, &["t"], &["si"]))
            .collect::<Vec<_>>()
            .join("\n"),
        "odt" | "ods" | "odp" => zip_entries(bytes, |n| n == "content.xml")
            .iter()
            .map(|x| xml_text(x, &["p", "h", "span"], &["p", "h"]))
            .collect::<Vec<_>>()
            .join("\n"),
        _ if mime.starts_with("text/") => String::from_utf8_lossy(bytes).into_owned(),
        _ => return None,
    };
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(MAX_TEXT)
        .collect();
    let clean = clean.trim().to_string();
    (!clean.is_empty()).then_some(clean)
}

/// A small PNG of an image.
pub fn thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader.decode().ok()?;
    let small = picture.thumbnail(THUMB, THUMB);
    let mut out = Vec::new();
    small
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

fn cache_key(kind: &str, blob: &str) -> String {
    format!("drive-{kind}/{}", blob.replace('/', "_"))
}

/// The text of a file's current version (made and kept on first ask).
pub async fn text(instance: &Instance, item: &Item) -> Option<String> {
    if item.is_folder() || item.size > MAX_SOURCE {
        return None;
    }
    let v = store::version(instance.store.as_ref(), item, None)
        .await
        .ok()??;
    let key = cache_key("text", &v.blob);
    if let Ok(Some(cached)) = instance.files.get(&key).await {
        let t = String::from_utf8_lossy(&cached).into_owned();
        return (!t.is_empty()).then_some(t);
    }
    let bytes = instance.files.get(&v.blob).await.ok()??;
    let (name, mime) = (item.name.clone(), item.mime.clone());
    let found = tokio::task::spawn_blocking(move || text_of(&name, &mime, &bytes))
        .await
        .ok()
        .flatten();
    let _ = instance
        .files
        .put(&key, found.as_deref().unwrap_or_default().as_bytes())
        .await;
    found
}

fn is_image(item: &Item) -> bool {
    item.mime.starts_with("image/")
        || ["png", "jpg", "jpeg", "gif", "webp"].contains(&extension(&item.name).as_str())
}

async fn thumb(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let (item, _) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::View,
    )
    .await?;
    if !is_image(&item) || item.size > MAX_SOURCE {
        return Err(drive_error(DriveError::NotFound));
    }
    let v = store::version(instance.store.as_ref(), &item, None)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    let key = cache_key("thumbs", &v.blob);
    let png = match instance
        .files
        .get(&key)
        .await
        .map_err(|e| drive_error(e.into()))?
    {
        Some(png) => png,
        None => {
            let bytes = instance
                .files
                .get(&v.blob)
                .await
                .map_err(|e| drive_error(e.into()))?
                .unwrap_or_default();
            let png = tokio::task::spawn_blocking(move || thumbnail(&bytes))
                .await
                .ok()
                .flatten()
                .ok_or_else(|| drive_error(DriveError::NotFound))?;
            let _ = instance.files.put(&key, &png).await;
            png
        }
    };
    let header = |n: &str, v: &str| Header {
        name: n.to_string().into(),
        value: v.to_string().into(),
    };
    let body = rust_web_server::range::Range::get_content_range(png, "image/png".into());
    Ok(Response::get_response(
        STATUS_CODE_REASON_PHRASE.n200_ok,
        Some(vec![
            header("X-Content-Type-Options", "nosniff"),
            header("Cache-Control", "private, max-age=3600"),
        ]),
        Some(vec![body]),
    ))
}

async fn text_route(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let (item, _) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::View,
    )
    .await?;
    let found = text(instance, &item).await;
    Ok(ortus_core::api::ok(&json!({ "text": found })))
}

fn h_thumb(
    request: Request,
    params: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, thumb(&request, &params, &instance).await) })
}

fn h_text(
    request: Request,
    params: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, text_route(&request, &params, &instance).await) })
}

fn object() -> Value {
    json!({"type": "object"})
}

pub fn routes() -> Vec<Route> {
    vec![
        Route {
            method: Method::Get,
            path: "/drive/items/:item/thumbnail",
            id: "driveThumbnail",
            tag: "drive",
            summary: "A small PNG of an image file.",
            access: Access::Signed,
            response: object,
            handler: h_thumb,
        },
        Route {
            method: Method::Get,
            path: "/drive/items/:item/text",
            id: "driveText",
            tag: "drive",
            summary: "The text of a document (DOCX, XLSX, PPTX, ODF, text), when it has one.",
            access: Access::Signed,
            response: object,
            handler: h_text,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut z = zip::ZipWriter::new(&mut out);
        for (name, body) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn documents_give_their_text() {
        let docx = zip_with(&[(
            "word/document.xml",
            r#"<w:document xmlns:w="w"><w:body><w:p><w:r><w:t>Shelter agreement</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Clause 4 &amp; the school</w:t></w:r></w:p></w:body></w:document>"#,
        )]);
        assert_eq!(
            Some("Shelter agreement\nClause 4 & the school".to_string()),
            text_of("a.docx", "", &docx)
        );
        let xlsx = zip_with(&[(
            "xl/sharedStrings.xml",
            r#"<sst><si><t>Beds</t></si><si><t>412</t></si></sst>"#,
        )]);
        assert_eq!(Some("Beds\n412".to_string()), text_of("b.xlsx", "", &xlsx));
        let odt = zip_with(&[(
            "content.xml",
            r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><text:p>Gauge 4.2 m</text:p></office:body></office:document-content>"#,
        )]);
        assert_eq!(Some("Gauge 4.2 m".to_string()), text_of("c.odt", "", &odt));
        assert_eq!(
            None,
            text_of("d.bin", "application/octet-stream", b"\x00\x01")
        );
        assert_eq!(None, text_of("report.pdf", "application/pdf", b"%PDF-1.4"));
    }

    #[test]
    fn images_get_thumbnails() {
        let mut png = Vec::new();
        image::RgbImage::from_pixel(1200, 800, image::Rgb([200, 80, 40]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let small = image::load_from_memory(&thumbnail(&png).unwrap()).unwrap();
        assert_eq!((320, 213), (small.width(), small.height()));
        assert!(thumbnail(b"not an image").is_none());
    }
}
