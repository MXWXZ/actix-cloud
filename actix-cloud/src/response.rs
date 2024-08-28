use std::fmt::{self, Display};

use actix_web::{
    http::{
        header::{ContentDisposition, DispositionParam, DispositionType},
        StatusCode,
    },
    HttpResponse, HttpResponseBuilder,
};
use futures::{future, stream::once};

pub type RspResult<T> = Result<T, ResponseError>;

#[derive(Debug)]
pub struct ResponseError(anyhow::Error);

impl Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_string())
    }
}

impl actix_web::ResponseError for ResponseError {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code()).finish()
    }
}

impl<T> From<T> for ResponseError
where
    T: Into<anyhow::Error>,
{
    fn from(t: T) -> Self {
        Self(t.into())
    }
}

pub trait ResponseCodeTrait {
    fn code(&self) -> i64;
    fn message(&self) -> &'static str;
}

#[cfg(feature = "response-build")]
#[derive(thiserror::Error, Debug)]
pub enum BuildError {
    #[error("response file format invalid")]
    Format,

    #[error("response file name invalid")]
    File,
}

#[cfg(feature = "response-build")]
/// Generate response file from yml.
///
/// This function should be used in `build.rs`.
/// ```ignore
/// [build-dependencies]
/// actix-cloud = { version = "xx", features = ["response-build"] }
/// ```
///
/// ```no_run
/// use actix_cloud::response::generate_response;
///
/// generate_response("response", "response.rs").unwrap();
/// ```
pub fn generate_response(input: &str, output: &str) -> anyhow::Result<()> {
    use std::io::Write;

    let outfile = std::path::Path::new(&std::env::var("OUT_DIR")?).join(output);
    let mut output = std::fs::File::create(&outfile)?;
    writeln!(output, "use actix_cloud::response::ResponseCodeTrait;")?;
    for entry in walkdir::WalkDir::new(input) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let file = std::fs::read_to_string(entry.path())?;
            let yaml = yaml_rust2::YamlLoader::load_from_str(&file)?;
            let doc = &yaml[0];
            let mut name_vec = Vec::new();
            let mut code_vec = Vec::new();
            let mut message_vec = Vec::new();
            for (name, field) in doc.as_hash().ok_or(BuildError::Format)? {
                name_vec.push(quote::format_ident!(
                    "{}",
                    name.as_str().ok_or(BuildError::Format)?
                ));
                code_vec.push(field["code"].as_i64().ok_or(BuildError::Format)?);
                message_vec.push(field["message"].as_str().ok_or(BuildError::Format)?);
            }

            let file_stem = entry.path().file_stem().ok_or(BuildError::File)?;
            let mut s = file_stem.to_str().ok_or(BuildError::File)?.to_owned();
            let s = s.remove(0).to_uppercase().to_string() + &s;
            let enum_name = quote::format_ident!("{}Response", s);
            let mut enum_code = Vec::new();
            for i in 0..code_vec.len() {
                let s = &name_vec[i];
                let c = code_vec[i];
                enum_code.push(quote::quote! {#enum_name::#s => #c});
            }
            let mut enum_message = Vec::new();
            for i in 0..code_vec.len() {
                let s = &name_vec[i];
                let c = message_vec[i];
                enum_message.push(quote::quote! {#enum_name::#s => #c});
            }
            let content = quote::quote! {
                pub enum #enum_name {
                    #(#name_vec),*
                }

                impl ResponseCodeTrait for #enum_name {
                    fn code(&self) -> i64 {
                        match self {
                            #(#enum_code),*
                        }
                    }

                    fn message(&self) -> &'static str {
                        match self {
                            #(#enum_message),*
                        }
                    }
                }
            };

            write!(
                output,
                "{}",
                prettyplease::unparse(&syn::parse_file(&content.to_string())?)
            )?;
        }
    }
    Ok(())
}

pub type ResponseBuilderFn = Box<dyn Fn(&mut HttpResponseBuilder)>;

pub struct Response<T> {
    pub http_code: u16,
    pub code: i64,
    pub message: String,
    pub data: Option<T>,
    pub builder: Option<ResponseBuilderFn>,
    #[cfg(feature = "i18n")]
    pub translate: bool,
}

impl<T> Response<T> {
    pub fn new<C>(r: C) -> Self
    where
        C: ResponseCodeTrait,
    {
        Self {
            http_code: 200,
            code: r.code(),
            message: r.message().to_owned(),
            data: None,
            builder: None,
            #[cfg(feature = "i18n")]
            translate: true,
        }
    }

    pub fn new_code(code: u16) -> Self {
        Self {
            http_code: code,
            code: 0,
            message: String::new(),
            data: None,
            builder: None,
            #[cfg(feature = "i18n")]
            translate: false,
        }
    }

    pub fn bad_request<S: Into<String>>(s: S) -> Self {
        Self::new_code(400).message(s)
    }

    pub fn not_found() -> Self {
        Self::new_code(404)
    }

    pub fn builder<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut HttpResponseBuilder) + 'static,
    {
        self.builder = Some(Box::new(f));
        self
    }

    pub fn message<S: Into<String>>(mut self, s: S) -> Self {
        self.message = s.into();
        self
    }

    pub fn data(mut self, data: T) -> Self {
        self.data = Some(data);
        self
    }

    pub fn file(name: String, data: Vec<u8>) -> HttpResponse {
        let body = once(future::ok::<_, actix_web::Error>(data.into()));
        let header = ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![DispositionParam::Filename(name)],
        };
        HttpResponse::Ok()
            .insert_header(("Content-Disposition", header))
            .content_type("application/octet-stream")
            .streaming(body)
    }

    #[cfg(feature = "i18n")]
    pub fn translate(mut self) -> Self {
        self.translate = true;
        self
    }

    #[cfg(feature = "i18n")]
    pub fn i18n_message(&self, req: &actix_web::HttpRequest) -> String {
        use actix_web::HttpMessage as _;

        if self.translate {
            req.app_data::<actix_web::web::Data<crate::state::GlobalState>>()
                .map_or_else(
                    || self.message.clone(),
                    |state| {
                        if let Some(ext) = req
                            .extensions()
                            .get::<std::sync::Arc<crate::request::Extension>>()
                        {
                            crate::t!(state.locale, &self.message, &ext.lang)
                        } else {
                            self.message.clone()
                        }
                    },
                )
        } else {
            self.message.clone()
        }
    }
}

#[cfg(feature = "response-json")]
pub type JsonResponse = Response<serde_json::Value>;

#[cfg(feature = "response-json")]
impl JsonResponse {
    pub fn json<T: serde::Serialize>(mut self, data: T) -> Self {
        self.data = Some(serde_json::json!(data));
        self
    }
}

#[cfg(feature = "response-json")]
impl actix_web::Responder for JsonResponse {
    type Body = actix_web::body::EitherBody<String>;

    fn respond_to(
        self,
        #[allow(unused_variables)] req: &actix_web::HttpRequest,
    ) -> HttpResponse<Self::Body> {
        if self.http_code == 200 {
            #[cfg(feature = "i18n")]
            let message = self.i18n_message(req);
            #[cfg(not(feature = "i18n"))]
            let message = self.message;
            let mut body = serde_json::json!({
                "code": self.code,
                "message": message,
            });
            if let Some(data) = self.data {
                body.as_object_mut()
                    .unwrap()
                    .insert(String::from("data"), data);
            }
            let body = body.to_string();
            let mut rsp =
                HttpResponse::build(actix_web::http::StatusCode::from_u16(self.http_code).unwrap());
            rsp.content_type(actix_web::http::header::ContentType::json());
            if let Some(builder) = self.builder {
                builder(&mut rsp);
            }
            rsp.message_body(body).unwrap().map_into_left_body()
        } else {
            HttpResponse::build(actix_web::http::StatusCode::from_u16(self.http_code).unwrap())
                .message_body(self.message)
                .unwrap()
                .map_into_left_body()
        }
    }
}
