//! Provide logger feature.
//! The inner library uses [tracing](https://crates.io/crates/tracing).
//! See their documents for how to log in the program.
//!
//! This wrapper makes it thread safe, even for FFI libraries.
//! You can use it everywhere and freely.
use std::{
    fmt::{Display, Write as _},
    future::Future,
    io::{self, stderr, stdout, Write},
    panic::Location,
    pin::Pin,
    str::FromStr,
    sync::{mpsc, OnceLock},
    thread::{self, JoinHandle},
};

use crate::Result;
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

/// Extension trait for [`Result`] adding [`FatalExt::unwrap_fatal`] and
/// [`FatalExt::expect_fatal`].
///
/// Import this trait to use `.unwrap_fatal()` / `.expect_fatal()`.
pub trait FatalExt<T, E> {
    /// Return the contained value, or log the error's [`Display`] output via [`fatal!`]
    /// and exit with code 1. Like [`Result::unwrap`], the log records the caller's location.
    ///
    /// # Examples
    /// ```no_run
    /// use actix_cloud::logger::FatalExt;
    ///
    /// let home = std::env::var("HOME").unwrap_fatal();
    /// ```
    #[track_caller]
    fn unwrap_fatal(self) -> T;

    /// Return the contained value, or log `msg` together with the error's [`Display`]
    /// output via [`fatal!`] and exit with code 1. Like [`Result::expect`], the log
    /// records the caller's location.
    ///
    /// # Examples
    /// ```no_run
    /// use actix_cloud::logger::FatalExt;
    ///
    /// let home = std::env::var("HOME").expect_fatal("HOME is not set");
    /// ```
    #[track_caller]
    fn expect_fatal(self, msg: &str) -> T;
}

impl<T, E: Display> FatalExt<T, E> for Result<T, E> {
    #[track_caller]
    fn unwrap_fatal(self) -> T {
        match self {
            Ok(value) => value,
            Err(e) => {
                let loc = Location::caller();
                crate::fatal!(_filename = loc.file(), _line_number = loc.line(), "{e}")
            }
        }
    }

    #[track_caller]
    fn expect_fatal(self, msg: &str) -> T {
        match self {
            Ok(value) => value,
            Err(e) => {
                let loc = Location::caller();
                crate::fatal!(
                    _filename = loc.file(),
                    _line_number = loc.line(),
                    "{msg}: {e}"
                )
            }
        }
    }
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
        let mut filename = s
            .get("filename")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut line_number = s.get("line_number").and_then(Value::as_i64);
        let mut fields = Self::json_take_object(&mut s, "fields");
        // Reserved overrides, set e.g. by [`FatalExt`] to attribute the entry to its
        // real caller. Always removed so they never reach the writers.
        if let Some(v) = fields.remove("_filename").as_ref().and_then(Value::as_str) {
            filename = Some(v.to_owned());
        }
        if let Some(v) = fields.remove("_line_number").and_then(|v| v.as_i64()) {
            line_number = Some(v);
        }
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
pub type WriterFn = Box<dyn Fn(LogItem, Box<dyn Write>) -> Result<()> + Send>;
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

    fn default_json_writer(item: LogItem, mut writer: Box<dyn Write>) -> Result<()> {
        let v = serde_json::to_string(&item).unwrap_or_default();
        writer.write_fmt(format_args!("{v}\n"))?;
        writer.flush().map_err(Into::into)
    }

    fn default_color_writer(item: LogItem, mut writer: Box<dyn Write>) -> Result<()> {
        let mut buf = String::new();
        write!(
            buf,
            "{} {} {}",
            item.time.as_str().unwrap_or_default().bright_black(),
            Self::fmt_level(&item.level),
            item.target.bright_black()
        )?;
        if let (Some(filename), Some(line_number)) = (item.filename, item.line_number) {
            buf += &format!("({filename}:{line_number})")
                .bright_black()
                .to_string();
        }
        write!(buf, "{} {}", ":".bright_black(), item.message)?;
        for (k, v) in &item.fields {
            if !k.starts_with("log.") {
                buf += &format!(" field.{k}={v}").bright_black().to_string();
            }
        }
        for (k, v) in item.span {
            if !k.starts_with("http.") && !k.starts_with("otel.") && k != "name" {
                buf += &format!(" span.{k}={v}").bright_black().to_string();
            }
        }

        writer.write_fmt(format_args!("{buf}\n"))?;
        writer.flush().map_err(Into::into)
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
