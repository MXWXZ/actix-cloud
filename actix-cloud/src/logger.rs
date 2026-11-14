//! Provide logger feature.
//! The inner library uses [tracing](https://crates.io/crates/tracing).
//! See their documents for how to log in the program.
//!
//! This wrapper makes it thread safe, even for FFI libraries.
//! You can use it everywhere and freely.
use std::{
    borrow::Cow,
    future::Future,
    io::{self, stderr, stdout, Write},
    pin::Pin,
    str::FromStr,
    sync::{mpsc, OnceLock},
    thread::{self, JoinHandle},
};

use chrono::{DateTime, Local, Utc};
use colored::{Color, Colorize as _};
use futures::executor::block_on;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::{serde_as, DisplayFromStr};
use tokio::{
    select,
    sync::mpsc::{unbounded_channel, UnboundedSender},
};
use tracing::Level;

/// Log a message at `ERROR` level and terminate the process with exit code 1.
///
/// Takes the same arguments as [`tracing::error!`].
///
/// Before exiting, [`crate::logger::flush`] is called so that every log entry queued so
/// far (including this one) has been written out. Destructors will not run — in particular
/// [`LoggerGuard`] is **not** dropped — so only call this when the process cannot continue.
///
/// # Examples
/// ```no_run
/// use actix_cloud::fatal;
///
/// let path = "config.yml";
/// fatal!("config file not found: {}", path);
/// ```
#[macro_export]
macro_rules! fatal {
    ($($arg:tt)*) => {{
        $crate::tracing::error!($($arg)*);
        $crate::logger::flush();
        std::process::exit(1);
    }};
}

/// Unwrap a [`Result`](std::result::Result), or log the error's
/// [`Display`](std::fmt::Display) output via [`fatal!`] and exit with code 1.
///
/// Expands at the call site, so the log records the caller's module, file and line.
///
/// # Examples
/// ```no_run
/// use actix_cloud::unwrap_fatal;
///
/// let home = unwrap_fatal!(std::env::var("HOME"));
/// ```
#[macro_export]
macro_rules! unwrap_fatal {
    ($e:expr) => {
        match $e {
            ::core::result::Result::Ok(v) => v,
            ::core::result::Result::Err(e) => $crate::fatal!("{}", e),
        }
    };
}

/// Unwrap a [`Result`](std::result::Result), or log `msg` together with the error's
/// [`Display`](std::fmt::Display) output via [`fatal!`] and exit with code 1.
///
/// Expands at the call site, so the log records the caller's module, file and line.
///
/// # Examples
/// ```no_run
/// use actix_cloud::expect_fatal;
///
/// let home = expect_fatal!(std::env::var("HOME"), "HOME is not set");
/// ```
#[macro_export]
macro_rules! expect_fatal {
    ($e:expr, $msg:expr) => {
        match $e {
            ::core::result::Result::Ok(v) => v,
            ::core::result::Result::Err(e) => $crate::fatal!("{}: {}", $msg, e),
        }
    };
}

/// One structured log entry, handed to the writers (and `filter`/`transformer`).
#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub struct LogItem {
    /// Timestamp: microseconds since epoch (JSON output) or formatted string
    /// (color output). Overridable through the reserved `_time` field.
    pub time: Value,
    #[serde_as(as = "DisplayFromStr")]
    pub level: Level,
    pub message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// Extra key-value pairs attached to the event (`log.*` fields are internal).
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Value>,
    /// Key-value pairs of the current span (`http.*`/`otel.*`/`name` are internal).
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub span: Map<String, Value>,
    /// File name of the event. Overridable through the reserved `_filename` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Line number of the event. Overridable through the reserved `_line_number` field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_number: Option<i64>,
}

impl LogItem {
    fn json_take_object(mp: &mut Map<String, Value>, key: &str) -> Map<String, Value> {
        if let Value::Object(x) = mp.remove(key).unwrap_or_default() {
            x
        } else {
            Map::default()
        }
    }

    fn from_json(mut s: Map<String, Value>) -> Self {
        let target = s
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let level = Level::from_str(s.get("level").and_then(Value::as_str).unwrap_or("ERROR"))
            .unwrap_or(Level::ERROR);
        let filename = s
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string);
        let line_number = s.get("line_number").and_then(Value::as_i64);
        let mut fields = Self::json_take_object(&mut s, "fields");
        let message = fields
            .remove("message")
            .unwrap_or_default()
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let span = Self::json_take_object(&mut s, "span");
        Self {
            time: Value::default(),
            level,
            message,
            target,
            fields,
            span,
            filename,
            line_number,
        }
    }
}

struct LogSender {
    tx: UnboundedSender<Map<String, Value>>,
}

impl Write for LogSender {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx
            .send(serde_json::from_slice(buf)?)
            .or(Err(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // We do not buffer output.
        Ok(())
    }
}

impl LogSender {
    fn new(tx: UnboundedSender<Map<String, Value>>) -> impl Fn() -> Self {
        move || Self { tx: tx.clone() }
    }
}

/// Build and register the tracing subscriber that forwards JSON events into `tx`.
fn init_subscriber(
    level: Level,
    filename: bool,
    line_number: bool,
    tx: UnboundedSender<Map<String, Value>>,
) {
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(LogSender::new(tx))
        .without_time()
        .with_file(filename)
        .with_line_number(line_number)
        .json()
        .init();
}

/// Handle to the running logger; clone and pass around (e.g. store in `GlobalState`)
/// to re-init the subscriber or feed raw log maps from plugins.
#[derive(Clone)]
pub struct Logger {
    tx: UnboundedSender<Map<String, Value>>,
}

impl Logger {
    /// Get logger sender.
    pub fn sender(&self) -> UnboundedSender<Map<String, Value>> {
        self.tx.clone()
    }

    /// Init tracing logger.
    /// A new subscriber will be registered.
    pub fn init(&self, builder: &LoggerBuilder) {
        init_subscriber(
            builder.level,
            builder.filename,
            builder.line_number,
            self.tx.clone(),
        );
    }
}

/// Writer callback: render one [`LogItem`] to the given sink (stdout/stderr).
pub type WriterFn = Box<dyn Fn(LogItem, Box<dyn Write>) -> io::Result<()> + Send>;
/// Filter callback: return `false` to drop a log entry before the transformer.
pub type FilterFn = Box<dyn Fn(&LogItem) -> bool + Send>;
/// Transformer callback: rewrite a log entry before writing.
pub type TransformerFn = Box<dyn Fn(LogItem) -> LogItem + Send>;
/// Handler callback: invoked first on the raw JSON map; return `false` to stop
/// further processing of the entry.
pub type HandlerFn = Box<dyn Fn(&Map<String, Value>) -> Pin<Box<dyn Future<Output = bool>>> + Send>;

/// Keep this guard alive when you use the logger.
/// No more logs will be record after dropping the guard.
///
/// # Warning
/// When dropping the guard, it will wait for the logger thread to exit.
pub struct LoggerGuard {
    stop_tx: UnboundedSender<()>,
    join: Option<JoinHandle<()>>,
}

impl Drop for LoggerGuard {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(x) = self.join.take() {
            let _ = x.join();
        }
    }
}

/// Control channel sender to the logger thread started by [`LoggerBuilder::start`],
/// used by [`flush`]. A flush request carries the channel to ack on.
static FLUSH_TX: OnceLock<UnboundedSender<mpsc::Sender<()>>> = OnceLock::new();

/// Wait until every log entry queued before this call has been handled by the logger thread.
///
/// Sends a flush request over the control channel; on receipt the logger thread drains the
/// log channel and replies through the sender carried by the request, so by the time this
/// returns everything queued before the call — including anything the caller logged right
/// before — has been written out. Used by [`fatal!`], and usable before any other kind of
/// process exit.
///
/// Returns immediately if no logger has been started or if the logger thread is gone.
pub fn flush() {
    let Some(tx) = FLUSH_TX.get() else {
        return;
    };
    let (ack_tx, ack_rx) = mpsc::channel();
    if tx.send(ack_tx).is_err() {
        return;
    }
    // Blocks until the logger thread replies, or drops the ack sender on exit.
    let _ = ack_rx.recv();
}

/// Builder for the logger thread spawned by [`LoggerBuilder::start`].
///
/// Defaults to the colorful writer at `INFO` level without filename/line number;
/// see the individual setters for the customization points.
pub struct LoggerBuilder {
    json: bool,
    level: Level,
    filename: bool,
    line_number: bool,
    filter: Option<FilterFn>,
    transformer: Option<TransformerFn>,
    json_writer: WriterFn,
    color_writer: WriterFn,
    handler: Option<HandlerFn>,
}

impl Default for LoggerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Escape control characters in untrusted text for terminal output.
///
/// Printable text, including non-ASCII characters, is kept as-is. JSON escaping
/// libraries stop at C0 controls, but terminals in UTF-8 mode also interpret C1
/// controls (e.g. `U+009B` as CSI), so every `char::is_control` character is escaped.
///
/// ```
/// use actix_cloud::logger::escape_control;
///
/// assert_eq!(escape_control("a\nb"), "a\\nb");
/// assert_eq!(escape_control("plain 文本"), "plain 文本");
/// assert_eq!(escape_control("\u{9b}[31mred"), "\\u{9b}[31mred");
/// ```
pub fn escape_control(s: &str) -> Cow<'_, str> {
    if !s.chars().any(char::is_control) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

impl LoggerBuilder {
    /// Return colored string of `level`.
    ///
    /// - TRACE/DEBUG => Magenta
    /// - INFO => Green
    /// - WARN => Yellow
    /// - ERROR => Red
    pub fn fmt_level(level: &Level) -> String {
        format!("{: >5}", level.to_string())
            .bold()
            .color(match *level {
                Level::TRACE | Level::DEBUG => Color::Magenta,
                Level::INFO => Color::Green,
                Level::WARN => Color::Yellow,
                Level::ERROR => Color::Red,
            })
            .to_string()
    }

    fn default_json_writer(item: LogItem, mut writer: Box<dyn Write>) -> io::Result<()> {
        let v = serde_json::to_string(&item).unwrap_or_default();
        writer.write_fmt(format_args!("{v}\n"))?;
        writer.flush()
    }

    fn default_color_writer(item: LogItem, mut writer: Box<dyn Write>) -> io::Result<()> {
        let mut buf = String::new();
        buf += &format!(
            "{} {} {}",
            item.time.as_str().unwrap_or_default().bright_black(),
            Self::fmt_level(&item.level),
            item.target.bright_black()
        );
        if let (Some(filename), Some(line_number)) = (item.filename, item.line_number) {
            buf += &format!("({filename}:{line_number})")
                .bright_black()
                .to_string();
        }
        // The message and field/span values can carry untrusted data. Targets, file
        // names and field names come from tracing metadata (module paths and
        // identifiers).
        buf += &format!("{} {}", ":".bright_black(), escape_control(&item.message));
        for (k, v) in &item.fields {
            if !k.starts_with("log.") {
                buf += &format!(" field.{k}={}", escape_control(&v.to_string()))
                    .bright_black()
                    .to_string();
            }
        }
        for (k, v) in item.span {
            if (k == "http.method" || k == "http.target")
                || (!k.starts_with("http.") && !k.starts_with("otel.") && k != "name")
            {
                buf += &format!(" span.{k}={}", escape_control(&v.to_string()))
                    .bright_black()
                    .to_string();
            }
        }

        writer.write_fmt(format_args!("{buf}\n"))?;
        writer.flush()
    }

    /// Create new logger instance.
    /// Default is colorful writer, INFO level, no filename and line number.
    pub fn new() -> Self {
        Self {
            json: false,
            level: Level::INFO,
            filename: false,
            line_number: false,
            filter: None,
            transformer: None,
            json_writer: Box::new(Self::default_json_writer),
            color_writer: Box::new(Self::default_color_writer),
            handler: None,
        }
    }

    /// Use custom json writer.
    ///
    /// # Warning
    /// Do not perform heavy workloads, it can block other logs!
    pub fn json_writer(mut self, writer: WriterFn) -> Self {
        self.json_writer = writer;
        self
    }

    /// Use custom colorful writer.
    ///
    /// # Warning
    /// Do not perform heavy workloads, it can block other logs!
    pub fn color_writer(mut self, writer: WriterFn) -> Self {
        self.color_writer = writer;
        self
    }

    /// Use json format writer.
    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    /// Set log level.
    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Enable filename in the log.
    pub fn filename(mut self) -> Self {
        self.filename = true;
        self
    }

    /// Enable line number in the log.
    pub fn line_number(mut self) -> Self {
        self.line_number = true;
        self
    }

    /// Customize the handler.
    ///
    /// The customized handler will be invoked first, even before the filter.
    /// When the return value is false, further handler will be skipped.
    /// Otherwise, normal log handler will still be invoked.
    ///
    /// # Warning
    /// Do not perform heavy workloads, it can block other logs!
    pub fn handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(&Map<String, Value>) -> Pin<Box<dyn Future<Output = bool>>> + Send + 'static,
    {
        self.handler = Some(Box::new(handler));
        self
    }

    /// Customize the filter. Filter out unwanted logs.
    ///
    /// When the filter function return false, no logs will be sent to the transformer.
    ///
    /// # Warning
    /// Do not perform heavy workloads, it can block other logs!
    pub fn filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&LogItem) -> bool + Send + 'static,
    {
        self.filter = Some(Box::new(filter));
        self
    }

    /// Customize the transformer. Change the logs on the fly.
    ///
    /// After this function, LogItem will be sent to the corresponding writer.
    ///
    /// # Warning
    /// Do not perform heavy workloads, it can block other logs!
    pub fn transformer<F>(mut self, transformer: F) -> Self
    where
        F: Fn(LogItem) -> LogItem + Send + 'static,
    {
        self.transformer = Some(Box::new(transformer));
        self
    }

    /// Start logger.
    /// This method will spawn a new thread to print the log.
    ///
    /// You should call this method only once for the entire program.
    /// For FFI library, you need to call this method once in the library code and keep the return values alive.
    /// Then customize the [Self::handler] and send output back to the main program.
    pub fn start(self) -> (Logger, LoggerGuard) {
        let (tx, mut rx) = unbounded_channel();
        let (stop_tx, mut stop_rx) = unbounded_channel();
        init_subscriber(self.level, self.filename, self.line_number, tx.clone());
        let (flush_tx, mut flush_rx) = unbounded_channel::<mpsc::Sender<()>>();
        let _ = FLUSH_TX.set(flush_tx);

        let join = thread::spawn(move || {
            let handler = |v: Map<String, Value>| async {
                if let Some(x) = &self.handler {
                    if !x(&v).await {
                        return;
                    }
                }
                let mut item = LogItem::from_json(v);
                let time = item.fields.remove("_time").unwrap_or_default().as_i64();
                if self.json {
                    item.time = time.unwrap_or_else(|| Utc::now().timestamp_micros()).into();
                } else {
                    item.time = time
                        .map_or_else(Local::now, |v| {
                            DateTime::from_timestamp_micros(v)
                                .unwrap_or_default()
                                .into()
                        })
                        .format("%F %T%.6f")
                        .to_string()
                        .into();
                }

                // Reserved location overrides, only applied when the builder option
                // is enabled. Always removed so they never reach the writers.
                if let Some(v) = item.fields.remove("_filename") {
                    if self.filename {
                        item.filename = v.as_str().map(str::to_owned);
                    }
                }
                if let Some(v) = item.fields.remove("_line_number") {
                    if self.line_number {
                        item.line_number = v.as_i64();
                    }
                }

                if let Some(filter) = &self.filter {
                    if !filter(&item) {
                        return;
                    }
                }
                if let Some(transformer) = &self.transformer {
                    item = transformer(item);
                }
                let writer: Box<dyn io::Write> = if item.level <= Level::WARN {
                    Box::new(stderr())
                } else {
                    Box::new(stdout())
                };
                if self.json {
                    let _ = (self.json_writer)(item, writer);
                } else {
                    let _ = (self.color_writer)(item, writer);
                }
            };
            block_on(async move {
                loop {
                    select! {
                        Some(v) = rx.recv() => {
                            handler(v).await;
                        },
                        Some(ack) = flush_rx.recv() => {
                            while let Ok(v) = rx.try_recv(){
                                handler(v).await;
                            }
                            let _ = ack.send(());
                        },
                        _ = stop_rx.recv() => {
                            while let Ok(v) = rx.try_recv(){
                                handler(v).await;
                            }
                            break;
                        }
                    }
                }
            })
        });
        (
            Logger { tx },
            LoggerGuard {
                stop_tx,
                join: Some(join),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_item_is_parsed_from_json() {
        let map = serde_json::json!({
            "target": "my::target",
            "level": "WARN",
            "filename": "src/main.rs",
            "line_number": 42,
            "fields": {
                "message": "hello",
                "extra": 1
            },
            "span": {"name": "span"}
        })
        .as_object()
        .unwrap()
        .clone();

        let item = LogItem::from_json(map);
        assert_eq!(item.level, Level::WARN);
        assert_eq!(item.message, "hello");
        assert_eq!(item.target, "my::target");
        assert_eq!(item.filename.as_deref(), Some("src/main.rs"));
        assert_eq!(item.line_number, Some(42));
        assert_eq!(item.fields.get("extra"), Some(&serde_json::json!(1)));
        assert!(!item.fields.contains_key("message"));
        assert_eq!(item.span.get("name"), Some(&serde_json::json!("span")));
    }

    #[test]
    fn log_item_defaults_missing_fields() {
        let item = LogItem::from_json(Map::new());

        assert_eq!(item.level, Level::ERROR);
        assert_eq!(item.message, "");
        assert_eq!(item.target, "");
        assert!(item.filename.is_none());
        assert!(item.line_number.is_none());
        assert!(item.fields.is_empty());
        assert!(item.span.is_empty());
    }

    #[test]
    fn fmt_level_pads_level_names() {
        assert!(LoggerBuilder::fmt_level(&Level::INFO).contains("INFO"));
        assert!(LoggerBuilder::fmt_level(&Level::TRACE).contains("TRACE"));
        assert!(LoggerBuilder::fmt_level(&Level::WARN).contains("WARN"));
        assert!(LoggerBuilder::fmt_level(&Level::ERROR).contains("ERROR"));
    }

    #[test]
    fn builder_defaults_and_setters() {
        let b = LoggerBuilder::new();
        assert!(!b.json);
        assert_eq!(b.level, Level::INFO);
        assert!(!b.filename);
        assert!(!b.line_number);
        assert!(b.filter.is_none());
        assert!(b.transformer.is_none());
        assert!(b.handler.is_none());

        let b = LoggerBuilder::new()
            .json()
            .level(Level::DEBUG)
            .filename()
            .line_number();
        assert!(b.json);
        assert!(b.filename);
        assert!(b.line_number);
        assert_eq!(b.level, Level::DEBUG);
    }

    #[test]
    fn log_sender_forwards_json_maps() {
        let (tx, mut rx) = unbounded_channel();
        let mut writer = LogSender::new(tx)();

        let n = io::Write::write(&mut writer, br#"{"level":"INFO"}"#).unwrap();
        assert_eq!(n, 16);
        assert!(rx.try_recv().is_ok());

        // Non-JSON input is reported as a write error.
        assert!(io::Write::write(&mut writer, b"not json").is_err());
        assert!(io::Write::flush(&mut writer).is_ok());
    }

    #[test]
    fn log_item_handles_malformed_fields() {
        let map = serde_json::json!({
            "target": 1,
            "level": "NOT_A_LEVEL",
            "filename": 2,
            "line_number": "not a number",
            "fields": {"message": 3},
            "span": []
        })
        .as_object()
        .unwrap()
        .clone();

        let item = LogItem::from_json(map);
        assert_eq!(item.level, Level::ERROR);
        assert_eq!(item.target, "");
        assert_eq!(item.message, "");
        assert!(item.filename.is_none());
        assert!(item.line_number.is_none());
        assert!(item.fields.is_empty());
        assert!(item.span.is_empty());
    }

    fn item() -> LogItem {
        let mut fields = Map::new();
        fields.insert("visible".to_owned(), serde_json::json!(1));
        fields.insert("log.internal".to_owned(), serde_json::json!("hidden"));

        let mut span = Map::new();
        span.insert("http.method".to_owned(), serde_json::json!("GET"));
        span.insert("http.target".to_owned(), serde_json::json!("/"));
        span.insert("http.status_code".to_owned(), serde_json::json!(200));
        span.insert("otel.trace_id".to_owned(), serde_json::json!("id"));
        span.insert("name".to_owned(), serde_json::json!("span"));
        span.insert("custom".to_owned(), serde_json::json!("v"));

        LogItem {
            time: serde_json::json!("2020-01-01 00:00:00.000000"),
            level: Level::INFO,
            message: "hello".to_owned(),
            target: "target".to_owned(),
            fields,
            span,
            filename: Some("src/main.rs".to_owned()),
            line_number: Some(42),
        }
    }

    /// `Box<dyn Write>` requires a `'static` writer, so share the buffer instead of
    /// borrowing it.
    #[derive(Clone, Default)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedBuf {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    #[test]
    fn default_json_writer_emits_one_json_line() {
        let buf = SharedBuf::default();
        LoggerBuilder::default_json_writer(item(), Box::new(buf.clone())).unwrap();

        let text = buf.text();
        assert!(text.ends_with('\n'));
        let value: Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["level"], "INFO");
        assert_eq!(value["message"], "hello");
        assert_eq!(value["target"], "target");
        assert_eq!(value["filename"], "src/main.rs");
        assert_eq!(value["line_number"], 42);
        assert_eq!(value["fields"]["visible"], 1);
        assert_eq!(value["span"]["http.method"], "GET");
    }

    #[test]
    fn json_writer_escapes_log_injection_attempts() {
        let mut item = item();
        item.message = "ok\nforged: entry".to_owned();
        item.fields
            .insert("user".to_owned(), serde_json::json!("a\r\nforged"));
        item.fields
            .insert("ansi".to_owned(), serde_json::json!("\u{1b}[31mred"));

        let buf = SharedBuf::default();
        LoggerBuilder::default_json_writer(item, Box::new(buf.clone())).unwrap();

        let text = buf.text();
        // Only the writer's own trailing newline may appear outside the JSON string.
        assert_eq!(text.matches('\n').count(), 1);
        assert!(!text.contains('\r'));
        assert!(!text.contains('\u{1b}'));

        let value: Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(value["message"], "ok\nforged: entry");
    }

    #[test]
    fn default_color_writer_filters_internal_fields() {
        let buf = SharedBuf::default();
        LoggerBuilder::default_color_writer(item(), Box::new(buf.clone())).unwrap();

        let text = buf.text();
        assert!(text.contains("INFO"));
        assert!(text.contains("target"));
        assert!(text.contains("src/main.rs:42"));
        assert!(text.contains("hello"));
        assert!(text.contains("field.visible=1"));
        assert!(!text.contains("log.internal"));
        assert!(text.contains("span.http.method=\"GET\""));
        assert!(text.contains("span.http.target=\"/\""));
        assert!(text.contains("span.custom=\"v\""));
        assert!(!text.contains("http.status_code"));
        assert!(!text.contains("otel.trace_id"));
        assert!(!text.contains("span.name="));
    }

    #[test]
    fn default_color_writer_escapes_control_characters() {
        let mut item = item();
        item.message = "ok\nforged: entry\u{1b}[31mred\u{9b}[32mgreen".to_owned();
        item.fields
            .insert("user".to_owned(), serde_json::json!("x\n\u{1b}\u{9b}y"));
        item.span
            .insert("custom".to_owned(), serde_json::json!("z\u{9b}"));

        let buf = SharedBuf::default();
        LoggerBuilder::default_color_writer(item, Box::new(buf.clone())).unwrap();

        let text = buf.text();
        // Only the writer's own trailing newline remains.
        assert_eq!(text.matches('\n').count(), 1);
        assert!(!text.contains('\r'));
        // Injected C0 and C1 escape sequences are rendered literally.
        assert!(!text.contains("\u{1b}[31m"));
        assert!(!text.contains('\u{9b}'), "raw C1 leaked: {text}");
        assert!(text.contains("ok\\nforged: entry\\u{1b}[31mred\\u{9b}[32mgreen"));
        // Field and span values are escaped as well (serde_json already escapes C0
        // inside a JSON string; the raw C1 must be handled here).
        assert!(
            text.contains("field.user=\"x\\n\\u001b\\u{9b}y\""),
            "{text}"
        );
        assert!(text.contains("span.custom=\"z\\u{9b}\""), "{text}");
    }

    #[test]
    fn escape_control_keeps_printable_text() {
        assert_eq!(escape_control("plain 文本"), "plain 文本");
        assert_eq!(escape_control("emoji 🦀"), "emoji 🦀");
        assert_eq!(escape_control("a\tb"), "a\\tb");
        assert_eq!(escape_control("a\u{7f}b"), "a\\u{7f}b");
        // U+00A0 is printable, not a control character.
        assert_eq!(escape_control("\u{a0}"), "\u{a0}");
    }

    #[test]
    fn escape_control_escapes_c1_controls() {
        // C1 controls are encoded as `0xC2 0x80..=0x9F`; terminals in UTF-8 mode
        // may interpret U+009B as an 8-bit CSI.
        assert_eq!(escape_control("\u{9b}"), "\\u{9b}");
        assert_eq!(escape_control("a\u{9b}[31mb"), "a\\u{9b}[31mb");
        assert_eq!(escape_control("\u{80}"), "\\u{80}");
        assert_eq!(escape_control("\u{9f}"), "\\u{9f}");
    }

    #[test]
    fn escape_control_escapes_every_control_character() {
        for c in (0..=0x10FFFF).filter_map(char::from_u32) {
            if !c.is_control() {
                continue;
            }
            let s = c.to_string();
            let escaped = escape_control(&s);
            assert!(
                !escaped.chars().any(char::is_control),
                "U+{:04X} was not escaped",
                c as u32
            );
        }
    }

    #[test]
    fn fatal_macros_pass_through_ok_values() {
        assert_eq!(crate::unwrap_fatal!(Ok::<_, io::Error>(3)), 3);
        assert_eq!(crate::expect_fatal!(Ok::<_, io::Error>(4), "message"), 4);
    }

    #[test]
    fn logger_pipeline_applies_all_stages_and_reserved_fields() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        };

        let captured: Arc<Mutex<Vec<LogItem>>> = Arc::new(Mutex::new(Vec::new()));
        let handler_seen = Arc::new(AtomicBool::new(false));
        let captured_in_writer = captured.clone();
        let handler_seen_clone = handler_seen.clone();

        let builder = LoggerBuilder::new()
            .level(Level::TRACE)
            .filename()
            .line_number()
            .handler(move |map| {
                handler_seen_clone.store(true, Ordering::SeqCst);
                let drop_it = map
                    .get("fields")
                    .and_then(|fields| fields.get("message"))
                    .and_then(Value::as_str)
                    == Some("handler-drop");
                let fut: Pin<Box<dyn Future<Output = bool>>> = Box::pin(async move { !drop_it });
                fut
            })
            .filter(|item| item.message != "filter-drop")
            .transformer(|mut item| {
                item.message.push('!');
                item
            })
            .color_writer(Box::new(move |item, _| {
                captured_in_writer.lock().unwrap().push(item);
                Ok(())
            }));

        let (logger, guard) = builder.start();

        // Raw plugin-style map with reserved time/location overrides.
        logger
            .sender()
            .send(
                serde_json::json!({
                    "level": "INFO",
                    "target": "plugin",
                    "fields": {
                        "message": "raw",
                        "_time": 1_000_000,
                        "_filename": "virtual.rs",
                        "_line_number": 7,
                        "extra": 1
                    },
                    "span": {"http.method": "GET"}
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();

        tracing::info!("live");
        tracing::info!("filter-drop");
        logger
            .sender()
            .send(
                serde_json::json!({
                    "level": "INFO",
                    "fields": {"message": "handler-drop"}
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .unwrap();

        crate::logger::flush();
        drop(guard);

        assert!(handler_seen.load(Ordering::SeqCst));
        let items = captured.lock().unwrap();

        let raw = items
            .iter()
            .find(|item| item.message.starts_with("raw"))
            .expect("raw item");
        assert_eq!(raw.message, "raw!");
        assert_eq!(raw.target, "plugin");
        assert_eq!(raw.filename.as_deref(), Some("virtual.rs"));
        assert_eq!(raw.line_number, Some(7));
        assert!(raw.time.as_str().unwrap().ends_with(".000000"));
        assert_eq!(raw.fields.get("extra"), Some(&serde_json::json!(1)));
        assert_eq!(raw.span.get("http.method"), Some(&serde_json::json!("GET")));

        assert!(items.iter().any(|item| item.message == "live!"));
        assert!(!items
            .iter()
            .any(|item| item.message.contains("filter-drop")));
        assert!(!items
            .iter()
            .any(|item| item.message.contains("handler-drop")));
    }
}
