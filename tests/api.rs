//! The drive's API on both shapes: uploads in parts, streamed downloads
//! with ranges, versions, names, moves, copies, trash, lists and quota;
//! others see nothing that is not shared with them.

mod common;

use std::sync::Arc;

use ortus_core::auth::accounts::Role;
use ortus_core::instance::testing_auth::signed_in;
use rust_web_server::test_client::TestClient;
use serde_json::{json, Value};

struct Client<A: rust_web_server::application::Application> {
    inner: TestClient<A>,
    token: String,
}

impl<A: rust_web_server::application::Application> Client<A> {
    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
        json_body: bool,
        headers: &[(&str, &str)],
    ) -> (i16, Vec<u8>, Vec<(String, String)>) {
        let url = format!("/api/v1{path}");
        let mut r = match method {
            "GET" => self.inner.get(&url),
            "POST" => self.inner.post(&url),
            "PUT" => self.inner.put(&url),
            "PATCH" => self.inner.patch(&url),
            _ => self.inner.delete(&url),
        };
        r = r
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("X-Requested-With", "ortus");
        for (k, v) in headers {
            r = r.header(k, v);
        }
        if let Some(b) = body {
            if json_body {
                r = r.header("Content-Type", "application/json");
            }
            r = r.body_bytes(b);
        }
        let mut response = r.send();
        let heads = response
            .headers()
            .iter()
            .map(|h| (h.name.to_string().to_ascii_lowercase(), h.value.to_string()))
            .collect();
        // A streamed body (a download) is read from its stream.
        let body = match response.take_stream() {
            Some(mut stream) => {
                let mut all = Vec::new();
                std::io::Read::read_to_end(&mut stream, &mut all).unwrap();
                all
            }
            None => response.body_bytes().to_vec(),
        };
        (response.status(), body, heads)
    }
    fn json(&self, method: &str, path: &str, body: Option<Value>) -> (i16, Value) {
        let (status, bytes, _) = self.send(
            method,
            path,
            body.map(|b| b.to_string().into_bytes()),
            true,
            &[],
        );
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

#[test]
fn files_through_the_api() {
    for (name, instance, _keep) in common::installations() {
        let olena = Client {
            inner: TestClient::new(ortus_core::app(Arc::clone(&instance))),
            token: signed_in(&instance, "olena@ministry.example", Role::Member),
        };
        let marek = Client {
            inner: TestClient::new(ortus_core::app(Arc::clone(&instance))),
            token: signed_in(&instance, "marek@ministry.example", Role::Member),
        };

        let (_, folder) = olena.json("POST", "/drive/folders", Some(json!({"name": "Emergency"})));
        let folder = folder["id"].as_str().unwrap().to_string();
        // 16 MiB and a bit, in two parts.
        let big: Vec<u8> = (0..16 * 1024 * 1024 + 100)
            .map(|i: usize| (i % 253) as u8)
            .collect();
        let (status, up) = olena.json("POST", "/drive/uploads", Some(json!({"parent": folder, "name": "site-photos.zip", "size": big.len(), "mime": "application/zip"})));
        assert_eq!(200, status, "{name}: {up}");
        let upload = up["id"].as_str().unwrap().to_string();
        let part = up["partSize"].as_u64().unwrap() as usize;
        assert_eq!(
            200,
            olena
                .send(
                    "PUT",
                    &format!("/drive/uploads/{upload}/parts/1"),
                    Some(big[part..].to_vec()),
                    false,
                    &[]
                )
                .0
        );
        let (status, early) = olena.json("POST", &format!("/drive/uploads/{upload}/finish"), None);
        assert_eq!(400, status, "{name}: part 0 is missing: {early}");
        assert_eq!(
            200,
            olena
                .send(
                    "PUT",
                    &format!("/drive/uploads/{upload}/parts/0"),
                    Some(big[..part].to_vec()),
                    false,
                    &[]
                )
                .0
        );
        let (status, file) = olena.json("POST", &format!("/drive/uploads/{upload}/finish"), None);
        assert_eq!(200, status, "{name}: {file}");
        let id = file["id"].as_str().unwrap().to_string();
        assert_eq!(json!(big.len()), file["size"]);

        // Downloads: whole, and a range across the parts.
        let (status, bytes, heads) = olena.send(
            "GET",
            &format!("/drive/items/{id}/content"),
            None,
            false,
            &[],
        );
        assert_eq!((200, big.len()), (status, bytes.len()), "{name}");
        assert_eq!(big, bytes);
        assert!(heads
            .iter()
            .any(|(k, v)| k == "content-disposition" && v.starts_with("attachment")));
        let (status, bytes, heads) = olena.send(
            "GET",
            &format!("/drive/items/{id}/content"),
            None,
            false,
            &[("Range", &format!("bytes={}-{}", part - 2, part + 1))],
        );
        assert_eq!(
            (206, big[part - 2..part + 2].to_vec()),
            (status, bytes),
            "{name}"
        );
        assert!(heads.iter().any(|(k, v)| k == "content-range"
            && v == &format!("bytes {}-{}/{}", part - 2, part + 1, big.len())));
        assert_eq!(
            416,
            olena
                .send(
                    "GET",
                    &format!("/drive/items/{id}/content"),
                    None,
                    false,
                    &[("Range", "bytes=999999999-")]
                )
                .0
        );

        // A small file twice: two versions; the older comes back.
        let small = |text: &str| -> String {
            let (_, u) = olena.json(
                "POST",
                "/drive/uploads",
                Some(json!({"name": "notes.txt", "size": text.len(), "mime": "text/plain"})),
            );
            let u = u["id"].as_str().unwrap().to_string();
            olena.send(
                "PUT",
                &format!("/drive/uploads/{u}/parts/0"),
                Some(text.as_bytes().to_vec()),
                false,
                &[],
            );
            olena
                .json("POST", &format!("/drive/uploads/{u}/finish"), None)
                .1["id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let notes = small("first");
        assert_eq!(notes, small("second, longer"));
        let (_, versions) = olena.json("GET", &format!("/drive/items/{notes}/versions"), None);
        assert_eq!(2, versions["versions"].as_array().unwrap().len(), "{name}");
        olena.json(
            "POST",
            &format!("/drive/items/{notes}/versions/1/restore"),
            None,
        );
        assert_eq!(
            b"first".to_vec(),
            olena
                .send(
                    "GET",
                    &format!("/drive/items/{notes}/content"),
                    None,
                    false,
                    &[]
                )
                .1
        );
        let (_, inline) = (
            0,
            olena
                .send(
                    "GET",
                    &format!("/drive/items/{notes}/content?inline=1"),
                    None,
                    false,
                    &[],
                )
                .2,
        );
        assert!(
            inline
                .iter()
                .any(|(k, v)| k == "content-disposition" && v.starts_with("inline")),
            "{name}: text may be viewed"
        );

        // Rename, move, star, copy; lists.
        let (status, renamed) = olena.json(
            "PATCH",
            &format!("/drive/items/{notes}"),
            Some(json!({"name": "minutes.txt", "parent": folder, "starred": true})),
        );
        assert_eq!(
            (200, json!("minutes.txt"), json!(folder)),
            (status, renamed["name"].clone(), renamed["parent"].clone()),
            "{name}"
        );
        let (_, listed) = olena.json("GET", &format!("/drive/items?parent={folder}"), None);
        assert_eq!(2, listed["items"].as_array().unwrap().len());
        assert_eq!(json!("Emergency"), listed["path"][0]["name"]);
        let (_, starred) = olena.json("GET", "/drive/special?which=starred", None);
        assert_eq!(1, starred["items"].as_array().unwrap().len());
        let (status, copied) = olena.json(
            "POST",
            &format!("/drive/items/{notes}/copy"),
            Some(json!({"parent": ""})),
        );
        assert_eq!(
            (200, json!("Copy of minutes.txt")),
            (status, copied["name"].clone()),
            "{name}"
        );

        // Marek sees none of it.
        assert_eq!(
            404,
            marek.json("GET", &format!("/drive/items/{id}"), None).0,
            "{name}"
        );
        assert_eq!(
            404,
            marek
                .send(
                    "GET",
                    &format!("/drive/items/{id}/content"),
                    None,
                    false,
                    &[]
                )
                .0
        );
        assert_eq!(
            404,
            marek
                .json(
                    "PATCH",
                    &format!("/drive/items/{id}"),
                    Some(json!({"name": "mine"}))
                )
                .0
        );
        assert_eq!(
            404,
            marek
                .json("GET", &format!("/drive/items?parent={folder}"), None)
                .0
        );
        assert_eq!(
            404,
            marek
                .send(
                    "PUT",
                    &format!("/drive/uploads/{upload}/parts/0"),
                    Some(vec![1]),
                    false,
                    &[]
                )
                .0
        );

        // Trash, back, and for good.
        assert_eq!(
            200,
            olena
                .json("DELETE", &format!("/drive/items/{folder}"), None)
                .0
        );
        let (_, trash) = olena.json("GET", "/drive/special?which=trash", None);
        assert_eq!(1, trash["items"].as_array().unwrap().len(), "{name}");
        assert_eq!(
            200,
            olena
                .json("POST", &format!("/drive/items/{folder}/restore"), None)
                .0
        );
        olena.json("DELETE", &format!("/drive/items/{folder}"), None);
        assert_eq!(
            200,
            olena
                .json("DELETE", &format!("/drive/items/{folder}?forever=1"), None)
                .0
        );
        assert_eq!(
            404,
            olena.json("GET", &format!("/drive/items/{id}"), None).0,
            "{name}: gone with its folder"
        );
        let (_, quota) = olena.json("GET", "/drive/quota", None);
        assert_eq!(
            json!(5),
            quota["used"],
            "{name}: only the copy is left: {quota}"
        );
    }
}
