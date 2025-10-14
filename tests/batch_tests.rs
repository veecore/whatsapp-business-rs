mod common;

use common::*;
use serde_json::json;
use whatsapp_business_rs::{Client, message::Media};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{bearer_token, body_string_contains, method, path},
};

/// A helper matcher to verify that a specific sub-request is present in the batch JSON payload.
fn contains_sub_request(method: &str, relative_url: &str) -> impl wiremock::Match + 'static {
    body_string_contains(format!(
        r#""method":"{}","relative_url":"{}""#,
        method, relative_url
    ))
}
#[tokio::test]
async fn test_simple_batch_execution() {
    let mock_server = MockServer::start().await;

    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    let sender_phone_id = "phone_id_sender";
    let recipient = "111222333";
    let message_id_to_read = "wamid.to_read";
    let sent_message_id = "wamid.sent_msg";

    // The batch API responds with an array of objects.
    // Each object's 'body' field is a JSON *string*, so we must serialize it first.
    let batch_response = json!([
        {
            "code": 200,
            "body": json!({
                "messages": [{"id": sent_message_id}]
            }).to_string()
        },
        {
            "code": 200,
            "body": json!({
                "success": true
            }).to_string()
        }
    ]);

    Mock::given(method("POST"))
        .and(path("/"))
        .and(bearer_token(ACCESS_TOKEN))
        // Verify both sub-requests are in the JSON part of the form data
        .and(contains_sub_request(
            "POST",
            &format!("/v23.0/{}/messages", sender_phone_id),
        ))
        .and(body_string_contains(recipient))
        .and(body_string_contains(message_id_to_read))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response))
        .mount(&mock_server)
        .await;

    // Act
    let output = hold_env_var_for_me! {
        mock_server,
        client
            .batch()
            .include(client.message(sender_phone_id).send(recipient, "Message 1"))
            .include(client.message(sender_phone_id).set_read(message_id_to_read))
            .execute(),
    };

    // Assert
    let (send_res, read_res) = output.unwrap().flatten().unwrap();

    assert!(send_res.is_ok());
    assert_eq!(send_res.unwrap().message_id(), sent_message_id);

    assert!(read_res.is_ok());
}

#[tokio::test]
async fn test_batch_with_file_upload() {
    let mock_server = MockServer::start().await;

    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    let sender_phone_id = "phone_id_upload";
    let recipient = "777888999";
    let final_message_id = "wamid.media_msg";

    // The batch response simulates the result of the message send request.
    // The media upload part is handled implicitly by the API.
    let batch_response = json!([
        // The first response corresponds to the media upload call
        null,
        // The second response corresponds to the send message call
        {
            "code": 200,
            "body": json!({
                "messages": [{"id": final_message_id}]
            }).to_string()
        }
    ]);

    Mock::given(method("POST"))
        .and(path("/"))
        .and(bearer_token(ACCESS_TOKEN))
        // The multipart body must contain the main "batch" JSON field...
        .and(body_string_contains(
            "Content-Disposition: form-data; name=\"batch\"",
        ))
        // ...and the attached file field (name is determined by the library, e.g., "file").
        .and(body_string_contains(
            "Content-Disposition: form-data; name=\"file\"",
        ))
        // The batch JSON must reference this attached file.
        .and(body_string_contains(r#"attached_files"#))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response))
        .mount(&mock_server)
        .await;

    // Act
    let media = Media::png(vec![0; 10]);
    let output = hold_env_var_for_me! {
        mock_server,
        client
            .batch()
            .include(client.message(sender_phone_id).send(recipient, media))
            .execute(),
    };

    // Assert
    let (media_msg_res,) = output.unwrap().flatten().unwrap();

    assert_eq!(media_msg_res.unwrap().message_id(), final_message_id);
}

#[tokio::test]
async fn test_batch_with_headers_and_auth_override() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let override_token = "OVERRIDE_ACCESS_TOKEN_FOR_BATCH";

    let batch_response = json!([
        {
            "code": 200,
            "headers": [
                {"name": "Content-Type", "value": "application/json"},
                {"name": "facebook-api-version", "value": "v19.0"}
            ],
            "body": json!({"success": true}).to_string()
        }
    ]);

    Mock::given(method("POST"))
        .and(path("/"))
        // Check for the overridden auth token on the top-level request.
        .and(bearer_token(override_token))
        // The form data must include the 'include_headers' field set to 'true'.
        .and(body_string_contains("\"include_headers\"\r\n\r\ntrue"))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response))
        .mount(&mock_server)
        .await;

    // Act
    let output = hold_env_var_for_me! {
        mock_server,
        client
            .batch()
            .include(client.app(APP_ID).configure_webhook(("v_tok", "https://url.com"))) // Dummy request
            .execute()
            .include_headers(true)
            .with_auth(override_token),
    };

    // Assert
    output.unwrap().result().unwrap().unwrap()
}

#[tokio::test]
async fn test_batch_with_partial_failure() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    let sender_phone_id = "phone_id_partial_fail";
    let recipient_ok = "123456789";
    let recipient_fail = "invalid_number";
    let message_id_ok = "wamid.ok_message";

    let batch_response = json!([
        {
            "code": 200,
            "body": json!({
                "messages": [{"id": message_id_ok}]
            }).to_string()
        },
        {
            "code": 400,
            "body": json!({
                "error": {
                    "message": "Recipient phone number not in allowed list",
                    "type": "OAuthException",
                    "code": 131030,
                }
            }).to_string()
        }
    ]);

    Mock::given(method("POST"))
        .and(path("/"))
        .and(bearer_token(ACCESS_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(batch_response))
        .mount(&mock_server)
        .await;

    // Act
    let output = hold_env_var_for_me! {
        mock_server,
        client
            .batch()
            .include(client.message(sender_phone_id).send(recipient_ok, "Hi"))
            .include(client.message(sender_phone_id).send(recipient_fail, "Hi again"))
            .execute(),
    };

    // Assert
    let (res_ok, res_fail) = output.unwrap().flatten().unwrap();

    assert_eq!(res_ok.unwrap().message_id(), message_id_ok);

    assert!(res_fail.is_err());
    if let Err(e) = res_fail {
        let error_string = e.to_string();
        assert!(error_string.contains("131030"));
        assert!(error_string.contains("Recipient phone number not in allowed list"));
    }
}
