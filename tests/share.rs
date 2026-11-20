//! Sharing by label rules, on both shapes: colleagues and groups get
//! roles; links and people outside only as the label allows (a warning
//! needs a reason); a stricter label takes shares away; links work
//! without an account, with a password and "no download"; it is all in
//! the audit log.

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
    mime: &str,
    bytes: &[u8],
) -> String {
    let (_, u) = c.json(
        "POST",
        "/drive/uploads",
        Some(json!({"parent": parent, "name": name, "size": bytes.len(), "mime": mime})),
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

#[test]
fn shares_follow_the_label_rules() {
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
        let (_, folder) = olena.json("POST", "/drive/folders", Some(json!({"name": "Shelters"})));
        let folder = folder["id"].as_str().unwrap().to_string();
        let doc = upload(
            &olena,
            &folder,
            "shelter-agreement.txt",
            "text/plain",
            b"Clause 4: the school on Park Street.",
        );

        // A colleague: allowed at any label; he may edit, not give more.
        let (status, shares) = olena.json(
            "POST",
            &format!("/drive/items/{folder}/shares"),
            Some(json!({"email": "marek@ministry.example", "role": "comment"})),
        );
        assert_eq!(200, status, "{name}: {shares}");
        assert_eq!(json!("comment"), shares["people"][1]["role"]);
        assert_eq!(
            200,
            marek
                .send(
                    "GET",
                    &format!("/drive/items/{doc}/content"),
                    None,
                    false,
                    &[]
                )
                .0,
            "{name}: through the folder"
        );
        assert_eq!(
            403,
            marek
                .json(
                    "POST",
                    &format!("/drive/items/{doc}/shares"),
                    Some(json!({"email": "sam@ministry.example", "role": "edit"}))
                )
                .0,
            "{name}: comment cannot share"
        );
        let (_, shared) = marek.json("GET", "/drive/shared", None);
        assert_eq!(json!("Shelters"), shared["items"][0]["name"], "{name}");

        // Unlabelled is INTERNAL: no links; outside needs a reason.
        let (status, refused) =
            olena.json("POST", &format!("/drive/items/{doc}/link"), Some(json!({})));
        assert_eq!(
            (403, json!("policy")),
            (status, refused["error"]["code"].clone()),
            "{name}: {refused}"
        );
        let (status, ask) = olena.json(
            "POST",
            &format!("/drive/items/{doc}/shares"),
            Some(json!({"email": "anna@regional.example"})),
        );
        assert_eq!(
            (409, json!("needs_reason")),
            (status, ask["error"]["code"].clone()),
            "{name}"
        );
        let (status, out) = olena.json(
            "POST",
            &format!("/drive/items/{doc}/shares"),
            Some(
                json!({"email": "anna@regional.example", "reason": "Regional review of clause 4"}),
            ),
        );
        assert_eq!(200, status, "{name}: {out}");
        let personal = out["outside"][0]["url"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .to_string();
        let (_, seen) = visitor.json("GET", &format!("/drive/links/{personal}"), None);
        assert_eq!(
            json!("shelter-agreement.txt"),
            seen["item"]["name"],
            "{name}: {seen}"
        );

        // PUBLIC allows a link: with a password and no download.
        let (status, _) = olena.json(
            "PUT",
            &format!("/drive/items/{doc}/label"),
            Some(json!({"label": "public"})),
        );
        assert_eq!(200, status);
        let (status, linked) = olena.json(
            "POST",
            &format!("/drive/items/{doc}/link"),
            Some(json!({"password": "river-gauge-42", "noDownload": true})),
        );
        assert_eq!(200, status, "{name}: {linked}");
        let token = linked["link"]["url"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .to_string();
        assert_eq!(
            json!(true),
            visitor
                .json("GET", &format!("/drive/links/{token}"), None)
                .1["needsPassword"]
        );
        assert_eq!(
            401,
            visitor
                .send(
                    "GET",
                    &format!("/drive/links/{token}"),
                    None,
                    false,
                    &[("X-Ortus-Link-Password", "wrong-password")]
                )
                .0
        );
        let (status, body, _) = visitor.send(
            "GET",
            &format!("/drive/links/{token}"),
            None,
            false,
            &[("X-Ortus-Link-Password", "river-gauge-42")],
        );
        let opened: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            (200, json!(true)),
            (status, opened["noDownload"].clone()),
            "{name}"
        );
        let pw = [("X-Ortus-Link-Password", "river-gauge-42")];
        assert_eq!(
            403,
            visitor
                .send(
                    "GET",
                    &format!("/drive/links/{token}/content"),
                    None,
                    false,
                    &pw
                )
                .0,
            "{name}: no download"
        );
        let (status, bytes, _) = visitor.send(
            "GET",
            &format!("/drive/links/{token}/content?inline=1"),
            None,
            false,
            &pw,
        );
        assert_eq!(
            (200, b"Clause 4: the school on Park Street.".to_vec()),
            (status, bytes),
            "{name}: viewing is fine"
        );
        let (_, listed) = olena.json("GET", &format!("/drive/items/{doc}/shares"), None);
        assert_eq!(json!(1), listed["link"]["opened"], "{name}");

        // SECRET: the link and the outside share go; the colleague stays.
        let (status, strict) = olena.json(
            "PUT",
            &format!("/drive/items/{doc}/label"),
            Some(json!({"label": "secret"})),
        );
        assert_eq!(
            (200, json!(2)),
            (status, strict["removed"].clone()),
            "{name}: {strict}"
        );
        assert_eq!(
            404,
            visitor
                .json("GET", &format!("/drive/links/{token}"), None)
                .0
        );
        assert_eq!(
            404,
            visitor
                .json("GET", &format!("/drive/links/{personal}"), None)
                .0
        );
        assert_eq!(
            200,
            marek
                .send(
                    "GET",
                    &format!("/drive/items/{doc}/content"),
                    None,
                    false,
                    &[]
                )
                .0
        );

        // A folder link (on a PUBLIC folder) lets anyone look inside, not outside.
        olena.json(
            "PUT",
            &format!("/drive/items/{folder}/label"),
            Some(json!({"label": "public"})),
        );
        let other = upload(
            &olena,
            "",
            "elsewhere.txt",
            "text/plain",
            b"not in the folder",
        );
        let notes = upload(&olena, &folder, "notes.txt", "text/plain", b"public notes");
        let (_, flink) = olena.json(
            "POST",
            &format!("/drive/items/{folder}/link"),
            Some(json!({})),
        );
        let ft = flink["link"]["url"]
            .as_str()
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap()
            .to_string();
        let (_, inside) = visitor.json("GET", &format!("/drive/links/{ft}"), None);
        assert!(
            inside["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["name"] == "notes.txt"),
            "{name}: {inside}"
        );
        assert_eq!(
            200,
            visitor
                .send(
                    "GET",
                    &format!("/drive/links/{ft}/content?item={notes}"),
                    None,
                    false,
                    &[]
                )
                .0
        );
        assert_eq!(
            404,
            visitor
                .send(
                    "GET",
                    &format!("/drive/links/{ft}/content?item={other}"),
                    None,
                    false,
                    &[]
                )
                .0,
            "{name}: nothing outside the folder"
        );

        // The audit log has it all.
        let actions: Vec<String> = ortus_core::runtime::block_on(
            instance
                .store
                .query("SELECT action FROM core_audit ORDER BY seq", &[]),
        )
        .unwrap()
        .iter()
        .filter_map(|r| r.text(0).map(str::to_string))
        .collect();
        for want in [
            "drive.share",
            "policy.refused",
            "policy.overridden",
            "drive.share_outside",
            "drive.link",
            "drive.label",
            "drive.unshare_by_label",
        ] {
            assert!(
                actions.iter().any(|a| a == want),
                "{name}: {want} in {actions:?}"
            );
        }
    }
}
