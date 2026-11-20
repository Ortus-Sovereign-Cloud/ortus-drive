//! The drive's JSON API: folders and files, uploads in parts, downloads
//! with ranges (streamed, never whole in memory), names, moves, copies,
//! trash, versions, stars and the quota.

use std::sync::Arc;

use ortus_core::api::{answer, body, ok, ApiError, Method, Route};
use ortus_core::auth::Access;
use ortus_core::instance::Instance;
use ortus_core::storage::BoxFuture;
use rust_web_server::header::Header;
use rust_web_server::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::store::{self, DriveError, Item, Role};

/// Largest part of an upload.
pub const MAX_PART: usize = 16 * 1024 * 1024;
/// Largest file.
pub const MAX_FILE: i64 = 10 * 1024 * 1024 * 1024;
/// Parts of an upload, at most.
const MAX_PARTS: i64 = 10_000;
/// Downloads are read from the store this much at a time.
const READ_CHUNK: u64 = 1024 * 1024;

pub(crate) fn query(request: &Request, name: &str) -> Option<String> {
    let q = request.request_uri.split_once('?')?.1;
    q.split('&').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k == name).then(|| percent(v))
    })
}

fn percent(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok();
            if let Some(x) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(crate) async fn signed(
    request: &Request,
    instance: &Arc<Instance>,
) -> Result<ortus_core::auth::accounts::User, ApiError> {
    Ok(ortus_core::auth::current(request, instance)
        .await
        .ok_or_else(ApiError::sign_in)?
        .user)
}

pub(crate) fn drive_error(e: DriveError) -> ApiError {
    match e {
        DriveError::NotFound => ApiError::not_found("There is no such file or folder."),
        DriveError::Forbidden => ApiError::forbidden(),
        DriveError::NameTaken => ApiError::conflict("Something there already has that name."),
        DriveError::BadName => {
            ApiError::bad_request("Names are 1 to 255 characters, without / or \\.")
        }
        DriveError::OverQuota => ApiError {
            status: 507,
            code: "quota",
            message: "There is no room left in your drive.".into(),
        },
        DriveError::BadMove => ApiError::conflict("It cannot go there."),
        DriveError::Store(m) => {
            eprintln!("ortus: drive: {m}");
            ApiError::internal()
        }
    }
}

/// An item the person may see at least `need` of; others get "not found".
pub(crate) async fn reach(
    instance: &Instance,
    user_id: &str,
    id: &str,
    need: Role,
) -> Result<(Item, Role), ApiError> {
    let s = instance.store.as_ref();
    let item = store::item(s, id)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    match store::role(s, user_id, &item).await? {
        Some(role) if role >= need => Ok((item, role)),
        Some(_) => Err(ApiError::forbidden()),
        None => Err(drive_error(DriveError::NotFound)),
    }
}

/// The names people see for those who changed things.
async fn names(instance: &Instance, ids: &[&str]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for id in ids {
        if out.contains_key(*id) {
            continue;
        }
        if let Ok(Some(u)) = ortus_core::auth::accounts::by_id(instance.store.as_ref(), id).await {
            out.insert(id.to_string(), u.name);
        }
    }
    out
}

pub(crate) fn item_json(
    item: &Item,
    role: Option<Role>,
    people: &std::collections::HashMap<String, String>,
    me: &str,
) -> Value {
    json!({
        "id": item.id,
        "parent": item.parent_id,
        "kind": item.kind,
        "name": item.name,
        "size": item.size,
        "mime": item.mime,
        "version": item.version,
        "label": item.label,
        "source": item.source,
        "owner": if item.owner_id == me { None } else { people.get(&item.owner_id).cloned() },
        "mine": item.owner_id == me,
        "modifiedAt": item.modified_at,
        "modifiedBy": if item.modified_by == me { None } else { people.get(&item.modified_by).cloned() },
        "trashedAt": item.trashed_at,
        "role": role.map(Role::name),
    })
}

async fn items_json(
    instance: &Instance,
    me: &str,
    items: &[Item],
    role: Option<Role>,
) -> Vec<Value> {
    let ids: Vec<&str> = items
        .iter()
        .flat_map(|i| [i.owner_id.as_str(), i.modified_by.as_str()])
        .collect();
    let people = names(instance, &ids).await;
    items
        .iter()
        .map(|i| item_json(i, role, &people, me))
        .collect()
}

/// Each item's sharing in short, for the list.
async fn with_access(
    instance: &Instance,
    mut out: Vec<Value>,
    items: &[Item],
) -> Result<Vec<Value>, ApiError> {
    for (v, item) in out.iter_mut().zip(items) {
        v["access"] = crate::share::access(instance, &item.id).await?;
    }
    Ok(out)
}

/// A folder's contents with the way to it, or the person's own root.
async fn list(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let s = instance.store.as_ref();
    let parent = query(request, "parent").unwrap_or_default();
    if parent.is_empty() {
        let items = store::children(s, &me.id, store::ROOT).await?;
        let out = with_access(
            instance,
            items_json(instance, &me.id, &items, Some(Role::Owner)).await,
            &items,
        )
        .await?;
        return Ok(ok(&json!({ "items": out, "path": [], "role": "owner" })));
    }
    let (folder, role) = reach(instance, &me.id, &parent, Role::View).await?;
    if !folder.is_folder() {
        return Err(drive_error(DriveError::NotFound));
    }
    let items = store::children(s, &folder.owner_id, &folder.id).await?;
    // The way down, as far as the person can see it.
    let mut path = vec![json!({"id": folder.id, "name": folder.name})];
    for a in store::ancestors(s, &folder).await? {
        if store::role(s, &me.id, &a).await?.is_none() {
            break;
        }
        path.push(json!({"id": a.id, "name": a.name}));
    }
    path.reverse();
    Ok(ok(
        &json!({ "items": with_access(instance, items_json(instance, &me.id, &items, Some(role)).await, &items).await?, "path": path, "role": role.name() }),
    ))
}

async fn get_item(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let (item, role) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::View,
    )
    .await?;
    let mut v = items_json(instance, &me.id, std::slice::from_ref(&item), Some(role))
        .await
        .remove(0);
    v["starred"] = json!(store::is_starred(instance.store.as_ref(), &me.id, &item.id).await?);
    v["effectiveLabel"] = json!(store::effective_label(instance.store.as_ref(), &item).await?);
    Ok(ok(&v))
}

#[derive(Deserialize)]
struct NewFolder {
    #[serde(default)]
    parent: String,
    name: String,
}

async fn add_folder(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: NewFolder = body(request)?;
    let folder = store::create_folder(
        instance.store.as_ref(),
        &me.id,
        &input.parent,
        input.name.trim(),
    )
    .await
    .map_err(drive_error)?;
    crate::index::item_changed(instance, &folder).await;
    Ok(ok(&items_json(
        instance,
        &me.id,
        &[folder],
        Some(Role::Owner),
    )
    .await[0]))
}

#[derive(Deserialize)]
struct NewUpload {
    #[serde(default)]
    parent: String,
    name: String,
    size: i64,
    #[serde(default)]
    mime: String,
}

pub(crate) fn safe_mime(mime: &str) -> String {
    let m = mime.trim().to_ascii_lowercase();
    if !m.is_empty()
        && m.len() <= 100
        && m.contains('/')
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/.+-_".contains(c))
    {
        m
    } else {
        "application/octet-stream".into()
    }
}

/// Starts an upload: answers its id; the parts follow, then `finish`.
async fn start_upload(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: NewUpload = body(request)?;
    if !store::name_ok(&input.name) {
        return Err(drive_error(DriveError::BadName));
    }
    if input.size < 0 || input.size > MAX_FILE {
        return Err(ApiError::bad_request("A file is at most 10 GB."));
    }
    let s = instance.store.as_ref();
    // Room and a place to put it, checked now and again at the end.
    if !input.parent.is_empty() {
        reach(instance, &me.id, &input.parent, Role::Edit).await?;
    }
    let owner = if input.parent.is_empty() {
        me.id.clone()
    } else {
        store::item(s, &input.parent)
            .await?
            .map(|p| p.owner_id)
            .unwrap_or_default()
    };
    // Uploads under way count against the quota too, and are few at a time.
    let pending = s
        .query("SELECT COUNT(*), CAST(COALESCE(SUM(size), 0) AS BIGINT) FROM drive_uploads WHERE user_id = $1", &[me.id.as_str().into()])
        .await?;
    let (open, pending_bytes) = pending
        .first()
        .map(|r| (r.int(0).unwrap_or(0), r.int(1).unwrap_or(0)))
        .unwrap_or((0, 0));
    if open >= 20 {
        return Err(ApiError::too_many("Finish or cancel some uploads first."));
    }
    if !store::fits(s, &owner, input.size + pending_bytes).await? {
        return Err(drive_error(DriveError::OverQuota));
    }
    let id = format!("u{}", ortus_core::auth::random_hex(16));
    let parts = (input.size + MAX_PART as i64 - 1) / MAX_PART as i64;
    s.execute(
        "INSERT INTO drive_uploads (id, user_id, parent_id, name, mime, size, parts, received, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8)",
        &[id.as_str().into(), me.id.as_str().into(), input.parent.as_str().into(), input.name.as_str().into(), safe_mime(&input.mime).into(), input.size.into(), parts.max(1).into(), ortus_core::program::now_ms().into()],
    )
    .await?;
    Ok(ok(&json!({ "id": id, "partSize": MAX_PART })))
}

struct Upload {
    id: String,
    parent: String,
    name: String,
    mime: String,
    size: i64,
}

async fn upload_of(instance: &Instance, user_id: &str, id: &str) -> Result<Upload, ApiError> {
    let rows = instance.store.query("SELECT id, parent_id, name, mime, size FROM drive_uploads WHERE id = $1 AND user_id = $2", &[id.into(), user_id.into()]).await?;
    let r = rows
        .first()
        .ok_or_else(|| ApiError::not_found("There is no such upload."))?;
    Ok(Upload {
        id: r.text(0).unwrap_or_default().into(),
        parent: r.text(1).unwrap_or_default().into(),
        name: r.text(2).unwrap_or_default().into(),
        mime: r.text(3).unwrap_or_default().into(),
        size: r.int(4).unwrap_or(0),
    })
}

fn part_key(upload: &str, n: i64) -> String {
    format!("drive-uploads/{upload}/{n:06}")
}

/// One part of an upload (the body is the bytes); sent again, it replaces.
async fn put_part(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let upload = upload_of(instance, &me.id, params.get("upload").unwrap_or_default()).await?;
    let n: i64 = params
        .get("part")
        .and_then(|p| p.parse().ok())
        .filter(|n| (0..MAX_PARTS).contains(n))
        .ok_or_else(|| ApiError::bad_request("Parts are numbered from 0."))?;
    let bytes = request.body.as_bytes();
    if bytes.len() > MAX_PART {
        return Err(ApiError::bad_request("A part is at most 16 MB."));
    }
    // Only the parts the declared size needs, each at most its share.
    let parts = (upload.size + MAX_PART as i64 - 1) / MAX_PART as i64;
    let expected = (upload.size - n * MAX_PART as i64).min(MAX_PART as i64);
    if n >= parts.max(1) || bytes.len() as i64 > expected.max(0) {
        return Err(ApiError::bad_request("That part is not in this upload."));
    }
    instance
        .files
        .put(&part_key(&upload.id, n), bytes)
        .await
        .map_err(|e| drive_error(e.into()))?;
    Ok(ok(&json!({ "part": n, "size": bytes.len() })))
}

/// Puts the parts together into the file (a new version if it exists).
async fn finish_upload(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let upload = upload_of(instance, &me.id, params.get("upload").unwrap_or_default()).await?;
    let parts = instance
        .files
        .list(&format!("drive-uploads/{}/", upload.id))
        .await
        .map_err(|e| drive_error(e.into()))?;
    let s = instance.store.as_ref();
    let owner = if upload.parent.is_empty() {
        me.id.clone()
    } else {
        store::item(s, &upload.parent)
            .await?
            .map(|p| p.owner_id)
            .ok_or_else(|| drive_error(DriveError::NotFound))?
    };
    let blob = store::blob_key(&owner);
    let mut writer = instance
        .files
        .writer(&blob)
        .await
        .map_err(|e| drive_error(e.into()))?;
    let mut size: i64 = 0;
    for (i, key) in parts.iter().enumerate() {
        if *key != part_key(&upload.id, i as i64) {
            let _ = writer.abort().await;
            return Err(ApiError::bad_request(format!("Part {i} is missing.")));
        }
        // Whatever goes wrong, the half-written file is given up.
        let bytes = match instance.files.get(key).await {
            Ok(b) => b.unwrap_or_default(),
            Err(e) => {
                let _ = writer.abort().await;
                return Err(drive_error(e.into()));
            }
        };
        size += bytes.len() as i64;
        if size > MAX_FILE {
            let _ = writer.abort().await;
            return Err(ApiError::bad_request("A file is at most 10 GB."));
        }
        if let Err(e) = writer.write(&bytes).await {
            let _ = writer.abort().await;
            return Err(drive_error(e.into()));
        }
    }
    if size != upload.size {
        let _ = writer.abort().await;
        return Err(ApiError::bad_request(format!(
            "The parts make {size} bytes, not {}.",
            upload.size
        )));
    }
    writer.close().await.map_err(|e| drive_error(e.into()))?;
    let added = store::add_file(
        s,
        &me.id,
        &upload.parent,
        &store::Upload {
            name: &upload.name,
            mime: &upload.mime,
            blob: &blob,
            size,
            source: None,
        },
    )
    .await;
    // The parts are done with, whatever happened.
    for key in &parts {
        let _ = instance.files.delete(key).await;
    }
    s.execute(
        "DELETE FROM drive_uploads WHERE id = $1",
        &[upload.id.as_str().into()],
    )
    .await?;
    let item = match added {
        Ok(item) => item,
        Err(e) => {
            let _ = instance.files.delete(&blob).await;
            return Err(drive_error(e));
        }
    };
    let role = store::role(s, &me.id, &item).await?;
    crate::index::item_changed(instance, &item).await;
    Ok(ok(&items_json(instance, &me.id, &[item], role).await[0]))
}

/// Gives up an upload.
async fn cancel_upload(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let upload = upload_of(instance, &me.id, params.get("upload").unwrap_or_default()).await?;
    for key in instance
        .files
        .list(&format!("drive-uploads/{}/", upload.id))
        .await
        .map_err(|e| drive_error(e.into()))?
    {
        let _ = instance.files.delete(&key).await;
    }
    instance
        .store
        .execute(
            "DELETE FROM drive_uploads WHERE id = $1",
            &[upload.id.as_str().into()],
        )
        .await?;
    Ok(ok(&json!({})))
}

/// Uploads not finished in a day are given up (a periodic job).
pub async fn sweep_uploads(instance: &Instance) -> Result<(), String> {
    let before = ortus_core::program::now_ms() - 86_400_000;
    let rows = instance
        .store
        .query(
            "SELECT id FROM drive_uploads WHERE created_at < $1 LIMIT 200",
            &[before.into()],
        )
        .await
        .map_err(|e| e.to_string())?;
    for r in rows {
        let id = r.text(0).unwrap_or_default().to_string();
        for key in instance
            .files
            .list(&format!("drive-uploads/{id}/"))
            .await
            .map_err(|e| e.to_string())?
        {
            let _ = instance.files.delete(&key).await;
        }
        instance
            .store
            .execute(
                "DELETE FROM drive_uploads WHERE id = $1",
                &[id.as_str().into()],
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    store::empty_old_trash(instance.store.as_ref(), &instance.files, 30)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// A file's contents, read in parts as the client takes them.
pub(crate) struct Stream {
    rx: tokio::sync::mpsc::Receiver<Result<Vec<u8>, String>>,
    chunk: Vec<u8>,
    pos: usize,
}

impl std::io::Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.chunk.len() {
            // Read on a blocking thread (the server's), never an async worker.
            match self.rx.blocking_recv() {
                Some(Ok(next)) => {
                    self.chunk = next;
                    self.pos = 0;
                }
                Some(Err(e)) => return Err(std::io::Error::other(e)),
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.chunk.len() - self.pos);
        out[..n].copy_from_slice(&self.chunk[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Streams bytes `start..end` of the blob, read on the core's I/O runtime.
pub(crate) fn stream(instance: &Arc<Instance>, blob: &str, start: u64, end: u64) -> Stream {
    // An async channel: a slow client makes the reader wait, without
    // holding one of the I/O runtime's threads.
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let (instance, blob) = (Arc::clone(instance), blob.to_string());
    ortus_core::runtime::io().spawn(async move {
        let mut at = start;
        while at < end {
            let length = READ_CHUNK.min(end - at);
            let part = match instance.files.read_range(&blob, at, length).await {
                Ok(Some(bytes)) if !bytes.is_empty() => bytes,
                Ok(_) => break,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string())).await;
                    break;
                }
            };
            at += part.len() as u64;
            // The client went away: stop reading.
            if tx.send(Ok(part)).await.is_err() {
                break;
            }
        }
    });
    Stream {
        rx,
        chunk: Vec::new(),
        pos: 0,
    }
}

/// One byte range from a Range header (`bytes=a-b`, `bytes=a-`, `bytes=-n`).
/// Always a non-empty range inside the file.
pub(crate) fn range(request: &Request, size: u64) -> Option<Result<(u64, u64), ()>> {
    let value = request
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("range"))?
        .value
        .to_string();
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return Some(Err(()));
    }
    let (a, b) = spec.split_once('-')?;
    let r = match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
        (Some(a), Some(b)) if a <= b && a < size => Ok((a, b.saturating_add(1).min(size))),
        (Some(a), None) if b.trim().is_empty() && a < size => Ok((a, size)),
        (None, Some(n)) if a.trim().is_empty() && n > 0 && size > 0 => {
            Ok((size.saturating_sub(n), size))
        }
        _ => Err(()),
    };
    Some(r)
}

fn header(name: &str, value: String) -> Header {
    Header {
        name: name.to_string().into(),
        value: value.into(),
    }
}

/// A file as a download: never shown as a page of this site.
pub(crate) fn file_response(
    request: &Request,
    instance: &Arc<Instance>,
    name: &str,
    mime: &str,
    blob: &str,
    size: u64,
    inline: bool,
) -> Response {
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "._- ()".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    // Shown in the page only for kinds that cannot run anything.
    let viewable = inline
        && [
            "image/png",
            "image/jpeg",
            "image/gif",
            "image/webp",
            "application/pdf",
            "text/plain",
        ]
        .contains(&mime);
    let disposition = if viewable { "inline" } else { "attachment" };
    let mut headers = vec![
        header(
            "Content-Disposition",
            format!("{disposition}; filename=\"{safe_name}\"; filename*=UTF-8''{encoded}"),
        ),
        header("X-Content-Type-Options", "nosniff".into()),
        header(
            "Content-Security-Policy",
            "sandbox; default-src 'none'; img-src 'self'; style-src 'unsafe-inline'".into(),
        ),
        header("Cache-Control", "private, no-store".into()),
        header("Accept-Ranges", "bytes".into()),
        header(
            "Content-Type",
            if viewable {
                if mime == "text/plain" {
                    "text/plain; charset=utf-8".into()
                } else {
                    mime.to_string()
                }
            } else {
                "application/octet-stream".into()
            },
        ),
    ];
    let (status, start, end) = match range(request, size) {
        None => (STATUS_CODE_REASON_PHRASE.n200_ok, 0, size),
        Some(Ok((a, b))) => {
            headers.push(header(
                "Content-Range",
                format!("bytes {a}-{}/{size}", b - 1),
            ));
            (STATUS_CODE_REASON_PHRASE.n206_partial_content, a, b)
        }
        Some(Err(())) => {
            headers.push(header("Content-Range", format!("bytes */{size}")));
            return Response::get_response(
                STATUS_CODE_REASON_PHRASE.n416_range_not_satisfiable,
                Some(headers),
                None,
            );
        }
    };
    headers.push(header("Content-Length", (end - start).to_string()));
    let mut response = Response::get_response(status, Some(headers), None);
    response.stream_pipe = Some(Box::new(stream(instance, blob, start, end)));
    response
}

/// Downloads a file (?version=n for an older one; ?inline=1 to view).
async fn content(
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
    if item.is_folder() {
        return Err(drive_error(DriveError::NotFound));
    }
    let s = instance.store.as_ref();
    if item.mime == crate::service::DOC_MIME {
        return Err(ApiError::conflict("Open it in Docs."));
    }
    let version = query(request, "version").and_then(|v| v.parse().ok());
    let v = store::version(s, &item, version)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    store::touch(s, &me.id, &item.id).await?;
    Ok(file_response(
        request,
        instance,
        &item.name,
        &item.mime,
        &v.blob,
        v.size.max(0) as u64,
        query(request, "inline").as_deref() == Some("1"),
    ))
}

#[derive(Deserialize)]
struct Change {
    name: Option<String>,
    parent: Option<String>,
    starred: Option<bool>,
}

async fn change(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: Change = body(request)?;
    let id = params.get("item").unwrap_or_default();
    let s = instance.store.as_ref();
    if let Some(on) = input.starred {
        reach(instance, &me.id, id, Role::View).await?;
        store::star(s, &me.id, id, on).await?;
    }
    if input.name.is_some() || input.parent.is_some() {
        let (mut item, _) = reach(instance, &me.id, id, Role::Edit).await?;
        if let Some(name) = &input.name {
            item = store::rename(s, &me.id, &item, name.trim())
                .await
                .map_err(drive_error)?;
        }
        if let Some(parent) = &input.parent {
            item = store::move_to(s, &me.id, &item, parent)
                .await
                .map_err(drive_error)?;
            // A stricter folder takes away the shares it does not allow.
            crate::share::recheck(instance, &me.id, &item).await?;
        }
        crate::index::item_changed(instance, &item).await;
    }
    let (item, role) = reach(instance, &me.id, id, Role::View).await?;
    Ok(ok(
        &items_json(instance, &me.id, &[item], Some(role)).await[0]
    ))
}

/// To the trash; from the trash (owner) with ?forever=1, for good.
async fn remove(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let id = params.get("item").unwrap_or_default();
    let s = instance.store.as_ref();
    if query(request, "forever").as_deref() == Some("1") {
        let (item, _) = reach(instance, &me.id, id, Role::Owner).await?;
        if item.trashed_at.is_none() {
            return Err(ApiError::conflict("Put it in the trash first."));
        }
        store::destroy(s, &instance.files, &item).await?;
        crate::index::item_removed(instance, &item.id).await;
        return Ok(ok(&json!({})));
    }
    let (item, _) = reach(instance, &me.id, id, Role::Edit).await?;
    store::trash(s, &item).await?;
    crate::index::item_removed(instance, &item.id).await;
    Ok(ok(&json!({})))
}

async fn restore(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let (item, _) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::Owner,
    )
    .await?;
    let back = store::restore(instance.store.as_ref(), &item)
        .await
        .map_err(drive_error)?;
    crate::share::recheck(instance, &me.id, &back).await?;
    crate::index::tree_changed(instance, &back).await;
    Ok(ok(&items_json(
        instance,
        &me.id,
        &[back],
        Some(Role::Owner),
    )
    .await[0]))
}

#[derive(Deserialize)]
struct CopyTo {
    #[serde(default)]
    parent: String,
}

/// A copy of a file (the current version) into a folder one may add to.
async fn copy(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: CopyTo = body(request)?;
    let (item, _) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::View,
    )
    .await?;
    if item.is_folder() {
        return Err(ApiError::bad_request("Folders are copied file by file."));
    }
    let s = instance.store.as_ref();
    let v = store::version(s, &item, None)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    // Allowed there, and room for it, before anything is written.
    let owner = store::place(s, &me.id, &input.parent)
        .await
        .map_err(drive_error)?;
    if !store::fits(s, &owner, v.size).await? {
        return Err(drive_error(DriveError::OverQuota));
    }
    // The copy gets its own contents, so each can go its own way.
    let blob = store::blob_key(&owner);
    let mut writer = instance
        .files
        .writer(&blob)
        .await
        .map_err(|e| drive_error(e.into()))?;
    let mut at = 0u64;
    while at < v.size as u64 {
        let part = match instance.files.read_range(&v.blob, at, 8 * READ_CHUNK).await {
            Ok(p) => p.unwrap_or_default(),
            Err(e) => {
                let _ = writer.abort().await;
                return Err(drive_error(e.into()));
            }
        };
        if part.is_empty() {
            break;
        }
        at += part.len() as u64;
        if let Err(e) = writer.write(&part).await {
            let _ = writer.abort().await;
            return Err(drive_error(e.into()));
        }
    }
    writer.close().await.map_err(|e| drive_error(e.into()))?;
    let name = format!("Copy of {}", item.name);
    let name: String = name.chars().take(255).collect();
    let copy = match store::add_file(
        s,
        &me.id,
        &input.parent,
        &store::Upload {
            name: &name,
            mime: &item.mime,
            blob: &blob,
            size: v.size,
            source: None,
        },
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            let _ = instance.files.delete(&blob).await;
            return Err(drive_error(e));
        }
    };
    // The copy keeps the classification of what it was copied from.
    if let Some(label) = store::effective_label(s, &item).await? {
        store::set_label(s, &copy.id, Some(&label)).await?;
    }
    let copy = store::item(s, &copy.id)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    crate::index::item_changed(instance, &copy).await;
    let role = store::role(s, &me.id, &copy).await?;
    Ok(ok(&items_json(instance, &me.id, &[copy], role).await[0]))
}

async fn versions(
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
    let list = store::versions(instance.store.as_ref(), &item.id).await?;
    let ids: Vec<&str> = list.iter().map(|v| v.created_by.as_str()).collect();
    let people = names(instance, &ids).await;
    let out: Vec<Value> = list.iter().map(|v| json!({"version": v.version, "size": v.size, "createdAt": v.created_at, "createdBy": people.get(&v.created_by)})).collect();
    Ok(ok(&json!({ "versions": out, "current": item.version })))
}

async fn restore_version(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let (item, role) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::Edit,
    )
    .await?;
    let version: i64 = params
        .get("version")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ApiError::bad_request("Which version?"))?;
    let back = store::restore_version(instance.store.as_ref(), &me.id, &item, version)
        .await
        .map_err(drive_error)?;
    crate::index::item_changed(instance, &back).await;
    Ok(ok(
        &items_json(instance, &me.id, &[back], Some(role)).await[0]
    ))
}

/// Recent, starred or trash: ?which=recent|starred|trash.
async fn special(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let s = instance.store.as_ref();
    let which = query(request, "which").unwrap_or_default();
    let found = match which.as_str() {
        "recent" => store::recent(s, &me.id, 50).await?,
        "starred" => store::starred(s, &me.id).await?,
        "trash" => store::trashed(s, &me.id).await?,
        "email" => store::from_source(s, &me.id, "mail").await?,
        _ => {
            return Err(ApiError::bad_request(
                "?which= is recent, starred, trash or email.",
            ))
        }
    };
    // Only what the person may still see.
    let mut items = Vec::new();
    for item in found {
        if let Some(role) = store::role(s, &me.id, &item).await? {
            items.push((item, role));
        }
    }
    let list: Vec<Item> = items.iter().map(|(i, _)| i.clone()).collect();
    let mut out = items_json(instance, &me.id, &list, None).await;
    for (v, (item, role)) in out.iter_mut().zip(&items) {
        v["role"] = json!(role.name());
        if item.source.is_some() {
            v["note"] = json!(store::note(s, &item.id).await?);
        }
    }
    Ok(ok(&json!({ "items": out })))
}

async fn quota(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let s = instance.store.as_ref();
    Ok(ok(
        &json!({ "used": store::used(s, &me.id).await?, "quota": store::quota(s, &me.id).await? }),
    ))
}

macro_rules! plain {
    ($name:ident, $f:ident) => {
        fn $name(
            request: Request,
            _: PathParams,
            _: ConnectionInfo,
            instance: Arc<Instance>,
        ) -> BoxFuture<'static, Response> {
            Box::pin(async move { answer(&request, $f(&request, &instance).await) })
        }
    };
}
macro_rules! with_params {
    ($name:ident, $f:ident) => {
        fn $name(
            request: Request,
            params: PathParams,
            _: ConnectionInfo,
            instance: Arc<Instance>,
        ) -> BoxFuture<'static, Response> {
            Box::pin(async move { answer(&request, $f(&request, &params, &instance).await) })
        }
    };
}
plain!(h_list, list);
plain!(h_add_folder, add_folder);
plain!(h_start_upload, start_upload);
plain!(h_special, special);
plain!(h_quota, quota);
with_params!(h_get, get_item);
with_params!(h_part, put_part);
with_params!(h_finish, finish_upload);
with_params!(h_cancel, cancel_upload);
with_params!(h_content, content);
with_params!(h_change, change);
with_params!(h_remove, remove);
with_params!(h_restore, restore);
with_params!(h_copy, copy);
with_params!(h_versions, versions);
with_params!(h_restore_version, restore_version);

fn object() -> Value {
    json!({"type": "object"})
}

pub fn routes() -> Vec<Route> {
    let route = |method, path, id, summary, handler| Route {
        method,
        path,
        id,
        tag: "drive",
        summary,
        access: Access::Signed,
        response: object,
        handler,
    };
    vec![
        route(Method::Get, "/drive/items", "driveList", "A folder's contents (?parent=, none for one's own drive), with the way to it and one's role.", h_list as ortus_core::api::Handler),
        route(Method::Get, "/drive/items/:item", "driveItem", "One file or folder.", h_get),
        route(Method::Post, "/drive/folders", "driveAddFolder", "A new folder: {parent, name}.", h_add_folder),
        route(Method::Post, "/drive/uploads", "driveStartUpload", "Start an upload: {parent, name, size, mime}. Then PUT its parts (at most 16 MB each, numbered from 0) and finish it.", h_start_upload),
        route(Method::Put, "/drive/uploads/:upload/parts/:part", "driveUploadPart", "One part of an upload: the body is its bytes. Sending a part again replaces it.", h_part),
        route(Method::Post, "/drive/uploads/:upload/finish", "driveFinishUpload", "Put the parts together into the file (a new version when the name exists there).", h_finish),
        route(Method::Delete, "/drive/uploads/:upload", "driveCancelUpload", "Give up an upload.", h_cancel),
        route(Method::Get, "/drive/items/:item/content", "driveDownload", "A file's contents, with Range; ?version=n for an older one, ?inline=1 to view images, PDF and text.", h_content),
        route(Method::Patch, "/drive/items/:item", "driveChange", "Rename, move or star: {name?, parent?, starred?}.", h_change),
        route(Method::Delete, "/drive/items/:item", "driveTrash", "To the trash; with ?forever=1 (from the trash, owner only) for good.", h_remove),
        route(Method::Post, "/drive/items/:item/restore", "driveRestore", "Back from the trash.", h_restore),
        route(Method::Post, "/drive/items/:item/copy", "driveCopy", "A copy of a file into a folder: {parent}.", h_copy),
        route(Method::Get, "/drive/items/:item/versions", "driveVersions", "A file's versions.", h_versions),
        route(Method::Post, "/drive/items/:item/versions/:version/restore", "driveRestoreVersion", "Make an older version the current one.", h_restore_version),
        route(Method::Get, "/drive/special", "driveSpecial", "?which=recent, starred, trash or email (files saved from mail).", h_special),
        route(Method::Get, "/drive/quota", "driveQuota", "How much of one's quota is used.", h_quota),
    ]
}
