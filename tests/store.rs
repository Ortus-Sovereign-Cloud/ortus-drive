//! The drive's store on both installation shapes (SQLite with a directory,
//! PostgreSQL with S3): folders, files and versions with their contents in
//! the file interface, names, moves, trash, stars, recent files, quotas and
//! who may do what through shares.

mod common;

use ortus_core::auth::accounts::Role as Account;
use ortus_core::instance::testing_auth::signed_in;
use ortus_core::runtime::block_on;
use ortus_drive::store::{self, DriveError, Role, Upload};

fn put(instance: &ortus_core::instance::Instance, owner: &str, contents: &[u8]) -> String {
    let key = store::blob_key(owner);
    block_on(instance.files.put(&key, contents)).unwrap();
    key
}

#[test]
fn folders_files_versions_and_trash() {
    for (name, instance, _keep) in common::installations() {
        signed_in(&instance, "olena@ministry.example", Account::Member);
        let s = instance.store.as_ref();
        let olena = block_on(ortus_core::auth::accounts::by_email(
            s,
            "olena@ministry.example",
        ))
        .unwrap()
        .unwrap()
        .id;
        let emergency =
            block_on(store::create_folder(s, &olena, store::ROOT, "Emergency")).unwrap();
        let shelters =
            block_on(store::create_folder(s, &olena, &emergency.id, "Shelters")).unwrap();
        assert_eq!(
            Err(DriveError::NameTaken),
            block_on(store::create_folder(s, &olena, store::ROOT, "emergency")).map(|_| ()),
            "{name}"
        );
        assert_eq!(
            Err(DriveError::BadName),
            block_on(store::create_folder(s, &olena, store::ROOT, "a/b")).map(|_| ())
        );

        let v1 = put(&instance, &olena, b"first draft");
        let doc = block_on(store::add_file(
            s,
            &olena,
            &shelters.id,
            &Upload {
                name: "shelter-agreement.docx",
                mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                blob: &v1,
                size: 11,
                source: None,
            },
        ))
        .unwrap();
        assert_eq!((1, 11), (doc.version, doc.size));
        // The same name again is a new version.
        let v2 = put(&instance, &olena, b"second draft, longer");
        let doc2 = block_on(store::add_file(
            s,
            &olena,
            &shelters.id,
            &Upload {
                name: "shelter-agreement.docx",
                mime: "application/octet-stream",
                blob: &v2,
                size: 20,
                source: None,
            },
        ))
        .unwrap();
        assert_eq!(
            (doc.id.clone(), 2),
            (doc2.id.clone(), doc2.version),
            "{name}"
        );
        let versions = block_on(store::versions(s, &doc.id)).unwrap();
        assert_eq!(
            vec![2, 1],
            versions.iter().map(|v| v.version).collect::<Vec<_>>()
        );
        let restored = block_on(store::restore_version(s, &olena, &doc2, 1)).unwrap();
        assert_eq!((3, 11), (restored.version, restored.size), "{name}");
        assert_eq!(
            Some(b"first draft".to_vec()),
            block_on(
                instance.files.get(
                    &block_on(store::version(s, &restored, None))
                        .unwrap()
                        .unwrap()
                        .blob
                )
            )
            .unwrap()
        );

        // Lists, names, moves.
        assert_eq!(
            vec!["Shelters"],
            block_on(store::children(s, &olena, &emergency.id))
                .unwrap()
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
        );
        let renamed = block_on(store::rename(s, &olena, &restored, "agreement.docx")).unwrap();
        assert_eq!("agreement.docx", renamed.name);
        assert_eq!(
            Err(DriveError::BadMove),
            block_on(store::move_to(s, &olena, &emergency, &shelters.id)).map(|_| ()),
            "{name}: not into itself"
        );
        let moved = block_on(store::move_to(s, &olena, &renamed, store::ROOT)).unwrap();
        assert_eq!("", moved.parent_id);

        // Stars and recent.
        block_on(store::star(s, &olena, &moved.id, true)).unwrap();
        block_on(store::star(s, &olena, &moved.id, true)).unwrap();
        assert_eq!(1, block_on(store::starred(s, &olena)).unwrap().len());
        assert_eq!(
            moved.id,
            block_on(store::recent(s, &olena, 10)).unwrap()[0].id
        );

        // Trash: the folder and what is in it; back again; then for good.
        block_on(store::trash(s, &emergency)).unwrap();
        assert_eq!(
            1,
            block_on(store::children(s, &olena, store::ROOT))
                .unwrap()
                .len(),
            "{name}: only the file is left at the root"
        );
        let back = block_on(store::restore(
            s,
            &block_on(store::item(s, &emergency.id)).unwrap().unwrap(),
        ))
        .unwrap();
        assert!(back.trashed_at.is_none());
        block_on(store::trash(s, &moved)).unwrap();
        let gone = block_on(store::item(s, &moved.id)).unwrap().unwrap();
        block_on(store::destroy(s, &instance.files, &gone)).unwrap();
        assert!(block_on(store::item(s, &moved.id)).unwrap().is_none());
        assert_eq!(
            None,
            block_on(instance.files.get(&v1)).unwrap(),
            "{name}: contents gone"
        );
        assert_eq!(None, block_on(instance.files.get(&v2)).unwrap());
    }
}

#[test]
fn quotas_and_who_may_do_what() {
    for (name, instance, _keep) in common::installations() {
        signed_in(&instance, "olena@ministry.example", Account::Member);
        signed_in(&instance, "marek@ministry.example", Account::Member);
        signed_in(&instance, "sam@ministry.example", Account::Member);
        let s = instance.store.as_ref();
        let id = |e: &str| {
            block_on(ortus_core::auth::accounts::by_email(s, e))
                .unwrap()
                .unwrap()
                .id
        };
        let (olena, marek, sam) = (
            id("olena@ministry.example"),
            id("marek@ministry.example"),
            id("sam@ministry.example"),
        );
        block_on(s.execute(
            "UPDATE core_users SET quota_bytes = 100 WHERE id = $1",
            &[olena.as_str().into()],
        ))
        .unwrap();
        let folder = block_on(store::create_folder(s, &olena, store::ROOT, "Team")).unwrap();
        let blob = put(&instance, &olena, &[0u8; 80]);
        let file = block_on(store::add_file(
            s,
            &olena,
            &folder.id,
            &Upload {
                name: "a.bin",
                mime: "application/octet-stream",
                blob: &blob,
                size: 80,
                source: None,
            },
        ))
        .unwrap();
        let more = put(&instance, &olena, &[0u8; 30]);
        assert_eq!(
            Err(DriveError::OverQuota),
            block_on(store::add_file(
                s,
                &olena,
                &folder.id,
                &Upload {
                    name: "b.bin",
                    mime: "x",
                    blob: &more,
                    size: 30,
                    source: None
                }
            ))
            .map(|_| ()),
            "{name}"
        );
        assert_eq!(80, block_on(store::used(s, &olena)).unwrap());

        // Nobody but Olena, until the folder is shared with Marek's group.
        assert_eq!(
            None,
            block_on(store::role(s, &marek, &file)).unwrap(),
            "{name}"
        );
        block_on(s.execute(
            "INSERT INTO core_groups (id, name, created_at) VALUES ('g1', 'flood-team', 0)",
            &[],
        ))
        .unwrap();
        block_on(s.execute(
            "INSERT INTO core_group_members (group_id, user_id) VALUES ('g1', $1)",
            &[marek.as_str().into()],
        ))
        .unwrap();
        block_on(s.execute("INSERT INTO drive_shares (id, item_id, kind, target, role, no_download, opened, created_by, created_at) VALUES ('s1', $1, 'group', 'g1', 'edit', 0, 0, $2, 0)", &[folder.id.as_str().into(), olena.as_str().into()])).unwrap();
        block_on(s.execute("INSERT INTO drive_shares (id, item_id, kind, target, role, expires_at, no_download, opened, created_by, created_at) VALUES ('s2', $1, 'person', $2, 'edit', 1, 0, 0, $3, 0)", &[file.id.as_str().into(), sam.as_str().into(), olena.as_str().into()])).unwrap();
        assert_eq!(
            Some(Role::Edit),
            block_on(store::role(s, &marek, &file)).unwrap(),
            "{name}: through the folder and the group"
        );
        assert_eq!(
            None,
            block_on(store::role(s, &sam, &file)).unwrap(),
            "{name}: an expired share counts for nothing"
        );
        // An editor adds to Olena's folder; it is hers and counts on her quota.
        let his = put(&instance, &marek, &[1u8; 10]);
        let added = block_on(store::add_file(
            s,
            &marek,
            &folder.id,
            &Upload {
                name: "notes.txt",
                mime: "text/plain",
                blob: &his,
                size: 10,
                source: None,
            },
        ))
        .unwrap();
        assert_eq!(
            (olena.clone(), 90),
            (
                added.owner_id.clone(),
                block_on(store::used(s, &olena)).unwrap()
            ),
            "{name}"
        );
        // A trashed folder hides its insides from others.
        block_on(store::trash(s, &folder)).unwrap();
        assert_eq!(
            None,
            block_on(store::role(s, &marek, &file)).unwrap(),
            "{name}"
        );
        // Someone without a share cannot add to it.
        let folder2 = block_on(store::create_folder(s, &olena, store::ROOT, "Private")).unwrap();
        assert_eq!(
            Err(DriveError::NotFound),
            block_on(store::create_folder(s, &sam, &folder2.id, "x")).map(|_| ())
        );
    }
}
