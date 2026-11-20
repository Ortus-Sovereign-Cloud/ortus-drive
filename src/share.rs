//! Sharing: with colleagues and groups (view, comment, edit), with people
//! outside and with anyone holding a link (view only; expiry, password, no
//! download). The core's label policy decides every share before it is
//! made, and the audit log keeps each one. Changing a label to a stricter
//! one takes away the shares it no longer allows.

use std::sync::Arc;

use ortus_core::api::{answer, body, ok, ApiError, Method, Route};
use ortus_core::auth::Access;
use ortus_core::instance::Instance;
use ortus_core::policy::{self, Action, Audience, Channel, Decision};
use ortus_core::storage::BoxFuture;
use rust_web_server::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{drive_error, reach, signed};
use crate::store::{self, DriveError, Item, Role};

/// A share as stored.
#[derive(Clone, Debug)]
pub struct Share {
    pub id: String,
    pub item_id: String,
    /// person, group, outside or link.
    pub kind: String,
    /// A user id, a group id, an address (outside), or the link's token.
    pub target: String,
    pub role: String,
    pub expires_at: Option<i64>,
    pub password: Option<String>,
    pub no_download: bool,
    pub opened: i64,
    /// Whom an outside share is for.
    pub email: Option<String>,
}

const SHARE: &str = "SELECT id, item_id, kind, target, role, expires_at, password, no_download, opened, email FROM drive_shares";

fn share_of(r: &ortus_core::storage::Row) -> Share {
    Share {
        id: r.text(0).unwrap_or_default().into(),
        item_id: r.text(1).unwrap_or_default().into(),
        kind: r.text(2).unwrap_or_default().into(),
        target: r.text(3).unwrap_or_default().into(),
        role: r.text(4).unwrap_or_default().into(),
        expires_at: r.int(5),
        password: r.text(6).map(str::to_string),
        no_download: r.int(7).unwrap_or(0) != 0,
        opened: r.int(8).unwrap_or(0),
        email: r.text(9).map(str::to_string),
    }
}

pub async fn shares_of(instance: &Instance, item_id: &str) -> Result<Vec<Share>, ApiError> {
    Ok(instance
        .store
        .query(
            &format!("{SHARE} WHERE item_id = $1 ORDER BY created_at"),
            &[item_id.into()],
        )
        .await?
        .iter()
        .map(share_of)
        .collect())
}

/// The share a link token names, unexpired.
pub async fn by_token(instance: &Instance, token: &str) -> Result<Option<Share>, ApiError> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let now = ortus_core::program::now_ms();
    Ok(instance
        .store
        .query(
            &format!("{SHARE} WHERE kind IN ('link', 'outside') AND target = $1"),
            &[token.into()],
        )
        .await?
        .first()
        .map(share_of)
        .filter(|s| s.expires_at.is_none_or(|e| e > now)))
}

pub fn link_url(instance: &Instance, token: &str) -> String {
    format!(
        "{}/drive/public/s/{token}",
        instance.config.public_url.trim_end_matches('/')
    )
}

/// What the policy says about sharing an item this way. `Ok(watermark)`
/// when allowed.
async fn ask(
    instance: &Instance,
    user: &str,
    item: &Item,
    action: Action,
    audience: &Audience,
    reason: Option<&str>,
) -> Result<bool, ApiError> {
    let label = store::effective_label(instance.store.as_ref(), item).await?;
    let decision = policy::decide(
        instance,
        &policy::Ask {
            actor: user,
            action,
            label: label.as_deref(),
            audience,
            channel: Channel::Files,
            object: &format!("drive:{}:{}", item.kind, item.id),
            reason: reason.filter(|r| !r.trim().is_empty()),
        },
    )
    .await?;
    match decision {
        Decision::Allow { watermark } => Ok(watermark),
        Decision::NeedsReason { label } => Err(ApiError {
            status: 409,
            code: "needs_reason",
            message: format!("{label} files need a reason to be shared this way."),
        }),
        Decision::Deny { label, .. } => Err(ApiError {
            status: 403,
            code: "policy",
            message: format!("{label} files cannot be shared this way."),
        }),
    }
}

async fn audit(instance: &Instance, user: &str, action: &str, item: &Item, detail: Value) {
    ortus_core::auth::audit(
        instance,
        ortus_core::audit::Event::new(action, ortus_core::audit::Outcome::Done)
            .by(user)
            .on(&item.id)
            .with(detail),
    )
    .await;
}

#[derive(Deserialize)]
struct AddShare {
    /// An address (a colleague's or someone outside).
    email: Option<String>,
    /// Or a group id.
    group: Option<String>,
    #[serde(default = "view")]
    role: String,
    reason: Option<String>,
    /// For a link: when it stops working (Unix ms).
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
}

fn view() -> String {
    "view".into()
}

fn token() -> String {
    ortus_core::auth::random_hex(16)
}

#[allow(clippy::too_many_arguments)]
async fn insert(
    instance: &Instance,
    user: &str,
    item: &Item,
    kind: &str,
    target: &str,
    role: &str,
    expires_at: Option<i64>,
    no_download: bool,
) -> Result<String, ApiError> {
    let id = format!("s{}", ortus_core::auth::random_hex(12));
    let mut tx = instance.store.begin().await?;
    // Sharing again with the same one replaces the old share.
    if kind == "person" || kind == "group" {
        tx.execute(
            "DELETE FROM drive_shares WHERE item_id = $1 AND kind = $2 AND target = $3",
            &[item.id.as_str().into(), kind.into(), target.into()],
        )
        .await?;
    }
    tx.execute(
        "INSERT INTO drive_shares (id, item_id, kind, target, role, expires_at, no_download, opened, created_by, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, $9)",
        &[id.as_str().into(), item.id.as_str().into(), kind.into(), target.into(), role.into(), expires_at.into(), i64::from(no_download).into(), user.into(), ortus_core::program::now_ms().into()],
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Shares an item with a colleague, a group, or someone outside (who gets
/// a personal view link to send them).
async fn add(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: AddShare = body(request)?;
    let (item, my_role) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::Edit,
    )
    .await?;
    let role = Role::parse(&input.role)
        .ok_or_else(|| ApiError::bad_request("A role is view, comment or edit."))?;
    // One cannot give more than one has.
    if role > my_role {
        return Err(ApiError::forbidden());
    }
    let s = instance.store.as_ref();
    if let Some(group) = input.group.as_deref() {
        if s.query("SELECT 1 FROM core_groups WHERE id = $1", &[group.into()])
            .await?
            .is_empty()
        {
            return Err(ApiError::not_found("There is no such group."));
        }
        ask(
            instance,
            &me.id,
            &item,
            Action::Share,
            &Audience::Internal,
            None,
        )
        .await?;
        insert(
            instance,
            &me.id,
            &item,
            "group",
            group,
            role.name(),
            None,
            false,
        )
        .await?;
        audit(
            instance,
            &me.id,
            "drive.share",
            &item,
            json!({"group": group, "role": role.name()}),
        )
        .await;
        return list_response(instance, &me.id, &item).await;
    }
    let email = input
        .email
        .as_deref()
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| e.contains('@') && e.len() <= 254 && !e.contains(char::is_whitespace))
        .ok_or_else(|| ApiError::bad_request("Share with an address or a group."))?;
    let colleague = ortus_core::auth::accounts::by_email(s, &email)
        .await?
        .filter(|u| u.state != ortus_core::auth::accounts::State::Unverified);
    match colleague {
        Some(user) => {
            if user.id == item.owner_id {
                return Err(ApiError::conflict("That is the owner."));
            }
            ask(
                instance,
                &me.id,
                &item,
                Action::Share,
                &Audience::Internal,
                None,
            )
            .await?;
            insert(
                instance,
                &me.id,
                &item,
                "person",
                &user.id,
                role.name(),
                None,
                false,
            )
            .await?;
            audit(
                instance,
                &me.id,
                "drive.share",
                &item,
                json!({"person": user.id, "role": role.name()}),
            )
            .await;
        }
        None => {
            // Outside: the policy decides; they get their own view link.
            let watermark = ask(
                instance,
                &me.id,
                &item,
                Action::Share,
                &Audience::Outside(email.clone()),
                input.reason.as_deref(),
            )
            .await?;
            let t = token();
            let id = insert(
                instance,
                &me.id,
                &item,
                "outside",
                &t,
                "view",
                input.expires_at,
                watermark,
            )
            .await?;
            instance
                .store
                .execute(
                    "UPDATE drive_shares SET email = $1 WHERE id = $2",
                    &[email.as_str().into(), id.as_str().into()],
                )
                .await?;
            audit(
                instance,
                &me.id,
                "drive.share_outside",
                &item,
                json!({"domain": email.rsplit('@').next(), "watermark": watermark}),
            )
            .await;
        }
    }
    list_response(instance, &me.id, &item).await
}

#[derive(Deserialize)]
struct LinkInput {
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    password: Option<String>,
    #[serde(default, rename = "noDownload")]
    no_download: bool,
    reason: Option<String>,
}

/// Makes (or replaces) the item's link for anyone who has it: view only.
async fn link(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: LinkInput = body(request)?;
    let (item, _) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::Edit,
    )
    .await?;
    let watermark = ask(
        instance,
        &me.id,
        &item,
        Action::Link,
        &Audience::Anyone,
        input.reason.as_deref(),
    )
    .await?;
    let now = ortus_core::program::now_ms();
    if input
        .expires_at
        .is_some_and(|e| e <= now || e > now + 366 * 86_400_000)
    {
        return Err(ApiError::bad_request("A link ends within a year."));
    }
    let password = match input
        .password
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(p) if p.len() < 8 || p.len() > 200 => {
            return Err(ApiError::bad_request(
                "A link's password is 8 to 200 characters.",
            ))
        }
        Some(p) => {
            let p = p.to_string();
            Some(
                tokio::task::spawn_blocking(move || {
                    ortus_core::auth::credentials::hash_password(&p)
                })
                .await
                .map_err(|_| ApiError::internal())?,
            )
        }
        None => None,
    };
    let s = instance.store.as_ref();
    s.execute(
        "DELETE FROM drive_shares WHERE item_id = $1 AND kind = 'link'",
        &[item.id.as_str().into()],
    )
    .await?;
    let t = token();
    let id = insert(
        instance,
        &me.id,
        &item,
        "link",
        &t,
        "view",
        input.expires_at,
        input.no_download || watermark,
    )
    .await?;
    s.execute(
        "UPDATE drive_shares SET password = $1 WHERE id = $2",
        &[password.into(), id.as_str().into()],
    )
    .await?;
    audit(instance, &me.id, "drive.link", &item, json!({"expires": input.expires_at, "password": input.password.is_some(), "noDownload": input.no_download || watermark})).await;
    list_response(instance, &me.id, &item).await
}

#[derive(Deserialize)]
struct LinkChange {
    /// Unix ms; 0 for no end.
    #[serde(rename = "expiresAt")]
    expires_at: Option<i64>,
    /// Empty for none.
    password: Option<String>,
    #[serde(rename = "noDownload")]
    no_download: Option<bool>,
}

/// Changes a link's end, password or downloads; its address stays.
async fn change_link(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: LinkChange = body(request)?;
    let id = params.get("share").unwrap_or_default();
    let rows = instance
        .store
        .query(
            &format!("{SHARE} WHERE id = $1 AND kind = 'link'"),
            &[id.into()],
        )
        .await?;
    let share = rows
        .first()
        .map(share_of)
        .ok_or_else(|| ApiError::not_found("There is no such link."))?;
    let (item, _) = reach(instance, &me.id, &share.item_id, Role::Edit).await?;
    let s = instance.store.as_ref();
    let now = ortus_core::program::now_ms();
    if let Some(e) = input.expires_at {
        if e != 0 && (e <= now || e > now + 366 * 86_400_000) {
            return Err(ApiError::bad_request("A link ends within a year."));
        }
        s.execute(
            "UPDATE drive_shares SET expires_at = $1 WHERE id = $2",
            &[(if e == 0 { None } else { Some(e) }).into(), id.into()],
        )
        .await?;
    }
    if let Some(p) = input.password.as_deref().map(str::trim) {
        let hash = match p {
            "" => None,
            p if p.len() < 8 || p.len() > 200 => {
                return Err(ApiError::bad_request(
                    "A link's password is 8 to 200 characters.",
                ))
            }
            p => {
                let p = p.to_string();
                Some(
                    tokio::task::spawn_blocking(move || {
                        ortus_core::auth::credentials::hash_password(&p)
                    })
                    .await
                    .map_err(|_| ApiError::internal())?,
                )
            }
        };
        s.execute(
            "UPDATE drive_shares SET password = $1 WHERE id = $2",
            &[hash.into(), id.into()],
        )
        .await?;
    }
    if let Some(no) = input.no_download {
        // A label that wants a watermark keeps downloads blocked.
        let label = store::effective_label(s, &item).await?;
        let forced = policy::label(s, label.as_deref()).await?.rules.watermark;
        s.execute(
            "UPDATE drive_shares SET no_download = $1 WHERE id = $2",
            &[i64::from(no || forced).into(), id.into()],
        )
        .await?;
    }
    audit(instance, &me.id, "drive.link_changed", &item, json!({"expires": input.expires_at, "password": input.password.as_ref().map(|p| !p.is_empty()), "noDownload": input.no_download})).await;
    list_response(instance, &me.id, &item).await
}

async fn remove(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let id = params.get("share").unwrap_or_default();
    let rows = instance
        .store
        .query(&format!("{SHARE} WHERE id = $1"), &[id.into()])
        .await?;
    let share = rows
        .first()
        .map(share_of)
        .ok_or_else(|| ApiError::not_found("There is no such share."))?;
    let (item, _) = reach(instance, &me.id, &share.item_id, Role::Edit).await?;
    instance
        .store
        .execute("DELETE FROM drive_shares WHERE id = $1", &[id.into()])
        .await?;
    audit(
        instance,
        &me.id,
        "drive.unshare",
        &item,
        json!({"kind": share.kind}),
    )
    .await;
    list_response(instance, &me.id, &item).await
}

#[derive(Deserialize)]
struct LabelInput {
    label: Option<String>,
}

/// The owner labels an item; shares the new label does not allow go.
async fn set_label(
    request: &Request,
    params: &PathParams,
    instance: &Arc<Instance>,
) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let input: LabelInput = body(request)?;
    let (item, _) = reach(
        instance,
        &me.id,
        params.get("item").unwrap_or_default(),
        Role::Owner,
    )
    .await?;
    let s = instance.store.as_ref();
    let label = input.label.filter(|l| !l.is_empty());
    if let Some(l) = &label {
        if !policy::labels(s).await?.iter().any(|x| &x.id == l) {
            return Err(ApiError::bad_request("There is no such label."));
        }
    }
    store::set_label(s, &item.id, label.as_deref()).await?;
    let item = store::item(s, &item.id)
        .await?
        .ok_or_else(|| drive_error(DriveError::NotFound))?;
    audit(
        instance,
        &me.id,
        "drive.label",
        &item,
        json!({"label": label}),
    )
    .await;
    let removed = recheck(instance, &me.id, &item).await?;
    let mut out = list_body(instance, &me.id, &item).await?;
    out["removed"] = json!(removed);
    Ok(ok(&out))
}

/// Takes away the link and outside shares of an item, and of everything
/// under a folder, that its label no longer allows. Answers how many.
pub async fn recheck(instance: &Instance, user: &str, item: &Item) -> Result<usize, ApiError> {
    let s = instance.store.as_ref();
    let mut queue = vec![item.clone()];
    let mut removed = 0;
    let mut seen = 0;
    while let Some(it) = queue.pop() {
        seen += 1;
        if seen > 50_000 {
            break;
        }
        for share in shares_of(instance, &it.id).await? {
            let audience = match share.kind.as_str() {
                "link" => Audience::Anyone,
                "outside" => Audience::Outside(share.email.clone().unwrap_or_default()),
                _ => continue,
            };
            let action = if share.kind == "link" {
                Action::Link
            } else {
                Action::Share
            };
            let label = store::effective_label(s, &it).await?;
            let (decision, _, _) = policy::judge(
                s,
                &policy::Ask {
                    actor: user,
                    action,
                    label: label.as_deref(),
                    audience: &audience,
                    channel: Channel::Files,
                    object: &format!("drive:{}:{}", it.kind, it.id),
                    reason: None,
                },
            )
            .await?;
            if !matches!(decision, Decision::Allow { .. }) {
                s.execute(
                    "DELETE FROM drive_shares WHERE id = $1",
                    &[share.id.as_str().into()],
                )
                .await?;
                audit(
                    instance,
                    user,
                    "drive.unshare_by_label",
                    &it,
                    json!({"kind": share.kind}),
                )
                .await;
                removed += 1;
            }
        }
        if it.is_folder() {
            queue.extend(store::children(s, &it.owner_id, &it.id).await?);
        }
    }
    Ok(removed)
}

/// Who has an item and how, as the share dialog shows it.
async fn list_body(instance: &Instance, me: &str, item: &Item) -> Result<Value, ApiError> {
    let s = instance.store.as_ref();
    let mut people = Vec::new();
    if let Some(owner) = ortus_core::auth::accounts::by_id(s, &item.owner_id).await? {
        people.push(json!({"name": owner.name, "email": owner.email, "role": "owner", "you": owner.id == me}));
    }
    let mut groups = Vec::new();
    let mut outside = Vec::new();
    let mut link = Value::Null;
    for share in shares_of(instance, &item.id).await? {
        match share.kind.as_str() {
            "person" => {
                if let Some(u) = ortus_core::auth::accounts::by_id(s, &share.target).await? {
                    people.push(json!({"share": share.id, "name": u.name, "email": u.email, "role": share.role, "you": u.id == me}));
                }
            }
            "group" => {
                let name = s.query("SELECT name FROM core_groups WHERE id = $1", &[share.target.as_str().into()]).await?.first().and_then(|r| r.text(0).map(str::to_string)).unwrap_or_default();
                groups.push(json!({"share": share.id, "group": share.target, "name": name, "role": share.role}));
            }
            "outside" => outside.push(json!({"share": share.id, "email": share.email, "role": "view", "url": link_url(instance, &share.target), "expiresAt": share.expires_at, "opened": share.opened})),
            "link" => link = json!({"share": share.id, "url": link_url(instance, &share.target), "expiresAt": share.expires_at, "password": share.password.is_some(), "noDownload": share.no_download, "opened": share.opened}),
            _ => {}
        }
    }
    Ok(
        json!({ "people": people, "groups": groups, "outside": outside, "link": link, "label": store::effective_label(s, item).await? }),
    )
}

/// How an item is shared, in short: a link, groups by name, people.
pub async fn access(instance: &Instance, item_id: &str) -> Result<Value, ApiError> {
    let s = instance.store.as_ref();
    let mut link = false;
    let mut people = 0;
    let mut groups = Vec::new();
    for share in shares_of(instance, item_id).await? {
        match share.kind.as_str() {
            "link" => link = true,
            "person" | "outside" => people += 1,
            "group" => {
                if let Some(n) = s
                    .query(
                        "SELECT name FROM core_groups WHERE id = $1",
                        &[share.target.as_str().into()],
                    )
                    .await?
                    .first()
                    .and_then(|r| r.text(0).map(str::to_string))
                {
                    groups.push(n);
                }
            }
            _ => {}
        }
    }
    Ok(json!({"link": link, "people": people, "groups": groups}))
}

async fn list_response(instance: &Instance, me: &str, item: &Item) -> Result<Response, ApiError> {
    Ok(ok(&list_body(instance, me, item).await?))
}

async fn list(
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
    list_response(instance, &me.id, &item).await
}

/// What others share with the person directly (or with a group of theirs).
async fn shared_with_me(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    let me = signed(request, instance).await?;
    let s = instance.store.as_ref();
    let groups = store::groups_of(s, &me.id).await?;
    let mut ids: Vec<String> = s
        .query(
            "SELECT item_id FROM drive_shares WHERE kind = 'person' AND target = $1",
            &[me.id.as_str().into()],
        )
        .await?
        .iter()
        .filter_map(|r| r.text(0).map(str::to_string))
        .collect();
    for g in &groups {
        ids.extend(
            s.query(
                "SELECT item_id FROM drive_shares WHERE kind = 'group' AND target = $1",
                &[g.as_str().into()],
            )
            .await?
            .iter()
            .filter_map(|r| r.text(0).map(str::to_string)),
        );
    }
    ids.sort();
    ids.dedup();
    let mut items = Vec::new();
    for id in ids.iter().take(500) {
        if let Some(item) = store::item(s, id).await? {
            if item.owner_id != me.id {
                if let Some(role) = store::role(s, &me.id, &item).await? {
                    items.push((item, role));
                }
            }
        }
    }
    let mut out = Vec::new();
    for (item, role) in &items {
        let people = std::collections::HashMap::from([(
            item.owner_id.clone(),
            ortus_core::auth::accounts::by_id(s, &item.owner_id)
                .await?
                .map(|u| u.name)
                .unwrap_or_default(),
        )]);
        out.push(crate::api::item_json(item, Some(*role), &people, &me.id));
    }
    Ok(ok(&json!({ "items": out })))
}

/// The labels one can give a file or folder, weakest first.
async fn labels(request: &Request, instance: &Arc<Instance>) -> Result<Response, ApiError> {
    signed(request, instance).await?;
    let mut list = policy::labels(instance.store.as_ref()).await?;
    list.sort_by_key(|l| l.rank);
    let out: Vec<Value> = list
        .iter()
        .map(|l| json!({"id": l.id, "name": l.name}))
        .collect();
    Ok(ok(&json!({ "labels": out })))
}

fn h_labels(
    request: Request,
    _: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, labels(&request, &instance).await) })
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
with_params!(h_list, list);
with_params!(h_add, add);
with_params!(h_link, link);
with_params!(h_remove, remove);
with_params!(h_change_link, change_link);
with_params!(h_label, set_label);

fn h_shared(
    request: Request,
    _: PathParams,
    _: ConnectionInfo,
    instance: Arc<Instance>,
) -> BoxFuture<'static, Response> {
    Box::pin(async move { answer(&request, shared_with_me(&request, &instance).await) })
}

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
        route(Method::Get, "/drive/items/:item/shares", "driveShares", "Who has a file or folder, and its link.", h_list as ortus_core::api::Handler),
        route(Method::Post, "/drive/items/:item/shares", "driveShare", "Share with {email} (a colleague, or someone outside who gets a view link) or {group}, as {role}: view, comment or edit. The label policy decides; a warning needs {reason}.", h_add),
        route(Method::Post, "/drive/items/:item/link", "driveLink", "A link for anyone who has it, view only: {expiresAt, password, noDownload, reason}. Replaces the old one. The label policy decides.", h_link),
        route(Method::Delete, "/drive/shares/:share", "driveUnshare", "Take a share or link away.", h_remove),
        route(Method::Patch, "/drive/shares/:share", "driveChangeLink", "Change a link, keeping its address: {expiresAt (0 for none), password (empty for none), noDownload}.", h_change_link),
        route(Method::Put, "/drive/items/:item/label", "driveLabel", "The owner labels an item: {label}. Shares the label no longer allows are taken away.", h_label),
        route(Method::Get, "/drive/labels", "driveLabels", "The labels one can give a file or folder.", h_labels),
        route(Method::Get, "/drive/shared", "driveSharedWithMe", "What others share with one, directly or through a group.", h_shared),
    ]
}
