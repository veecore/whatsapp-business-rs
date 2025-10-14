use criterion::{Criterion, black_box};
use serde_json::json;
use tokio::runtime::Runtime;
use whatsapp_business_rs::{
    Client,
    message::{Button, Location, Media, Reaction},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{bearer_token, method, path},
};

use crate::{ACCESS_TOKEN, PHONE_ID, RECIPIENT_ID, set_env_till_m_done};

pub fn bench_roundtrip_send_message(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Set up the mock server just once outside the benchmark loop
    let _env = rt.block_on(async {
        let mock_server = MockServer::start().await;
        let response_body = json!({"messages": [{"id": "wamid.mock_message_id"}]});

        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&mock_server)
            .await;

        set_env_till_m_done(mock_server.uri())
    });

    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    let mut group = c.benchmark_group("SendMessage");

    macro_rules! declare_message_bench {
        (
            $name:literal,
            $into_draft:expr
        ) => {
            group.bench_function($name, |b| {
                b.to_async(&rt).iter(|| async {
                    let result = client
                        .message(PHONE_ID)
                        .send(RECIPIENT_ID, $into_draft)
                        .await;
                    black_box(result).unwrap()
                });
            });
        };
    }

    declare_message_bench! {
        "media_from_id",
        Media::png("134594093")
    }

    declare_message_bench! {
        "text_message",
        "Hello, world!"
    }

    declare_message_bench! {
        "reaction",
        Reaction::new('👍', "wamid.incoming_message_id_for_reaction")
    }

    declare_message_bench! {
        "location",
        Location::new(-12.345, 67.890)
            .name("Some Place")
            .address("123 Main St")
    }

    declare_message_bench! {
        "interactive",
        (
            "Please select an option:",
            [
                Button::reply("btn_1", "Option 1"),
                Button::reply("btn_2", "Option 2"),
            ],
            "Footer text"
        )
    }

    group.finish();
}

pub fn bench_roundtrip_mark_message_as_read(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Set up the mock server just once outside the benchmark loop
    let _env = rt.block_on(async {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        set_env_till_m_done(mock_server.uri())
    });

    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    c.bench_function("Mark Message as Read", |b| {
        b.to_async(&rt).iter(|| async {
            let result = client
                .message(PHONE_ID)
                .set_read("wamid.incoming_message_id")
                .await;
            black_box(result).unwrap()
        })
    });
}

pub fn bench_roundtrip_set_typing(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Set up the mock server just once outside the benchmark loop
    let _env = rt.block_on(async {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        set_env_till_m_done(mock_server.uri())
    });

    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    c.bench_function("Mark Message as Read", |b| {
        b.to_async(&rt).iter(|| async {
            let result = client
                .message(PHONE_ID)
                .set_replying("wamid.incoming_message_id")
                .await;
            black_box(result).unwrap()
        })
    });
}
