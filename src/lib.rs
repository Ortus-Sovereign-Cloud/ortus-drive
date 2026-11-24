//! Drive: files, versions, sharing by label, previews.
//!
//! An Ortus app crate: it depends on `ortus-core` only and hands its apps
//! to a program through [`apps`].

pub mod api;
pub mod index;
pub mod preview;
pub mod public;
pub mod restore;
pub mod service;
pub mod share;
pub mod store;

use std::sync::Arc;

use ortus_core::app::{App, AppInfo, Group, JobKind, OutsideJobKind};
use ortus_core::instance::Instance;
use ortus_core::storage::{AsyncTx, BoxFuture, Migration};

/// The built web client (web/dist).
#[derive(rust_embed::RustEmbed)]
#[folder = "web/dist"]
struct Web;

/// Hourly: unfinished uploads and month-old trash go.
pub const SWEEP_JOB: &str = "drive.sweep";

/// The drive app.
pub struct Drive;

impl App for Drive {
    fn info(&self) -> AppInfo {
        AppInfo {
            id: "drive",
            group: Group::Create,
            path: "/drive",
            order: 1,
        }
    }

    fn migrations(&self) -> &'static [Migration] {
        store::MIGRATIONS
    }

    fn web_file(&self, path: &str) -> Option<std::borrow::Cow<'static, [u8]>> {
        Web::get(path).map(|file| file.data)
    }

    fn routes(&self) -> Vec<ortus_core::api::Route> {
        let mut all = api::routes();
        all.extend(share::routes());
        all.extend(public::routes());
        all.extend(preview::routes());
        all
    }

    fn provide(&self, instance: &Arc<Instance>, services: &ortus_core::services::Services) {
        let drive: Arc<dyn ortus_core::interfaces::Drive> = Arc::new(service::Service {
            instance: Arc::downgrade(instance),
        });
        services.provide("drive", drive);
    }

    fn search_owners<'a>(
        &'a self,
        instance: &'a Instance,
        user: &'a str,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(index::owners_sharing_with(instance, user))
    }

    fn search_allowed<'a>(
        &'a self,
        instance: &'a Instance,
        user: &'a str,
        ids: Vec<String>,
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(index::visible(instance, user, ids))
    }

    // Link tokens are sealed: a key rotation re-seals them.
    fn sealed(&self) -> Vec<ortus_core::secrets::Place> {
        vec![ortus_core::secrets::Place {
            table: "drive_shares",
            id: "id",
            column: "token_sealed",
            fields: &[],
        }]
    }

    fn periodic(&self) -> Vec<(&'static str, i64)> {
        vec![(SWEEP_JOB, 60 * 60 * 1000)]
    }

    fn jobs(&self, instance: &Arc<Instance>) -> Vec<JobKind> {
        let instance = Arc::clone(instance);
        vec![JobKind {
            kind: store::FORGET_FILES_JOB.into(),
            handler: Box::new(move |job, _tx| {
                let (instance, owner) = (Arc::clone(&instance), job.payload.clone());
                Box::pin(async move { store::forget_files(&instance.files, &owner).await })
            }),
        }]
    }

    fn outside_jobs(&self, instance: &Arc<Instance>) -> Vec<OutsideJobKind> {
        let instance = Arc::clone(instance);
        vec![OutsideJobKind {
            kind: SWEEP_JOB.into(),
            handler: Box::new(move |_| {
                let instance = Arc::clone(&instance);
                Box::pin(async move {
                    share::hash_old_links(&instance).await?;
                    api::sweep_uploads(&instance).await
                })
            }),
        }]
    }

    fn forget_user<'a>(
        &'a self,
        tx: &'a mut dyn AsyncTx,
        user_id: &'a str,
        _rules: &'a ortus_core::retention::Rules,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move { store::forget(tx, user_id).await.map_err(|e| e.to_string()) })
    }

    fn restorable(&self) -> &'static [&'static str] {
        &["folder"]
    }

    fn restore_choices<'a>(
        &'a self,
        point: &'a ortus_core::points::Snapshot,
        user: &'a str,
        _what: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, String)>, String>> {
        Box::pin(restore::choices(point, user))
    }

    fn restore<'a>(
        &'a self,
        instance: &'a Arc<Instance>,
        point: &'a ortus_core::points::Snapshot,
        user: &'a str,
        _what: &'a str,
        id: &'a str,
        into: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(restore::folder(instance, point, user, id, into))
    }
}

/// The apps of this crate, for a program to run.
pub fn apps() -> Vec<Arc<dyn App>> {
    vec![Arc::new(Drive)]
}
