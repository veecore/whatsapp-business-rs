use criterion::Criterion;
use whatsapp_business_rs::{Client, message::Media};

use crate::{ACCESS_TOKEN, PHONE_ID, RECIPIENT_ID};

pub fn bench_roundtrip_send_message(c: &mut Criterion) {
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    let mut group = c.benchmark_group("Message Request Preparation");

    group.bench_function("media_from_id", |b| {
        b.iter(|| {
            let _prepared_request = client
                .message(PHONE_ID)
                .send(RECIPIENT_ID, Media::png("134594093"))
                .into_future();
        });
    });
    // let text_message = Text::from("Hello from a benchmark!");
    // let recipient_id = "444555666";

    // c.bench_function("prepare_text_message_request", |b| {
    //     b.iter(|| {
    //         let _prepared_request = ;
    //     })
    // });

    group.finish();
}
