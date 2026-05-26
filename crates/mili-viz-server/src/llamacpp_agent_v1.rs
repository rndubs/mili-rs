//! FunctionGemma **v1 SFT** agent via llama-server (M6 integration).
//!
//! M6 deltas vs the M4 [`crate::llamacpp_agent`] module:
//! - Talks to `/v1/chat/completions` with `--jinja`, not `/completion` with a
//!   bespoke prompt renderer. The GGUF's baked-in jinja produces the exact
//!   prompt distribution the v1 SFT trained against.
//! - System prompt is read from
//!   `data/posttraining/grammar/system_prompt.txt`, which the Python bench
//!   driver also points at — single source of truth, sha256 prefix
//!   `9f36d0deb5e98a89`.
//! - Parser tries (1) the OpenAI-shape `message.tool_calls` field (no-op on
//!   FG GGUFs where `supports_tool_calls=false`), (2) free-floating
//!   `{"name":…,"arguments":…}` JSON in `message.content` (v1's instructed
//!   shape), (3) `<start_function_call>call:NAME{…}<end_function_call>`
//!   envelopes — JSON-literal body first, then `<escape>` / bare-scalar
//!   fallback.
//! - tool→Cmd mapping + dispatch-outcome shaping are ported verbatim from
//!   the M4 module so the dispatcher contract is unchanged.

use std::collections::HashMap;
use std::pin::Pin;

use mili_viz_proto::v1 as pb;
use serde_json::{json, Value};

use crate::agent::{ran_summary, AgentBackend, AgentTurnCtx, DispatchOutcome};

const DEFAULT_SERVER_URL: &str = "http://localhost:8080";
const MAX_STEPS: usize = 4;

const TOOLS_JSON: &str = include_str!("../../../data/posttraining/grammar/tools.json");
const SYSTEM_PROMPT: &str = include_str!("../../../data/posttraining/grammar/system_prompt.txt");

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

            let tools = match parse_tools_json(TOOLS_JSON) {
                Ok(t) => t,
                Err(e) => {
                    ctx.emit_token(format!("Error loading tools: {e}"));
                    return;
                }
            };
            let openai_tools: Vec<Value> = tools.iter().map(convert_to_openai_tool).collect();

            // OpenAI-shape message history matching the Python bench's
            // `harness.run_turn` (driver.py:247, harness.py:253/511).
            // *The role is `"developer"`, not `"system"`*: the FG jinja
            // template's first turn is `<start_of_turn>developer`, and
            // the v1 SFT model was trained against this exact prompt
            // shape. Sending `"system"` drops the bench-pinned prompt
            // from the rendered prefix and the model loses grounding.
            let mut messages: Vec<Value> = vec![
                json!({"role": "developer", "content": SYSTEM_PROMPT}),
                json!({"role": "user", "content": ctx.request.text.clone()}),
            ];

            let mut signature_window: Vec<String> = Vec::new();
            const SIGNATURE_WINDOW_SIZE: usize = 4;

            for step in 0..MAX_STEPS {
                if ctx.cancelled() {
                    return;
                }

                let response =
                    match post_chat_completion(&self.server_url, &messages, &openai_tools).await {
                        Ok(r) => r,
                        Err(e) => {
                            ctx.emit_token(format!("Error: {e}"));
                            return;
                        }
                    };

                eprintln!(
                    "[llamacpp_agent_v1 turn={} step={}] raw content ({} bytes):\n{}\n[/raw]",
                    ctx.turn_id,
                    step,
                    response.content.len(),
                    response.content,
                );

                if let Some(tool_calls) = parse_tool_calls(&response) {
                    eprintln!(
                        "[llamacpp_agent_v1 turn={} step={}] parsed {} call(s): {}",
                        ctx.turn_id,
                        step,
                        tool_calls.len(),
                        tool_calls
                            .iter()
                            .map(|c| format!("{}({})", c.name, c.arguments))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    let signature = call_signature(&tool_calls);
                    if signature_window.contains(&signature) {
                        ctx.emit_token("(stopped: detected repeated tool call pattern)");
                        return;
                    }
                    signature_window.push(signature);
                    if signature_window.len() > SIGNATURE_WINDOW_SIZE {
                        signature_window.remove(0);
                    }

                    ctx.emit_status(pb::AgentStatusKind::AgentRunning, "");

                    // Build the OpenAI-shape assistant `tool_calls` list
                    // and matching `tool` response messages, mirroring
                    // harness.py:242–261 and harness.py:488–512. The
                    // bench appends ONE assistant message carrying the
                    // structured tool_calls list (no `content`) followed
                    // by one `tool` message per call (with matching
                    // `tool_call_id`). This is what the FG jinja
                    // template renders as `<start_function_call>` /
                    // `<start_function_response>` blocks — sending the
                    // raw model text as `content` instead drifts away
                    // from the trained prompt distribution.
                    let mut assistant_tool_calls: Vec<Value> = Vec::with_capacity(tool_calls.len());
                    let mut tool_messages: Vec<Value> = Vec::with_capacity(tool_calls.len());

                    for (i, tc) in tool_calls.iter().enumerate() {
                        let call_id = format!("{}-step{}-call-{}", ctx.turn_id, step, i);
                        ctx.emit_tool_begin(&call_id, ran_summary(&tc.name), "");

                        let cmd = tool_to_cmd(&tc.name, &tc.arguments);
                        let cmd_known = cmd.is_some();

                        let (seq, result_json) = if let Some(c) = cmd {
                            let outcome = ctx.dispatch(c);
                            let response = outcome_to_response(&tc.name, &tc.arguments, &outcome);
                            (outcome.seq, response)
                        } else {
                            (
                                0,
                                json!({
                                    "ok": false,
                                    "error": format!("tool '{}' is not implemented in this agent", tc.name),
                                    "error_kind": "unknown_tool"
                                }),
                            )
                        };

                        let tool_ok = result_json
                            .get("ok")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let result_summary = if tool_ok {
                            "ok"
                        } else if cmd_known {
                            "failed"
                        } else {
                            "tool not implemented"
                        };
                        ctx.emit_tool_end(&call_id, tool_ok, result_summary, seq);

                        let args_str = serde_json::to_string(&tc.arguments)
                            .unwrap_or_else(|_| "{}".to_string());
                        assistant_tool_calls.push(json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": tc.name, "arguments": args_str},
                        }));
                        tool_messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "name": tc.name,
                            "content": result_json.to_string(),
                        }));
                    }

                    messages.push(json!({
                        "role": "assistant",
                        "tool_calls": assistant_tool_calls,
                    }));
                    messages.extend(tool_messages);
                } else if !response.content.is_empty() {
                    for word in response.content.split_whitespace() {
                        if ctx.cancelled() {
                            return;
                        }
                        ctx.emit_token(word);
                        ctx.emit_token(" ");
                    }
                    return;
                } else {
                    ctx.emit_token("(no response from model)");
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
// Types
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ChatResponse {
    content: String,
    tool_calls: Option<Vec<ParsedToolCall>>,
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
// HTTP
// ──────────────────────────────────────────────────────────────────────

async fn post_chat_completion(
    server_url: &str,
    messages: &[Value],
    tools: &[Value],
) -> Result<ChatResponse, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/chat/completions", server_url);

    let mut payload = json!({
        "messages": messages,
        "temperature": 0.0,
        "max_tokens": 256,
        "seed": 0,
    });
    if !tools.is_empty() {
        payload["tools"] = json!(tools);
    }

    let resp = client
        .post(&url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    let message = body
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|choice| choice.get("message"))
        .ok_or_else(|| format!("No message in response: {body}"))?;

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();

    let tool_calls = message
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .and_then(|arr| {
            let calls: Vec<ParsedToolCall> = arr
                .iter()
                .filter_map(|tc| {
                    let function = tc.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let args_val = function.get("arguments")?;
                    let arguments = match args_val {
                        Value::String(s) => serde_json::from_str(s).ok()?,
                        _ => args_val.clone(),
                    };
                    Some(ParsedToolCall { name, arguments })
                })
                .collect();
            if calls.is_empty() {
                None
            } else {
                Some(calls)
            }
        });

    Ok(ChatResponse {
        content,
        tool_calls,
    })
}

// ──────────────────────────────────────────────────────────────────────
// Tool parsing
// ──────────────────────────────────────────────────────────────────────

fn parse_tool_calls(response: &ChatResponse) -> Option<Vec<ParsedToolCall>> {
    if let Some(calls) = response.tool_calls.clone() {
        return Some(calls);
    }
    if let Some(calls) = parse_fg_envelopes(&response.content) {
        return Some(calls);
    }
    parse_json_tool_calls(&response.content)
}

/// Rev-21 FG envelope parser: try JSON-literal body first, fall through to
/// `<escape>` / bare-scalar form. v1 SFT emits the JSON-literal shape; the
/// fallback covers stock FG-270M emissions for backwards-compat with the
/// M4 fixture set.
fn parse_fg_envelopes(text: &str) -> Option<Vec<ParsedToolCall>> {
    let mut calls = vec![];
    let pattern_start = "<start_function_call>";
    let pattern_end = "<end_function_call>";

    let mut remaining = text;
    while let Some(start_pos) = remaining.find(pattern_start) {
        remaining = &remaining[start_pos + pattern_start.len()..];
        let Some(end_pos) = remaining.find(pattern_end) else {
            break;
        };
        let content = remaining[..end_pos].trim();
        remaining = &remaining[end_pos + pattern_end.len()..];
        if let Some(call) = parse_fg_envelope_content(content) {
            calls.push(call);
        }
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

fn parse_fg_envelope_content(content: &str) -> Option<ParsedToolCall> {
    let call_prefix = "call:";
    let remaining = content.strip_prefix(call_prefix)?;
    let brace_pos = remaining.find('{')?;
    let name = remaining[..brace_pos]
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();

    let after_brace = &remaining[brace_pos + 1..];
    let closing_brace = after_brace.rfind('}')?;
    let args_str = after_brace[..closing_brace].trim();

    // JSON-literal body first (v1 SFT shape — Stage 5 rollouts wrote
    // function.arguments as a JSON string, so the FG chat template's
    // string-arguments branch inserts the literal `{"k": "v"}` inside
    // the call braces). The trimmed body IS the complete JSON object.
    if let Ok(value) = serde_json::from_str::<Value>(args_str) {
        if value.is_object() {
            return Some(ParsedToolCall {
                name,
                arguments: value,
            });
        }
    }

    // Fall back to <escape> / bare-scalar (stock FG shape).
    let mut arguments = serde_json::Map::new();
    for part in args_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(colon_pos) = part.find(':') {
            let key = part[..colon_pos].trim();
            let val_part = &part[colon_pos + 1..];
            let value_str = if val_part.contains("<escape>") {
                extract_escaped_value(val_part)
            } else {
                val_part.trim().trim_matches('"').to_string()
            };
            arguments.insert(key.to_string(), coerce_value(&value_str));
        }
    }

    if arguments.is_empty() {
        None
    } else {
        Some(ParsedToolCall {
            name,
            arguments: Value::Object(arguments),
        })
    }
}

fn extract_escaped_value(val_part: &str) -> String {
    let escape_tag = "<escape>";
    let Some(start) = val_part.find(escape_tag) else {
        return val_part.trim().to_string();
    };
    let content_start = start + escape_tag.len();
    if let Some(end) = val_part[content_start..].find(escape_tag) {
        val_part[content_start..content_start + end].to_string()
    } else {
        val_part[content_start..].to_string()
    }
}

/// Free-floating `{"name":…,"arguments":…}` JSON in the response body —
/// the shape the bench-pinned system prompt instructs the model to emit
/// when the GGUF has no native tool-call serialization.
fn parse_json_tool_calls(text: &str) -> Option<Vec<ParsedToolCall>> {
    let mut calls = vec![];
    let mut depth: i32 = 0;
    let mut start_idx: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start_idx = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
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
    let obj: Value =
        serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {e}"))?;
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

fn coerce_value(s: &str) -> Value {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(n) = trimmed.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Value::Number(num);
        }
    }
    Value::String(s.to_string())
}

// ──────────────────────────────────────────────────────────────────────
// Tools.json → OpenAI shape
// ──────────────────────────────────────────────────────────────────────

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
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        tools.push(Tool {
            name,
            description,
            input_schema,
        });
    }
    Ok(tools)
}

fn convert_to_openai_tool(tool: &Tool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

// ──────────────────────────────────────────────────────────────────────
// Cycle detection
// ──────────────────────────────────────────────────────────────────────

fn call_signature(calls: &[ParsedToolCall]) -> String {
    let mut buf = String::new();
    for c in calls {
        buf.push_str(&c.name);
        buf.push(';');
    }
    buf
}

// ──────────────────────────────────────────────────────────────────────
// Tool → griz Cmd mapping (ported from M4 llamacpp_agent)
// ──────────────────────────────────────────────────────────────────────

fn tool_to_cmd(name: &str, args: &Value) -> Option<pb::command::Cmd> {
    match name {
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
            let state = args.get("state").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            Some(pb::command::Cmd::SetState(pb::SetState { state }))
        }
        "step" => {
            let dir = match args.get("dir").and_then(|v| v.as_str()).unwrap_or("next") {
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
            material: args
                .get("material")
                .and_then(|v| v.as_u64())
                .map(|m| m as u32),
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
            let op = match args.get("op").and_then(|v| v.as_str()).unwrap_or("RESTORE") {
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
            args.get("line")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        )),
        _ => None,
    }
}

fn outcome_to_response(name: &str, args: &Value, outcome: &DispatchOutcome) -> Value {
    use pb::state_delta::Payload;
    match &outcome.payload {
        Payload::Loaded(loaded) => {
            let ok = !loaded.db.is_empty() || loaded.num_states > 0;
            if ok {
                json!({
                    "ok": true,
                    "action_complete": true,
                    "num_states": loaded.num_states,
                    "classes": loaded.class_names,
                })
            } else {
                json!({
                    "ok": false,
                    "error": "load failed: no database opened",
                    "error_kind": "dispatch_error"
                })
            }
        }
        Payload::Result(r) => {
            if r.geometry.is_some() {
                json!({
                    "ok": true,
                    "action_complete": true,
                    "result": r.result,
                    "component": r.component,
                    "range": [r.min, r.max],
                })
            } else {
                let requested = args
                    .get("result")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<unknown>");
                json!({
                    "ok": false,
                    "error": format!("'{requested}' is not a recognized result on this database"),
                    "error_kind": "nonexistent_result"
                })
            }
        }
        Payload::State(state) => {
            let requested = args.get("state").and_then(|v| v.as_u64()).map(|v| v as u32);
            match (name, requested) {
                ("set_state", Some(req)) => json!({
                    "ok": *state == req,
                    "action_complete": *state == req,
                    "state": state,
                    "requested_state": req,
                }),
                _ => json!({
                    "ok": true,
                    "action_complete": true,
                    "state": state,
                }),
            }
        }
        Payload::Selection(sel) => json!({
            "ok": true,
            "action_complete": true,
            "selection": sel.by_class,
        }),
        Payload::Materials(m) => {
            let hidden: Vec<u32> = m
                .visible
                .iter()
                .filter_map(|(k, v)| if !v { Some(*k) } else { None })
                .collect();
            json!({
                "ok": true,
                "action_complete": true,
                "hidden_materials": hidden,
            })
        }
        Payload::Camera(_)
        | Payload::Isosurface(_)
        | Payload::Snapshot(_)
        | Payload::Closed(_)
        | Payload::Agent(_) => json!({
            "ok": true,
            "action_complete": true,
        }),
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_byte_length_matches_bench_pin() {
        // The bench-pinned prompt is sha256[:16] = 9f36d0deb5e98a89 and
        // exactly 2044 bytes. Any change here means the prompt drifted
        // away from the shared `data/posttraining/grammar/system_prompt.txt`
        // — re-verify with the Python driver and update both ends.
        assert_eq!(SYSTEM_PROMPT.len(), 2044);
    }

    #[test]
    fn parses_fg_envelope_json_literal_body() {
        // v1 SFT shape — JSON dict inside the envelope (the trimmed body
        // is a complete JSON object, optionally prefixed by whitespace
        // from the chat-template's string-arguments branch).
        let text = r#"<start_function_call>call:show{                    {"result": "vx"}}<end_function_call>"#;
        let calls = parse_fg_envelopes(text).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "show");
        assert_eq!(calls[0].arguments["result"], "vx");
    }

    #[test]
    fn parses_multiple_v1_envelopes() {
        let text = r#"<start_function_call>call:load{ {"root": "cylinder"}}<end_function_call><start_function_call>call:show{ {"result": "vx"}}<end_function_call>"#;
        let calls = parse_fg_envelopes(text).expect("two calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "load");
        assert_eq!(calls[0].arguments["root"], "cylinder");
        assert_eq!(calls[1].name, "show");
        assert_eq!(calls[1].arguments["result"], "vx");
    }

    #[test]
    fn parses_fg_envelope_escape_body() {
        // Stock FG fallback shape — preserved for compat.
        let text =
            "<start_function_call>call:set_state{state:<escape>5<escape>}<end_function_call>";
        let calls = parse_fg_envelopes(text).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "set_state");
        assert_eq!(calls[0].arguments["state"], 5);
    }

    #[test]
    fn parses_freestanding_json_tool_call() {
        // What the bench-pinned prompt instructs the model to emit when
        // the GGUF has no tool_calls support.
        let text = r#"{"name": "load", "arguments": {"root": "cylinder"}}"#;
        let resp = ChatResponse {
            content: text.to_string(),
            tool_calls: None,
        };
        let calls = parse_tool_calls(&resp).expect("one call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "load");
        assert_eq!(calls[0].arguments["root"], "cylinder");
    }
}
