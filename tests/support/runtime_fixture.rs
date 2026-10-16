use niles_withings::{
    adapter::{Reading, Sink},
    application::{AccountSecretStore, SelectionStore},
    coordinator::ScopedBackend,
    runtime::AccountRecordRead,
    withings_client::{HttpRequest, HttpResponse, HttpTransport},
};
use std::{cell::RefCell, collections::{BTreeMap, VecDeque}, rc::Rc};

#[derive(Clone, Default)]
pub struct Shared(pub Rc<RefCell<Ledger>>);
#[derive(Default)]
pub struct Ledger {
    pub accounts: Option<String>,
    pub selections: Option<String>,
    pub values: BTreeMap<String, String>,
    pub actions: Vec<&'static str>,
    pub readings: Vec<Reading>,
    pub fail_final_save: bool,
    pub fail_account_save: bool,
}
impl AccountSecretStore for Shared {
    type Error = &'static str;
    fn set_account_record(&mut self, value: &str) -> Result<(), Self::Error> {
        if self.0.borrow().fail_account_save || (self.0.borrow().fail_final_save && !value.contains("\"refresh_pending\":true")) { return Err("injected final save failure"); }
        self.0.borrow_mut().accounts = Some(value.into()); Ok(())
    }
}
impl AccountRecordRead for Shared {
    fn read_account_record(&mut self) -> Result<Option<String>, Self::Error> { Ok(self.0.borrow().accounts.clone()) }
}
impl SelectionStore for Shared {
    type Error = &'static str;
    fn load_selection(&mut self) -> Result<Option<String>, Self::Error> { Ok(self.0.borrow().selections.clone()) }
    fn save_selection(&mut self, value: &str) -> Result<(), Self::Error> { self.0.borrow_mut().selections = Some(value.into()); Ok(()) }
}
impl ScopedBackend for Shared {
    type Error = &'static str;
    fn load_value(&mut self, key: &str) -> Result<Option<String>, Self::Error> { Ok(self.0.borrow().values.get(key).cloned()) }
    fn save_value(&mut self, key: &str, value: &str) -> Result<(), Self::Error> {
        let mut ledger = self.0.borrow_mut();

        ledger.values.insert(key.into(), value.into()); Ok(())
    }
}
impl Sink for Shared {
    type Error = &'static str;
    fn publish_reading(&mut self, value: Reading) -> Result<(), Self::Error> { self.0.borrow_mut().readings.push(value); Ok(()) }
    fn publish_action(&mut self, value: &'static str) -> Result<(), Self::Error> { self.0.borrow_mut().actions.push(value); Ok(()) }
}
#[derive(Default)]
pub struct SyntheticTransport { pub replies: VecDeque<HttpResponse>, pub requests: usize }
impl HttpTransport for SyntheticTransport {
    type Error = &'static str;
    fn post(&mut self, request: HttpRequest) -> Result<HttpResponse, Self::Error> {
        self.requests += 1;
        if !request.form.contains("grant_type=authorization_code") && !request.form.contains("grant_type=refresh_token") && !request.form.starts_with("action=list") && request.form != "action=getdevice" { return Err("unexpected synthetic request"); }
        self.replies.pop_front().ok_or("no synthetic response")
    }
}
pub fn device_reply() -> HttpResponse {
    HttpResponse { status: 200, body: br#"{"status":0,"body":{"devices":[{"deviceid":"offline-mat","type":"Sleep Monitor","model":"Synthetic","model_id":1}]}}"#.to_vec() }
}
pub fn event(date: i64) -> niles_withings::application::WebhookRequest {
    niles_withings::application::WebhookRequest { method: "POST".into(), content_type: "application/x-www-form-urlencoded".into(), body: format!("userid=123&deviceid=offline-mat&appli=50&date={date}").into_bytes() }
}
