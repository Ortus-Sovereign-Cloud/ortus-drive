//! The drive as other apps see it (`interfaces::Drive`): mail saves
//! attachments here ("From email").

use std::sync::{Arc, Weak};

use ortus_core::instance::Instance;
use ortus_core::interfaces::ItemRef;
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

impl ortus_core::interfaces::Drive for Service {
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
