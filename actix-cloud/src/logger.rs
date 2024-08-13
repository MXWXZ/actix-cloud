use std::{
    fmt::Write as _,
    io::{self, stderr, stdout, Write},
    str::FromStr,
    sync::mpsc::{self, Sender},
    thread,
};

use crate::Result;
use chrono::{DateTime, Local, Utc};
use colored::{Color, Colorize as _};
use serde::Serialize;
use serde_json::{Map, Value};
use serde_with::{serde_as, DisplayFromStr};
use tracing::Level;

#[serde_as]
#[derive(Serialize, Debug)]
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
    tx: Sender<Map<String, Value>>,
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
    fn new(tx: Sender<Map<String, Value>>) -> impl Fn() -> Self {
        move || Self { tx: tx.clone() }
    }
}

pub struct Logger {
    tx: Sender<Map<String, Value>>,
}

impl Logger {
    /// Get logger sender.
    pub fn sender(&self) -> Sender<Map<String, Value>> {
        self.tx.clone()
    }

    /// Init tracing logger.
    /// A new subscriber will be registered.
    pub fn init(&self, builder: LoggerBuilder) {
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

pub struct LoggerBuilder {
    json: bool,
    level: Level,
    filename: bool,
    line_number: bool,
    filter: Option<Box<dyn Fn(&LogItem) -> bool + Send>>,
    transformer: Option<Box<dyn Fn(LogItem) -> LogItem + Send>>,
    json_writer: WriterFn,
    color_writer: WriterFn,
}

impl LoggerBuilder {
    /// Return colored string of `level`.
    ///
    /// - TRACE/DEBUG => Magenta
    /// - INFO => Green
    /// - WARN => Yellow
    /// - ERROR => Red
    fn fmt_level(level: &Level) -> String {
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
        writer.write_fmt(format_args!("{v}\n")).map_err(Into::into)
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
            buf += &format!(" {k}={v}").bright_black().to_string();
        }
        for (k, v) in item.span {
            buf += &format!(" {k}={v}").bright_black().to_string();
        }

        writer
            .write_fmt(format_args!("{buf}\n"))
            .map_err(Into::into)
    }

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
        }
    }

    pub fn json_writer(mut self, writer: WriterFn) -> Self {
        self.json_writer = writer;
        self
    }

    pub fn color_writer(mut self, writer: WriterFn) -> Self {
        self.color_writer = writer;
        self
    }

    pub fn json(mut self) -> Self {
        self.json = true;
        self
    }

    pub fn level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    pub fn filename(mut self) -> Self {
        self.filename = true;
        self
    }

    pub fn line_number(mut self) -> Self {
        self.line_number = true;
        self
    }

    pub fn filter<F>(mut self, filter: F) -> Self
    where
        F: Fn(&LogItem) -> bool + Send + 'static,
    {
        self.filter = Some(Box::new(filter));
        self
    }

    pub fn transformer<F>(mut self, transformer: F) -> Self
    where
        F: Fn(LogItem) -> LogItem + Send + 'static,
    {
        self.transformer = Some(Box::new(transformer));
        self
    }

    /// Start logger.
    /// This method will spawn a new thread to print the log.
    pub fn start(self) -> Logger {
        let (tx, rx) = mpsc::channel();
        tracing_subscriber::fmt()
            .with_max_level(self.level)
            .with_writer(LogSender::new(tx.clone()))
            .without_time()
            .with_file(self.filename)
            .with_line_number(self.line_number)
            .json()
            .init();

        thread::spawn(move || {
            let filter = self.filter;
            let transformer = self.transformer;
            let json_writer = self.json_writer;
            let color_writer = self.color_writer;
            while let Ok(v) = rx.recv() {
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

                if let Some(filter) = &filter {
                    if !filter(&item) {
                        continue;
                    }
                }
                if let Some(transformer) = &transformer {
                    item = transformer(item);
                }
                let writer: Box<dyn io::Write> = if item.level <= Level::WARN {
                    Box::new(stderr())
                } else {
                    Box::new(stdout())
                };
                if self.json {
                    let _ = json_writer(item, writer);
                } else {
                    let _ = color_writer(item, writer);
                }
            }
        });
        Logger { tx }
    }
}
