//! What a link opens, for anyone who has it (no account): a file, or a
//! folder to look through. A password, when set, comes in the
//! `X-Ortus-Link-Password` header and wrong guesses are slowed down; a
//! link marked "no download" shows images, PDF and text in the page only.

use std::sync::Arc;

use ortus_core::api::{answer, ok, ApiError, Method, Route};
use ortus_core::auth::Access;
use ortus_core::instance::Instance;
use ortus_core::storage::BoxFuture;
use rust_web_server::prelude::*;
use serde_json::{json, Value};

use crate::api::{drive_error, file_response, query};
use crate::share::{by_token, Share};
use crate::store::{self, DriveError, Item};

fn gone() -> ApiError {
    ApiError::not_found("This link has ended.")
}

/// The share and its item, once the password (if any) is right.
async fn open(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<(Share, Item), ApiError> {
    let token = params.get("token").unwrap_or_default();
    let share = by_token(instance, token).await?.ok_or_else(gone)?;
    let s = instance.store.as_ref();
    let item = store::item(s, &share.item_id)
        .await?
        .filter(|i| i.trashed_at.is_none())
        .ok_or_else(gone)?;
    if store::ancestors(s, &item)
        .await?
        .iter()
        .any(|a| a.trashed_at.is_some())
    {
        return Err(gone());
    }
    if let Some(hash) = share.password.clone() {
        let given = request
            .headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("x-ortus-link-password"))
            .map(|h| h.value.to_string())
            .unwrap_or_default();
        let key = format!("drive-link:{token}");
        let now = ortus_core::program::now_ms();
        if given.is_empty() {
            return Err(ApiError {
                status: 401,
                code: "password",
                message: "This link needs its password.".into(),
            });
        }
        if ortus_core::auth::limits::blocked_until(s, &key, now)
            .await?
            .is_some()
        {
            return Err(ApiError::too_many(
                "Too many wrong passwords; wait a while.",
            ));
        }
        let right = tokio::task::spawn_blocking(move || {
            ortus_core::auth::credentials::password_matches(&given, &hash)
        })
        .await
        .map_err(|_| ApiError::internal())?;
        if !right {
            return Err(ApiError {
                status: 401,
                code: "password",
                message: "That password is not right.".into(),
            });
        }
        ortus_core::auth::limits::succeeded(s, &[key]).await?;
    }
    Ok((share, item))
}

/// An item under the link's folder (or the link's own item).
async fn within(instance: &Instance, root: &Item, id: Option<String>) -> Result<Item, ApiError> {
    let Some(id) = id.filter(|i| !i.is_empty() && *i != root.id) else {
        return Ok(root.clone());
    };
    let s = instance.store.as_ref();
    let item = store::item(s, &id)
        .await?
        .filter(|i| i.trashed_at.is_none())
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    let path = store::ancestors(s, &item).await?;
    if !path.iter().any(|a| a.id == root.id)
        || path
            .iter()
            .take_while(|a| a.id != root.id)
            .any(|a| a.trashed_at.is_some())
    {
        return Err(drive_error(DriveError::NotFound));
    }
    Ok(item)
}

fn entry(item: &Item) -> Value {
    json!({"id": item.id, "kind": item.kind, "name": item.name, "size": item.size, "mime": item.mime, "modifiedAt": item.modified_at})
}

/// What the link shows: the file, or a folder's contents (?item= inside it).
async fn show(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let token = params.get("token").unwrap_or_default();
    // Before the password: only that one is needed.
    if let Some(share) = by_token(instance, token).await? {
        if share.password.is_some()
            && !request
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("x-ortus-link-password"))
        {
            return Ok(ok(&json!({ "needsPassword": true })));
        }
    }
    let (share, root) = open(request, params, instance).await?;
    let item = within(instance, &root, query(request, "item")).await?;
    let s = instance.store.as_ref();
    if query(request, "item").is_none() {
        s.execute(
            "UPDATE drive_shares SET opened = opened + 1 WHERE id = $1",
            &[share.id.as_str().into()],
        )
        .await?;
    }
    let owner = ortus_core::auth::accounts::by_id(s, &root.owner_id)
        .await?
        .map(|u| u.name)
        .unwrap_or_default();
    let children: Vec<Value> = if item.is_folder() {
        store::children(s, &item.owner_id, &item.id)
            .await?
            .iter()
            .map(entry)
            .collect()
    } else {
        Vec::new()
    };
    let mut path = vec![json!({"id": item.id, "name": item.name})];
    if item.id != root.id {
        for a in store::ancestors(s, &item).await? {
            path.push(json!({"id": a.id, "name": a.name}));
            if a.id == root.id {
                break;
            }
        }
    }
    path.reverse();
    Ok(ok(&json!({
        "item": entry(&item),
        "children": children,
        "path": path,
        "owner": owner,
        "noDownload": share.no_download,
        "expiresAt": share.expires_at,
        "needsPassword": false,
    })))
}

/// A file through the link (?item= inside a folder; ?inline=1 to view).
async fn content(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let (share, root) = open(request, params, instance).await?;
    let item = within(instance, &root, query(request, "item")).await?;
    if item.is_folder() {
        return Err(drive_error(DriveError::NotFound));
    }
    let inline = query(request, "inline").as_deref() == Some("1");
    if share.no_download && !inline {
        return Err(ApiError {
            status: 403,
            code: "no_download",
            message: "This link is for viewing only.".into(),
        });
    }
    let v = store::version(instance.store.as_ref(), &item, None)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    let viewable = [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "application/pdf",
        "text/plain",
    ]
    .contains(&item.mime.as_str());
    if share.no_download && !viewable {
        return Err(ApiError {
            status: 403,
            code: "no_download",
            message: "This link is for viewing only.".into(),
        });
    }
    Ok(file_response(
        request,
        instance,
        &item.name,
        &item.mime,
        &v.blob,
        v.size.max(0) as u64,
        inline,
    ))
}

fn h_show(
    request: Request,
    params: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, show(&request, &params, &instance).await) })
}

fn h_content(
    request: Request,
    params: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, content(&request, &params, &instance).await) })
}

fn object() -> Value {
    json!({"type": "object"})
}

pub fn routes() -> Vec<Route> {
    vec![
        Route { method: Method::Get, path: "/drive/links/:token", id: "driveLinkOpen", tag: "drive", summary: "What a link shows, for anyone who has it: the file, or a folder's contents (?item= inside it). A password goes in X-Ortus-Link-Password.", access: Access::Public, response: object, handler: h_show },
        Route { method: Method::Get, path: "/drive/links/:token/content", id: "driveLinkContent", tag: "drive", summary: "A file through a link (?item= inside a folder; ?inline=1 to view). Links marked no download only show images, PDF and text.", access: Access::Public, response: object, handler: h_content },
    ]
}
