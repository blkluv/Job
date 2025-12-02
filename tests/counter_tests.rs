use jobmcp::{Counter, common::StructRequest};
use rmcp::{handler::server::wrapper::Parameters, model::*};

/// Helper to extract &str from Annotated<RawContent>
fn unwrap_raw_text(raw: &Annotated<RawContent>) -> &str {
    // as_text() returns &RawTextContent, which has a .text field
    raw.as_text().unwrap().text.as_str()
}

#[tokio::test]
async fn test_counter_increment() -> anyhow::Result<()> {
    let counter = Counter::new();

    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "0");

    counter.increment().await?;
    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "1");

    counter.increment().await?;
    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "2");

    Ok(())
}

#[tokio::test]
async fn test_counter_decrement() -> anyhow::Result<()> {
    let counter = Counter::new();

    counter.decrement().await?;
    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "-1");

    counter.decrement().await?;
    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "-2");

    Ok(())
}

#[tokio::test]
async fn test_counter_increment_and_decrement() -> anyhow::Result<()> {
    let counter = Counter::new();

    counter.increment().await?;
    counter.increment().await?;
    counter.increment().await?;
    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "3");

    counter.decrement().await?;
    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "2");

    Ok(())
}

#[tokio::test]
async fn test_say_hello() -> anyhow::Result<()> {
    let counter = Counter::new();

    let result = counter.say_hello()?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "hello");
    assert!(result.is_error.is_none() || !result.is_error.unwrap());

    Ok(())
}

#[tokio::test]
async fn test_echo() -> anyhow::Result<()> {
    let counter = Counter::new();

    let mut object = serde_json::Map::new();
    object.insert("message".to_string(), serde_json::json!("test message"));
    object.insert("number".to_string(), serde_json::json!(42));

    let result = counter.echo(Parameters(object.clone()))?;
    let response_text = unwrap_raw_text(&result.content[0]);

    assert!(response_text.contains("test message"));
    assert!(response_text.contains("42"));

    Ok(())
}

#[tokio::test]
async fn test_sum() -> anyhow::Result<()> {
    let counter = Counter::new();

    let request = StructRequest { a: 5, b: 7 };
    let result = counter.sum(Parameters(request))?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "12");

    let request = StructRequest { a: -10, b: 3 };
    let result = counter.sum(Parameters(request))?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "-7");

    Ok(())
}

// Note: Tests for prompt functions (example_prompt, counter_analysis) are skipped
// because they require a full RequestContext with a Peer, which is complex to mock
// in unit tests. These would be better tested in integration tests with a real MCP connection.

#[tokio::test]
async fn test_tool_router_has_tools() -> anyhow::Result<()> {
    let counter = Counter::new();

    assert!(counter.tool_router.has_route("increment"));
    assert!(counter.tool_router.has_route("decrement"));
    assert!(counter.tool_router.has_route("get_value"));
    assert!(counter.tool_router.has_route("say_hello"));
    assert!(counter.tool_router.has_route("echo"));
    assert!(counter.tool_router.has_route("sum"));

    assert!(!counter.tool_router.has_route("non_existent_tool"));

    Ok(())
}

#[tokio::test]
async fn test_prompt_router_has_prompts() -> anyhow::Result<()> {
    let counter = Counter::new();

    assert!(counter.prompt_router.has_route("example_prompt"));
    assert!(counter.prompt_router.has_route("counter_analysis"));

    assert!(!counter.prompt_router.has_route("non_existent_prompt"));

    Ok(())
}

#[tokio::test]
async fn test_concurrent_counter_operations() -> anyhow::Result<()> {
    let counter = Counter::new();

    let mut handles = vec![];
    for _ in 0..10 {
        let counter_clone = counter.clone();
        let handle = tokio::spawn(async move { counter_clone.increment().await });
        handles.push(handle);
    }

    for handle in handles {
        handle.await??;
    }

    let result = counter.get_value().await?;
    assert_eq!(unwrap_raw_text(&result.content[0]), "10");

    Ok(())
}
