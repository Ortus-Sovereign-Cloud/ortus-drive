//! The drive's store: folders and files (each file a series of versions
//! whose contents are in the file interface), trash, stars, recent files,
//! shares and quotas. One person owns each tree; others reach items
//! through shares on the item or a folder above it.

use ortus_core::files::Files;
use ortus_core::storage::{AsyncStore, AsyncTx, Migration, Row, StoreError, StoreResult, Value};
use serde::Serialize;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "folders, files, versions, shares",
        sql: "CREATE TABLE drive_items (
        id TEXT PRIMARY KEY,
        owner_id TEXT NOT NULL,
        parent_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        name TEXT NOT NULL,
        size BIGINT NOT NULL,
        mime TEXT NOT NULL,
        version BIGINT NOT NULL,
        label TEXT,
        source TEXT,
        created_at BIGINT NOT NULL,
        modified_at BIGINT NOT NULL,
        modified_by TEXT NOT NULL,
        trashed_at BIGINT
    );
    CREATE INDEX drive_items_parent ON drive_items (parent_id, name);
    CREATE INDEX drive_items_owner ON drive_items (owner_id, modified_at);
    CREATE TABLE drive_versions (
        item_id TEXT NOT NULL,
        version BIGINT NOT NULL,
        blob TEXT NOT NULL,
        size BIGINT NOT NULL,
        created_at BIGINT NOT NULL,
        created_by TEXT NOT NULL,
        PRIMARY KEY (item_id, version)
    );
    CREATE TABLE drive_stars (
        user_id TEXT NOT NULL,
        item_id TEXT NOT NULL,
        PRIMARY KEY (user_id, item_id)
    );
    CREATE TABLE drive_recent (
        user_id TEXT NOT NULL,
        item_id TEXT NOT NULL,
        at BIGINT NOT NULL,
        PRIMARY KEY (user_id, item_id)
    );
    CREATE TABLE drive_shares (
        id TEXT PRIMARY KEY,
        item_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        target TEXT NOT NULL,
        role TEXT NOT NULL,
        expires_at BIGINT,
        password TEXT,
        no_download BIGINT NOT NULL,
        opened BIGINT NOT NULL,
        created_by TEXT NOT NULL,
        created_at BIGINT NOT NULL
    );
    CREATE INDEX drive_shares_item ON drive_shares (item_id);
    CREATE INDEX drive_shares_target ON drive_shares (kind, target)",
    },
    Migration {
        version: 2,
        name: "uploads in parts",
        sql: "CREATE TABLE drive_uploads (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        parent_id TEXT NOT NULL,
        name TEXT NOT NULL,
        mime TEXT NOT NULL,
        size BIGINT NOT NULL,
        parts BIGINT NOT NULL,
        received BIGINT NOT NULL,
        created_at BIGINT NOT NULL
    )",
    },
    Migration {
        version: 3,
        name: "addresses of outside shares",
        sql: "ALTER TABLE drive_shares ADD COLUMN email TEXT",
    },
    Migration {
        version: 4,
        name: "notes on saved files",
        sql: "ALTER TABLE drive_items ADD COLUMN note TEXT",
    },
];

/// A person's own tree hangs from this parent.
pub const ROOT: &str = "";

/// Names longer than this are refused.
const MAX_NAME: usize = 255;
/// How deep folders go.
const MAX_DEPTH: usize = 64;

fn now() -> i64 {
    ortus_core::program::now_ms()
}

pub fn new_id() -> String {
    format!("d{}", ortus_core::auth::random_hex(12))
}

/// What one may do with an item, weakest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    View,
    Comment,
    Edit,
    Owner,
}

impl Role {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "view" => Some(Role::View),
            "comment" => Some(Role::Comment),
            "edit" => Some(Role::Edit),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::View => "view",
            Role::Comment => "comment",
            Role::Edit => "edit",
            Role::Owner => "owner",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: String,
    pub owner_id: String,
    pub parent_id: String,
    /// "folder" or "file".
    pub kind: String,
    pub name: String,
    pub size: i64,
    pub mime: String,
    pub version: i64,
    pub label: Option<String>,
    /// Where it came from, such as "mail" for a saved attachment.
    pub source: Option<String>,
    pub created_at: i64,
    pub modified_at: i64,
    pub modified_by: String,
    pub trashed_at: Option<i64>,
}

impl Item {
    pub fn is_folder(&self) -> bool {
        self.kind == "folder"
    }
}

const ITEM: &str = "SELECT id, owner_id, parent_id, kind, name, size, mime, version, label, source, created_at, modified_at, modified_by, trashed_at FROM drive_items";

fn item_of(r: &Row) -> Item {
    Item {
        id: r.text(0).unwrap_or_default().into(),
        owner_id: r.text(1).unwrap_or_default().into(),
        parent_id: r.text(2).unwrap_or_default().into(),
        kind: r.text(3).unwrap_or_default().into(),
        name: r.text(4).unwrap_or_default().into(),
        size: r.int(5).unwrap_or(0),
        mime: r.text(6).unwrap_or_default().into(),
        version: r.int(7).unwrap_or(0),
        label: r.text(8).map(str::to_string),
        source: r.text(9).map(str::to_string),
        created_at: r.int(10).unwrap_or(0),
        modified_at: r.int(11).unwrap_or(0),
        modified_by: r.text(12).unwrap_or_default().into(),
        trashed_at: r.int(13),
    }
}

/// Why a change was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum DriveError {
    NotFound,
    /// Not allowed for this person.
    Forbidden,
    /// Another item there has that name.
    NameTaken,
    BadName,
    /// The account's quota would be passed.
    OverQuota,
    /// A folder into itself, or too deep.
    BadMove,
    Store(String),
}

impl From<StoreError> for DriveError {
    fn from(e: StoreError) -> Self {
        DriveError::Store(e.to_string())
    }
}

impl From<ortus_core::files::FilesError> for DriveError {
    fn from(e: ortus_core::files::FilesError) -> Self {
        DriveError::Store(e.to_string())
    }
}

pub type DriveResult<T> = Result<T, DriveError>;

/// A name a file or folder may have.
pub fn name_ok(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n == name
        && n.chars().count() <= MAX_NAME
        && n != "."
        && n != ".."
        && !n.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

pub async fn item(store: &dyn AsyncStore, id: &str) -> StoreResult<Option<Item>> {
    Ok(store
        .query(&format!("{ITEM} WHERE id = $1"), &[id.into()])
        .await?
        .first()
        .map(item_of))
}

async fn item_tx(tx: &mut dyn AsyncTx, id: &str) -> StoreResult<Option<Item>> {
    Ok(tx
        .query(&format!("{ITEM} WHERE id = $1"), &[id.into()])
        .await?
        .first()
        .map(item_of))
}

/// The folders above an item, nearest first (not the item itself).
pub async fn ancestors(store: &dyn AsyncStore, item: &Item) -> StoreResult<Vec<Item>> {
    let mut out = Vec::new();
    let mut parent = item.parent_id.clone();
    while !parent.is_empty() && out.len() < MAX_DEPTH {
        let Some(p) = self::item(store, &parent).await? else {
            break;
        };
        parent = p.parent_id.clone();
        out.push(p);
    }
    Ok(out)
}

/// The groups a person is in.
pub async fn groups_of(store: &dyn AsyncStore, user_id: &str) -> StoreResult<Vec<String>> {
    Ok(store
        .query(
            "SELECT group_id FROM core_group_members WHERE user_id = $1",
            &[user_id.into()],
        )
        .await?
        .iter()
        .filter_map(|r| r.text(0).map(str::to_string))
        .collect())
}

/// What `user` may do with `item`: owner of the tree, or the strongest
/// share on it or any folder above it (to them or a group of theirs,
/// unexpired). Trashed items are the owner's only.
pub async fn role(store: &dyn AsyncStore, user_id: &str, item: &Item) -> StoreResult<Option<Role>> {
    if item.owner_id == user_id {
        return Ok(Some(Role::Owner));
    }
    if item.trashed_at.is_some() {
        return Ok(None);
    }
    let mut ids = vec![item.id.clone()];
    for a in ancestors(store, item).await? {
        if a.trashed_at.is_some() {
            return Ok(None);
        }
        ids.push(a.id);
    }
    let groups = groups_of(store, user_id).await?;
    let now = now();
    let mut best: Option<Role> = None;
    for id in &ids {
        for r in store.query("SELECT kind, target, role, expires_at FROM drive_shares WHERE item_id = $1 AND kind IN ('person', 'group')", &[id.as_str().into()]).await? {
            let (kind, target) = (r.text(0).unwrap_or_default(), r.text(1).unwrap_or_default());
            let theirs = (kind == "person" && target == user_id) || (kind == "group" && groups.iter().any(|g| g == target));
            if !theirs || r.int(3).is_some_and(|e| e <= now) {
                continue;
            }
            if let Some(role) = Role::parse(r.text(2).unwrap_or_default()) {
                best = best.max(Some(role));
            }
        }
    }
    Ok(best)
}

/// The items in a folder (or a person's root), not trashed, folders first.
pub async fn children(
    store: &dyn AsyncStore,
    owner_id: &str,
    parent_id: &str,
) -> StoreResult<Vec<Item>> {
    let rows = if parent_id.is_empty() {
        store.query(&format!("{ITEM} WHERE owner_id = $1 AND parent_id = '' AND trashed_at IS NULL ORDER BY kind DESC, name"), &[owner_id.into()]).await?
    } else {
        store
            .query(
                &format!(
                    "{ITEM} WHERE parent_id = $1 AND trashed_at IS NULL ORDER BY kind DESC, name"
                ),
                &[parent_id.into()],
            )
            .await?
    };
    Ok(rows.iter().map(item_of).collect())
}

async fn name_taken(
    tx: &mut dyn AsyncTx,
    owner_id: &str,
    parent_id: &str,
    name: &str,
    except: &str,
) -> StoreResult<bool> {
    let rows = tx
        .query(
            "SELECT id FROM drive_items WHERE owner_id = $1 AND parent_id = $2 AND LOWER(name) = LOWER($3) AND trashed_at IS NULL AND id <> $4",
            &[owner_id.into(), parent_id.into(), name.into(), except.into()],
        )
        .await?;
    Ok(!rows.is_empty())
}

/// The owner of the tree a new item goes into, after checking the person
/// may add there (owner or editor).
async fn place(store: &dyn AsyncStore, user_id: &str, parent_id: &str) -> DriveResult<String> {
    if parent_id.is_empty() {
        return Ok(user_id.to_string());
    }
    let parent = item(store, parent_id)
        .await?
        .filter(|p| p.is_folder() && p.trashed_at.is_none())
        .ok_or(DriveError::NotFound)?;
    match role(store, user_id, &parent).await? {
        Some(Role::Owner | Role::Edit) => Ok(parent.owner_id),
        Some(_) => Err(DriveError::Forbidden),
        None => Err(DriveError::NotFound),
    }
}

pub async fn create_folder(
    store: &dyn AsyncStore,
    user_id: &str,
    parent_id: &str,
    name: &str,
) -> DriveResult<Item> {
    if !name_ok(name) {
        return Err(DriveError::BadName);
    }
    let owner = place(store, user_id, parent_id).await?;
    let id = new_id();
    let now = now();
    let mut tx = store.begin().await?;
    if name_taken(tx.as_mut(), &owner, parent_id, name, "").await? {
        tx.rollback().await?;
        return Err(DriveError::NameTaken);
    }
    tx.execute(
        "INSERT INTO drive_items (id, owner_id, parent_id, kind, name, size, mime, version, created_at, modified_at, modified_by) VALUES ($1, $2, $3, 'folder', $4, 0, '', 0, $5, $5, $6)",
        &[id.as_str().into(), owner.as_str().into(), parent_id.into(), name.into(), now.into(), user_id.into()],
    )
    .await?;
    tx.commit().await?;
    item(store, &id).await?.ok_or(DriveError::NotFound)
}

/// Bytes a person's files take, every version counted.
pub async fn used(store: &dyn AsyncStore, owner_id: &str) -> StoreResult<i64> {
    Ok(store
        .query("SELECT CAST(COALESCE(SUM(v.size), 0) AS BIGINT) FROM drive_versions v JOIN drive_items i ON i.id = v.item_id WHERE i.owner_id = $1", &[owner_id.into()])
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0))
}

/// The account's quota in bytes, if it has one.
pub async fn quota(store: &dyn AsyncStore, owner_id: &str) -> StoreResult<Option<i64>> {
    Ok(store
        .query(
            "SELECT quota_bytes FROM core_users WHERE id = $1",
            &[owner_id.into()],
        )
        .await?
        .first()
        .and_then(|r| r.int(0)))
}

/// Whether `more` bytes fit in the owner's quota.
pub async fn fits(store: &dyn AsyncStore, owner_id: &str, more: i64) -> StoreResult<bool> {
    Ok(match quota(store, owner_id).await? {
        Some(q) => used(store, owner_id).await? + more <= q,
        None => true,
    })
}

/// Where a version's contents are in the file interface.
pub fn blob_key(owner_id: &str) -> String {
    format!(
        "drive/{}/{}",
        owner_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
        ortus_core::auth::random_hex(16)
    )
}

/// What a new file or version is.
pub struct Upload<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    /// Its contents, already in the file interface under this key.
    pub blob: &'a str,
    pub size: i64,
    pub source: Option<&'a str>,
}

/// Adds a file whose contents are already stored (under `upload.blob`).
/// A file of that name in the folder gets a new version instead.
pub async fn add_file(
    store: &dyn AsyncStore,
    user_id: &str,
    parent_id: &str,
    upload: &Upload<'_>,
) -> DriveResult<Item> {
    if !name_ok(upload.name) {
        return Err(DriveError::BadName);
    }
    let owner = place(store, user_id, parent_id).await?;
    if !fits(store, &owner, upload.size).await? {
        return Err(DriveError::OverQuota);
    }
    let now = now();
    let mut tx = store.begin().await?;
    let same = tx
        .query(
            &format!("{ITEM} WHERE owner_id = $1 AND parent_id = $2 AND LOWER(name) = LOWER($3) AND trashed_at IS NULL"),
            &[owner.as_str().into(), parent_id.into(), upload.name.into()],
        )
        .await?
        .first()
        .map(item_of);
    let id = match same {
        Some(existing) if existing.is_folder() => {
            tx.rollback().await?;
            return Err(DriveError::NameTaken);
        }
        Some(existing) => {
            let version = existing.version + 1;
            tx.execute(
                "UPDATE drive_items SET size = $1, mime = $2, version = $3, modified_at = $4, modified_by = $5 WHERE id = $6",
                &[upload.size.into(), upload.mime.into(), version.into(), now.into(), user_id.into(), existing.id.as_str().into()],
            )
            .await?;
            tx.execute(
                "INSERT INTO drive_versions (item_id, version, blob, size, created_at, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
                &[existing.id.as_str().into(), version.into(), upload.blob.into(), upload.size.into(), now.into(), user_id.into()],
            )
            .await?;
            existing.id
        }
        None => {
            let id = new_id();
            tx.execute(
                "INSERT INTO drive_items (id, owner_id, parent_id, kind, name, size, mime, version, source, created_at, modified_at, modified_by) VALUES ($1, $2, $3, 'file', $4, $5, $6, 1, $7, $8, $8, $9)",
                &[id.as_str().into(), owner.as_str().into(), parent_id.into(), upload.name.into(), upload.size.into(), upload.mime.into(), upload.source.into(), now.into(), user_id.into()],
            )
            .await?;
            tx.execute(
                "INSERT INTO drive_versions (item_id, version, blob, size, created_at, created_by) VALUES ($1, 1, $2, $3, $4, $5)",
                &[id.as_str().into(), upload.blob.into(), upload.size.into(), now.into(), user_id.into()],
            )
            .await?;
            id
        }
    };
    tx.commit().await?;
    touch(store, user_id, &id).await?;
    item(store, &id).await?.ok_or(DriveError::NotFound)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Version {
    pub version: i64,
    #[serde(skip)]
    pub blob: String,
    pub size: i64,
    pub created_at: i64,
    pub created_by: String,
}

pub async fn versions(store: &dyn AsyncStore, item_id: &str) -> StoreResult<Vec<Version>> {
    Ok(store
        .query("SELECT version, blob, size, created_at, created_by FROM drive_versions WHERE item_id = $1 ORDER BY version DESC", &[item_id.into()])
        .await?
        .iter()
        .map(|r| Version {
            version: r.int(0).unwrap_or(0),
            blob: r.text(1).unwrap_or_default().into(),
            size: r.int(2).unwrap_or(0),
            created_at: r.int(3).unwrap_or(0),
            created_by: r.text(4).unwrap_or_default().into(),
        })
        .collect())
}

/// A version of a file (the current one when `version` is `None`).
pub async fn version(
    store: &dyn AsyncStore,
    item: &Item,
    version: Option<i64>,
) -> StoreResult<Option<Version>> {
    let want = version.unwrap_or(item.version);
    Ok(versions(store, &item.id)
        .await?
        .into_iter()
        .find(|v| v.version == want))
}

/// Makes an older version the current one (as a new version).
pub async fn restore_version(
    store: &dyn AsyncStore,
    user_id: &str,
    item: &Item,
    version: i64,
) -> DriveResult<Item> {
    let old = self::version(store, item, Some(version))
        .await?
        .ok_or(DriveError::NotFound)?;
    let now = now();
    let next = item.version + 1;
    let mut tx = store.begin().await?;
    tx.execute(
        "INSERT INTO drive_versions (item_id, version, blob, size, created_at, created_by) VALUES ($1, $2, $3, $4, $5, $6)",
        &[item.id.as_str().into(), next.into(), old.blob.as_str().into(), old.size.into(), now.into(), user_id.into()],
    )
    .await?;
    tx.execute(
        "UPDATE drive_items SET size = $1, version = $2, modified_at = $3, modified_by = $4 WHERE id = $5",
        &[old.size.into(), next.into(), now.into(), user_id.into(), item.id.as_str().into()],
    )
    .await?;
    tx.commit().await?;
    self::item(store, &item.id)
        .await?
        .ok_or(DriveError::NotFound)
}

pub async fn rename(
    store: &dyn AsyncStore,
    user_id: &str,
    item: &Item,
    name: &str,
) -> DriveResult<Item> {
    if !name_ok(name) {
        return Err(DriveError::BadName);
    }
    let mut tx = store.begin().await?;
    if name_taken(tx.as_mut(), &item.owner_id, &item.parent_id, name, &item.id).await? {
        tx.rollback().await?;
        return Err(DriveError::NameTaken);
    }
    tx.execute(
        "UPDATE drive_items SET name = $1, modified_at = $2, modified_by = $3 WHERE id = $4",
        &[
            name.into(),
            now().into(),
            user_id.into(),
            item.id.as_str().into(),
        ],
    )
    .await?;
    tx.commit().await?;
    self::item(store, &item.id)
        .await?
        .ok_or(DriveError::NotFound)
}

/// Moves an item into another folder of the same tree (or its root).
pub async fn move_to(
    store: &dyn AsyncStore,
    user_id: &str,
    item: &Item,
    parent_id: &str,
) -> DriveResult<Item> {
    let owner = place(store, user_id, parent_id).await?;
    if owner != item.owner_id {
        // Across trees, a file changes owner and quota: not by moving.
        return Err(DriveError::BadMove);
    }
    if !parent_id.is_empty() {
        let target = self::item(store, parent_id)
            .await?
            .ok_or(DriveError::NotFound)?;
        if target.id == item.id
            || ancestors(store, &target)
                .await?
                .iter()
                .any(|a| a.id == item.id)
        {
            return Err(DriveError::BadMove);
        }
        if ancestors(store, &target).await?.len() + 1 >= MAX_DEPTH {
            return Err(DriveError::BadMove);
        }
    }
    let mut tx = store.begin().await?;
    if name_taken(tx.as_mut(), &item.owner_id, parent_id, &item.name, &item.id).await? {
        tx.rollback().await?;
        return Err(DriveError::NameTaken);
    }
    tx.execute(
        "UPDATE drive_items SET parent_id = $1, modified_at = $2, modified_by = $3 WHERE id = $4",
        &[
            parent_id.into(),
            now().into(),
            user_id.into(),
            item.id.as_str().into(),
        ],
    )
    .await?;
    tx.commit().await?;
    self::item(store, &item.id)
        .await?
        .ok_or(DriveError::NotFound)
}

/// To the trash (with what is inside a folder, which goes with it).
pub async fn trash(store: &dyn AsyncStore, item: &Item) -> StoreResult<()> {
    store
        .execute(
            "UPDATE drive_items SET trashed_at = $1 WHERE id = $2",
            &[now().into(), item.id.as_str().into()],
        )
        .await?;
    Ok(())
}

/// Back from the trash; a name now taken gets " (restored)".
pub async fn restore(store: &dyn AsyncStore, item: &Item) -> DriveResult<Item> {
    let mut tx = store.begin().await?;
    let mut name = item.name.clone();
    // The folder it was in may be gone: then it comes back to the root.
    let parent_ok = item.parent_id.is_empty()
        || item_tx(tx.as_mut(), &item.parent_id)
            .await?
            .is_some_and(|p| p.trashed_at.is_none());
    let parent = if parent_ok {
        item.parent_id.clone()
    } else {
        String::new()
    };
    if name_taken(tx.as_mut(), &item.owner_id, &parent, &name, &item.id).await? {
        name = format!("{name} (restored)");
    }
    tx.execute(
        "UPDATE drive_items SET trashed_at = NULL, parent_id = $1, name = $2 WHERE id = $3",
        &[
            parent.as_str().into(),
            name.as_str().into(),
            item.id.as_str().into(),
        ],
    )
    .await?;
    tx.commit().await?;
    self::item(store, &item.id)
        .await?
        .ok_or(DriveError::NotFound)
}

/// What is in a person's trash (the items trashed themselves, not what
/// went with a trashed folder).
pub async fn trashed(store: &dyn AsyncStore, owner_id: &str) -> StoreResult<Vec<Item>> {
    Ok(store
        .query(&format!("{ITEM} WHERE owner_id = $1 AND trashed_at IS NOT NULL ORDER BY trashed_at DESC LIMIT 500"), &[owner_id.into()])
        .await?
        .iter()
        .map(item_of)
        .collect())
}

/// Deletes an item for good, with everything under it and every version's
/// contents.
pub async fn destroy(store: &dyn AsyncStore, files: &Files, item: &Item) -> StoreResult<()> {
    let mut queue = vec![item.id.clone()];
    let mut all = Vec::new();
    while let Some(id) = queue.pop() {
        if all.len() > 100_000 {
            break;
        }
        for r in store
            .query(
                "SELECT id FROM drive_items WHERE parent_id = $1",
                &[id.as_str().into()],
            )
            .await?
        {
            if let Some(child) = r.text(0) {
                queue.push(child.to_string());
            }
        }
        all.push(id);
    }
    for id in &all {
        let blobs: Vec<String> = store
            .query(
                "SELECT blob FROM drive_versions WHERE item_id = $1",
                &[id.as_str().into()],
            )
            .await?
            .iter()
            .filter_map(|r| r.text(0).map(str::to_string))
            .collect();
        let mut tx = store.begin().await?;
        for sql in [
            "DELETE FROM drive_versions WHERE item_id = $1",
            "DELETE FROM drive_shares WHERE item_id = $1",
            "DELETE FROM drive_stars WHERE item_id = $1",
            "DELETE FROM drive_recent WHERE item_id = $1",
            "DELETE FROM drive_items WHERE id = $1",
        ] {
            tx.execute(sql, &[id.as_str().into()]).await?;
        }
        tx.commit().await?;
        for blob in blobs {
            // A version restored from another shares its contents.
            let still = store
                .query(
                    "SELECT 1 FROM drive_versions WHERE blob = $1",
                    &[blob.as_str().into()],
                )
                .await?;
            if still.is_empty() {
                let _ = files.delete(&blob).await;
            }
        }
    }
    Ok(())
}

/// Trash older than `days` goes for good.
pub async fn empty_old_trash(
    store: &dyn AsyncStore,
    files: &Files,
    days: i64,
) -> StoreResult<usize> {
    let before = now() - days * 86_400_000;
    let old: Vec<Item> = store
        .query(
            &format!("{ITEM} WHERE trashed_at IS NOT NULL AND trashed_at < $1 LIMIT 1000"),
            &[before.into()],
        )
        .await?
        .iter()
        .map(item_of)
        .collect();
    for item in &old {
        destroy(store, files, item).await?;
    }
    Ok(old.len())
}

pub async fn star(
    store: &dyn AsyncStore,
    user_id: &str,
    item_id: &str,
    on: bool,
) -> StoreResult<()> {
    if on {
        let mut tx = store.begin().await?;
        tx.execute(
            "DELETE FROM drive_stars WHERE user_id = $1 AND item_id = $2",
            &[user_id.into(), item_id.into()],
        )
        .await?;
        tx.execute(
            "INSERT INTO drive_stars (user_id, item_id) VALUES ($1, $2)",
            &[user_id.into(), item_id.into()],
        )
        .await?;
        tx.commit().await?;
    } else {
        store
            .execute(
                "DELETE FROM drive_stars WHERE user_id = $1 AND item_id = $2",
                &[user_id.into(), item_id.into()],
            )
            .await?;
    }
    Ok(())
}

pub async fn starred(store: &dyn AsyncStore, user_id: &str) -> StoreResult<Vec<Item>> {
    Ok(store
        .query(&format!("{ITEM} WHERE id IN (SELECT item_id FROM drive_stars WHERE user_id = $1) AND trashed_at IS NULL ORDER BY name"), &[user_id.into()])
        .await?
        .iter()
        .map(item_of)
        .collect())
}

pub async fn is_starred(store: &dyn AsyncStore, user_id: &str, item_id: &str) -> StoreResult<bool> {
    Ok(!store
        .query(
            "SELECT 1 FROM drive_stars WHERE user_id = $1 AND item_id = $2",
            &[user_id.into(), item_id.into()],
        )
        .await?
        .is_empty())
}

/// Notes that the person opened or changed a file.
pub async fn touch(store: &dyn AsyncStore, user_id: &str, item_id: &str) -> StoreResult<()> {
    let mut tx = store.begin().await?;
    tx.execute(
        "DELETE FROM drive_recent WHERE user_id = $1 AND item_id = $2",
        &[user_id.into(), item_id.into()],
    )
    .await?;
    tx.execute(
        "INSERT INTO drive_recent (user_id, item_id, at) VALUES ($1, $2, $3)",
        &[user_id.into(), item_id.into(), now().into()],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Files the person opened or changed lately, newest first.
pub async fn recent(store: &dyn AsyncStore, user_id: &str, limit: i64) -> StoreResult<Vec<Item>> {
    Ok(store
        .query(
            "SELECT i.id, i.owner_id, i.parent_id, i.kind, i.name, i.size, i.mime, i.version, i.label, i.source, i.created_at, i.modified_at, i.modified_by, i.trashed_at FROM drive_items i JOIN drive_recent r ON r.item_id = i.id AND r.user_id = $1 WHERE i.trashed_at IS NULL AND i.kind = 'file' ORDER BY r.at DESC LIMIT $2",
            &[user_id.into(), limit.into()],
        )
        .await?
        .iter()
        .map(item_of)
        .collect())
}

/// Files of a person's that came from somewhere ("mail"), newest first.
pub async fn from_source(
    store: &dyn AsyncStore,
    owner_id: &str,
    source: &str,
) -> StoreResult<Vec<Item>> {
    Ok(store
        .query(&format!("{ITEM} WHERE owner_id = $1 AND source = $2 AND trashed_at IS NULL ORDER BY modified_at DESC LIMIT 200"), &[owner_id.into(), source.into()])
        .await?
        .iter()
        .map(item_of)
        .collect())
}

/// A name free in a folder: the name itself, or "name (2).ext" and on.
pub async fn free_name(
    store: &dyn AsyncStore,
    owner_id: &str,
    parent_id: &str,
    name: &str,
) -> StoreResult<String> {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1..1000 {
        let candidate = if n == 1 {
            name.to_string()
        } else {
            format!("{stem} ({n}){ext}")
        };
        let taken = store
            .query("SELECT 1 FROM drive_items WHERE owner_id = $1 AND parent_id = $2 AND LOWER(name) = LOWER($3) AND trashed_at IS NULL", &[owner_id.into(), parent_id.into(), candidate.as_str().into()])
            .await?;
        if taken.is_empty() {
            return Ok(candidate);
        }
    }
    Ok(format!("{stem} ({}){ext}", ortus_core::auth::random_hex(3)))
}

/// A note on how a file came (a saved attachment's sender).
pub async fn note(store: &dyn AsyncStore, item_id: &str) -> StoreResult<Option<String>> {
    Ok(store
        .query(
            "SELECT note FROM drive_items WHERE id = $1",
            &[item_id.into()],
        )
        .await?
        .first()
        .and_then(|r| r.text(0).map(str::to_string)))
}

/// Sets the classification label of an item (checked by the caller).
pub async fn set_label(
    store: &dyn AsyncStore,
    item_id: &str,
    label: Option<&str>,
) -> StoreResult<()> {
    store
        .execute(
            "UPDATE drive_items SET label = $1 WHERE id = $2",
            &[label.into(), item_id.into()],
        )
        .await?;
    Ok(())
}

/// The label that governs an item: its own, else the nearest folder's.
pub async fn effective_label(store: &dyn AsyncStore, item: &Item) -> StoreResult<Option<String>> {
    if item.label.is_some() {
        return Ok(item.label.clone());
    }
    Ok(ancestors(store, item)
        .await?
        .into_iter()
        .find_map(|a| a.label))
}

/// Everything a person's account leaves behind when it is removed.
pub async fn forget(tx: &mut dyn AsyncTx, user_id: &str) -> StoreResult<()> {
    for sql in [
        "DELETE FROM drive_stars WHERE user_id = $1",
        "DELETE FROM drive_recent WHERE user_id = $1",
        "DELETE FROM drive_shares WHERE kind = 'person' AND target = $1",
        "DELETE FROM drive_shares WHERE item_id IN (SELECT id FROM drive_items WHERE owner_id = $1)",
        "DELETE FROM drive_versions WHERE item_id IN (SELECT id FROM drive_items WHERE owner_id = $1)",
        "DELETE FROM drive_items WHERE owner_id = $1",
    ] {
        tx.execute(sql, &[Value::from(user_id)]).await?;
    }
    Ok(())
}
