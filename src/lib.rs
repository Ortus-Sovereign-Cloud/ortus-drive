//! Drive: files, versions, sharing by label, previews.
//!
//! An Ortus app crate: it depends on `ortus-core` only and hands its apps
//! to a program through [`apps`].

pub mod api;
pub mod index;
pub mod public;
pub mod share;
pub mod store;

use std::sync::Arc;

use ortus_core::app::{App, AppInfo, Group, OutsideJobKind};
use ortus_core::instance::Instance;
use ortus_core::storage::{AsyncTx, BoxFuture, Migration};

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

    fn routes(&self) -> Vec<ortus_core::api::Route> {
        let mut all = api::routes();
        all.extend(share::routes());
        all.extend(public::routes());
        all
    }

    fn periodic(&self) -> Vec<(&'static str, i64)> {
        vec![(SWEEP_JOB, 60 * 60 * 1000)]
    }

    fn outside_jobs(&self, instance: &Arc<Instance>) -> Vec<OutsideJobKind> {
        let instance = Arc::clone(instance);
        vec![OutsideJobKind {
            kind: SWEEP_JOB.into(),
            handler: Box::new(move |_| {
                let instance = Arc::clone(&instance);
                Box::pin(async move { api::sweep_uploads(&instance).await })
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
}

/// The apps of this crate, for a program to run.
pub fn apps() -> Vec<Arc<dyn App>> {
    vec![Arc::new(Drive)]
}
