mod batch_benches;
mod messages_benches;

// --- CONSTANTS ---
#[allow(dead_code)]
pub const APP_ID: &str = "123456789012345";
#[allow(dead_code)]
pub const APP_SECRET: &str = "a1b2c3d4e5f6";
#[allow(dead_code)]
pub const WABA_ID: &str = "987654321098765";
#[allow(dead_code)]
pub const PHONE_ID: &str = "phone_id_222";
#[allow(dead_code)]
pub const CATALOG_ID: &str = "cat_id_333";
#[allow(dead_code)]
pub const ACCESS_TOKEN: &str = "EAAD...";
#[allow(dead_code)]
pub const RECIPIENT_ID: &str = "16505551234";

use batch_benches::bench_roundtrip_batch;
use criterion::{criterion_group, criterion_main};

use messages_benches::{
    bench_roundtrip_mark_message_as_read, bench_roundtrip_send_message, bench_roundtrip_set_typing,
};

// TODO
pub struct ReleaseLock;
impl Drop for ReleaseLock {
    fn drop(&mut self) {}
}

pub fn set_env_till_m_done(uri: String) -> ReleaseLock {
    // Benches are run in series
    unsafe { std::env::set_var("API_BASE_URL_FOR_TESTING", uri) };
    ReleaseLock
}

criterion_group!(
    benches,
    bench_roundtrip_send_message,
    bench_roundtrip_mark_message_as_read,
    bench_roundtrip_set_typing,
    bench_roundtrip_batch
);
criterion_main!(benches);
