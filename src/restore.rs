//! Bringing back a Drive folder as it was at a restore point: a new folder
//! in the person's own Drive with the folders and files under it, each
//! file as its newest version was then. Nothing of theirs is changed.

use std::collections::BTreeMap;
use std::sync::Arc;

use ortus_core::instance::Instance;
use ortus_core::points::{int, text, Snapshot};
use serde_json::{Map, Value};

use crate::store::{self, Upload};

type Row = Map<String, Value>;

/// The person's folders at the point: (id, "Parent / Name").
pub async fn choices(point: &Snapshot, user: &str) -> Result<Vec<(String, String)>, String> {
    let folders: BTreeMap<String, Row> = point
        .rows("drive_items", |r| {
            text(r, "owner_id") == user
                && text(r, "kind") == "folder"
                && r.get("trashed_at").is_none_or(Value::is_null)
        })
        .await?
        .into_iter()
        .map(|r| (text(&r, "id").to_string(), r))
        .collect();
    let mut list: Vec<(String, String)> = folders
        .iter()
        .map(|(id, r)| {
            // Its path, from the top of the person's Drive.
            let mut names = vec![text(r, "name").to_string()];
            let mut at = text(r, "parent_id");
            while let Some(p) = folders.get(at).filter(|_| names.len() < 32) {
                names.push(text(p, "name").to_string());
                at = text(p, "parent_id");
            }
            names.reverse();
            (id.clone(), names.join(" / "))
        })
        .collect();
    list.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(list)
}

/// Restores folder `id` of `user` into a new folder `into` at the top of
/// their Drive; gives its path.
pub async fn folder(
    instance: &Arc<Instance>,
    point: &Snapshot,
    user: &str,
    id: &str,
    into: &str,
) -> Result<String, String> {
    let items: Vec<Row> = point
        .rows("drive_items", |r| {
            text(r, "owner_id") == user && r.get("trashed_at").is_none_or(Value::is_null)
        })
        .await?;
    let mut children: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in &items {
        children
            .entry(text(r, "parent_id").to_string())
            .or_default()
            .push(r);
    }
    // The newest version of each file then.
    let mut newest: BTreeMap<String, (i64, String, i64)> = BTreeMap::new();
    for v in point.rows("drive_versions", |_| true).await? {
        let (item, version) = (
            text(&v, "item_id").to_string(),
            int(&v, "version").unwrap_or(0),
        );
        if newest.get(&item).is_none_or(|(n, _, _)| *n < version) {
            newest.insert(
                item,
                (
                    version,
                    text(&v, "blob").to_string(),
                    int(&v, "size").unwrap_or(0),
                ),
            );
        }
    }
    let s = instance.store.as_ref();
    let top = store::create_folder(s, user, store::ROOT, into)
        .await
        .map_err(|e| format!("{e:?}"))?;
    // Depth first, folders before their files.
    let mut todo = vec![(id.to_string(), top.id.clone(), 0usize)];
    while let Some((from, to, depth)) = todo.pop() {
        if depth > 32 {
            continue;
        }
        for r in children.get(&from).map(Vec::as_slice).unwrap_or_default() {
            let name = text(r, "name");
            if text(r, "kind") == "folder" {
                let made = store::create_folder(s, user, &to, name)
                    .await
                    .map_err(|e| format!("{e:?}"))?;
                todo.push((text(r, "id").to_string(), made.id, depth + 1));
                continue;
            }
            let Some((_, blob, size)) = newest.get(text(r, "id")) else {
                continue;
            };
            let Some(contents) = point.file(blob).await? else {
                continue;
            };
            let key = store::blob_key(user);
            instance
                .files
                .put(&key, &contents)
                .await
                .map_err(|e| e.to_string())?;
            let added = store::add_file(
                s,
                user,
                &to,
                &Upload {
                    name,
                    mime: text(r, "mime"),
                    blob: &key,
                    size: *size,
                    source: None,
                },
            )
            .await;
            if let Err(e) = added {
                let _ = instance.files.delete(&key).await;
                return Err(format!("{e:?}"));
            }
        }
    }
    Ok(format!("/drive?folder={}", top.id))
}
