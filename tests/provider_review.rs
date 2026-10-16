use niles_withings::withings_client::*;
#[derive(Default)]
struct Mock {
    requests: Vec<HttpRequest>,
}
impl HttpTransport for Mock {
    type Error = ();
    fn post(&mut self, r: HttpRequest) -> Result<HttpResponse, ()> {
        self.requests.push(r);
        Ok(HttpResponse {
            status: 200,
            body: br#"{"status":0,"body":{}}"#.to_vec(),
        })
    }
}
#[test]
fn subscribes_to_both_bed_directions() {
    let mut t = Mock::default();
    subscribe_bed(
        &mut t,
        &Secret::new("synthetic-token").unwrap(),
        &CallbackUrl::parse("https://example.org/callback").unwrap(),
        "installation",
    )
    .unwrap();
    assert_eq!(t.requests.len(), 2);
    assert!(t.requests.iter().any(|r| r.form.contains("appli=50")));
    assert!(t.requests.iter().any(|r| r.form.contains("appli=51")));
}
#[test]
fn response_debug_redacts_token_body() {
    let r = HttpResponse {
        status: 200,
        body: b"synthetic-access-secret".to_vec(),
    };
    let d = format!("{r:?}");
    assert!(
        !d.contains("115, 121, 110"),
        "byte-list debug still reveals response secrets"
    );
    assert!(!d.contains("synthetic-access-secret"));
}
#[test]
fn callback_rejects_numeric_ip() {
    assert!(CallbackUrl::parse("https://127.0.0.1/callback").is_err());
}
#[test]
fn callback_accepts_standard_explicit_port() {
    assert!(CallbackUrl::parse("https://example.org:443/callback").is_ok());
}
#[test]
fn callback_debug_redacts_route_secret() {
    let cb = CallbackUrl::parse("https://example.org/synthetic-route-secret").unwrap();
    assert!(!format!("{cb:?}").contains("synthetic-route-secret"));
}
