//! Regressions of the slice 5 review, on both shapes.

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

fn upload<A: rust_web_server::application::Application>(
    c: &Client<A>,
    parent: &str,
    name: &str,
    bytes: &[u8],
) -> String {
    let (_, u) = c.json(
        "POST",
        "/drive/uploads",
        Some(json!({"parent": parent, "name": name, "size": bytes.len(), "mime": "text/plain"})),
    );
    let u = u["id"].as_str().unwrap().to_string();
    c.send(
        "PUT",
        &format!("/drive/uploads/{u}/parts/0"),
        Some(bytes.to_vec()),
        false,
        &[],
    );
    c.json("POST", &format!("/drive/uploads/{u}/finish"), None)
        .1["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn token(v: &Value) -> String {
    v["link"]["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string()
}

#[test]
fn labels_hold_through_folders_moves_and_copies() {
    for (name, instance, _keep) in common::installations() {
        let app = || TestClient::new(ortus_core::app(Arc::clone(&instance)));
        let olena = Client {
            inner: app(),
            token: signed_in(&instance, "olena@ministry.example", Role::Member),
        };
        let marek = Client {
            inner: app(),
            token: signed_in(&instance, "marek@ministry.example", Role::Member),
        };
        let visitor = Client {
            inner: app(),
            token: String::new(),
        };
        let folder = olena
            .json("POST", "/drive/folders", Some(json!({"name": "Public"})))
            .1["id"]
            .as_str()
            .unwrap()
            .to_string();
        olena.json(
            "PUT",
            &format!("/drive/items/{folder}/label"),
            Some(json!({"label": "public"})),
        );
        let open = upload(&olena, &folder, "open.txt", b"open");
        let closed = upload(&olena, &folder, "closed.txt", b"secret plans");
        olena.json(
            "PUT",
            &format!("/drive/items/{closed}/label"),
            Some(json!({"label": "secret"})),
        );
        let t = token(
            &olena
                .json(
                    "POST",
                    &format!("/drive/items/{folder}/link"),
                    Some(json!({})),
                )
                .1,
        );
        // A stricter file inside a linked folder is neither listed nor served.
        let (_, seen) = visitor.json("GET", &format!("/drive/links/{t}"), None);
        let names: Vec<&str> = seen["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(vec!["open.txt"], names, "{name}");
        assert_eq!(
            404,
            visitor
                .send(
                    "GET",
                    &format!("/drive/links/{t}/content?item={closed}"),
                    None,
                    false,
                    &[]
                )
                .0,
            "{name}"
        );
        assert_eq!(
            200,
            visitor
                .send(
                    "GET",
                    &format!("/drive/links/{t}/content?item={open}"),
                    None,
                    false,
                    &[]
                )
                .0
        );
        // A linked file moved into a SECRET folder loses its link.
        let secret = olena
            .json("POST", "/drive/folders", Some(json!({"name": "Secret"})))
            .1["id"]
            .as_str()
            .unwrap()
            .to_string();
        olena.json(
            "PUT",
            &format!("/drive/items/{secret}/label"),
            Some(json!({"label": "secret"})),
        );
        let ft = token(
            &olena
                .json(
                    "POST",
                    &format!("/drive/items/{open}/link"),
                    Some(json!({})),
                )
                .1,
        );
        assert_eq!(
            200,
            olena
                .json(
                    "PATCH",
                    &format!("/drive/items/{open}"),
                    Some(json!({"parent": secret}))
                )
                .0
        );
        assert_eq!(
            404,
            visitor.json("GET", &format!("/drive/links/{ft}"), None).0,
            "{name}: moved under SECRET"
        );
        // A looser label on the file does not escape the folder's.
        olena.json(
            "PUT",
            &format!("/drive/items/{open}/label"),
            Some(json!({"label": "public"})),
        );
        assert_eq!(
            403,
            olena
                .json(
                    "POST",
                    &format!("/drive/items/{open}/link"),
                    Some(json!({}))
                )
                .0,
            "{name}"
        );
        // A copy keeps the classification.
        olena.json(
            "POST",
            &format!("/drive/items/{closed}/shares"),
            Some(json!({"email": "marek@ministry.example", "role": "view"})),
        );
        let (status, copy) = marek.json(
            "POST",
            &format!("/drive/items/{closed}/copy"),
            Some(json!({"parent": ""})),
        );
        assert_eq!(200, status, "{name}: {copy}");
        assert_eq!(json!("secret"), copy["label"], "{name}");
        assert_eq!(
            403,
            marek
                .json(
                    "POST",
                    &format!("/drive/items/{}/link", copy["id"].as_str().unwrap()),
                    Some(json!({}))
                )
                .0
        );
        // Viewers do not see link addresses.
        let (_, as_viewer) = marek.json("GET", &format!("/drive/items/{folder}/shares"), None);
        assert!(as_viewer["link"]["url"].is_null(), "{name}: {as_viewer}");
    }
}

#[test]
fn depth_parts_and_ranges_are_bounded() {
    for (name, instance, _keep) in common::installations() {
        let olena = Client {
            inner: TestClient::new(ortus_core::app(Arc::clone(&instance))),
            token: signed_in(&instance, "olena@ministry.example", Role::Member),
        };
        let mut parent = String::new();
        let mut refused = false;
        for i in 0..70 {
            let (status, f) = olena.json(
                "POST",
                "/drive/folders",
                Some(json!({"parent": parent, "name": format!("level {i}")})),
            );
            if status != 200 {
                refused = true;
                break;
            }
            parent = f["id"].as_str().unwrap().to_string();
        }
        assert!(refused, "{name}: folders do not nest forever");
        // Parts only within the declared size.
        let (_, u) = olena.json(
            "POST",
            "/drive/uploads",
            Some(json!({"name": "a.txt", "size": 3, "mime": "text/plain"})),
        );
        let u = u["id"].as_str().unwrap().to_string();
        assert_eq!(
            400,
            olena
                .send(
                    "PUT",
                    &format!("/drive/uploads/{u}/parts/1"),
                    Some(vec![1]),
                    false,
                    &[]
                )
                .0,
            "{name}"
        );
        assert_eq!(
            400,
            olena
                .send(
                    "PUT",
                    &format!("/drive/uploads/{u}/parts/0"),
                    Some(vec![1; 4]),
                    false,
                    &[]
                )
                .0,
            "{name}"
        );
        // Ranges on an empty file and huge ends.
        let empty = upload(&olena, "", "empty.txt", b"");
        assert_eq!(
            416,
            olena
                .send(
                    "GET",
                    &format!("/drive/items/{empty}/content"),
                    None,
                    false,
                    &[("Range", "bytes=-5")]
                )
                .0,
            "{name}"
        );
        let some = upload(&olena, "", "some.txt", b"abcdef");
        let (status, bytes, _) = olena.send(
            "GET",
            &format!("/drive/items/{some}/content"),
            None,
            false,
            &[("Range", "bytes=2-18446744073709551615")],
        );
        assert_eq!((206, b"cdef".to_vec()), (status, bytes), "{name}");
    }
}
