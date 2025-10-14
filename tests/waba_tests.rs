mod common;

use common::*;
use futures::TryStreamExt;
use serde_json::json;
use whatsapp_business_rs::{
    Client,
    waba::{PhoneNumberMetadataField, UnverifiedPhoneNumber, VerificationMethod},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{bearer_token, body_json, method, path, query_param, query_param_contains},
};

#[tokio::test]
async fn test_list_phone_numbers() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let waba = client.waba(WABA_ID);

    let response_body = json!({
        "data": [
            { "id": "phone1", "verified_name": "My Business", "display_phone_number": "+1 555-123-4567" },
            { "id": "phone2", "verified_name": "My Other Business", "display_phone_number": "+44 20 7123 4567" }
        ]
    });

    Mock::given(method("GET"))
        .and(path(format!("/v23.0/{}/phone_numbers", WABA_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(query_param_contains("fields", "verified_name"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    let numbers: Vec<_> = hold_env_var_for_me! {
        mock_server,
        waba
            .list_phone_numbers()
            .metadata([PhoneNumberMetadataField::VerifiedName].into())
            .into_stream()
            .try_collect(),
    }
    .unwrap();

    // Assert
    assert_eq!(numbers.len(), 2);
    assert_eq!(numbers[0].id, "phone1");
}

#[tokio::test]
async fn test_list_catalogs() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let waba = client.waba(WABA_ID);

    let response_body = json!({
        "data": [
            { "id": "cat_id_1", "name": "Summer Collection" },
            { "id": "cat_id_2", "name": "Winter Sale" }
        ]
    });

    Mock::given(method("GET"))
        .and(path(format!("/v23.0/{}/product_catalogs", WABA_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    let catalogs: Vec<_> = hold_env_var_for_me! {
        mock_server,
        waba
            .list_catalogs()
            .into_stream()
            .try_collect(),
    }
    .unwrap();

    // Assert
    assert_eq!(catalogs.len(), 2);
    assert_eq!(catalogs[0].id, "cat_id_1");
}

#[tokio::test]
async fn test_list_subscribed_apps() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let waba = client.waba(WABA_ID);

    let response_body = json!({
        "data": [
            {
                "whatsapp_business_api_data": {
                    "id": "app_id_1",
                    "name": "My Main App",
                    "link": "https://www.facebook.com/games/?app_id=app_id_1"
                }
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path(format!("/v23.0/{}/subscribed_apps", WABA_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    let apps: Vec<_> = hold_env_var_for_me! {
        mock_server,
        waba
            .list_subcribed_apps()
            .into_stream()
            .try_collect(),
    }
    .unwrap();

    // Assert
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].id, "app_id_1");
}

#[tokio::test]
async fn test_list_flows() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let waba = client.waba(WABA_ID);

    let response_body = json!({
        "data": [
            { "id": "flow_id_1", "name": "Appointment Booking", "status": "ACTIVE" },
            { "id": "flow_id_2", "name": "Customer Feedback", "status": "DRAFT" }
        ]
    });

    Mock::given(method("GET"))
        .and(path(format!("/v23.0/{}/flows", WABA_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    let flows: Vec<_> = hold_env_var_for_me! {
        mock_server,
        waba
            .list_flows()
            .into_stream()
            .try_collect(),
    }
    .unwrap();

    // Assert
    assert_eq!(flows.len(), 2);
    assert_eq!(flows[1].name, "Customer Feedback");
}

setup! {
    async fn test_full_number_registration_flow(mock_server: _) {
        // Arrange
        let client = Client::builder()
            .api_version("23.0")
            .auth(ACCESS_TOKEN)
            .build()
            .unwrap();
        let waba = client.waba(WABA_ID);

        // Mock Step 1: Register phone number (initial request)
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/phone_numbers", WABA_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .and(body_json(
                json!({"cc": "+1", "phone_number": "23456789", "verified_name": "My Biz"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"success": true, "id": PHONE_ID})),
            )
            .mount(&mock_server)
            .await;

        // Mock Step 2: Request verification code
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/request_code", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .and(query_param("code_method", "SMS"))
            .and(query_param("language", "en"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        // Mock Step 3: Verify code
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/verify_code", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .and(query_param("code", "123456"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        // Mock Step 4: Set PIN
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/register", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .and(body_json(json!({"messaging_product":"whatsapp","pin":"000000"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        // Act & Assert
        let phone = UnverifiedPhoneNumber {
            country_code: "+1".to_string(),
            phone_number: "23456789".to_string(),
            verified_name: "My Biz".to_string(),
        };

        let result = waba
            .start_number_registration(&phone)
            .register_phone_number()
            .await
            .unwrap()
            .request_verification_code(VerificationMethod::Sms, "en")
            .await
            .unwrap()
            .verify_phone_number("123456")
            .await
            .unwrap()
            .set_pin("000000")
            .await
            .unwrap()
            .finalize();

        assert_eq!(result.phone_number_id, PHONE_ID);
    }
}
