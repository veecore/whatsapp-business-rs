mod common;

use common::*;
use serde_json::json;
use whatsapp_business_rs::{
    Client, IdentityRef,
    catalog::ProductRef,
    message::{
        Button, Location, Media, MessageRef, OptionButton, OptionList, ProductList, Reaction,
        Section,
    },
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{bearer_token, body_json, body_string_contains, method, path},
};

#[tokio::test]
async fn test_send_media_message() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();

    let messages = client.message(PHONE_ID);
    let message_id = "wamid.your_message_id";

    let media = "0xf434rj4n5f4r0irj44xf2";
    let media_id = "133489585902390";

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "image",
        "image": {
            "caption": "Hello, world!",
            "id": media_id
        }
    });

    let response_body = json!({
        "messaging_product": "whatsapp",
        "contacts": [{"input": RECIPIENT_ID, "wa_id": RECIPIENT_ID}],
        "messages": [{"id": message_id}]
    });

    // Mock Step 1: Upload Media
    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/media", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_string_contains("image/png"))
        .and(body_string_contains(media))
        .respond_with(ResponseTemplate::new(200).set_body_json(json! {{"id": media_id}}))
        .mount(&mock_server)
        .await;

    // Mock Step 1: Send Message
    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    let recipient = IdentityRef::user(RECIPIENT_ID);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(recipient, Media::png(media.as_bytes().to_vec()).caption("Hello, world!")),
    };

    // Assert
    assert_eq!(result.unwrap().message_id(), message_id);
}

/// Tests sending a media message (e.g., image) using its pre-uploaded ID.
#[tokio::test]
async fn test_send_media_message_by_id() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);
    let media_id = "uploaded_media_id_123";

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": RECIPIENT_ID,
        "type": "image",
        "image": {
            "id": media_id
        }
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages": [{"id": "wamid.mock_message_id"}]})),
        )
        .mount(&mock_server)
        .await;

    // Act
    let image_message = Media::png(media_id);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(RECIPIENT_ID, image_message),
    };

    // Assert
    result.unwrap();
}

/// Tests sending a basic text message.
#[tokio::test]
async fn test_send_text_message() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let message_id = "wamid.your_message_id";

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "text",
        "text": {
            "body": "Hello, world!"
        }
    });

    let response_body = json!({
        "messaging_product": "whatsapp",
        "contacts": [{"input": RECIPIENT_ID, "wa_id": RECIPIENT_ID}],
        "messages": [{"id": message_id}]
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    let recipient = IdentityRef::user(RECIPIENT_ID);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(recipient, "Hello, world!"),
    };

    // Assert
    assert_eq!(result.unwrap().message_id(), message_id);
}

/// Tests sending an emoji reaction to a specific message.
#[tokio::test]
async fn test_send_reaction_message() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);
    let message_to_react_to_id = "wamid.incoming_message_id_for_reaction";

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "reaction",
        "reaction": {
            "message_id": message_to_react_to_id,
            "emoji": "👍"
        }
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages": [{"id": "wamid.mock_reaction_id"}]})),
        )
        .mount(&mock_server)
        .await;

    // Act
    let reaction_message = Reaction::new('👍', message_to_react_to_id);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(RECIPIENT_ID, reaction_message),
    };

    // Assert
    result.unwrap();
}

/// Tests sending a location message with coordinates, name, and address.
#[tokio::test]
async fn test_send_location_message() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "location",
        "location": {
            "latitude": "-12.345",
            "longitude": "67.89",
            "name": "Some Place",
            "address": "123 Main St"
        }
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages": [{"id": "wamid.mock_message_id"}]})),
        )
        .mount(&mock_server)
        .await;

    // Act
    let location_message = Location::new(-12.345, 67.890)
        .name("Some Place")
        .address("123 Main St");
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(RECIPIENT_ID, location_message),
    };

    // Assert
    result.unwrap();
}

/// Tests sending an interactive message with reply buttons.
#[tokio::test]
async fn test_send_interactive_message_with_buttons() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "interactive",
        "interactive": {
            "type": "button",
            "body": {
                "text": "Please select an option:"
            },
            "footer": {
                "text": "Footer text"
            },
            "action": {
                "buttons": [
                    { "type": "reply", "reply": { "id": "btn_1", "title": "Option 1" } },
                    { "type": "reply", "reply": { "id": "btn_2", "title": "Option 2" } }
                ]
            }
        }
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages": [{"id": "wamid.mock_message_id"}]})),
        )
        .mount(&mock_server)
        .await;

    // Act
    let buttons = [
        Button::reply("btn_1", "Option 1"),
        Button::reply("btn_2", "Option 2"),
    ];
    let interactive_message = ("Please select an option:", buttons, "Footer text");
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(RECIPIENT_ID, interactive_message),
    };

    // Assert
    result.unwrap();
}

/// Tests sending an interactive message with a list of options.
#[tokio::test]
async fn test_send_interactive_message_with_list() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "interactive",
        "interactive": {
            "type": "list",
            "body": {
                "text": "Choose your item:"
            },
            "action": {
                "button": "View Items",
                "sections": [
                    {
                        "title": "Category 1",
                        "rows": [
                            {
                                "id": "item_1_id",
                                "title": "Item 1",
                                "description": "Description for Item 1"
                            }
                        ]
                    }
                ]
            }
        }
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages": [{"id": "wamid.mock_message_id"}]})),
        )
        .mount(&mock_server)
        .await;

    // Act
    let options = Section::new(
        "Category 1",
        [OptionButton::new(
            "Description for Item 1",
            "Item 1",
            "item_1_id",
        )],
    );
    let option_list = OptionList::new_section(options, "View Items");
    let interactive_message = ("Choose your item:", option_list);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(RECIPIENT_ID, interactive_message),
    };

    // Assert
    result.unwrap();
}

/// Tests sending an interactive message with a list of products from a catalog.
#[tokio::test]
async fn test_send_interactive_message_with_product_list() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);
    let catalog_id = "catalog_id_456";

    let request_body = json!({
        "messaging_product": "whatsapp",
        "recipient_type":"individual",
        "to": RECIPIENT_ID,
        "type": "interactive",
        "interactive": {
            "type": "product_list",
            "header": {
                "type": "text",
                "text": "Our Products"
            },
            "body": {
                "text": "Check out our new arrivals."
            },
            "action": {
                "catalog_id": catalog_id,
                "sections": [
                    {
                        "title": "New Arrivals",
                        "product_items": [
                            { "product_retailer_id": "sku_123" }
                        ]
                    }
                ]
            },
            "footer": {
                "text": ""
            }
        }
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"messages": [{"id": "wamid.mock_message_id"}]})),
        )
        .mount(&mock_server)
        .await;

    // Act
    let products = Section::new("New Arrivals", [ProductRef::from("sku_123")]);
    let product_list = ProductList::new_section(products, catalog_id);
    // Using the (Header, Body, Action, Footer) tuple to create the draft
    let interactive_message = (
        "Our Products",
        "Check out our new arrivals.",
        product_list,
        "",
    );
    let result = hold_env_var_for_me! {
        mock_server,
        messages.send(RECIPIENT_ID, interactive_message),
    };

    // Assert
    result.unwrap();
}

#[tokio::test]
async fn test_mark_message_as_read() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let message_to_mark_id = "wamid.incoming_message_id";

    let request_body = json!({
        "messaging_product": "whatsapp",
        "status": "read",
        "message_id": message_to_mark_id,
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&mock_server)
        .await;

    // Act
    let message_ref = MessageRef::from(message_to_mark_id);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.set_read(message_ref),
    };

    // Assert
    result.unwrap()
}

#[tokio::test]
async fn test_set_typing() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let message_to_mark_id = "wamid.incoming_message_id";
    // Typing indicates read
    let request_body = json!({
        "messaging_product": "whatsapp",
        "status": "read",
        "typing_indicator": {
            "type": "text"
        },
        "message_id": message_to_mark_id,
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/messages", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&request_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&mock_server)
        .await;

    // Act
    let message_ref = MessageRef::from(message_to_mark_id);
    let result = hold_env_var_for_me! {
        mock_server,
        messages.set_replying(message_ref),
    };

    // Assert
    result.unwrap()
}

#[tokio::test]
async fn test_upload_media_success() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID);

    let uploaded_media_id = "media_id_777";

    // Wiremock's multipart matching is more complex; we'll match headers and path.
    // Body matching can be done with `body_string_contains` if needed.
    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/media", PHONE_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": uploaded_media_id})))
        .mount(&mock_server)
        .await;

    // Act
    let image_bytes = vec![0u8; 1024]; // Dummy bytes

    let result = hold_env_var_for_me! {
        mock_server,
        messages
            .upload_media(image_bytes, "image/png".parse().unwrap(), "image.png"),
    };

    // Assert
    assert_eq!(result.unwrap(), uploaded_media_id);
}

#[tokio::test]
async fn test_delete_media_success() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let messages = client.message(PHONE_ID); // The phone_id isn't part of the path, but manager is needed

    let media_to_delete_id = "media_id_to_delete_888";

    Mock::given(method("DELETE"))
        .and(path(format!("/v23.0/{}", media_to_delete_id)))
        .and(bearer_token(ACCESS_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&mock_server)
        .await;

    // Act
    let result = hold_env_var_for_me! {
        mock_server,
        messages.delete_media(media_to_delete_id),
    };

    // Assert
    result.unwrap()
}
