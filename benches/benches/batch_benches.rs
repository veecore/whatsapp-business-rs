use criterion::{BenchmarkId, Criterion, black_box};
use tokio::runtime::Runtime;
use whatsapp_business_rs::{Client, message::Media};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

use crate::{ACCESS_TOKEN, APP_ID, PHONE_ID, set_env_till_m_done};

#[allow(dead_code)]
pub const MEDIA_ID: &str = "media_id";
#[allow(dead_code)]
pub const MESSAGE_ID: &str = "wamid.mock_message_id";

// FIXME: Not correct.... given the complexity of measuring roundtrip,
// let's simply measure half (prepare request, send to server, do not handle response)
// hmm serial is eager
pub fn bench_roundtrip_batch(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Set up the mock server just once outside the benchmark loop
    let _env = rt.block_on(async {
        let mock_server = MockServer::start().await;
        // Too hard, let's bench failure
        // SendMessage
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        set_env_till_m_done(mock_server.uri())
    });

    let mut group = c.benchmark_group("Batch vs Serial");

    fn generate_requests<F, R>(f: F, size: usize) -> impl Iterator<Item = R>
    where
        F: Fn() -> R,
    {
        (0..size).map(move |_| f())
    }

    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    // A simple for loop will fail
    macro_rules! for_each {
        (
            $(($name:literal, || $expr:expr)),*
            {
                |$id:ident, $request:ident| $for_each:expr
            }
        ) => {
            $(
                let mut for_each = |$id, $request| $for_each;
                for_each($name, || $expr);
            )*
        }
    }

    // Iterate over different forms of requests.
    for_each! {
        ("NoPayload (DeleteMedia)", || client.message(PHONE_ID).delete_media(MEDIA_ID)),
        ("SimplePayload (SetRead)", || client.message(PHONE_ID).set_read(MESSAGE_ID)),
        ("QueryPayload (ConfigureWebhook)", || client.app(APP_ID).configure_webhook(("ver_token",  "https://example.com"))),
        ("PayloadWithFile (SendMessage)", || client.message(PHONE_ID).send(PHONE_ID, Media::png(vec![0; 1024]).caption("Hello, world!")))
        {
            |id, request| {
                // Iterate over different sizes.
                for size in [1, 5, 10, 25, 50] {
                    let bench_id = format!("{id}/{size}Request(s)");

                    // ---- 1. Batch API Benchmark
                    group.bench_function(
                        BenchmarkId::new("Batch", &bench_id),
                        |b| {
                            b.to_async(&rt).iter(|| async {
                                let requests = generate_requests(request, size);
                                let batch_output = client.batch().include_iter(requests).execute().await;
                                let _ = black_box(batch_output);
                            })
                        },
                    );

                    // ---- 2. Serial Benchmark
                    group.bench_function(
                        BenchmarkId::new("Serial", &bench_id),
                        |b| {
                            b.to_async(&rt).iter(|| async {
                                let requests = generate_requests(request, size);

                                for request in requests {
                                    let result: Result<_, _> = request.await;
                                    let _ = black_box(result);
                                }
                            })
                        },
                    );
                }
            }
        }
    }

    group.finish();
}
