//! Drive: files, versions, sharing by label, previews.
//!
//! An Ortus app crate: it depends on `ortus-core` only and hands its apps
//! to a program through [`apps`].

use std::sync::Arc;

use ortus_core::app::{App, AppInfo, Group};

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
}

/// The apps of this crate, for a program to run.
pub fn apps() -> Vec<Arc<dyn App>> {
    vec![Arc::new(Drive)]
}
