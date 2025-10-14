mod common;

use common::*;
use serde_json::json;
use whatsapp_business_rs::{
    Business, Client, Fields,
    app::{SubscriptionField, WebhookConfig},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{bearer_token, body_json, method, path, query_param},
};

setup! {
    async fn test_update_token_success(mock_server: _) {
        // Arrange
        let client = Client::builder()
            .api_version("23.0")
            .auth(ACCESS_TOKEN)
            .build()
            .unwrap();
        let app = client.app(APP_ID);

        let expected_token = "NEW_EXTENDED_TOKEN";
        let response_body = json!({
            "access_token": expected_token,
            "token_type": "bearer",
            "expires_in": 5184000
        });

        Mock::given(method("GET"))
            .and(path("/v23.0/oauth/access_token"))
            .and(bearer_token(ACCESS_TOKEN))
            .and(query_param("grant_type", "fb_exchange_token"))
            .and(query_param("client_id", APP_ID))
            .and(query_param("client_secret", APP_SECRET))
            .and(query_param("fb_exchange_token", ACCESS_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&mock_server)
            .await;

        // Act
        let result = app.update_token(ACCESS_TOKEN, APP_SECRET).await;

        // Assert
        let token = result.unwrap();
        assert_eq!(token.access_token(), expected_token);
    }
}

setup! {
    async fn test_debug_token_valid(mock_server: _) {
        // Arrange
        let client = Client::builder()
            .api_version("23.0")
            .auth(ACCESS_TOKEN)
            .build()
            .unwrap();
        let app = client.app(APP_ID);

        let response_body = json!({
            "data": {
                "app_id": APP_ID,
                "type": "USER",
                "application": "Your App Name",
                "is_valid": true,
                "data_access_expires_at": 123456,
                "expires_at": 123456,
                "scopes": ["whatsapp_business_management"]
            }
        });

        Mock::given(method("GET"))
            .and(path("/v23.0/debug_token"))
            .and(bearer_token(ACCESS_TOKEN))
            .and(query_param("input_token", "TOKEN_TO_DEBUG"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
            .mount(&mock_server)
            .await;

        // Act
        let result = app.debug_token("TOKEN_TO_DEBUG").await;

        // Assert
        let debug_info = result.unwrap();
        assert!(debug_info.is_valid);
        assert_eq!(debug_info.app_id, APP_ID);
    }
}

#[tokio::test]
async fn test_configure_webhook_success() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let app = client.app(APP_ID);

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/subscriptions", APP_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(query_param("object", "whatsapp_business_account"))
        .and(query_param("callback_url", "https://your.domain/webhook"))
        .and(query_param("verify_token", "YOUR_SECRET_VERIFY_TOKEN"))
        .and(query_param("fields", "messages,account_update"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&mock_server)
        .await;

    // Act
    let config = WebhookConfig {
        verify_token: "YOUR_SECRET_VERIFY_TOKEN".into(),
        webhook_url: "https://your.domain/webhook".into(),
    };
    let events = Fields::new()
        .with(SubscriptionField::Messages)
        .with(SubscriptionField::AccountUpdate);

    let result = hold_env_var_for_me! {
        mock_server,
        app.configure_webhook(config).events(events),
    };

    // Assert
    result.unwrap();
}

setup! {
    async fn test_full_onboarding_flow(mock_server: _) {
        // Arrange
        let client = Client::builder()
            .api_version("23.0")
            .auth("DUMMY_INITIAL_TOKEN")
            .build()
            .unwrap(); // Token will be acquired in step 1

        // Mock Step 1: Get Access Token
        Mock::given(method("GET"))
            .and(path("/v23.0/oauth/access_token"))
            .and(bearer_token("DUMMY_INITIAL_TOKEN"))
            .and(query_param("code", "AUTH_CODE_FROM_META"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": ACCESS_TOKEN, "token_type": "bearer"
            })))
            .mount(&mock_server)
            .await;

        // Mock Step 2: Subscribe App
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/subscribed_apps", WABA_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        // Mock Step 3: Register Phone Number
        Mock::given(method("POST"))
            .and(path(format!("/v23.0/{}/register", PHONE_ID)))
            .and(bearer_token(ACCESS_TOKEN))
            .and(body_json(json!({
                "messaging_product": "whatsapp",
                "pin": "123456"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .mount(&mock_server)
            .await;

        // Mock Step 4: Share Credit Line
        let credit_line_id = "credit_line_abc";
        Mock::given(method("POST"))
            .and(path(format!(
                "/v23.0/{}/whatsapp_credit_sharing_and_attach",
                credit_line_id
            )))
            .and(bearer_token("DUMMY_INITIAL_TOKEN"))
            .and(query_param("waba_id", WABA_ID))
            .and(query_param("waba_currency", "USD"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "allocation_config_id": "alloc_config_xyz"
            })))
            .mount(&mock_server)
            .await;

        // Act & Assert
        let business = Business::new(WABA_ID, PHONE_ID);
        let app = client.app(APP_ID);
        let flow = app.start_onboarding(business);
        let result = flow
            .get_access_token("AUTH_CODE_FROM_META", APP_SECRET)
            .await
            .unwrap()
            .subscribe()
            .await
            .unwrap()
            .register_phone_number("123456")
            .await
            .unwrap()
            .share_credit_line(credit_line_id, "USD")
            .await
            .unwrap()
            .finalize();

        assert_eq!(result.allocation_config_id, "alloc_config_xyz");
    }
}
