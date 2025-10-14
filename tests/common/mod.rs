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

// --- TEST SETUP MACRO ---

// Instead of holding the var for the entire test, we hold for the critical part.
#[macro_export]
macro_rules! hold_env_var_for_me {
    // Caller knows the whole request is already prepared on call to into_future
    // So no need to block
    (
        $mock_server:ident,
        $fut:expr,
    ) => {
        // Set the environment variable just for this test's scope.
        temp_env::with_var(
            "API_BASE_URL_FOR_TESTING",
            Some($mock_server.uri()),
            move || $fut.into_future(),
        )
        .await
    };
}

#[macro_export]
macro_rules! setup {
    (
        $async:ident $fn:ident $name:ident($server:ident: _) $body:block
    ) => {
        #[test]
        $fn $name() {
            let mock_server = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(wiremock::MockServer::start());

            fn closure($server: wiremock::MockServer) {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {$body});
            }

            // Set the environment variable just for this test's scope.
            // Returning the future from the closure makes us lose its protection.
            temp_env::with_var("API_BASE_URL_FOR_TESTING", Some(mock_server.uri()), move || {
                closure(mock_server)
            })
        }
    }
}
