//! Keeps the core's search index in step with the drive: a file's name
//! and the text of documents, a folder's name. Others find what is shared
//! with them, checked item by item.

use std::sync::Arc;

use ortus_core::instance::Instance;
use ortus_core::search::{self, Entry};
use serde_json::json;

use crate::store::{self, Item};

/// A file or folder was added or changed: (re)indexed in the background
/// (reading a document's text can take a moment).
pub async fn item_changed(instance: &Arc<Instance>, item: &Item) {
    let (instance, item) = (Arc::clone(instance), item.clone());
    tokio::spawn(async move {
        if let Err(e) = index_now(&instance, &item).await {
            eprintln!("ortus: drive: indexing {}: {e}", item.id);
        }
    });
}

/// Indexes one item and waits for it.
pub async fn index_now(instance: &Instance, item: &Item) -> Result<(), String> {
    if item.trashed_at.is_some() {
        return search::remove(instance.store.as_ref(), "drive", &item.id)
            .await
            .map_err(|e| e.to_string());
    }
    let body = if item.is_folder() {
        String::new()
    } else {
        crate::preview::text(instance, item)
            .await
            .unwrap_or_default()
    };
    let path = if item.is_folder() {
        format!("/drive?folder={}", item.id)
    } else {
        format!("/drive?file={}", item.id)
    };
    search::index(
        instance.store.as_ref(),
        &Entry {
            app: "drive",
            id: &item.id,
            owner: &item.owner_id,
            title: &item.name,
            body: &body,
            meta: json!({"path": path, "kind": item.kind, "size": item.size, "mime": item.mime}),
            at: item.modified_at,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

/// A folder (and everything in it) came back: indexed again.
pub async fn tree_changed(instance: &Arc<Instance>, root: &Item) {
    let (instance, root) = (Arc::clone(instance), root.clone());
    tokio::spawn(async move {
        let s = instance.store.as_ref();
        let mut queue = vec![root];
        let mut seen = 0;
        while let Some(item) = queue.pop() {
            seen += 1;
            if seen > 50_000 {
                break;
            }
            let _ = index_now(&instance, &item).await;
            if item.is_folder() {
                queue.extend(
                    store::children(s, &item.owner_id, &item.id)
                        .await
                        .unwrap_or_default(),
                );
            }
        }
    });
}

/// A file or folder went to the trash or away (with what is under it).
pub async fn item_removed(instance: &Arc<Instance>, id: &str) {
    let s = instance.store.as_ref();
    let mut queue = vec![id.to_string()];
    let mut seen = 0;
    while let Some(next) = queue.pop() {
        seen += 1;
        if seen > 50_000 {
            break;
        }
        let _ = search::remove(s, "drive", &next).await;
        if let Ok(rows) = s
            .query(
                "SELECT id FROM drive_items WHERE parent_id = $1",
                &[next.as_str().into()],
            )
            .await
        {
            queue.extend(rows.iter().filter_map(|r| r.text(0).map(str::to_string)));
        }
    }
}

/// The owners of trees shared with the person (whose items they may find).
pub async fn owners_sharing_with(instance: &Instance, user: &str) -> Vec<String> {
    let s = instance.store.as_ref();
    let groups = store::groups_of(s, user).await.unwrap_or_default();
    let mut owners = Vec::new();
    let mut rows = s.query("SELECT DISTINCT i.owner_id FROM drive_shares sh JOIN drive_items i ON i.id = sh.item_id WHERE sh.kind = 'person' AND sh.target = $1", &[user.into()]).await.unwrap_or_default();
    for g in &groups {
        rows.extend(s.query("SELECT DISTINCT i.owner_id FROM drive_shares sh JOIN drive_items i ON i.id = sh.item_id WHERE sh.kind = 'group' AND sh.target = $1", &[g.as_str().into()]).await.unwrap_or_default());
    }
    for r in rows {
        if let Some(o) = r.text(0) {
            if o != user && !owners.iter().any(|x: &String| x == o) {
                owners.push(o.to_string());
            }
        }
    }
    owners
}

/// Of these items, the ones the person may see.
pub async fn visible(instance: &Instance, user: &str, ids: Vec<String>) -> Vec<String> {
    let s = instance.store.as_ref();
    let mut out = Vec::new();
    for id in ids.into_iter().take(500) {
        if let Ok(Some(item)) = store::item(s, &id).await {
            if store::role(s, user, &item).await.ok().flatten().is_some() {
                out.push(id);
            }
        }
    }
    out
}
