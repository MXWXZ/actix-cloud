use std::{io, rc::Rc};

use actix_cloud::{
    actix_web::{
        dev::ServiceRequest,
        middleware,
        web::{get, scope},
        App, HttpServer,
    },
    async_trait, build_router,
    logger::LoggerBuilder,
    request,
    router::{Checker, Router},
    security::SecurityHeader,
    state::{GlobalState, ServerHandle},
    tracing_actix_web::TracingLogger,
    Result,
};
use qstring::QString;

async fn guest_page() -> &'static str {
    "This is guest page, you can visit /api/guest directly."
}

async fn admin_page() -> &'static str {
    "This is admin page, you need to use /api/admin?admin=1 to visit."
}

struct AuthChecker {
    need_admin: bool,
}

impl AuthChecker {
    fn new(need_admin: bool) -> Self {
        Self { need_admin }
    }
}

#[async_trait(?Send)]
impl Checker for AuthChecker {
    async fn check(&self, req: &mut ServiceRequest) -> Result<bool> {
        let qs = QString::from(req.query_string());
        let is_admin = if qs.get("admin").is_some_and(|x| x == "1") {
            true
        } else {
            false
        };
        if (is_admin && self.need_admin) || !self.need_admin {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn init_router() -> Vec<Router> {
    vec![
        Router {
            path: String::from("/guest"),
            route: get().to(guest_page),
            checker: Some(Rc::new(AuthChecker::new(false))),
        },
        Router {
            path: String::from("/admin"),
            route: get().to(admin_page),
            checker: Some(Rc::new(AuthChecker::new(true))),
        },
    ]
}

// use `#[actix_cloud::main]` for main function.
#[actix_cloud::main]
async fn main() -> io::Result<()> {
    // Start logger.
    let (logger, _guard) = LoggerBuilder::new().start();

    // Init state.
    let state = GlobalState {
        logger: Some(logger),
        server: ServerHandle::default(),
    }
    .build();

    // Configure server.
    let state_cloned = state.clone();
    let server = HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .wrap(request::Middleware::new())
            .wrap(TracingLogger::default())
            .wrap(middleware::Compress::default()) // compress page
            .wrap(SecurityHeader::default().build()) // default security header
            .service(scope("/api").configure(build_router(init_router())))
            .app_data(state_cloned.clone())
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    // Block and wait for connection.
    state.server.start(server).await.unwrap();
    Ok(())
}
