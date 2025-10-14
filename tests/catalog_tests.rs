mod common;

use common::*;
use serde_json::json;
use whatsapp_business_rs::{
    Client, Fields,
    catalog::{MetaProductRef, Price, ProductData, ProductDataField},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{bearer_token, body_json, method, path, query_param_contains},
};

#[tokio::test]
async fn test_create_product_success() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let catalog = client.catalog(CATALOG_ID);

    let new_product_id = "prod_12345";
    let product_data = json!({
        "retailer_id": "widget-sku-001",
        "name": "Fancy Widget",
        "price": "1999.00 USD",
        "currency": "USD"
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}/products", CATALOG_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&product_data))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": new_product_id})))
        .mount(&mock_server)
        .await;

    // Act
    let product = ProductData::default()
        .name("Fancy Widget")
        .price(Price(1999.0, "USD".into()))
        .currency("USD")
        .build("widget-sku-001");

    let result = hold_env_var_for_me! {
        mock_server,
        catalog.create_product(product),
    };

    // Assert
    assert_eq!(result.unwrap().product_id(), new_product_id);
}

#[tokio::test]
async fn test_list_products_with_metadata() {
    let mock_server = MockServer::start().await;

    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let catalog = client.catalog(CATALOG_ID);

    let response_body = json!({
        "data": [
            {"id": "p1", "retailer_id": "sku1", "name": "Product A", "price": "10.00 USD"},
            {"id": "p2", "retailer_id": "sku2", "name": "Product B", "price": "20.00 USD"}
        ]
    });

    Mock::given(method("GET"))
        .and(path(format!("/v23.0/{}/products", CATALOG_ID)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(query_param_contains("fields", "name"))
        .and(query_param_contains("fields", "price"))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(&mock_server)
        .await;

    // Act
    use futures::TryStreamExt;

    let products: Vec<_> = hold_env_var_for_me! {
        mock_server,
        catalog
            .list_products()
            .metadata(
                Fields::new()
                    .with(ProductDataField::Name)
                    .with(ProductDataField::Price),
            )
            .into_stream()
            .try_collect(),
    }
    .unwrap();

    // Assert
    assert_eq!(products.len(), 2);
    assert_eq!(products[0].name.as_deref(), Some("Product A"));
}

#[tokio::test]
async fn test_update_product_by_id_success() {
    let mock_server = MockServer::start().await;
    // Arrange
    let client = Client::builder()
        .api_version("23.0")
        .auth(ACCESS_TOKEN)
        .build()
        .unwrap();
    let catalog = client.catalog(CATALOG_ID);

    let product_to_update_id = "meta_prod_id_555";
    let update_payload = json!({
        "name": "Super Shiny Widget",
        "price": "8999.00 USD"
    });

    Mock::given(method("POST"))
        .and(path(format!("/v23.0/{}", product_to_update_id)))
        .and(bearer_token(ACCESS_TOKEN))
        .and(body_json(&update_payload))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&mock_server)
        .await;

    // Act
    let product_ref = MetaProductRef::from(product_to_update_id);
    let result = hold_env_var_for_me! {
        mock_server,
        catalog
            .update_product_by_id(product_ref)
            .name("Super Shiny Widget")
            .price(Price(8999.0, "USD".into())),
    };

    // Assert
    result.unwrap()
}
