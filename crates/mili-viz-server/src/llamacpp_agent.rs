//! FunctionGemma agent via llama-server (Phase 6+ development milestone).
//! Connects to a locally-running llama-server on the `/completion` endpoint,
//! builds FunctionGemma-format prompts, parses tool calls, and dispatches them
//! through the griz command vocabulary.
//!
//! Activation: `--agent llamacpp` at server startup.

use std::collections::HashMap;
use std::pin::Pin;

use mili_viz_proto::v1 as pb;
use serde_json::{json, Value};

use crate::agent::{AgentBackend, AgentTurnCtx, ran_summary};

const DEFAULT_SERVER_URL: &str = "http://localhost:8080";
const MAX_STEPS: usize = 8;

// Embed tools.json and system prompt at compile time
const TOOLS_JSON: &str = include_str!("../../../data/posttraining/grammar/tools.json");
const SYSTEM_PROMPT: &str = "You are an assistant that operates the Griz post-processor for the \
    Mili finite-element format. You drive Griz by emitting JSON function calls into the supplied \
    tool inventory. Inspect the user's request, call exactly the tools that satisfy it, and reply \
    with one short final text message only after the request is fully complete. Do not narrate \
    plans; emit a tool call instead. Prefer the typed tools over the `griz_raw` fallback when a \
    typed tool exists for the task.\n\n\
    UNDERSTANDING TOOL RESPONSES:\n\
    When a tool response includes 'action_complete': true, the action has succeeded and you should \
    move on. For state-changing tools (set_state): compare 'requested_state' with 'state' to verify \
    completion. Do not repeat the same tool call with identical arguments if you already received a \
    successful response. Only call a tool again if you need to verify something or if the previous \
    response indicated an error (ok: false).\n\n\
    JSON TOOL CALL FORMAT (REQUIRED):\n\
    Emit tool calls ONLY as valid JSON objects with 'name' and 'arguments' keys:\n\
    {\"name\": \"tool_name\", \"arguments\": {\"param1\": value1, \"param2\": value2}}\n\
    Do NOT wrap in markdown, comments, or extra text. Emit only the raw JSON object.\n\
    Ensure all argument values match their expected types (strings quoted, numbers unquoted, \
    booleans as true/false).\n\n\
    KEY TOOL MAPPINGS:\n\
    - Load/open a database: use `load` with root parameter\n\
    - Display/show/color a result: use `show` with result parameter\n\
    - Enable/disable materials: use `material` with enable (true/false) and material/class_name\n\
    - Select elements: use `select` or `clrsel` (clear selection)\n\
    - Change states: use `set_state` or `step`\n\
    - Adjust view: use `colormap`, `view`, `named_view`, `legend`\n\n\
    TASK COMPLETION:\n\
    When you have completed ALL sub-tasks in the user's request, emit the final text message and STOP. \
    Do not call extra verification tools. Do not loop. If no more actions are needed, just send the \
    final message.";

pub struct LlamaCppAgent {
    pub server_url: String,
}

impl LlamaCppAgent {
    pub fn new() -> Self {
        Self {
            server_url: DEFAULT_SERVER_URL.to_string(),
        }
    }

    pub fn with_url(url: impl Into<String>) -> Self {
        Self {
            server_url: url.into(),
        }
    }
}

impl Default for LlamaCppAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBackend for LlamaCppAgent {
    fn run_turn<'a>(
        &'a self,
        ctx: AgentTurnCtx,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            ctx.emit_status(pb::AgentStatusKind::AgentThinking, "");

            // Load tools from embedded JSON
            let tools = match parse_tools_json(TOOLS_JSON) {
                Ok(t) => t,
                Err(e) => {
                    ctx.emit_token(format!("Error loading tools: {e}"));
                    return;
                }
            };

            // Initialize message history with user request
            let mut messages: Vec<Message> = vec![Message {
                role: "user".to_string(),
                content: Some(ctx.request.text.clone()),
                tool_calls: vec![],
                tool_name: None,
            }];

            // Multi-turn loop (up to MAX_STEPS)
            for step in 0..MAX_STEPS {
                if ctx.cancelled() {
                    return;
                }

                // Build FunctionGemma prompt
                let prompt = build_functiongemma_prompt(&messages, &tools);

                // POST to llama-server
                let response = match post_completion(&self.server_url, &prompt).await {
                    Ok(r) => r,
                    Err(e) => {
                        ctx.emit_token(format!("Error: {e}"));
                        return;
                    }
                };

                // Try to parse tool calls from response
                if let Some(tool_calls) = parse_tool_calls(&response) {
                    ctx.emit_status(pb::AgentStatusKind::AgentRunning, "");

                    let mut assistant_msg = Message {
                        role: "assistant".to_string(),
                        content: None,
                        tool_calls: vec![],
                        tool_name: None,
                    };

                    let mut tool_responses: Vec<Message> = vec![];

                    for (i, tc) in tool_calls.iter().enumerate() {
                        let call_id = format!("{}-call-{}", ctx.turn_id, i);
                        ctx.emit_tool_begin(&call_id, ran_summary(&tc.name), "");

                        // Map tool call to griz command + synthesize tool response.
                        // tool_ok=false for unmapped tools so the model knows to retry / give up
                        // rather than thinking the call succeeded.
                        let (cmd, result_json, tool_ok) = tool_to_cmd(&tc.name, &tc.arguments);

                        let seq = if let Some(c) = cmd {
                            ctx.dispatch(c)
                        } else {
                            0
                        };

                        let result_summary = if tool_ok { "ok" } else { "tool not implemented" };
                        ctx.emit_tool_end(&call_id, tool_ok, result_summary, seq);

                        // Add to assistant message for history
                        assistant_msg.tool_calls.push(ToolCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments)
                                .unwrap_or_else(|_| "{}".to_string()),
                        });

                        // Add tool response message (tool_name is what
                        // FunctionGemma's `response:NAME{...}` expects).
                        tool_responses.push(Message {
                            role: "tool".to_string(),
                            content: Some(result_json.to_string()),
                            tool_calls: vec![],
                            tool_name: Some(tc.name.clone()),
                        });
                    }

                    // Append assistant message + tool responses to history
                    messages.push(assistant_msg);
                    messages.extend(tool_responses);
                } else {
                    // No tool calls — treat as final text response
                    for word in response.trim().split_whitespace() {
                        if ctx.cancelled() {
                            return;
                        }
                        ctx.emit_token(word);
                        ctx.emit_token(" ");
                    }
                    return;
                }

                if step == MAX_STEPS - 1 {
                    ctx.emit_token("(step cap reached)");
                    return;
                }
            }
        })
    }
}

// ──────────────────────────────────────────────────────────────────────
// Message structures for prompt building
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Message {
    role: String,
    content: Option<String>,
    tool_calls: Vec<ToolCall>,
    /// For role="tool" messages: the name of the tool that produced this
    /// response. FunctionGemma's `<start_function_response>response:NAME{...}`
    /// requires the actual tool name (e.g. `load`), not the call id.
    tool_name: Option<String>,
}

#[derive(Debug, Clone)]
struct ToolCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone)]
struct ParsedToolCall {
    name: String,
    arguments: Value,
}

#[derive(Debug)]
struct Tool {
    name: String,
    description: String,
    input_schema: Value,
}

// ──────────────────────────────────────────────────────────────────────
// HTTP and parsing
// ──────────────────────────────────────────────────────────────────────

async fn post_completion(server_url: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/completion", server_url);

    let payload = json!({
        "prompt": prompt,
        "temperature": 0.0,
        "n_predict": 256,
        "seed": 0,
        "stop": ["<start_function_response>", "<end_of_turn>"],
    });

    let resp = client
        .post(&url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let result: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    result
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "No content in response".to_string())
        .map(|s| s.to_string())
}

fn parse_tools_json(json_str: &str) -> Result<Vec<Tool>, String> {
    let tools_list: Vec<Value> =
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse tools.json: {e}"))?;

    let mut tools = vec![];
    for tool_val in tools_list {
        let name = tool_val
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Tool missing name")?
            .to_string();

        let description = tool_val
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let input_schema = tool_val
            .get("input_schema")
            .cloned()
            .unwrap_or(Value::Object(serde_json::Map::new()));

        tools.push(Tool {
            name,
            description,
            input_schema,
        });
    }

    Ok(tools)
}

fn parse_tool_calls(text: &str) -> Option<Vec<ParsedToolCall>> {
    // Try JSON format first: {"name": "...", "arguments": {...}}
    if let Some(calls) = parse_json_tool_calls(text) {
        if !calls.is_empty() {
            return Some(calls);
        }
    }

    // Fall back to FunctionGemma text format
    parse_functiongemma_tool_calls(text)
}

fn parse_json_tool_calls(text: &str) -> Option<Vec<ParsedToolCall>> {
    // Simple JSON parser: find {...} and try to parse as tool call
    let mut calls = vec![];

    // Look for patterns like {"name": "X", "arguments": {...}}
    let mut brace_depth = 0;
    let mut start_idx = None;

    for (i, ch) in text.chars().enumerate() {
        match ch {
            '{' => {
                if brace_depth == 0 {
                    start_idx = Some(i);
                }
                brace_depth += 1;
            }
            '}' => {
                brace_depth -= 1;
                if brace_depth == 0 {
                    if let Some(start) = start_idx {
                        let json_str = &text[start..=i];
                        if let Ok(call) = parse_single_json_tool_call(json_str) {
                            calls.push(call);
                        }
                        start_idx = None;
                    }
                }
            }
            _ => {}
        }
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

fn parse_single_json_tool_call(json_str: &str) -> Result<ParsedToolCall, String> {
    let obj: Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse JSON: {e}"))?;

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'name' in tool call")?
        .to_string();

    let arguments = obj
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    Ok(ParsedToolCall { name, arguments })
}

fn parse_functiongemma_tool_calls(text: &str) -> Option<Vec<ParsedToolCall>> {
    // Match <start_function_call>call:name{...}<end_function_call>
    let mut calls = vec![];

    // Simple regex-like parsing using string search
    let pattern_start = "<start_function_call>";
    let pattern_end = "<end_function_call>";

    let mut remaining = text;
    while let Some(start_pos) = remaining.find(pattern_start) {
        remaining = &remaining[start_pos + pattern_start.len()..];

        if let Some(end_pos) = remaining.find(pattern_end) {
            let content = &remaining[..end_pos];
            remaining = &remaining[end_pos + pattern_end.len()..];

            if let Some(call) = parse_functiongemma_content(content) {
                calls.push(call);
            }
        }
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

fn parse_functiongemma_content(content: &str) -> Option<ParsedToolCall> {
    // Format: call:name{key:<escape>val<escape>,...}
    // Extract name and arguments
    let call_prefix = "call:";
    if !content.starts_with(call_prefix) {
        return None;
    }

    let remaining = &content[call_prefix.len()..];
    let brace_pos = remaining.find('{')?;
    // Tolerate `material.disable` / `show-primal` by keeping the base name
    // (mirrors the Python parser at llamacpp.py:_parse_functiongemma_tool_calls).
    let name = remaining[..brace_pos]
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();

    let args_content = &remaining[brace_pos + 1..];
    let closing_brace = args_content.rfind('}')?;
    let args_str = &args_content[..closing_brace];

    // Parse key:<escape>val<escape> pairs
    let mut arguments = serde_json::Map::new();

    for part in args_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Match: key:<escape>val<escape> or key:val
        if let Some(colon_pos) = part.find(':') {
            let key = part[..colon_pos].trim();
            let val_part = &part[colon_pos + 1..];

            let value_str = if val_part.contains("<escape>") {
                // Extract content between <escape> tags
                let escape_prefix = "<escape>";
                if let Some(start) = val_part.find(escape_prefix) {
                    let content_start = start + escape_prefix.len();
                    if let Some(end) = val_part[content_start..].find(escape_prefix) {
                        val_part[content_start..content_start + end].to_string()
                    } else {
                        val_part[content_start..].to_string()
                    }
                } else {
                    val_part.to_string()
                }
            } else {
                val_part.trim().trim_matches('"').to_string()
            };

            arguments.insert(key.to_string(), coerce_value(&value_str));
        }
    }

    Some(ParsedToolCall {
        name,
        arguments: Value::Object(arguments),
    })
}

/// Coerce a stringified value into the most-specific JSON type.
/// FunctionGemma's text format wraps everything in `<escape>...<escape>`
/// so numbers and booleans arrive as strings — `tool_to_cmd` then asks
/// `.as_u64()` / `.as_bool()` and gets `None` unless we coerce here.
fn coerce_value(s: &str) -> Value {
    let trimmed = s.trim();
    // bool
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    // integer (i64 first so negatives work)
    if let Ok(n) = trimmed.parse::<i64>() {
        return Value::Number(n.into());
    }
    // float
    if let Ok(f) = trimmed.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Value::Number(num);
        }
    }
    // fall back to string (preserving the original, untrimmed if it had quotes)
    Value::String(s.to_string())
}

// ──────────────────────────────────────────────────────────────────────
// Prompt building
// ──────────────────────────────────────────────────────────────────────

fn build_functiongemma_prompt(messages: &[Message], tools: &[Tool]) -> String {
    let mut prompt = String::new();

    // Developer turn with tool declarations
    prompt.push_str("<start_of_turn>developer\n");
    prompt.push_str("You are a model that can do function calling with the following functions\n\n");

    for tool in tools {
        prompt.push_str(&format_tool_declaration(tool));
    }

    prompt.push_str("\n<end_of_turn>\n");

    // System prompt as developer context
    prompt.push_str("<start_of_turn>developer\n");
    prompt.push_str(SYSTEM_PROMPT);
    prompt.push_str("\n<end_of_turn>\n");

    // Process message history
    for msg in messages {
        match msg.role.as_str() {
            "user" => {
                prompt.push_str("<start_of_turn>user\n");
                if let Some(content) = &msg.content {
                    prompt.push_str(content);
                }
                prompt.push_str("\n<end_of_turn>\n");
            }
            "assistant" => {
                prompt.push_str("<start_of_turn>model\n");
                if let Some(content) = &msg.content {
                    prompt.push_str(content);
                }
                // Emit tool calls in FunctionGemma format
                for tc in &msg.tool_calls {
                    prompt.push_str(&format!(
                        "<start_function_call>call:{}{{{}}}<end_function_call>",
                        tc.name, tc.arguments
                    ));
                }
                prompt.push_str("\n<end_of_turn>\n");
            }
            "tool" => {
                if let Some(content) = &msg.content {
                    prompt.push_str(&format!(
                        "<start_function_response>response:{}{{{}}}<end_function_response>\n",
                        msg.tool_name.as_deref().unwrap_or("unknown"),
                        content
                    ));
                }
            }
            _ => {}
        }
    }

    // Prime the model for the next response
    prompt.push_str("<start_of_turn>model\n");

    prompt
}

fn format_tool_declaration(tool: &Tool) -> String {
    let mut decl = format!("<start_function_declaration>declaration:{}{{\n", tool.name);
    decl.push_str(&format!("description:<escape>{}<escape>", tool.description));

    if let Some(props) = tool.input_schema.get("properties") {
        if let Some(props_obj) = props.as_object() {
            if !props_obj.is_empty() {
                decl.push_str(",\nparameters:{");
                let mut first = true;
                for (param_name, param_schema) in props_obj {
                    if !first {
                        decl.push(',');
                    }
                    let param_type = param_schema
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("string");
                    decl.push_str(&format!(
                        "{param_name}:<escape>{}<escape>",
                        param_type
                    ));
                    first = false;
                }
                decl.push('}');
            }
        }
    }

    decl.push_str("\n}<end_function_declaration>\n");
    decl
}

// ──────────────────────────────────────────────────────────────────────
// Tool to command mapping
// ──────────────────────────────────────────────────────────────────────

/// Map a tool name + arguments to a griz `pb::command::Cmd` and synthesize
/// the tool response the model will see next turn. The third element is
/// `true` iff we actually mapped the tool to a command — when `false`, the
/// model should treat the call as a failure rather than retrying.
fn tool_to_cmd(name: &str, args: &Value) -> (Option<pb::command::Cmd>, Value, bool) {
    let cmd = match name {
        "load" => Some(pb::command::Cmd::Load(pb::Load {
            root: args
                .get("root")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })),
        "show" => Some(pb::command::Cmd::Show(pb::Show {
            result: args
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            component: args
                .get("component")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            opts: HashMap::new(),
        })),
        "set_state" => {
            let state = args
                .get("state")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u32;
            Some(pb::command::Cmd::SetState(pb::SetState { state }))
        }
        "step" => {
            let dir = match args
                .get("dir")
                .and_then(|v| v.as_str())
                .unwrap_or("next")
            {
                "prev" | "PREV" => pb::step::Dir::Prev as i32,
                "first" | "FIRST" => pb::step::Dir::First as i32,
                "last" | "LAST" => pb::step::Dir::Last as i32,
                _ => pb::step::Dir::Next as i32,
            };
            Some(pb::command::Cmd::Step(pb::Step { dir }))
        }
        "material" => Some(pb::command::Cmd::Material(pb::MaterialVisibility {
            enable: args.get("enable").and_then(|v| v.as_bool()).unwrap_or(true),
            class_name: args
                .get("class_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            material: args.get("material").and_then(|v| v.as_u64()).map(|m| m as u32),
        })),
        "select" => Some(pb::command::Cmd::Select(pb::Select {
            class_name: args
                .get("class_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            range: args
                .get("range")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })),
        "clrsel" => Some(pb::command::Cmd::Clrsel(pb::ClearSelection {
            class_name: args
                .get("class_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })),
        "colormap" => Some(pb::command::Cmd::Colormap(pb::Colormap {
            name: args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string(),
        })),
        "named_view" => {
            let op = match args
                .get("op")
                .and_then(|v| v.as_str())
                .unwrap_or("RESTORE")
            {
                "SAVE" => pb::named_view::Op::Save as i32,
                "LIST" => pb::named_view::Op::List as i32,
                _ => pb::named_view::Op::Restore as i32,
            };
            Some(pb::command::Cmd::NamedView(pb::NamedView {
                op,
                name: args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            }))
        }
        "close" => Some(pb::command::Cmd::Close(pb::Close {})),
        "griz_raw" => Some(pb::command::Cmd::Raw(
            args
                .get("line")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        )),
        // Unimplemented complex tools (view, iso, contour, legend, cutplane, query, snapshot)
        _ => None,
    };

    let tool_ok = cmd.is_some();

    let result = if !tool_ok {
        // Unknown/unmapped tool: tell the model so it can recover
        // (matches the Python harness's `unknown_tool` error_kind).
        json!({
            "ok": false,
            "error": format!("tool '{name}' is not implemented in this agent"),
            "error_kind": "unknown_tool"
        })
    } else {
        match name {
            "set_state" => {
                let state = args
                    .get("state")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                json!({
                    "ok": true,
                    "action_complete": true,
                    "state": state,
                    "requested_state": state
                })
            }
            "load" => json!({
                "ok": true,
                "action_complete": true,
                "num_states": 1,
                "classes": []
            }),
            _ => json!({
                "ok": true,
                "action_complete": true
            }),
        }
    };

    (cmd, result, tool_ok)
}

// ──────────────────────────────────────────────────────────────────────
// Tests — all pure functions; no llama-server required.
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── JSON tool-call parser ────────────────────────────────────────

    #[test]
    fn parses_a_basic_json_tool_call() {
        let text = r#"{"name": "load", "arguments": {"root": "cylinder"}}"#;
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "load");
        assert_eq!(calls[0].arguments["root"], "cylinder");
    }

    #[test]
    fn json_parser_ignores_objects_without_a_name() {
        // The parser walks every {...} block; non-tool-call JSON objects
        // should be silently dropped so the loop can treat the response
        // as final text.
        let text = r#"{"some": "other thing"}"#;
        assert!(parse_tool_calls(text).is_none());
    }

    #[test]
    fn json_parser_handles_multiple_calls() {
        let text = r#"
            {"name": "load", "arguments": {"root": "cylinder"}}
            {"name": "show", "arguments": {"result": "vx"}}
        "#;
        let calls = parse_tool_calls(text).expect("two calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "load");
        assert_eq!(calls[1].name, "show");
    }

    // ── FunctionGemma text-format parser (Bug #3 — type coercion) ────

    #[test]
    fn functiongemma_text_parser_coerces_int_args() {
        // Without coercion, `state` arrived as the string "5" and
        // `as_u64()` defaulted to 1 — set_state always jumped to state 1.
        let text = "<start_function_call>call:set_state{state:<escape>5<escape>}<end_function_call>";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls[0].name, "set_state");
        assert_eq!(calls[0].arguments["state"].as_u64(), Some(5));
    }

    #[test]
    fn functiongemma_text_parser_coerces_bool_args() {
        let text = "<start_function_call>call:material{enable:<escape>false<escape>,material:<escape>2<escape>}<end_function_call>";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls[0].arguments["enable"].as_bool(), Some(false));
        assert_eq!(calls[0].arguments["material"].as_u64(), Some(2));
    }

    #[test]
    fn functiongemma_text_parser_keeps_strings_as_strings() {
        let text = "<start_function_call>call:load{root:<escape>cylinder<escape>}<end_function_call>";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls[0].arguments["root"].as_str(), Some("cylinder"));
    }

    #[test]
    fn functiongemma_text_parser_strips_dotted_tool_names() {
        // Models sometimes emit `material.disable` — match Python's
        // behavior of taking the base name.
        let text = "<start_function_call>call:material.disable{material:<escape>2<escape>}<end_function_call>";
        let calls = parse_tool_calls(text).expect("one call");
        assert_eq!(calls[0].name, "material");
    }

    #[test]
    fn coerce_value_handles_all_three_scalar_types() {
        assert_eq!(coerce_value("true"), Value::Bool(true));
        assert_eq!(coerce_value("false"), Value::Bool(false));
        assert_eq!(coerce_value("42").as_u64(), Some(42));
        assert_eq!(coerce_value("-7").as_i64(), Some(-7));
        assert!(coerce_value("3.14").as_f64().unwrap().is_finite());
        assert_eq!(coerce_value("hello"), Value::String("hello".to_string()));
    }

    // ── tool_to_cmd mapping ──────────────────────────────────────────

    #[test]
    fn tool_to_cmd_maps_load() {
        let args = json!({"root": "cylinder"});
        let (cmd, _result, ok) = tool_to_cmd("load", &args);
        assert!(ok);
        assert!(matches!(cmd, Some(pb::command::Cmd::Load(pb::Load { ref root })) if root == "cylinder"));
    }

    #[test]
    fn tool_to_cmd_maps_set_state_with_integer() {
        let args = json!({"state": 5});
        let (cmd, result, ok) = tool_to_cmd("set_state", &args);
        assert!(ok);
        assert!(matches!(cmd, Some(pb::command::Cmd::SetState(pb::SetState { state: 5 }))));
        // Synthesized response should echo the requested state so the
        // model's "compare state vs requested_state" check passes.
        assert_eq!(result["state"], 5);
        assert_eq!(result["requested_state"], 5);
    }

    #[test]
    fn tool_to_cmd_maps_material_with_optional_id() {
        let args = json!({"enable": false, "material": 2});
        let (cmd, _result, ok) = tool_to_cmd("material", &args);
        assert!(ok);
        let Some(pb::command::Cmd::Material(m)) = cmd else { panic!("expected Material") };
        assert!(!m.enable);
        assert_eq!(m.material, Some(2));
    }

    #[test]
    fn tool_to_cmd_maps_step_dir_strings() {
        for (input, expected) in [
            ("next", pb::step::Dir::Next),
            ("prev", pb::step::Dir::Prev),
            ("first", pb::step::Dir::First),
            ("last", pb::step::Dir::Last),
        ] {
            let args = json!({"dir": input});
            let (cmd, _, _) = tool_to_cmd("step", &args);
            let Some(pb::command::Cmd::Step(s)) = cmd else { panic!("expected Step") };
            assert_eq!(s.dir, expected as i32, "dir={input}");
        }
    }

    // ── Bug #2 — unmapped tools return tool_ok=false ─────────────────

    #[test]
    fn unknown_tool_reports_failure_to_the_model() {
        let (cmd, result, ok) = tool_to_cmd("query", &json!({}));
        assert!(cmd.is_none(), "query is unmapped");
        assert!(!ok, "tool_ok must be false so caller emits ok=false");
        assert_eq!(result["ok"], false);
        assert_eq!(result["error_kind"], "unknown_tool");
        assert!(result["error"].as_str().unwrap().contains("query"));
    }

    // ── Bug #1 — prompt builder uses tool_name, not tool_use_id ──────

    #[test]
    fn prompt_builder_emits_tool_name_in_response_block() {
        let messages = vec![
            Message {
                role: "user".to_string(),
                content: Some("load cylinder".to_string()),
                tool_calls: vec![],
                tool_name: None,
            },
            Message {
                role: "assistant".to_string(),
                content: None,
                tool_calls: vec![ToolCall {
                    name: "load".to_string(),
                    arguments: r#"{"root":"cylinder"}"#.to_string(),
                }],
                tool_name: None,
            },
            Message {
                role: "tool".to_string(),
                content: Some(r#"{"ok":true}"#.to_string()),
                tool_calls: vec![],
                tool_name: Some("load".to_string()),
            },
        ];
        let prompt = build_functiongemma_prompt(&messages, &[]);
        // The response block must name the *tool*, not a call id.
        assert!(
            prompt.contains("response:load{"),
            "prompt should contain response:load{{ — got:\n{prompt}"
        );
        assert!(
            !prompt.contains("response:turn"),
            "prompt must not leak call ids into the response tag"
        );
    }

    #[test]
    fn prompt_builder_emits_assistant_tool_call_in_functiongemma_format() {
        let messages = vec![Message {
            role: "assistant".to_string(),
            content: None,
            tool_calls: vec![ToolCall {
                name: "set_state".to_string(),
                arguments: r#"{"state":5}"#.to_string(),
            }],
            tool_name: None,
        }];
        let prompt = build_functiongemma_prompt(&messages, &[]);
        assert!(prompt.contains("<start_function_call>call:set_state"));
        assert!(prompt.contains("<end_function_call>"));
    }

    // ── tools.json compiles in cleanly ────────────────────────────────

    #[test]
    fn embedded_tools_json_parses_into_a_nonempty_list() {
        // Guards against shipping a broken tools.json — include_str! only
        // verifies the file exists, not that it's valid JSON.
        let tools = parse_tools_json(TOOLS_JSON).expect("tools.json must parse");
        assert!(!tools.is_empty(), "expected at least one tool definition");
        // Sanity-check that the core griz tools we map are present.
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        for required in ["load", "show", "set_state", "material", "select"] {
            assert!(names.contains(&required), "missing tool: {required}");
        }
    }
}
