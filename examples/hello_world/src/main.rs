use std::io;

use actix_cloud::{
    actix_web::{
        dev::ServiceRequest,
        middleware,
        web::{get, scope},
        App, HttpServer,
    },
    build_router,
    logger::LoggerBuilder,
    memorydb::default::DefaultBackend,
    router::Router,
    security::SecurityHeader,
    state::{GlobalState, StopHandle},
};
use qstring::QString;

async fn guest_page() -> &'static str {
    "This is guest page, you can visit /api/guest directly."
}

async fn admin_page() -> &'static str {
    "This is admin page, you need to use /api/admin?admin=1 to visit."
}

// We assume guest permission has id 0.
fn is_guest(p: Vec<u32>) -> bool {
    p.into_iter().find(|x| *x == 0).is_some()
}

// We assume admin permission has id 1.
fn is_admin(p: Vec<u32>) -> bool {
    p.into_iter().find(|x| *x == 1).is_some()
}

fn perm_extractor(req: &mut ServiceRequest) -> Vec<u32> {
    let mut ret = Vec::new();
    ret.push(0); // guest permission is assigned by default.

    // test if query string has `admin=1`.
    let qs = QString::from(req.query_string());
    if qs.get("admin").is_some_and(|x| x == "1") {
        ret.push(1);
    }
    ret
}

fn init_router() -> Vec<Router<Vec<u32>>> {
    vec![
        Router {
            path: String::from("/guest"),
            route: get().to(guest_page),
            extractor: Box::new(perm_extractor),
            checker: Box::new(is_guest),
        },
        Router {
            path: String::from("/admin"),
            route: get().to(admin_page),
            extractor: Box::new(perm_extractor),
            checker: Box::new(is_admin),
        },
    ]
}

// use `#[actix_cloud::main]` for main function.
#[actix_cloud::main]
async fn main() -> io::Result<()> {
    // Start logger.
    let logger = LoggerBuilder::new().start();

    // Init state.
    let state = GlobalState {
        logger,
        memorydb: DefaultBackend::new().await.unwrap(),
        stop_handle: StopHandle::default(),
    }
    .build();

    // Configure server.
    let state_cloned = state.clone();
    let server = HttpServer::new(move || {
        App::new()
            .wrap(middleware::Compress::default()) // compress page
            .wrap(SecurityHeader::default().build()) // default security header
            .service(scope("/api").configure(build_router(init_router())))
            .app_data(state_cloned.clone())
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    // Block and wait for connection.
    state.stop_handle.start(server).await.unwrap();
    Ok(())
}
