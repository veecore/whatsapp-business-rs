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

use criterion::{criterion_group, criterion_main};

use serialization::*;

criterion_group!(
    benches,
    // encoding_benches
    // batch_body_encode,
    // bench_serde_json_collect_str,
);
criterion_main!(benches);
