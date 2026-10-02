//! The MCP contract of `jevify mcp`: the handshake of both protocol eras, the tool shapes,
//! a call per tool, abstention as a plain result, a jevify error as `isError` with its kind,
//! JSON-RPC errors for malformed input, and nothing but JSON-RPC on stdout.
mod common;
use common::{FakeJev, option_containing};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MODERN_META: &str = r#"{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

/// Runs `jevify mcp` on the messages, one per line, until stdin closes; every stdout line
/// must parse as JSON, and the parsed lines come back in order.
async fn session(mut cmd: assert_cmd::Command, messages: &[String]) -> Vec<Value> {
    let input = messages
        .iter()
        .map(|m| format!("{m}\n"))
        .collect::<String>();
    let out =
        tokio::task::spawn_blocking(move || cmd.arg("mcp").write_stdin(input).output().unwrap())
            .await
            .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    stdout
        .lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("not JSON on stdout: {e}: {line}"));
            assert_eq!(v["jsonrpc"], "2.0", "{line}");
            v
        })
        .collect()
}

fn request(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

fn modern(id: u64, method: &str, mut params: Value) -> String {
    if !params.is_object() {
        params = json!({});
    }
    params["_meta"] = serde_json::from_str(MODERN_META).unwrap();
    request(id, method, params)
}

fn call(id: u64, tool: &str, arguments: Value) -> String {
    request(
        id,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    )
}

fn cause_answers() -> FakeJev {
    FakeJev {
        choose: |_, s, o| option_containing(s, o, "error: cause"),
        noul: |_, _| 0.95,
    }
}

/// The legacy handshake echoes a known revision and offers the latest otherwise; the
/// initialized notification gets no reply; ping answers an empty result.
#[tokio::test(flavor = "multi_thread")]
async fn legacy_handshake_negotiates_the_version_and_notifications_get_no_reply() {
    let server = common::mock(cause_answers()).await;
    let replies = session(
        common::jevify(&server),
        &[
            request(1, "initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
            request(2, "ping", json!({})),
            request(3, "initialize", json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})),
            request(4, "initialize", json!({"protocolVersion": "1.0", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})),
        ],
    )
    .await;
    assert_eq!(replies.len(), 4, "{replies:?}");
    let first = &replies[0];
    assert_eq!(first["id"], 1);
    assert_eq!(first["result"]["protocolVersion"], "2025-11-25");
    assert!(first["result"]["capabilities"]["tools"].is_object());
    assert_eq!(first["result"]["serverInfo"]["name"], "jevify");
    assert_eq!(first["result"]["serverInfo"]["version"], VERSION);
    assert!(
        first["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("exit_code")
    );
    assert!(first["result"].get("resultType").is_none());
    assert_eq!(replies[1], json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
    assert_eq!(replies[2]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(replies[3]["result"]["protocolVersion"], "2025-11-25");
}

/// The modern era: `server/discover` lists the supported revisions, results carry
/// `resultType` and the server identity, an unsupported version is error -32022 with the
/// supported list, and a request without client capabilities is invalid params.
#[tokio::test(flavor = "multi_thread")]
async fn modern_requests_discover_the_server_and_an_unknown_version_is_refused() {
    let server = common::mock(cause_answers()).await;
    let replies = session(
        common::jevify(&server),
        &[
            modern(1, "server/discover", json!({})),
            modern(2, "ping", json!({})),
            request(3, "ping", json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "1900-01-01", "io.modelcontextprotocol/clientCapabilities": {}}})),
            request(4, "ping", json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}})),
        ],
    )
    .await;
    assert_eq!(replies.len(), 4);
    let discover = &replies[0]["result"];
    assert_eq!(discover["resultType"], "complete");
    assert_eq!(discover["supportedVersions"][0], "2026-07-28");
    assert!(discover["capabilities"]["tools"].is_object());
    assert_eq!(
        discover["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "jevify"
    );
    assert_eq!(replies[1]["result"]["resultType"], "complete");
    assert_eq!(replies[2]["error"]["code"], -32022);
    assert_eq!(replies[2]["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(replies[2]["error"]["data"]["supported"][0], "2026-07-28");
    assert_eq!(replies[3]["error"]["code"], -32602);
}

/// The three tools, in a fixed order, each with its input schema.
#[tokio::test(flavor = "multi_thread")]
async fn tools_list_names_why_is_and_pick_with_their_schemas() {
    let server = common::mock(cause_answers()).await;
    let replies = session(
        common::jevify(&server),
        &[request(1, "tools/list", json!({}))],
    )
    .await;
    let tools = replies[0]["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["why", "is", "pick"]);
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(
            tool["description"]
                .as_str()
                .unwrap()
                .contains("structuredContent")
        );
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
    }
    let why = &tools[0]["inputSchema"]["properties"];
    assert!(why["path"].is_object() && why["text"].is_object());
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["statement"]));
    assert_eq!(tools[2]["inputSchema"]["required"], json!(["description"]));
    assert_eq!(
        tools[2]["inputSchema"]["properties"]["from_kind"]["enum"],
        json!(["commit", "branch", "file", "tool", "pr", "run"])
    );
    assert_eq!(
        tools[2]["inputSchema"]["properties"]["items"]["items"]["type"],
        "string"
    );
}

/// `why` from text and from a path: the envelope as `structuredContent` with `exit_code` 0
/// and the cause line, a short text naming the line, no error; both inputs at once is usage.
#[tokio::test(flavor = "multi_thread")]
async fn why_answers_with_the_envelope_and_a_line_from_text_or_a_path() {
    let server = common::mock(cause_answers()).await;
    let log = "step one\nerror: cause of it all\nProcess completed with exit code 1\n";
    let dir = tempfile::tempdir().unwrap().keep();
    let file = dir.join("build.log");
    std::fs::write(&file, log).unwrap();
    let replies = session(
        common::jevify(&server),
        &[
            call(1, "why", json!({"text": log})),
            call(2, "why", json!({"path": file.to_str().unwrap()})),
            call(
                3,
                "why",
                json!({"text": log, "path": file.to_str().unwrap()}),
            ),
            call(4, "why", json!({})),
        ],
    )
    .await;
    for reply in &replies[..2] {
        let result = &reply["result"];
        assert_eq!(result["isError"], false, "{reply}");
        let envelope = &result["structuredContent"];
        assert_eq!(envelope["ok"], true);
        assert_eq!(envelope["command"], "why");
        assert_eq!(envelope["version"], VERSION);
        assert_eq!(envelope["exit_code"], 0);
        assert!(envelope["error"].is_null());
        assert_eq!(envelope["data"]["causes"][0]["line"], 2);
        assert_eq!(envelope["meta"]["decision"]["verb"], "why");
        assert!(envelope["meta"]["requests"].as_u64().unwrap() > 0);
        assert_eq!(result["content"][0]["type"], "text");
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("line 2: error: cause")
        );
    }
    for reply in &replies[2..] {
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert_eq!(
            reply["result"]["structuredContent"]["error"]["kind"],
            "usage"
        );
        assert_eq!(reply["result"]["structuredContent"]["exit_code"], 2);
    }
}

/// `is` answers a verdict with its probability from inline context or a file.
#[tokio::test(flavor = "multi_thread")]
async fn is_answers_a_verdict_from_context_or_a_context_path() {
    let server = common::mock(FakeJev {
        choose: |_, _, o| o[0].clone(),
        noul: |_, s| {
            if s.to_string().contains("refund") {
                0.9
            } else {
                0.1
            }
        },
    })
    .await;
    let dir = tempfile::tempdir().unwrap().keep();
    let file = dir.join("mail.txt");
    std::fs::write(&file, "hello there\n").unwrap();
    let replies = session(
        common::jevify(&server),
        &[
            call(
                1,
                "is",
                json!({"statement": "asks for a refund", "context": "I want a refund"}),
            ),
            call(
                2,
                "is",
                json!({"statement": "asks for a refund", "context_path": file.to_str().unwrap()}),
            ),
            call(3, "is", json!({"statement": "asks for a refund"})),
        ],
    )
    .await;
    let yes = &replies[0]["result"];
    assert_eq!(yes["isError"], false);
    assert_eq!(yes["structuredContent"]["exit_code"], 0);
    assert_eq!(yes["structuredContent"]["data"]["verdict"], "yes");
    assert_eq!(yes["content"][0]["text"], "yes (p 0.90)");
    let no = &replies[1]["result"];
    assert_eq!(no["structuredContent"]["exit_code"], 1);
    assert_eq!(no["structuredContent"]["ok"], true);
    assert_eq!(no["content"][0]["text"], "no (p 0.10)");
    assert_eq!(replies[2]["result"]["isError"], true);
    assert_eq!(
        replies[2]["result"]["structuredContent"]["error"]["kind"],
        "usage"
    );
}

/// `pick` over supplied items returns the match verbatim; an abstention is a plain result
/// with exit_code 3, `data.shortlist` and a null error, never `isError`.
#[tokio::test(flavor = "multi_thread")]
async fn pick_returns_the_item_and_abstains_as_a_plain_result_with_a_shortlist() {
    let server = common::mock(FakeJev {
        choose: |_, s, o| option_containing(s, o, "payment"),
        noul: |_, s| {
            if s.to_string().contains("payment") {
                0.9
            } else {
                0.1
            }
        },
    })
    .await;
    let replies = session(
        common::jevify(&server),
        &[
            call(1, "pick", json!({"description": "the payment timeout fix", "items": ["main", "fix/payment-timeout", "docs"]})),
            call(2, "pick", json!({"description": "the payment timeout fix", "items": ["main", "docs"]})),
            call(3, "pick", json!({"description": "x", "items": ["a"], "from_kind": "branch"})),
            call(4, "pick", json!({"description": "x", "from_kind": "tag"})),
        ],
    )
    .await;
    let found = &replies[0]["result"];
    assert_eq!(found["isError"], false);
    assert_eq!(found["structuredContent"]["exit_code"], 0);
    assert_eq!(
        found["structuredContent"]["data"]["matches"][0]["text"],
        "fix/payment-timeout"
    );
    assert_eq!(found["content"][0]["text"], "fix/payment-timeout");
    let abstained = &replies[1]["result"];
    assert_eq!(abstained["isError"], false, "{abstained}");
    let envelope = &abstained["structuredContent"];
    assert_eq!(envelope["exit_code"], 3);
    assert_eq!(envelope["ok"], true);
    assert!(envelope["error"].is_null());
    assert!(envelope["data"]["shortlist"].is_array());
    assert!(envelope["data"]["matches"].as_array().unwrap().is_empty());
    assert!(
        abstained["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not chosen")
    );
    for reply in &replies[2..] {
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        assert_eq!(
            reply["result"]["structuredContent"]["error"]["kind"],
            "usage"
        );
    }
}

/// A rejected key is a tool execution error: `isError` with the envelope's `error.kind` and
/// exit code, and the kind in the text.
#[tokio::test(flavor = "multi_thread")]
async fn a_backend_error_is_a_tool_error_with_its_kind() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "bad key"})))
        .mount(&server)
        .await;
    let replies = session(
        common::jevify(&server),
        &[call(1, "why", json!({"text": "error: cause\n"}))],
    )
    .await;
    let result = &replies[0]["result"];
    assert_eq!(result["isError"], true, "{result}");
    let envelope = &result["structuredContent"];
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["exit_code"], 5);
    assert_eq!(envelope["error"]["kind"], "bad_api_key");
    assert!(
        envelope["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("jevify health")
    );
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("bad_api_key: ")
    );
}

/// Malformed input is a JSON-RPC error with a null id; an unknown method, an unknown tool and
/// a batch each get their code; a notification of any kind gets no reply.
#[tokio::test(flavor = "multi_thread")]
async fn malformed_json_and_unknown_methods_are_json_rpc_errors() {
    let server = common::mock(cause_answers()).await;
    let replies = session(
        common::jevify(&server),
        &[
            "{not json".to_owned(),
            "[]".to_owned(),
            json!({"jsonrpc": "2.0", "id": 1, "method": "nope"}).to_string(),
            json!({"jsonrpc": "2.0", "id": "s", "method": "tools/call", "params": {"name": "label", "arguments": {}}}).to_string(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "why", "arguments": "x"}}).to_string(),
            json!({"jsonrpc": "1.0", "id": 3, "method": "ping"}).to_string(),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}).to_string(),
            json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}).to_string(),
        ],
    )
    .await;
    assert_eq!(replies.len(), 7, "{replies:?}");
    assert_eq!(replies[0]["error"]["code"], -32700);
    assert!(replies[0]["id"].is_null());
    assert_eq!(replies[1]["error"]["code"], -32600);
    assert_eq!(replies[2]["error"]["code"], -32601);
    assert_eq!(replies[2]["id"], 1);
    assert_eq!(replies[3]["error"]["code"], -32602);
    assert_eq!(replies[3]["id"], "s");
    assert!(
        replies[3]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("label")
    );
    assert_eq!(replies[4]["error"]["code"], -32602);
    assert_eq!(replies[5]["error"]["code"], -32600);
    assert_eq!(replies[6]["id"], 4);
    assert_eq!(replies[6]["result"], json!({}));
}
