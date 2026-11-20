//! Keeps the search index up to date with the drive (task 5.6/5.7 fills
//! these in; until then they do nothing).

use ortus_core::instance::Instance;

use crate::store::Item;

/// A file or folder was added or changed.
pub async fn item_changed(_instance: &Instance, _item: &Item) {}

/// A file or folder went to the trash or away.
pub async fn item_removed(_instance: &Instance, _id: &str) {}
