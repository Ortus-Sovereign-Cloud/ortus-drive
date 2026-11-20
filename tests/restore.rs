//! Bringing a Drive folder back as it was, on both shapes: its folders and
//! files (each file's newest version then) go into a new folder at the top
//! of the person's Drive; only the person's own folders are offered.

mod common;

use ortus_core::auth::accounts::Role;
use ortus_core::files::Files;
use ortus_core::instance::testing_auth::signed_in;
use ortus_core::points::{self, Snapshot};
use ortus_core::runtime::block_on;
use ortus_drive::store::{self, Upload, ROOT};

#[test]
fn a_drive_folder_comes_back_into_a_new_folder() {
    for (name, instance, _keep) in common::installations() {
        let s = instance.store.as_ref();
        signed_in(&instance, "olena@ministry.example", Role::Member);
        signed_in(&instance, "marek@ministry.example", Role::Member);
        let id = |e: &str| {
            block_on(ortus_core::auth::accounts::by_email(s, e))
                .unwrap()
                .unwrap()
                .id
        };
        let (olena, marek) = (id("olena@ministry.example"), id("marek@ministry.example"));
        let put = |parent: &str, file: &str, text: &str| {
            let key = store::blob_key(&olena);
            block_on(instance.files.put(&key, text.as_bytes())).unwrap();
            block_on(store::add_file(
                s,
                &olena,
                parent,
                &Upload {
                    name: file,
                    mime: "text/plain",
                    blob: &key,
                    size: text.len() as i64,
                    source: None,
                },
            ))
            .unwrap()
        };
        let shelters = block_on(store::create_folder(s, &olena, ROOT, "Shelters")).unwrap();
        let signed = block_on(store::create_folder(s, &olena, &shelters.id, "Signed")).unwrap();
        put(&shelters.id, "list.txt", "school, gym");
        put(&shelters.id, "list.txt", "school, gym, church");
        put(&signed.id, "agreement.txt", "clause 4");

        let dir = common::dir();
        let dest = Files::local(&dir.0.join("backups")).unwrap();
        let at = ortus_core::program::now_ms();
        block_on(points::take(&instance, &dest, at, true)).unwrap();
        // Since then: the folder's list changed again.
        put(&shelters.id, "list.txt", "gym only");

        let point = block_on(Snapshot::open(dest, &points::name_at(at))).unwrap();
        let drive = ortus_drive::apps().remove(0);
        let hers = block_on(drive.restore_choices(&point, &olena, "folder")).unwrap();
        assert!(
            hers.iter().any(|(_, n)| n == "Shelters / Signed"),
            "{name}: {hers:?}"
        );
        let his = block_on(drive.restore_choices(&point, &marek, "folder")).unwrap();
        assert!(his.is_empty(), "{name}: {his:?}");

        let path = block_on(drive.restore(
            &instance,
            &point,
            &olena,
            "folder",
            &shelters.id,
            "Shelters (2026-11-17 18:00)",
        ))
        .unwrap();
        let top = block_on(store::children(s, &olena, ROOT))
            .unwrap()
            .into_iter()
            .find(|i| i.name == "Shelters (2026-11-17 18:00)")
            .expect("the new folder");
        assert_eq!(format!("/drive?folder={}", top.id), path, "{name}");
        let inside = block_on(store::children(s, &olena, &top.id)).unwrap();
        let names: Vec<&str> = inside.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(vec!["Signed", "list.txt"], names, "{name}");
        let list = inside.iter().find(|i| i.name == "list.txt").unwrap();
        let versions = block_on(store::versions(s, &list.id)).unwrap();
        let blob = &versions.first().unwrap().blob;
        assert_eq!(
            Some(b"school, gym, church".to_vec()),
            block_on(instance.files.get(blob)).unwrap(),
            "{name}: the newest version then"
        );
        let sub = inside.iter().find(|i| i.name == "Signed").unwrap();
        let deeper = block_on(store::children(s, &olena, &sub.id)).unwrap();
        assert_eq!(
            vec!["agreement.txt"],
            deeper.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            "{name}"
        );
    }
}
