//! `opera mcp`: a pure-Rust Model Context Protocol (MCP) server over standard input and output,
//! enabling AI agents (Claude Code, Claude Desktop, Cursor, VS Code) to search, visualize, and
//! analyze NASA OPERA satellite data via NASA CMR and `titiler-cmr`.
//!
//! Modeled after `terrio mcp` and `lidar mcp`:
//! - Every CLI subcommand (except `mcp` and `help`) is automatically exposed as an MCP tool whose
//!   arguments are its CLI `args` array and whose description is its full Clap `--help`.
//! - Each tool runs in an isolated child process with bounded output capture (stderr prioritized),
//!   session-wide concurrency limiting, cancellation via `notifications/cancelled`, and clean child
//!   teardown on client EOF.
//! - `preview` automatically attaches its rendered PNG as MCP `image` content when `-o` is omitted
//!   so multimodal agents can inspect surface water, disturbance, and SAR maps directly.
//! - Curated MCP `prompts` guide agents through flood mapping, two-date change detection,
//!   time-series analysis, and area exploration workflows.
//!
//! ## Security & Trust Boundary
//! `opera mcp` runs locally with the invoking user's filesystem and network privileges (matching
//! `terrio mcp` and `lidar mcp`). Each tool advertises MCP `annotations` (`readOnlyHint`,
//! `destructiveHint`, `idempotentHint`, `openWorldHint`) so the MCP host/client can enforce user
//! approval rules for file-writing or network-accessing calls. Custom `--endpoint` values are
//! validated to use `http://` or `https://` schemes.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

use crate::usage::{command_names, commands, with_clap_stack};

/// Protocol versions supported by this server, newest first.
const VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Maximum combined bytes of command output (stderr + stdout + notices) returned to the agent.
const MAX_OUTPUT: usize = 100_000;

/// Maximum PNG size in bytes attached as MCP image content for `preview` (8 MiB).
const MAX_IMAGE: u64 = 8 << 20;

/// Maximum concurrent tool calls running across the session.
const MAX_CONCURRENT_TOOLS: usize = 8;

/// Maximum total in-flight (running + queued) tool calls allowed before rejecting new requests.
const MAX_PENDING_TOOLS: usize = 64;

/// Instructions sent to the AI agent during MCP `initialize`.
const INSTRUCTIONS: &str = "opera-rs searches, visualizes, and analyzes NASA OPERA satellite data (DSWx-HLS, DSWx-S1, DIST-ALERT, DIST-ANN, RTC-S1, CSLC-S1, DISP-S1) via public NASA CMR and titiler-cmr with zero GDAL dependencies and no Earthdata login required.\n\
- Each tool runs one `opera` subcommand; pass `args` as an array of strings after the command name, e.g. `search` with [\"--product\", \"dswx-hls\", \"--place\", \"Tulare Lake, California\", \"--start\", \"2023-04-01\", \"--end\", \"2023-04-30\", \"--json\"].\n\
- Each tool's description is the subcommand's full `--help`. Pass `--json` for structured output.\n\
- Start with `products --json` to inspect supported OPERA collections, aliases, and band defaults, or `geocode` to resolve a place name to a WGS 84 bounding box.\n\
- For flood mapping, combine optical `dswx-hls` with cloud-penetrating radar `dswx-s1` (band `B01_WTR`). Use `--water-only` on `preview` so clouds and ocean masks stay transparent.\n\
- Use `change` for two-date before/after analysis (water area deltas in km^2 or SAR backscatter shifts), `timeseries` for multi-date trends, and `flood-extent` to vectorize observed DSWx surface water into GeoJSON or GeoPackage polygons (explicitly OPERA-observed, not human-QAed).\n\
- Use `flood-extent` with `-o FILE` (or `search` with `-o FILE` for large footprint results) when requesting `--json` so bulky GeoJSON polygons are written to disk rather than exceeding the 100 KB MCP response limit.\n\
- Use `preview` without `-o` to receive the rendered map PNG directly as inline image content (with CARTO Voyager basemap, scale bar, and legend) without leaving temporary files behind.\n\
- For Sentinel-1 CSLC and DISP-S1 workflows (opera-utils), use `burst-id` to parse/group burst IDs and dates, `frame` and `frames-intersect` to query the official `opera-adt/burst_db` v0.9.0 DISP-S1 frame database, `missing-data` to select optimal rectangular burst/date subsets, and `disp-search` to find DISP-S1 products by frame ID or AOI.\n\
- Use `prompts/list` for guided step-by-step OPERA analysis workflows.";

/// Commands that only read data and never write output files.
const READ_ONLY: &[&str] = &[
    "products",
    "geocode",
    "tilejson",
    "point",
    "stats",
    "inspect",
    "burst-id",
    "disp-search",
];

/// Commands that contact external network services (NASA CMR, titiler-cmr, Nominatim, GitHub burst_db).
const OPEN_WORLD: &[&str] = &[
    "geocode",
    "search",
    "tilejson",
    "point",
    "stats",
    "change",
    "timeseries",
    "flood-extent",
    "preview",
    "animate",
    "export-html",
    "frame",
    "frames-intersect",
    "disp-search",
];

type RpcError = (i64, String);

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

type ToolCall = (Value, String, Vec<OsString>);

struct ConcurrencyLimiter {
    active: Mutex<usize>,
    cv: Condvar,
    limit: usize,
}

struct ConcurrencyPermit {
    limiter: Arc<ConcurrencyLimiter>,
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        let mut active = self
            .limiter
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *active = active.saturating_sub(1);
        self.limiter.cv.notify_one();
    }
}

impl ConcurrencyLimiter {
    fn new(limit: usize) -> Self {
        Self {
            active: Mutex::new(0),
            cv: Condvar::new(),
            limit: limit.max(1),
        }
    }

    fn acquire(self: &Arc<Self>, cancel: &AtomicBool) -> Option<ConcurrencyPermit> {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            if *active < self.limit {
                *active += 1;
                return Some(ConcurrencyPermit {
                    limiter: self.clone(),
                });
            }
            let (guard, _) = self
                .cv
                .wait_timeout(active, Duration::from_millis(50))
                .unwrap_or_else(PoisonError::into_inner);
            active = guard;
        }
    }
}

/// Stateful JSON-RPC 2.0 MCP server for `opera-rs`.
pub struct Server {
    opera: Arc<Vec<OsString>>,
    send: Sender<String>,
    running: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    limiter: Arc<ConcurrencyLimiter>,
    workers: Vec<JoinHandle<()>>,
}

impl Server {
    /// Creates a server whose tool calls run `opera` and send JSON-RPC replies to `send`.
    pub fn new(opera: Vec<OsString>, send: Sender<String>) -> Self {
        Self {
            opera: Arc::new(opera),
            send,
            running: Arc::default(),
            limiter: Arc::new(ConcurrencyLimiter::new(MAX_CONCURRENT_TOOLS)),
            workers: Vec::new(),
        }
    }

    /// Waits for all in-flight tool calls to finish and emit their replies (called on stdin EOF).
    pub fn drain(mut self) {
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }

    /// Cancels all running tool processes immediately and joins worker threads.
    pub fn shutdown(mut self) {
        for cancel in self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
        {
            cancel.store(true, Ordering::Relaxed);
        }
        self.limiter.cv.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }

    /// Handles one line of JSON-RPC input.
    pub fn handle(&mut self, message: &str) -> Option<String> {
        let msg: Value = match serde_json::from_str(message) {
            Ok(v) => v,
            Err(e) => {
                return Some(
                    error(&Value::Null, PARSE_ERROR, &format!("not JSON: {e}")).to_string(),
                );
            }
        };
        let reply = match &msg {
            Value::Array(batch) if batch.is_empty() => {
                error(&Value::Null, INVALID_REQUEST, "empty batch")
            }
            Value::Array(batch) => {
                let (mut calls, mut ids) = (Vec::new(), HashSet::new());
                let replies: Vec<Value> = batch
                    .iter()
                    .filter_map(|m| self.handle_value(m, Some((&mut calls, &mut ids))))
                    .collect();
                if !calls.is_empty() {
                    self.spawn_batch(replies, calls);
                    return None;
                }
                if replies.is_empty() {
                    return None;
                }
                Value::Array(replies)
            }
            _ => self.handle_value(&msg, None)?,
        };
        Some(reply.to_string())
    }

    fn handle_value(
        &mut self,
        msg: &Value,
        calls: Option<(&mut Vec<ToolCall>, &mut HashSet<String>)>,
    ) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            return Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "expected a JSON-RPC object",
            ));
        };
        let raw_id = obj.get("id").cloned();
        let reply_id = raw_id
            .as_ref()
            .filter(|id| matches!(id, Value::String(_) | Value::Number(_)))
            .cloned()
            .unwrap_or(Value::Null);

        if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error(&reply_id, INVALID_REQUEST, "jsonrpc must be \"2.0\""));
        }

        let Some(method_val) = obj.get("method") else {
            if obj.contains_key("result") || obj.contains_key("error") {
                return None;
            }
            return Some(error(&reply_id, INVALID_REQUEST, "expected a method"));
        };
        let Some(method) = method_val.as_str() else {
            return Some(error(&reply_id, INVALID_REQUEST, "method must be a string"));
        };
        if method == "notifications/cancelled" {
            let request = obj
                .get("params")
                .and_then(|p| p.get("requestId"))
                .map(Value::to_string);
            if let Some(cancel) = request.and_then(|r| {
                self.running
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&r)
                    .cloned()
            }) {
                cancel.store(true, Ordering::Relaxed);
                self.limiter.cv.notify_all();
            }
        }

        let id = raw_id?;
        if !matches!(id, Value::String(_) | Value::Number(_)) {
            return Some(error(
                &Value::Null,
                INVALID_REQUEST,
                "id must be a string or a number",
            ));
        }
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        let ok_reply = |r: Value| json!({"jsonrpc": "2.0", "id": id, "result": r});
        if method == "tools/call" {
            return Some(match parse_tool_call(&params) {
                Err((code, message)) => error(&id, code, &message),
                Ok((name, args)) => {
                    let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
                    if running.contains_key(&id.to_string()) {
                        return Some(error(
                            &id,
                            INVALID_REQUEST,
                            "a request id already in use by a running call",
                        ));
                    }
                    let queued_in_batch = calls.as_ref().map_or(0, |(c, _)| c.len());
                    if running.len() + queued_in_batch >= MAX_PENDING_TOOLS {
                        return Some(error(
                            &id,
                            INTERNAL_ERROR,
                            "too many pending tool calls; retry after active calls complete",
                        ));
                    }
                    drop(running);
                    match calls {
                        Some((calls, ids)) => {
                            if !ids.insert(id.to_string()) {
                                return Some(error(
                                    &id,
                                    INVALID_REQUEST,
                                    "a request id repeated within a batch",
                                ));
                            }
                            calls.push((id, name, args));
                        }
                        None => self.spawn(id, name, args),
                    }
                    return None;
                }
            });
        }
        Some(match self.call(method, &params) {
            Ok(r) => ok_reply(r),
            Err((code, message)) => error(&id, code, &message),
        })
    }

    fn spawn(&mut self, id: Value, name: String, args: Vec<OsString>) {
        let run = self.start((id, name, args));
        let send = self.send.clone();
        self.workers.retain(|w| !w.is_finished());
        self.workers.push(std::thread::spawn(move || {
            if let Some(reply) = run() {
                let _ = send.send(reply.to_string());
            }
        }));
    }

    fn spawn_batch(&mut self, mut replies: Vec<Value>, calls: Vec<ToolCall>) {
        let runs: Vec<_> = calls.into_iter().map(|call| self.start(call)).collect();
        let send = self.send.clone();
        self.workers.retain(|w| !w.is_finished());
        self.workers.push(std::thread::spawn(move || {
            let width = runs.len().min(MAX_CONCURRENT_TOOLS);
            let queue = Arc::new(Mutex::new(runs.into_iter()));
            let workers: Vec<_> = (0..width)
                .map(|_| {
                    let queue = queue.clone();
                    std::thread::spawn(move || {
                        let mut out = Vec::new();
                        loop {
                            let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
                            let Some(run) = next else { break };
                            out.extend(run());
                        }
                        out
                    })
                })
                .collect();
            replies.extend(
                workers
                    .into_iter()
                    .flat_map(|w| w.join().unwrap_or_default()),
            );
            if !replies.is_empty() {
                let _ = send.send(Value::Array(replies).to_string());
            }
        }));
    }

    fn start(
        &mut self,
        (id, name, args): ToolCall,
    ) -> impl FnOnce() -> Option<Value> + Send + 'static + use<> {
        let key = id.to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let previous = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), cancel.clone());
        debug_assert!(previous.is_none(), "a running call's id was reused");
        let (opera, running, limiter) = (
            self.opera.clone(),
            self.running.clone(),
            self.limiter.clone(),
        );
        move || {
            let reply = match limiter.acquire(&cancel) {
                None => None,
                Some(_permit) => {
                    if cancel.load(Ordering::Relaxed) {
                        None
                    } else {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            call_tool(&opera, &name, args, &cancel)
                        }))
                        .map(|r| r.map(|r| json!({"jsonrpc": "2.0", "id": id, "result": r})))
                        .unwrap_or_else(|_| {
                            Some(error(
                                &id,
                                INTERNAL_ERROR,
                                "the tool call failed unexpectedly",
                            ))
                        })
                    }
                }
            };
            running
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
            reply
        }
    }

    fn call(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_list()})),
            "prompts/list" => Ok(json!({"prompts": prompt_list()})),
            "prompts/get" => get_prompt(params),
            _ => Err((METHOD_NOT_FOUND, format!("unknown method '{method}'"))),
        }
    }
}

pub(crate) fn parse_tool_call(params: &Value) -> Result<(String, Vec<OsString>), RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "name is required".to_string()))?;
    if !command_names().iter().any(|c| c == name) {
        return Err((INVALID_PARAMS, format!("unknown tool '{name}'")));
    }
    let args = match params.get("arguments").and_then(|a| a.get("args")) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(OsString::from))
            .collect::<Option<Vec<_>>>()
            .ok_or((
                INVALID_PARAMS,
                "args must be an array of strings".to_string(),
            ))?,
        Some(_) => return Err((INVALID_PARAMS, "args must be an array of strings".into())),
    };
    Ok((name.to_string(), args))
}

pub(crate) fn call_tool(
    opera: &[OsString],
    name: &str,
    args: Vec<OsString>,
    cancel: &AtomicBool,
) -> Option<Value> {
    if name == "preview" {
        return call_preview(opera, args, cancel);
    }
    call_command(opera, name, args, cancel)
}

fn call_preview(opera: &[OsString], mut args: Vec<OsString>, cancel: &AtomicBool) -> Option<Value> {
    let failed =
        |text: String| Some(json!({"content": [{"type": "text", "text": text}], "isError": true}));

    let mut explicit_output: Option<PathBuf> = None;
    let mut i = 0usize;
    while i < args.len() {
        let s = args[i].to_string_lossy();
        if s == "--" {
            break;
        }
        if s == "-o" || s == "--output" {
            if let Some(val) = args.get(i + 1) {
                explicit_output = Some(PathBuf::from(val));
            }
            i += 2;
            continue;
        }
        if let Some(val) = s.strip_prefix("--output=") {
            explicit_output = Some(PathBuf::from(val));
        } else if let Some(val) = s.strip_prefix("-o")
            && !val.is_empty()
        {
            explicit_output = Some(PathBuf::from(val));
        }
        i += 1;
    }

    let temp_output = match explicit_output {
        Some(p) if p.as_os_str() == "-" => {
            return failed(
                "preview cannot send PNG bytes as tool text: pass -o FILE, or omit -o to receive image content"
                    .into(),
            );
        }
        Some(_) => None,
        None => {
            let dir = match tempfile::Builder::new().prefix("opera-preview-").tempdir() {
                Ok(d) => d,
                Err(e) => return failed(format!("could not create temporary directory: {e}")),
            };
            let p = dir.path().join("preview.png");
            args.splice(0..0, [OsString::from("-o"), p.clone().into_os_string()]);
            Some((p, dir))
        }
    };

    let mut result = call_command(opera, "preview", args, cancel);
    if let (Some((path, _dir)), Some(res)) = (
        temp_output,
        result.as_mut().filter(|r| r["isError"] == false),
    ) {
        let png = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.is_file() && m.len() > 0 && m.len() <= MAX_IMAGE)
            .and_then(|_| std::fs::read(&path).ok());
        let content = res["content"]
            .as_array_mut()
            .expect("tool results always have content array");
        let temp_path_str = path.display().to_string();
        let temp_phrase = format!(" to {temp_path_str}");
        let replacement = if png.is_some() {
            " as inline image content (no file kept; pass -o FILE to save it)"
        } else {
            " to a temporary file that has been removed"
        };
        for item in content.iter_mut() {
            if let Some(text) = item["text"].as_str() {
                if let Ok(mut parsed) = serde_json::from_str::<Value>(text)
                    && parsed.get("output").and_then(Value::as_str) == Some(temp_path_str.as_str())
                {
                    parsed["output"] = Value::Null;
                    parsed["inline_image"] = json!(png.is_some());
                    if let Ok(pretty) = serde_json::to_string_pretty(&parsed) {
                        item["text"] = Value::String(pretty);
                    }
                } else if text.contains(&temp_phrase) {
                    item["text"] = Value::String(text.replace(&temp_phrase, replacement));
                }
            }
        }
        match png {
            Some(bytes) => {
                content.push(json!({
                    "type": "image",
                    "data": encode_base64(&bytes),
                    "mimeType": "image/png"
                }));
            }
            None => {
                content.push(json!({
                    "type": "text",
                    "text": format!(
                        "The preview PNG exceeded {} MiB or could not be read; pass a smaller --max-dim.",
                        MAX_IMAGE >> 20
                    )
                }));
            }
        }
    }
    result
}

fn has_json_flag(args: &[OsString]) -> bool {
    for arg in args {
        let s = arg.to_string_lossy();
        if s == "--" {
            break;
        }
        if s == "--json" {
            return true;
        }
    }
    false
}

fn format_command_output(
    name: &str,
    json_requested: bool,
    status: ExitStatus,
    stdout: BoundedOutput,
    stderr: BoundedOutput,
) -> (String, bool) {
    let failed = !status.success();
    let footer = if failed {
        let code = status
            .code()
            .map_or_else(|| "a signal".to_string(), |c| format!("exit code {c}"));
        format!("\n(opera {name} failed with {code})")
    } else {
        String::new()
    };
    let stream_budget = MAX_OUTPUT.saturating_sub(footer.len());
    let stderr_text = clip(
        String::from_utf8_lossy(&stderr.0).into_owned(),
        stderr.1,
        stream_budget,
    );
    let sep_len = usize::from(!stderr_text.is_empty());
    let stdout_budget = stream_budget
        .saturating_sub(stderr_text.len())
        .saturating_sub(sep_len);
    let stdout_truncated =
        !failed && json_requested && (stdout.1 > stdout_budget || stdout.0.len() > stdout_budget);
    if stdout_truncated {
        return (
            format!(
                "opera {name} --json produced {} bytes, exceeding the {} byte MCP response cap. \
                 Pass `-o FILE` to save vector/GeoJSON output to disk (keeping `--json` compact) \
                 or reduce `--limit` / `--max-observations`.",
                stdout.1.max(stdout.0.len()),
                MAX_OUTPUT
            ),
            true,
        );
    }
    let mut text = if stdout_budget > 0 {
        clip(
            String::from_utf8_lossy(&stdout.0).into_owned(),
            stdout.1,
            stdout_budget,
        )
    } else {
        String::new()
    };
    if !stderr_text.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&stderr_text);
    }
    if !footer.is_empty() {
        text.push_str(&footer);
    }
    if text.trim().is_empty() {
        text = format!("opera {name} finished.");
    }
    (text, failed)
}

fn call_command(
    opera: &[OsString],
    name: &str,
    args: Vec<OsString>,
    cancel: &AtomicBool,
) -> Option<Value> {
    let json_requested = has_json_flag(&args);
    let (text, failed) = match run_child(opera, name, args, cancel) {
        Ok(None) => return None,
        Ok(Some((status, stdout, stderr))) => {
            format_command_output(name, json_requested, status, stdout, stderr)
        }
        Err(e) => (format!("could not run opera: {e}"), true),
    };
    Some(json!({"content": [{"type": "text", "text": text}], "isError": failed}))
}

type BoundedOutput = (Vec<u8>, usize);

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn as_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("child still owned")
    }

    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run_child(
    opera: &[OsString],
    name: &str,
    args: Vec<OsString>,
    cancel: &AtomicBool,
) -> std::io::Result<Option<(ExitStatus, BoundedOutput, BoundedOutput)>> {
    let (program, before) = opera
        .split_first()
        .expect("opera command line must contain a program");
    let child = Command::new(program)
        .args(before)
        .arg(name)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut guard = ChildGuard(Some(child));
    let stdout = read_bounded(guard.as_mut().stdout.take().expect("piped stdout"))?;
    let stderr = read_bounded(guard.as_mut().stderr.take().expect("piped stderr"))?;
    let status = loop {
        if let Some(status) = guard.as_mut().try_wait()? {
            guard.disarm();
            break status;
        }
        if cancel.load(Ordering::Relaxed) {
            drop(guard);
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let joined = |h: JoinHandle<BoundedOutput>| h.join().unwrap_or_default();
    Ok(Some((status, joined(stdout), joined(stderr))))
}

fn read_bounded(mut r: impl Read + Send + 'static) -> std::io::Result<JoinHandle<BoundedOutput>> {
    std::thread::Builder::new().spawn(move || {
        let (mut kept, mut total, mut buf) = (Vec::new(), 0usize, [0u8; 8192]);
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let room = MAX_OUTPUT.saturating_sub(kept.len());
            kept.extend_from_slice(&buf[..n.min(room)]);
            total += n;
        }
        (kept, total)
    })
}

/// Serves MCP over stdin and stdout (one JSON-RPC message per line) until EOF.
pub fn serve(opera: Vec<OsString>) -> std::io::Result<()> {
    let (send, replies) = std::sync::mpsc::channel::<String>();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut out = std::io::stdout().lock();
        for reply in replies {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
        Ok(())
    });
    let mut server = Server::new(opera, send.clone());
    let read = (|| {
        for line in std::io::stdin().lock().lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = server.handle(&line) {
                let _ = send.send(reply);
            }
        }
        Ok(())
    })();
    if read.is_ok() {
        server.drain();
    } else {
        server.shutdown();
    }
    drop(send);
    writer.join().unwrap_or(Ok(())).and(read)
}

/// Returns the command line that re-invokes this `opera` binary.
pub fn this_binary() -> std::io::Result<Vec<OsString>> {
    Ok(vec![std::env::current_exe()?.into_os_string()])
}

fn initialize(params: &Value) -> Value {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = VERSIONS
        .iter()
        .find(|v| **v == asked)
        .copied()
        .unwrap_or(VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": {"listChanged": false},
            "prompts": {"listChanged": false}
        },
        "serverInfo": {
            "name": "opera-rs",
            "title": "opera-rs",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": INSTRUCTIONS,
    })
}

fn tool_list() -> Vec<Value> {
    static TOOLS: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    TOOLS
        .get_or_init(|| {
            with_clap_stack(|| {
                commands()
                    .map(|mut c| {
                        let name = c.get_name().to_string();
                        let ro = READ_ONLY.contains(&name.as_str());
                        let open_world = OPEN_WORLD.contains(&name.as_str());
                        let description =
                            c.render_long_help().to_string().trim_end().to_string();
                        json!({
                            "name": name,
                            "description": description,
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "args": {
                                        "type": "array",
                                        "items": {"type": "string"},
                                        "description": format!("The command line after `opera {name}`, one argument per item"),
                                    }
                                },
                                "required": ["args"],
                            },
                            "annotations": {
                                "readOnlyHint": ro,
                                "destructiveHint": !ro,
                                "idempotentHint": ro,
                                "openWorldHint": open_world,
                            },
                        })
                    })
                    .collect()
            })
        })
        .clone()
}

fn aoi_flag(place_or_bbox: &str) -> &'static str {
    let trimmed = place_or_bbox.trim();
    if trimmed.split(',').count() == 4
        && trimmed
            .split(',')
            .all(|part| part.trim().parse::<f64>().is_ok())
    {
        "--bbox"
    } else if std::path::Path::new(trimmed).extension().is_some()
        && (trimmed.ends_with(".geojson")
            || trimmed.ends_with(".json")
            || trimmed.ends_with(".gpkg")
            || trimmed.ends_with(".shp"))
    {
        "--aoi-file"
    } else {
        "--place"
    }
}

fn prompt_list() -> Vec<Value> {
    vec![
        json!({
            "name": "map-flood-event",
            "description": "Run an end-to-end NASA OPERA flood assessment combining optical DSWx-HLS and cloud-penetrating radar DSWx-S1, before/after water area deltas, observed flood-extent polygon vectorization, and inline visual preview.",
            "arguments": [
                {"name": "place_or_bbox", "description": "Place name (e.g. 'Valencia Region, Spain'), WGS 84 bbox 'w,s,e,n', or AOI vector file path", "required": true},
                {"name": "start", "description": "Event window start date (YYYY-MM-DD)", "required": true},
                {"name": "end", "description": "Event window end date (YYYY-MM-DD)", "required": true},
                {"name": "out_dir", "description": "Optional output directory for flood polygons and interactive HTML map", "required": false}
            ]
        }),
        json!({
            "name": "detect-surface-change",
            "description": "Compare two dates of an OPERA product (DSWx water, DIST-ALERT disturbance, or RTC-S1 backscatter) over an AOI with quantified deltas and visual inspection.",
            "arguments": [
                {"name": "product", "description": "OPERA product short name or alias (e.g. dswx-hls, dswx-s1, dist-alert, rtc-s1)", "required": true},
                {"name": "place_or_bbox", "description": "Place name, WGS 84 bbox 'w,s,e,n', or AOI vector file path", "required": true},
                {"name": "before_date", "description": "Baseline date (YYYY-MM-DD)", "required": true},
                {"name": "after_date", "description": "Comparison date (YYYY-MM-DD)", "required": true},
                {"name": "band", "description": "Optional band token (e.g. B01_WTR, VEG-DIST-STATUS, VV)", "required": false}
            ]
        }),
        json!({
            "name": "analyze-water-timeseries",
            "description": "Run a multi-date OPERA surface water or backscatter time-series trend analysis and optionally export an animated GIF.",
            "arguments": [
                {"name": "place_or_bbox", "description": "Place name, WGS 84 bbox 'w,s,e,n', or AOI vector file path", "required": true},
                {"name": "start", "description": "Inclusive start date (YYYY-MM-DD)", "required": true},
                {"name": "end", "description": "Inclusive end date (YYYY-MM-DD)", "required": true},
                {"name": "product", "description": "OPERA product (default: dswx-hls)", "required": false},
                {"name": "out_dir", "description": "Optional output directory for CSV and animated GIF", "required": false}
            ]
        }),
        json!({
            "name": "explore-opera-area",
            "description": "Discover available NASA OPERA granules, dates, bands, and inline visual previews for any place or bounding box.",
            "arguments": [
                {"name": "place_or_bbox", "description": "Place name, WGS 84 bbox 'w,s,e,n', or AOI vector file path", "required": true},
                {"name": "product", "description": "Optional product short name or alias (default: dswx-hls)", "required": false}
            ]
        }),
    ]
}

fn get_prompt(params: &Value) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "prompt name is required".to_string()))?;
    let args = params.get("arguments").and_then(Value::as_object);
    let req_arg = |key: &str| -> Result<String, RpcError> {
        args.and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .ok_or((
                INVALID_PARAMS,
                format!("missing required prompt argument '{key}'"),
            ))
    };
    let opt_arg = |key: &str, default: &str| -> String {
        args.and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(default)
            .to_string()
    };

    match name {
        "map-flood-event" => {
            let aoi = req_arg("place_or_bbox")?;
            let start = req_arg("start")?;
            let end = req_arg("end")?;
            let out = opt_arg("out_dir", "");
            let flag = aoi_flag(&aoi);
            let out_step = if out.is_empty() {
                "4. Run `flood-extent` to compute the observed surface-water polygon count and area in km^2 (note clearly that this extent is OPERA-observed, not human-QAed).".to_string()
            } else {
                format!(
                    "4. Run `flood-extent` with `[\"{flag}\", \"{aoi}\", \"--start\", \"{start}\", \"--end\", \"{end}\", \"-o\", \"{out}/flood_extent.geojson\", \"--json\"]` and `export-html` into `{out}/flood_map.html`."
                )
            };
            Ok(json!({
                "description": "End-to-end NASA OPERA flood mapping workflow using DSWx-HLS and DSWx-S1.",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": format!(
                            "Map and analyze flooding for `{aoi}` from `{start}` to `{end}` using NASA OPERA:\n\
                             1. Search both `dswx-hls` (optical) and `dswx-s1` (cloud-penetrating SAR) with `search` using `[\"--product\", \"dswx-s1\", \"{flag}\", \"{aoi}\", \"--start\", \"{start}\", \"--end\", \"{end}\", \"--json\"]` (and repeat for `dswx-hls`).\n\
                             2. Run `change` comparing a baseline date ~14-30 days before `{start}` against `{end}` with `--band B01_WTR --json` to quantify open water and total surface water expansion (km^2).\n\
                             3. Visually inspect the flood water with `preview` using `[\"--product\", \"dswx-s1\", \"{flag}\", \"{aoi}\", \"--start\", \"{start}\", \"--end\", \"{end}\", \"--band\", \"B01_WTR\", \"--water-only\"]` (omit `-o` to inspect inline).\n\
                             {out_step}\n\
                             5. Report acquisition dates, granule IDs, open water vs. partial surface water area (km^2), and any cloud-cover limitations."
                        )
                    }
                }]
            }))
        }
        "detect-surface-change" => {
            let product = req_arg("product")?;
            let aoi = req_arg("place_or_bbox")?;
            let before = req_arg("before_date")?;
            let after = req_arg("after_date")?;
            let band = opt_arg("band", "");
            let flag = aoi_flag(&aoi);
            let band_note = if band.is_empty() {
                String::new()
            } else {
                format!(" with `--band {band}`")
            };
            Ok(json!({
                "description": "Two-date OPERA surface change detection workflow.",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": format!(
                            "Detect `{product}` change over `{aoi}` between `{before}` and `{after}`{band_note}:\n\
                             1. Call `change` with `[\"--product\", \"{product}\", \"{flag}\", \"{aoi}\", \"--before\", \"{before}\", \"--after\", \"{after}\", \"--window-days\", \"10\", \"--json\"]`.\n\
                             2. Visually inspect the post-event scene with `preview` (`--product {product} {flag} \"{aoi}\" --start {after} --end {after}` or using the matched granule UR) without `-o`.\n\
                             3. Summarize matched granules, dates, class area deltas (km^2) or continuous band shifts, and data quality (valid pixel %)."
                        )
                    }
                }]
            }))
        }
        "analyze-water-timeseries" => {
            let aoi = req_arg("place_or_bbox")?;
            let start = req_arg("start")?;
            let end = req_arg("end")?;
            let product = opt_arg("product", "dswx-hls");
            let out = opt_arg("out_dir", "");
            let flag = aoi_flag(&aoi);
            let steps = if out.is_empty() {
                "3. Summarize min/max/mean values, net change, linear trend slope per day, and key anomaly dates.".to_string()
            } else {
                format!(
                    "3. Save CSV to `{out}/timeseries.csv` (`--csv-out`) and render an animated GIF with `animate` into `{out}/timeseries.gif`.\n\
                     4. Summarize min/max/mean values, net change, linear trend slope per day, and key anomaly dates."
                )
            };
            Ok(json!({
                "description": "Multi-date OPERA time-series trend analysis.",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": format!(
                            "Analyze `{product}` temporal trends over `{aoi}` from `{start}` to `{end}`:\n\
                             1. Run `timeseries` with `[\"--product\", \"{product}\", \"{flag}\", \"{aoi}\", \"--start\", \"{start}\", \"--end\", \"{end}\", \"--max-observations\", \"12\", \"--json\"]`.\n\
                             2. Preview the peak observation date inline with `preview` (omit `-o`).\n\
                             {steps}"
                        )
                    }
                }]
            }))
        }
        "explore-opera-area" => {
            let aoi = req_arg("place_or_bbox")?;
            let product = opt_arg("product", "dswx-hls");
            let flag = aoi_flag(&aoi);
            Ok(json!({
                "description": "Explore available NASA OPERA data over a region.",
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": format!(
                            "Explore NASA OPERA `{product}` coverage for `{aoi}`:\n\
                             1. If `{aoi}` is a place name, run `geocode` (`[\"{aoi}\", \"--json\"]`) to confirm its bounds.\n\
                             2. Run `search` with `[\"--product\", \"{product}\", \"{flag}\", \"{aoi}\", \"--limit\", \"15\", \"--json\"]` to list recent granules, dates, and available bands.\n\
                             3. Run `preview` on the newest granule (omit `-o`) and `stats` (`--json`) over the AOI, then summarize coverage and statistics."
                        )
                    }
                }]
            }))
        }
        _ => Err((INVALID_PARAMS, format!("unknown prompt '{name}'"))),
    }
}

/// Pure-Rust RFC 4648 base64 encoder.
pub(crate) fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn clip(mut text: String, total: usize, max: usize) -> String {
    if text.len() <= max && total <= text.len() {
        return text;
    }
    let full_total = total.max(text.len());
    let suffix_for = |kept: usize| format!("\n... (output cut at {kept} of {full_total} bytes)");
    let max_suffix_len = suffix_for(max).len();
    if max <= max_suffix_len {
        let mut end = max.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        return text;
    }
    let mut end = (max - max_suffix_len).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    let suffix = suffix_for(end);
    text.push_str(&suffix);
    text
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server::new(vec!["opera".into()], std::sync::mpsc::channel().0)
    }

    fn request(s: &mut Server, method: &str, params: Value) -> Value {
        let msg =
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
        serde_json::from_str(&s.handle(&msg).expect("a reply")).unwrap()
    }

    #[test]
    fn initializes_with_supported_version_and_capabilities() {
        let r = request(
            &mut server(),
            "initialize",
            json!({"protocolVersion": "2025-03-26"}),
        );
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "opera-rs");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert!(r["result"]["capabilities"]["prompts"].is_object());
        assert!(
            r["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("dswx-s1")
        );

        let fallback = request(
            &mut server(),
            "initialize",
            json!({"protocolVersion": "1999-01-01"}),
        );
        assert_eq!(fallback["result"]["protocolVersion"], VERSIONS[0]);
    }

    #[test]
    fn lists_cli_commands_with_accurate_annotations() {
        let r = request(&mut server(), "tools/list", json!({}));
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for expected in [
            "products",
            "geocode",
            "search",
            "tilejson",
            "point",
            "stats",
            "change",
            "timeseries",
            "flood-extent",
            "preview",
            "animate",
            "export-html",
            "inspect",
        ] {
            assert!(names.contains(&expected), "missing MCP tool '{expected}'");
        }
        assert!(!names.contains(&"mcp"));
        assert!(!names.contains(&"help"));

        let products = tools.iter().find(|t| t["name"] == "products").unwrap();
        assert_eq!(products["annotations"]["readOnlyHint"], true);
        assert_eq!(products["annotations"]["destructiveHint"], false);
        assert_eq!(products["annotations"]["openWorldHint"], false);

        let search = tools.iter().find(|t| t["name"] == "search").unwrap();
        assert_eq!(search["annotations"]["openWorldHint"], true);
    }

    #[test]
    fn lists_and_renders_workflow_prompts() {
        let mut s = server();
        let list = request(&mut s, "prompts/list", json!({}));
        let prompts = list["result"]["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 4);

        let rendered = request(
            &mut s,
            "prompts/get",
            json!({
                "name": "map-flood-event",
                "arguments": {
                    "place_or_bbox": "Valencia Region, Spain",
                    "start": "2024-10-27",
                    "end": "2024-11-05"
                }
            }),
        );
        let text = rendered["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("--place"));
        assert!(text.contains("dswx-s1"));
        assert!(text.contains("--water-only"));

        let bbox_prompt = request(
            &mut s,
            "prompts/get",
            json!({
                "name": "explore-opera-area",
                "arguments": {"place_or_bbox": "-119.9,35.8,-119.4,36.3"}
            }),
        );
        assert!(
            bbox_prompt["result"]["messages"][0]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("--bbox")
        );
    }

    #[test]
    fn encodes_base64_and_clips_output() {
        assert_eq!(encode_base64(b"opera-rs"), "b3BlcmEtcnM=");
        assert_eq!(encode_base64(b"a"), "YQ==");
        let clipped = clip("hello world".repeat(10), 110, 60);
        assert!(clipped.len() <= 60);
        assert!(clipped.contains("output cut"));
    }

    #[test]
    fn numbers_timeseries_prompt_steps_and_rejects_truncated_json() {
        let mut s = server();
        let without_out = request(
            &mut s,
            "prompts/get",
            json!({
                "name": "analyze-water-timeseries",
                "arguments": {
                    "place_or_bbox": "Tulare Lake, California",
                    "start": "2023-03-01",
                    "end": "2023-06-30"
                }
            }),
        );
        let text = without_out["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("3. Summarize"));
        assert!(!text.contains("4. Summarize"));

        let with_out = request(
            &mut s,
            "prompts/get",
            json!({
                "name": "analyze-water-timeseries",
                "arguments": {
                    "place_or_bbox": "Tulare Lake, California",
                    "start": "2023-03-01",
                    "end": "2023-06-30",
                    "out_dir": "results"
                }
            }),
        );
        let text_out = with_out["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text_out.contains("3. Save CSV"));
        assert!(text_out.contains("4. Summarize"));

        let ok_status = std::process::Command::new(if cfg!(windows) { "cmd" } else { "true" })
            .args(if cfg!(windows) {
                &["/C", "exit 0"][..]
            } else {
                &[][..]
            })
            .status()
            .unwrap();
        let oversized = vec![b'x'; MAX_OUTPUT];
        let (msg, failed) = format_command_output(
            "flood-extent",
            true,
            ok_status,
            (oversized, MAX_OUTPUT + 500),
            (Vec::new(), 0),
        );
        assert!(failed);
        assert!(msg.contains("-o FILE"));
    }
}
