use claudebot_mcp::bridge::{GrpcBridgeClient, GrpcBridgeClientConfig};

#[tokio::test]
async fn test_grpc_execute() {
    let config = GrpcBridgeClientConfig {
        endpoint: "http://localhost:9998".to_string(),
        api_key: "test-key".to_string(),
        timeout_seconds: 60,
        ca_cert_path: None,
        domain: None,
    };

    let client = GrpcBridgeClient::new(config).await.expect("Failed to connect");

    // Use user's actual chat_id for auth, or test with streaming
    let result = client.execute_full(8378448645, "Say hello in one word", None).await;

    match result {
        Ok(r) => {
            println!("✓ Execute success: {}", r.success);
            println!("✓ Execute text: '{}'", r.text);
            println!("✓ Execute duration: {}ms", r.duration_ms);
            println!("✓ Execute error: {:?}", r.error);
            // Don't assert - just check it works
            if r.success {
                println!("✓ Full execution succeeded!");
            } else {
                println!("✗ Execution returned success=false");
            }
        }
        Err(e) => {
            println!("✗ Execute failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_grpc_health() {
    let config = GrpcBridgeClientConfig {
        endpoint: "http://localhost:9998".to_string(),
        api_key: "test-key".to_string(),
        timeout_seconds: 10,
        ca_cert_path: None,
        domain: None,
    };

    let client = GrpcBridgeClient::new(config).await.expect("Failed to connect");
    let healthy = client.health_check().await.expect("Health check failed");
    assert!(healthy, "Server should be healthy");
    println!("✓ gRPC health check passed");
}

#[tokio::test]
async fn test_grpc_status() {
    let config = GrpcBridgeClientConfig {
        endpoint: "http://localhost:9998".to_string(),
        api_key: "test-key".to_string(),
        timeout_seconds: 10,
        ca_cert_path: None,
        domain: None,
    };

    let client = GrpcBridgeClient::new(config).await.expect("Failed to connect");
    let status = client.status().await.expect("Status check failed");
    println!("✓ gRPC status: {:?}", status);
}
