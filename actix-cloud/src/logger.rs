//! Provide logger feature.
//! The inner library uses [tracing](https://crates.io/crates/tracing).
//! See their documents for how to log in the program.
//!
//! This wrapper makes it thread safe, even for FFI libraries.
//! You can use it everywhere and freely.
use std::{
    fmt::Write as _,
    future::Future,
    io::{self, stderr, stdout, Write},
    pin::Pin,
    str::FromStr,
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

#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
pub struct LogItem {
    pub time: Value,
    #[serde_as(as = "DisplayFromStr")]
    pub level: Level,
    pub message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub target: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Value>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub span: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
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
        tracing_subscriber::fmt()
            .with_max_level(builder.level)
            .with_writer(LogSender::new(self.tx.clone()))
            .without_time()
            .with_file(builder.filename)
            .with_line_number(builder.line_number)
            .json()
            .init();
    }
}

pub type WriterFn = Box<dyn Fn(LogItem, Box<dyn Write>) -> Result<()> + Send>;
pub type FilterFn = Box<dyn Fn(&LogItem) -> bool + Send>;
pub type TransformerFn = Box<dyn Fn(LogItem) -> LogItem + Send>;
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
        self.stop_tx.send(()).unwrap();
        if let Some(x) = self.join.take() {
            x.join().unwrap();
        }
    }
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
        if let Some(filename) = item.filename {
            if let Some(line_number) = item.line_number {
                buf += &format!("({}:{})", filename, line_number)
                    .bright_black()
                    .to_string();
            }
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
    /// Otherwise, normal log hander will still be invoked.
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
        tracing_subscriber::fmt()
            .with_max_level(self.level)
            .with_writer(LogSender::new(tx.clone()))
            .without_time()
            .with_file(self.filename)
            .with_line_number(self.line_number)
            .json()
            .init();

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
