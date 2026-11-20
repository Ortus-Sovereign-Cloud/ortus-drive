//! The drive as other apps see it (`interfaces::Drive`): mail saves
//! attachments here ("From email").

use std::sync::{Arc, Weak};

use ortus_core::instance::Instance;
use ortus_core::interfaces::{DriveItem, ItemRef};
use ortus_core::storage::BoxFuture;

use crate::store;

pub struct Service {
    pub instance: Weak<Instance>,
}

pub async fn save(
    instance: &Arc<Instance>,
    user: &str,
    name: &str,
    mime: &str,
    contents: &[u8],
    source: &str,
    note: Option<&str>,
) -> Result<ItemRef, String> {
    let s = instance.store.as_ref();
    let clean: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .take(200)
        .collect();
    let clean = clean.trim();
    let clean = if store::name_ok(clean) {
        clean.to_string()
    } else {
        "attachment".to_string()
    };
    let name = store::free_name(s, user, store::ROOT, &clean)
        .await
        .map_err(|e| e.to_string())?;
    let blob = store::blob_key(user);
    instance
        .files
        .put(&blob, contents)
        .await
        .map_err(|e| e.to_string())?;
    let mime = crate::api::safe_mime(mime);
    let mime = mime.as_str();
    let source = source
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(20)
        .collect::<String>();
    let item = match store::add_file(
        s,
        user,
        store::ROOT,
        &store::Upload {
            name: &name,
            mime,
            blob: &blob,
            size: contents.len() as i64,
            source: Some(&source),
        },
    )
    .await
    {
        Ok(item) => item,
        Err(e) => {
            let _ = instance.files.delete(&blob).await;
            return Err(format!("{e:?}"));
        }
    };
    if let Some(note) = note {
        let note: String = note.chars().filter(|c| !c.is_control()).take(200).collect();
        s.execute(
            "UPDATE drive_items SET note = $1 WHERE id = $2",
            &[note.as_str().into(), item.id.as_str().into()],
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    crate::index::item_changed(instance, &item).await;
    Ok(ItemRef {
        app: "drive".into(),
        id: item.id.clone(),
        path: format!("/drive?file={}", item.id),
    })
}

/// The type of items whose contents another app keeps; Drive opens them
/// in that app.
pub const DOC_MIME: &str = "application/vnd.ortus.doc";

pub async fn create(
    instance: &Arc<Instance>,
    user: &str,
    parent: &str,
    name: &str,
    mime: &str,
) -> Result<ItemRef, String> {
    let s = instance.store.as_ref();
    let clean: String = name
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .take(200)
        .collect();
    let clean = if store::name_ok(clean.trim()) {
        clean.trim().to_string()
    } else {
        "Untitled".into()
    };
    let owner = store::place(s, user, parent)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let name = store::free_name(s, &owner, parent, &clean)
        .await
        .map_err(|e| e.to_string())?;
    // No contents of its own here: the other app keeps them.
    let item = store::add_file(
        s,
        user,
        parent,
        &store::Upload {
            name: &name,
            mime,
            blob: "",
            size: 0,
            source: None,
        },
    )
    .await
    .map_err(|e| format!("{e:?}"))?;
    crate::index::item_changed(instance, &item).await;
    Ok(ItemRef {
        app: "drive".into(),
        id: item.id.clone(),
        path: format!("/docs?doc={}", item.id),
    })
}

pub async fn describe(
    instance: &Instance,
    user: &str,
    id: &str,
) -> Result<Option<DriveItem>, String> {
    let s = instance.store.as_ref();
    let Some(item) = store::item(s, id).await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Some(role) = store::role(s, user, &item)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let mut path = Vec::new();
    for a in store::ancestors(s, &item)
        .await
        .map_err(|e| e.to_string())?
    {
        if store::role(s, user, &a)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            break;
        }
        path.push(a.name);
    }
    path.reverse();
    Ok(Some(DriveItem {
        id: item.id.clone(),
        name: item.name.clone(),
        mime: item.mime.clone(),
        owner: item.owner_id.clone(),
        role: role.name().into(),
        label: store::effective_label(s, &item)
            .await
            .map_err(|e| e.to_string())?,
        path,
        modified_at: item.modified_at,
    }))
}

impl ortus_core::interfaces::Drive for Service {
    fn create<'a>(
        &'a self,
        user: &'a str,
        parent: &'a str,
        name: &'a str,
        mime: &'a str,
    ) -> BoxFuture<'a, Result<ItemRef, String>> {
        Box::pin(async move {
            let instance = self.instance.upgrade().ok_or("stopping")?;
            create(&instance, user, parent, name, mime).await
        })
    }
    fn describe<'a>(
        &'a self,
        user: &'a str,
        id: &'a str,
    ) -> BoxFuture<'a, Result<Option<DriveItem>, String>> {
        Box::pin(async move {
            let instance = self.instance.upgrade().ok_or("stopping")?;
            describe(&instance, user, id).await
        })
    }
    fn rename<'a>(
        &'a self,
        user: &'a str,
        id: &'a str,
        name: &'a str,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let instance = self.instance.upgrade().ok_or("stopping")?;
            let s = instance.store.as_ref();
            let item = store::item(s, id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("not_found")?;
            match store::role(s, user, &item)
                .await
                .map_err(|e| e.to_string())?
            {
                Some(store::Role::Edit | store::Role::Owner) => {}
                _ => return Err("forbidden".into()),
            }
            let item = store::rename(s, user, &item, name.trim())
                .await
                .map_err(|e| format!("{e:?}"))?;
            crate::index::item_changed(&instance, &item).await;
            Ok(())
        })
    }

    fn save<'a>(
        &'a self,
        user: &'a str,
        name: &'a str,
        mime: &'a str,
        contents: &'a [u8],
        source: &'a str,
        note: Option<&'a str>,
    ) -> BoxFuture<'a, Result<ItemRef, String>> {
        Box::pin(async move {
            let instance = self.instance.upgrade().ok_or("stopping")?;
            save(&instance, user, name, mime, contents, source, note).await
        })
    }
}
