use std::io;

use actix_cloud::{
    actix_web::{web::get, App, HttpServer, Responder},
    build_router,
    i18n::{i18n, Locale},
    logger::LoggerBuilder,
    map, request,
    response::{JsonResponse, RspResult},
    router::Router,
    state::{GlobalState, ServerHandle},
};
use qstring::QString;

include!(concat!(env!("OUT_DIR"), "/response.rs"));

async fn page() -> RspResult<impl Responder> {
    let data = map! {
        "key1" => "value1",
        "key2" => "value2"
    };
    Ok(JsonResponse::new(Srv1Response::ServiceOneSome).json(data))
}

async fn page_no_translate() -> RspResult<impl Responder> {
    Ok(JsonResponse::new_code(200).message("response.ok"))
}

fn init_router() -> Vec<Router> {
    vec![
        Router {
            path: String::from("/"),
            route: get().to(page),
            checker: None,
        },
        Router {
            path: String::from("/nolang"),
            route: get().to(page_no_translate),
            checker: None,
        },
    ]
}

// use `#[actix_cloud::main]` for main function.
#[actix_cloud::main]
async fn main() -> io::Result<()> {
    // Start logger.
    let (logger, _guard) = LoggerBuilder::new().start();

    // Init locale.
    let locale = Locale::new("en-US").add_locale(i18n!("locale"));

    // Init state.
    let state = GlobalState {
        logger: Some(logger),
        locale,
        server: ServerHandle::default(),
    }
    .build();

    // Configure server.
    let state_cloned = state.clone();
    let server = HttpServer::new(move || {
        App::new()
            .configure(build_router(init_router()))
            .wrap(request::Middleware::new().lang(|req| {
                QString::from(req.query_string())
                    .get("lang")
                    .map(ToOwned::to_owned)
            })) // add request middleware
            .app_data(state_cloned.clone())
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    // Block and wait for connection.
    state.server.start(server).await.unwrap();
    Ok(())
}
